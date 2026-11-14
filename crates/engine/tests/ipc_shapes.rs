//! Real engine responses against `docs/IPC.md` (W4-INT, item 6).
//!
//! A small fixture session (scan, a plan with a detector and an optimiser step, a dry run,
//! an apply) answers every read command the UI calls at start-up and in its views. Each
//! response is serialised and its field names are compared, recursively, with the
//! ```` ```json <Type> ```` example of the same type in IPC.md: a field the engine sends
//! that IPC.md does not show is a contract drift, and so is a documented field the engine
//! never sends (unless it is listed as optional below, i.e. `skip_serializing_if`).
//!
//! With `MJ_WRITE_UI_FIXTURES=1` the responses are also written to
//! `ui/js/testing/real/<command>.json`, which `ui/js/real.test.js` renders through the
//! real views. Regenerate with:
//!
//! ```text
//! MJ_WRITE_UI_FIXTURES=1 cargo test -p mj-engine --test ipc_shapes
//! ```

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::*;
use mj_core::testutil::TempDir;
use mj_core::{JobState, OpportunityId};
use mj_engine::api::*;
use serde_json::Value;

/// `ty.path` fields that may be absent in a real response (Option + skip_serializing_if,
/// or data-dependent maps such as candidate columns).
const OPTIONAL: &[&str] = &[
    "AppInfo.launch",
    "Opportunity.availability.reason",
    "Candidate.group",
    "Candidate.sub_group",
    "Candidate.columns",
    "CandidateRow.group",
    "CandidateRow.sub_group",
    "CandidateRow.blocked_reason",
    "CandidateRow.columns",
    "CandidatesPage.rows[].group",
    "CandidatesPage.rows[].sub_group",
    "CandidatesPage.rows[].blocked_reason",
    "CandidatesPage.rows[].columns",
    "Job.outcome",
    "Job.error",
    "Job.started",
    "Job.finished",
    "Job.run_id",
    "Job.step_id",
    "ScanStatus.error",
    "ScanStatus.progress.current",
    "ScanStatus.progress.phase",
    "ScanStatus.stats",
    "ScanStatus.started",
    "ScanStatus.finished",
    "JournalEntry.retained_at",
    "HistoryRunDetail.entries[].retained_at",
    "HistoryRunDetail.entries[].step_id",
    "HistoryRunDetail.entries[].error",
    "HistoryRunDetail.entries[].trash",
    "HistoryRunDetail.entries[].undo_deadline",
    "HistoryRunDetail.run.finished",
    "HistoryRunDetail.run.undo_until",
    "HistoryList.runs[].finished",
    "HistoryList.runs[].undo_until",
    "Report.sections[].items[].error",
    "Drive.label",
    "DiskNode.children",
    "DiskNode.children[].children",
    "Preview.text",
    "Preview.data",
    "Preview.waveform",
    "Preview.mime",
    "SearchParseResponse.errors",
    "SearchParseResponse.rule",
    "ExternalTool.path",
    "ExternalTool.version",
    "Schedule.next_run",
    "Schedule.last_run",
    "StepDetail.candidates",
    "Settings.general.quarantine_dir",
    "Settings.scan.max_depth",
    "ConfirmationInfo.synced",
    "Job.outcome",
    "Preview.reason",
    "StepDetail.step_id",
    "StepDetail.opportunity.sub_groups",
    "HistoryRunDetail.entries[].lossy",
    "HistoryRunDetail.entries[].original_mode",
    "HistoryRunDetail.entries[].original_sha256",
    "DiskNode.other",
    "DiskNode.children[].other",
];

/// Every ```json <Type> example of IPC.md.
fn examples() -> BTreeMap<String, Value> {
    let doc = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/IPC.md")).unwrap();
    let mut out = BTreeMap::new();
    let mut lines = doc.lines();
    while let Some(l) = lines.next() {
        let Some(ty) = l.strip_prefix("```json ") else { continue };
        let mut body = String::new();
        for l in lines.by_ref() {
            if l.starts_with("```") {
                break;
            }
            body.push_str(l);
        }
        out.entry(ty.trim().to_string()).or_insert_with(|| serde_json::from_str(&body).unwrap());
    }
    out
}

/// Field paths of a JSON value: objects by key, arrays through their first element.
/// Maps keyed by data (columns, params, counts) stop at the map itself.
fn paths(v: &Value, at: &str, out: &mut BTreeSet<String>) {
    const DATA_MAPS: &[&str] = &[
        "columns",
        "params",
        "values",
        "skipped",
        "type_groups",
        "tool_paths",
        "counts_by",
        "by_code",
        "option",
        "include",
        "exclude",
        "default_action",
        "action",
        "rule",
        "cadence",
        "default",
        "kind",
        "fields",
        "cond",
    ];
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                let p = if at.is_empty() { k.clone() } else { format!("{at}.{k}") };
                out.insert(p.clone());
                if !DATA_MAPS.contains(&k.as_str()) {
                    paths(x, &p, out);
                }
            }
        }
        Value::Array(a) => {
            if let Some(first) = a.first() {
                paths(first, &format!("{at}[]"), out);
            }
        }
        _ => {}
    }
}

fn compare(ty: &str, real: &Value, ex: &Value, problems: &mut Vec<String>) {
    let (mut r, mut e) = (BTreeSet::new(), BTreeSet::new());
    paths(real, "", &mut r);
    paths(ex, "", &mut e);
    // An optional field covers everything below it.
    let optional = |p: &str| {
        let full = format!("{ty}.{p}");
        OPTIONAL.iter().any(|o| full == *o || full.starts_with(&format!("{o}.")) || full.starts_with(&format!("{o}[]")))
    };
    // Only compare below arrays that both sides populate.
    let covered = |p: &str, other: &BTreeSet<String>| match p.rfind("[]") {
        Some(i) => other.iter().any(|o| o.starts_with(&p[..i + 2])),
        None => true,
    };
    for p in r.difference(&e) {
        if covered(p, &e) && !optional(p) {
            problems.push(format!("{ty}: the engine sends `{p}`, IPC.md does not show it"));
        }
    }
    for p in e.difference(&r) {
        if covered(p, &r) && !optional(p) {
            problems.push(format!("{ty}: IPC.md shows `{p}`, the engine does not send it"));
        }
    }
}

fn session() -> (Fixture, Vec<(&'static str, &'static str, Value)>) {
    let tree = standard_tree();
    tree.write("img/a.png", &common::fixtures::png()).unwrap();
    let f = engine_for(tree);
    let e = &f.engine;
    assert_eq!(scan(e).state, ScanRunState::Done);
    let png = OpportunityId("opt:png_lossless".into());
    e.plan_add_step(PlanAddStepRequest {
        opportunity_id: OpportunityId("det:temp_files".into()),
        action: None,
        params: None,
    })
    .unwrap();
    e.plan_add_step(PlanAddStepRequest { opportunity_id: png.clone(), action: None, params: None }).unwrap();
    let dry = e.plan_dry_run().unwrap();
    wait("dry run", || {
        f.events.events().iter().any(|ev| matches!(ev, Event::DryRun(r) if r.job_id == dry.id)).then_some(())
    });
    let step = e.plan_get().unwrap().steps[0].id;
    let confirmation = e.plan_confirmation_info(StepRequest { step_id: step }).unwrap();
    let plan = e.plan_get().unwrap();
    let started = e.plan_apply(PlanApplyRequest::default()).unwrap();
    for j in wait_jobs(e) {
        assert_eq!(j.state, JobState::Completed, "{j:?}");
    }
    // A second plan so plan_get is not empty in the fixtures.
    e.plan_add_step(PlanAddStepRequest {
        opportunity_id: OpportunityId("det:exact_duplicates".into()),
        action: None,
        params: None,
    })
    .unwrap();
    let dup = OpportunityId("det:exact_duplicates".into());
    let first_candidate = e
        .candidates_page(CandidatesPageRequest {
            opportunity_id: dup.clone(),
            sub_group: None,
            offset: 0,
            limit: 50,
            sort: CandidateSortKey::Size,
            descending: true,
            filter: None,
        })
        .unwrap();
    let cid = first_candidate.rows[0].candidate.id;
    macro_rules! v {
        ($e:expr) => {
            serde_json::to_value($e).unwrap()
        };
    }
    let tree_root = f.tree.path().to_path_buf();
    let responses = vec![
        ("app_info", "AppInfo", v!(e.app_info().unwrap())),
        ("scope_get", "ScanScope", v!(e.scope_get().unwrap())),
        ("scan_status", "ScanStatus", v!(e.scan_status().unwrap())),
        ("opportunities_list", "OpportunitiesList", v!(e.opportunities_list(OpportunitiesRequest::default()).unwrap())),
        ("plan_get", "Plan", v!(e.plan_get().unwrap())),
        ("plan_get_applied", "Plan", v!(plan)),
        ("jobs_list", "Job", v!(e.jobs_list().unwrap()[0].clone())),
        ("jobs_list_all", "", v!(e.jobs_list().unwrap())),
        ("report_last", "Report", v!(e.report_last().unwrap().unwrap())),
        ("drives_list", "", v!(e.drives_list().unwrap())),
        ("tools_list", "", v!(e.tools_list().unwrap())),
        ("step_detail", "StepDetail", v!(e.step_detail(OpportunityRequest { opportunity_id: dup.clone() }).unwrap())),
        ("candidates_page", "CandidatesPage", v!(first_candidate)),
        (
            "candidate_preview",
            "Preview",
            v!(e.candidate_preview(CandidatePreviewRequest { candidate_id: cid }).unwrap()),
        ),
        ("plan_confirmation_info", "ConfirmationInfo", v!(confirmation)),
        ("history_list", "HistoryList", v!(e.history_list(PageRequest::default()).unwrap())),
        (
            "history_run_detail",
            "HistoryRunDetail",
            v!(e.history_run_detail(RunDetailRequest { run_id: started.run_id, offset: 0, limit: 100 }).unwrap()),
        ),
        ("report_get", "Report", v!(e.report_get(RunRequest { run_id: started.run_id }).unwrap())),
        ("quarantine_info", "QuarantineInfo", v!(e.quarantine_info().unwrap())),
        ("settings_get", "Settings", v!(e.settings_get().unwrap())),
        ("protected_paths_get", "ProtectedPathsInfo", v!(e.protected_paths_get().unwrap())),
        ("cache_info", "CacheInfo", v!(e.cache_info().unwrap())),
        ("schedules_list", "", v!(e.schedules_list().unwrap())),
        ("presets_list", "", v!(e.presets_list().unwrap())),
        (
            "search_parse",
            "SearchParseResponse",
            v!(e.search_parse(SearchParseRequest { query: "ext:bin size:>1KB".into() }).unwrap()),
        ),
        (
            "disk_usage_tree",
            "DiskNode",
            v!(e.disk_usage_tree(DiskUsageRequest { path: None, depth: 2, min_fraction: Some(0.0) }).unwrap()),
        ),
        (
            "breakdown",
            "Breakdown",
            v!(e.breakdown(BreakdownRequest { by: BreakdownBy::Extension, path: None }).unwrap()),
        ),
        (
            "fs_list_dir",
            "FsListDirResponse",
            v!(e.fs_list_dir(FsListDirRequest { path: Some(tree_root), show_hidden: false, include_files: true })
                .unwrap()),
        ),
    ];
    (f, responses)
}

#[test]
fn real_responses_match_the_ipc_examples() {
    let (f, responses) = session();
    let ex = examples();
    let mut problems = Vec::new();
    let mut checked = 0;
    for (cmd, ty, real) in &responses {
        let (ty, real) = match (*ty, real) {
            ("", Value::Array(a)) => {
                // Lists: compare their first element with the element type's example.
                let elem = match *cmd {
                    "drives_list" => "Drive",
                    "tools_list" => "ExternalTool",
                    "schedules_list" => "Schedule",
                    "presets_list" => "RulePreset",
                    _ => continue,
                };
                match a.first() {
                    Some(v) => (elem, v.clone()),
                    None => continue,
                }
            }
            (t, v) => (t, v.clone()),
        };
        let Some(example) = ex.get(ty) else {
            problems.push(format!("{cmd}: IPC.md has no example for {ty}"));
            continue;
        };
        compare(ty, &real, example, &mut problems);
        checked += 1;
    }
    assert!(checked >= 25, "{checked}");
    if std::env::var_os("MJ_WRITE_UI_FIXTURES").is_some() {
        let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../ui/js/testing/real"));
        std::fs::create_dir_all(dir).unwrap();
        // Temp paths are replaced by a stable placeholder root so the fixtures do not churn.
        let root = f.tree.path().to_string_lossy().replace('\\', "\\\\");
        let app = f.app.path().to_string_lossy().replace('\\', "\\\\");
        for (cmd, _, v) in &responses {
            let text =
                serde_json::to_string_pretty(v).unwrap().replace(&root, "/fixture/tree").replace(&app, "/fixture/app");
            std::fs::write(dir.join(format!("{cmd}.json")), text + "\n").unwrap();
        }
    }
    let _ = TempDir::new("unused");
    assert!(problems.is_empty(), "IPC.md and the engine disagree:\n{}", problems.join("\n"));
}
