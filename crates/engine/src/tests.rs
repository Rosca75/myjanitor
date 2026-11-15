//! Engine tests: smoke tests over a temp fixture tree, and the IPC.md contract checks.

use std::sync::Arc;
use std::time::{Duration, Instant};

use mj_core::testutil::TempDir;

use crate::api::*;
use crate::{Engine, EngineConfig};

fn engine(events: Arc<CollectSink>) -> Arc<Engine> {
    let t = std::env::temp_dir();
    let mut cfg = EngineConfig::new(t.join("unused-data"), t.join("unused-cache"));
    cfg.persist = false;
    cfg.events = events;
    Engine::new(cfg).unwrap()
}

#[test]
fn start_up_commands_work_without_a_scan() {
    let e = engine(Arc::new(CollectSink::default()));
    let info = e.app_info().unwrap();
    assert_eq!(info.name, "MyJanitor");
    assert!(!info.journal_persistent);
    assert_eq!(e.scan_status().unwrap().state, ScanRunState::Idle);
    let list = e.opportunities_list(OpportunitiesRequest::default()).unwrap();
    assert!(list.items.is_empty());
    assert!(e.plan_get().unwrap().steps.is_empty());
    assert!(e.jobs_list().unwrap().is_empty());
    assert_eq!(e.history_list(PageRequest::default()).unwrap().total, 0);
    assert!(e.report_last().unwrap().is_none());
    assert!(!e.presets_list().unwrap().is_empty());
    assert_eq!(e.tools_list().unwrap().len(), mj_core::tools::KNOWN_TOOLS.len());
    let s = e.settings_get().unwrap();
    assert_eq!(e.settings_set(SettingsSetRequest { settings: s.clone() }).unwrap(), s);
    assert!(e.protected_paths_get().unwrap().builtin.len() > 3);
}

#[test]
fn scan_runs_end_to_end_on_a_fixture_tree() {
    let t = TempDir::new("engine-scan").unwrap();
    t.write("docs/a.txt", b"hello").unwrap();
    t.write("docs/b/c.bin", &[0u8; 100]).unwrap();
    let events = Arc::new(CollectSink::default());
    let e = engine(events.clone());
    e.scope_add_root(ScopePathRequest { path: t.path().to_path_buf() }).unwrap();
    let started = e.scan_start(ScanStartRequest::default()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let s = e.scan_status().unwrap();
        if s.state == ScanRunState::Done {
            assert_eq!(s.generation, started.generation);
            assert_eq!(s.stats.as_ref().unwrap().files, 2);
            break;
        }
        assert!(!matches!(s.state, ScanRunState::Failed | ScanRunState::Cancelled), "{s:?}");
        assert!(Instant::now() < deadline, "scan did not finish");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(events.names().contains(&"scan://done"));
    let tree = e.disk_usage_tree(DiskUsageRequest { path: None, depth: 3, min_fraction: Some(0.0) }).unwrap();
    assert_eq!(tree.size, 105);
}

#[test]
fn every_command_is_unique_and_documented() {
    let commands: Vec<String> = COMMANDS.iter().map(|c| c.name.to_string()).collect();
    let shell: Vec<String> = SHELL_COMMANDS.iter().map(|c| c.name.to_string()).collect();
    let events: Vec<String> = EVENTS.iter().map(|e| e.name.to_string()).collect();
    let mut names: Vec<&String> = commands.iter().chain(&shell).collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), COMMANDS.len() + SHELL_COMMANDS.len(), "a command name is used twice");
    let doc = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/IPC.md")).expect("docs/IPC.md");
    let documented: Vec<String> = doc
        .lines()
        .filter_map(|l| l.strip_prefix("### `"))
        .filter_map(|l| l.split('`').next())
        .map(String::from)
        .collect();
    let mut expected = commands;
    expected.extend(shell);
    expected.extend(events);
    assert_eq!(
        documented, expected,
        "docs/IPC.md headings must list every engine command, then every shell command, then every event, in registry order"
    );
}

/// The types a shell command names exist, and IPC.md documents each command with them.
#[test]
fn shell_commands_name_documented_types() {
    let doc = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/IPC.md")).expect("docs/IPC.md");
    for c in SHELL_COMMANDS {
        let heading = format!("### `{}`", c.name);
        let section = doc.split(&heading).nth(1).unwrap_or_else(|| panic!("IPC.md has no section for {}", c.name));
        let section = section.split("\n### ").next().unwrap();
        let signature = format!("`{}` \u{2192} `{}`", c.request, c.response);
        assert!(section.contains(&signature), "{}: IPC.md must state {signature}", c.name);
        assert!(
            section.contains(&format!("```json {}\n", c.request)),
            "{}: IPC.md must show a {} example (`ipc_examples_parse` then checks that it parses)",
            c.name,
            c.request
        );
        // `check` panics on a type name it does not know, so a typo in the table fails
        // here. Every shell command answers with a path or `null`.
        check(c.response, "null");
        check(c.response, "\"C:\\\\Users\\\\Michel\\\\Pictures\"");
    }
}

/// The dialog response is the bare path or `null`, with the lossless path codec.
#[test]
fn dialog_path_is_a_bare_path_or_null() {
    use std::path::PathBuf;
    assert_eq!(serde_json::to_string(&DialogPath(None)).unwrap(), "null");
    let p = DialogPath(Some(PathBuf::from("/home/me/Pictures")));
    assert_eq!(serde_json::to_string(&p).unwrap(), "\"/home/me/Pictures\"");
    assert_eq!(serde_json::from_str::<DialogPath>("\"/home/me/Pictures\"").unwrap(), p);
    assert_eq!(serde_json::from_str::<DialogPath>("null").unwrap(), DialogPath(None));
    // A request with an unknown field is rejected like every other request.
    assert!(serde_json::from_str::<DialogPickFolderRequest>("{\"path\":\"/x\"}").is_err());
    assert!(serde_json::from_str::<DialogSaveFileRequest>("{\"dest\":\"/x\"}").is_err());
    let req: DialogSaveFileRequest = serde_json::from_str(
        "{\"file_name\":\"report.csv\",\"filters\":[{\"name\":\"CSV file\",\"extensions\":[\"csv\"]}]}",
    )
    .unwrap();
    assert_eq!(req.filters[0].extensions, ["csv"]);
    assert_eq!(req.start_dir, None);
}

/// Every ```json <Type> block in IPC.md must deserialize into that type.
#[test]
fn ipc_examples_parse() {
    let doc = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/IPC.md")).expect("docs/IPC.md");
    let mut lines = doc.lines();
    let mut checked = 0;
    while let Some(l) = lines.next() {
        let Some(ty) = l.strip_prefix("```json ") else { continue };
        let mut body = String::new();
        for l in lines.by_ref() {
            if l.starts_with("```") {
                break;
            }
            body.push_str(l);
            body.push('\n');
        }
        check(ty.trim(), &body);
        checked += 1;
    }
    assert!(checked >= 40, "only {checked} examples found");
}

fn check(ty: &str, body: &str) {
    macro_rules! parse {
        ($($t:ty),* $(,)?) => {{
            $( if ty == stringify!($t) { if let Err(e) = serde_json::from_str::<$t>(body) { panic!("IPC.md example for {ty} does not parse: {e}\n{body}"); } return; } )*
        }};
    }
    use mj_core::*;
    parse!(
        AppInfo,
        ScanScope,
        ScopeSetRequest,
        ScopePathRequest,
        DialogPickFolderRequest,
        DialogSaveFileRequest,
        DialogPath,
        ScanStartRequest,
        ScanStarted,
        ScanStatus,
        ScanProgressEvent,
        DetectorTotalsEvent,
        OpportunitiesRequest,
        OpportunitiesList,
        Opportunity,
        OpportunityRequest,
        StepDetail,
        StepSummary,
        CandidatesPageRequest,
        CandidatesPage,
        CandidateRow,
        SelectionSetRequest,
        SelectionKeepRuleRequest,
        StepParamsRequest,
        StepParamsResponse,
        CandidatePreviewRequest,
        Preview,
        SearchParseRequest,
        SearchParseResponse,
        SearchSuggestRequest,
        Suggestion,
        SearchRunRequest,
        RulePreset,
        PresetSaveRequest,
        IdRequest,
        Plan,
        PlanStep,
        PlanAddStepRequest,
        PlanUpdateStepRequest,
        StepRequest,
        PlanReorderRequest,
        Job,
        DryRunReport,
        ConfirmationInfo,
        PlanApplyRequest,
        StepConfirmation,
        ApplyStarted,
        JobRequest,
        JobsReorderRequest,
        JobProgressEvent,
        JobDoneEvent,
        PageRequest,
        HistoryList,
        RunDetailRequest,
        HistoryRunDetail,
        JournalEntry,
        RunRequest,
        RestoreEntriesRequest,
        QuarantineInfo,
        QuarantinePurgeRequest,
        Report,
        ReportExportRequest,
        ResultsExportRequest,
        ExportResult,
        Settings,
        SettingsSetRequest,
        ProtectedPathsInfo,
        ProtectedPathsSetRequest,
        CacheInfo,
        Schedule,
        ScheduleSaveRequest,
        ScheduleFromPlanRequest,
        ExternalTool,
        Drive,
        DiskUsageRequest,
        DiskNode,
        BreakdownRequest,
        Breakdown,
        OpportunitiesChanged,
        HistoryChanged,
        ErrorInfo,
        Rule,
        Action,
        Candidate,
    );
    panic!("IPC.md example names unknown type `{ty}`");
}
