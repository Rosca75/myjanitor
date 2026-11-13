//! `plan`, `dry-run`, `apply`, `clean`. Owner: W3-E.
//!
//! Safety rules (SAF-11): `apply` never runs without either `--yes` or a typed
//! confirmation on a terminal, refuses on a non-terminal without `--yes`, and
//! lossy or permanent steps need their own flags (`--confirm-lossy`,
//! `--confirm-permanent`) in addition to `--yes`.

use mj_core::{Action, Job, JobState, OpportunityId, OriginalsPolicy, SafetyClass, StepConfirmation};
use mj_engine::api::{DryRunReport, Event, PlanAddStepRequest, PlanApplyRequest};

use crate::args::Parsed;
use crate::cmd_scan::scope;
use crate::ctx::{confirm_typed, interactive, Ctx, Fail, Res, Session, Template};
use crate::out::{bytes, json, table};

/// Parse `--action` (and `--dest`) into an action.
pub fn parse_action(ctx: &Ctx, a: &Parsed) -> Res<Option<Action>> {
    let Some(name) = a.value("action") else { return Ok(None) };
    let dest = || -> Res<std::path::PathBuf> {
        let d = a.value("dest").ok_or_else(|| Fail::Usage(format!("--action {name} needs --dest <DIR>")))?;
        let p = std::path::PathBuf::from(d);
        if p.is_absolute() {
            Ok(p)
        } else {
            Err(Fail::Usage(format!("--dest must be an absolute path: {d}")))
        }
    };
    Ok(Some(match name {
        "trash" => Action::Trash,
        "quarantine" => Action::Quarantine { purge_after_days: ctx.engine.settings_get()?.general.quarantine_days },
        "permanent" | "permanent-delete" => Action::PermanentDelete,
        "hardlink" => Action::Hardlink,
        "reflink" => Action::Reflink,
        "dehydrate" => Action::Dehydrate,
        "move" => Action::Move { dest: dest()?, keep_structure: false },
        "copy" => Action::Copy { dest: dest()?, keep_structure: false },
        other => return Err(Fail::Usage(format!(
            "unknown action `{other}`; use trash, quarantine, permanent, hardlink, reflink, dehydrate, move or copy"
        ))),
    }))
}

/// `plan add|remove|show|clear`.
pub fn plan(a: &Parsed) -> Res {
    let ctx = Ctx::open(a)?;
    let mut session = ctx.session();
    match a.cmd.sub {
        Some("add") => {
            let id = OpportunityId(a.positionals[0].clone());
            if session.plan.iter().any(|t| t.opportunity == id) {
                return Err(Fail::Error(format!("{} is already in the plan", id.0)));
            }
            let session_scanned = ctx.ensure_scanned()?;
            ctx.replay_plan(&session_scanned.plan, None)?;
            let action = parse_action(&ctx, a)?;
            let plan = ctx.engine.plan_add_step(PlanAddStepRequest {
                opportunity_id: id.clone(),
                action: action.clone(),
                params: None,
            })?;
            session.plan.push(Template { opportunity: id, action, params: None });
            ctx.save_session(&session)?;
            println!(
                "Added. The plan now has {} step(s), about {} saved.",
                plan.steps.len(),
                bytes(plan.estimate.saving())
            );
        }
        Some("remove") => {
            let id = &a.positionals[0];
            let before = session.plan.len();
            session.plan.retain(|t| &t.opportunity.0 != id);
            if session.plan.len() == before {
                return Err(Fail::Error(format!("{id} is not in the plan; see `plan show`")));
            }
            ctx.save_session(&session)?;
            println!("Removed {id}.");
        }
        Some("clear") => {
            session.plan.clear();
            ctx.save_session(&session)?;
            println!("The plan is empty.");
        }
        _ => {
            let s = if session.plan.is_empty() { session.clone() } else { ctx.ensure_scanned()? };
            ctx.replay_plan(&s.plan, None)?;
            print_plan(&ctx)?;
        }
    }
    Ok(())
}

fn print_plan(ctx: &Ctx) -> Res {
    let plan = ctx.engine.plan_get()?;
    if ctx.json {
        println!("{}", json(&plan));
        return Ok(());
    }
    if plan.steps.is_empty() {
        println!("The plan is empty. Add steps with `plan add <OPPORTUNITY-ID>` (see `list`).");
        return Ok(());
    }
    let mut rows = vec![["ID", "STEP", "ACTION", "SAVING", "SAFETY"].map(String::from).to_vec()];
    for s in &plan.steps {
        rows.push(vec![
            s.opportunity.0.clone(),
            s.title.clone(),
            s.summary.clone(),
            bytes(s.estimate.saving()),
            s.safety.label().to_string(),
        ]);
    }
    print!("{}", table(&rows, &[3]));
    println!("Total: about {} saved from {} files.", bytes(plan.estimate.saving()), plan.estimate.files);
    for c in &plan.checklist {
        println!("  - {}", c.text);
    }
    Ok(())
}

/// Run the dry run of the engine's current plan and print it.
pub fn dry_run_report(ctx: &Ctx) -> Res<DryRunReport> {
    let job = ctx.engine.plan_dry_run()?;
    let jobs = ctx.wait_jobs(&[job.id])?;
    if let Some(j) = jobs.first().filter(|j| j.state == JobState::Failed) {
        return Err(Fail::Error(
            j.error.as_ref().map(|e| e.message.clone()).unwrap_or_else(|| "the dry run failed".into()),
        ));
    }
    let report =
        mj_core::lock(&ctx.sink.0)
            .iter()
            .rev()
            .find_map(|e| if let Event::DryRun(r) = e { Some(r.clone()) } else { None });
    report.ok_or_else(|| Fail::Error("the dry run finished without a report".into()))
}

fn print_dry_run(ctx: &Ctx, r: &DryRunReport) {
    if ctx.json {
        println!("{}", json(r));
        return;
    }
    let mut rows = vec![["STEP", "FILES", "NOW", "AFTER", "SAVING", "SKIPPED"].map(String::from).to_vec()];
    for s in &r.steps {
        rows.push(vec![
            s.title.clone(),
            s.estimate.files.to_string(),
            bytes(s.estimate.bytes_before),
            bytes(s.estimate.bytes_after),
            bytes(s.estimate.saving()),
            s.skipped_count.to_string(),
        ]);
        for w in &s.warnings {
            rows.push(vec![format!("  ! {w}")]);
        }
    }
    print!("{}", table(&rows, &[1, 2, 3, 4, 5]));
    println!("Total: {} files, {} saved. Nothing was changed.", r.total.files, bytes(r.total.saving()));
}

/// `dry-run`.
pub fn dry_run(a: &Parsed) -> Res {
    let ctx = Ctx::open(a)?;
    let s = ctx.ensure_scanned()?;
    ctx.replay_plan(&s.plan, None)?;
    if ctx.engine.plan_get()?.steps.is_empty() {
        return Err(Fail::Error("the plan is empty: add steps with `plan add <OPPORTUNITY-ID>`".into()));
    }
    let r = dry_run_report(&ctx)?;
    print_dry_run(&ctx, &r);
    Ok(())
}

fn originals(a: &Parsed) -> Res<Option<OriginalsPolicy>> {
    Ok(match a.value("originals") {
        None => None,
        Some("trash") => Some(OriginalsPolicy::RecycleBin),
        Some("quarantine") => Some(OriginalsPolicy::Quarantine),
        Some(o) => {
            return Err(Fail::Usage(format!(
                "--originals must be trash or quarantine, got `{o}` (deleting originals is only offered in the app)"
            )))
        }
    })
}

/// Refuse to continue without consent: `--yes`, or a terminal to type on.
fn require_consent(a: &Parsed) -> Res {
    if !a.flag("yes") && !interactive() {
        return Err(Fail::Usage("refusing to apply without --yes: this is not a terminal, so the typed confirmation is impossible. Review with `dry-run` first, then add --yes.".into()));
    }
    Ok(())
}

/// Confirm, apply the engine's current plan and report. The plan must be loaded.
fn apply_loaded(ctx: &Ctx, a: &Parsed, policy: Option<OriginalsPolicy>) -> Res {
    let plan = ctx.engine.plan_get()?;
    if plan.steps.is_empty() {
        return Err(Fail::Error("the plan is empty: add steps with `plan add <OPPORTUNITY-ID>`".into()));
    }
    let lossy: Vec<_> = plan.steps.iter().filter(|s| s.safety == SafetyClass::LossyOptIn).collect();
    let perm: Vec<_> = plan.steps.iter().filter(|s| s.safety == SafetyClass::Permanent).collect();
    if !lossy.is_empty() && !a.flag("confirm-lossy") {
        return Err(Fail::Usage(format!("the plan has lossy step(s) ({}): they re-encode files. Add --confirm-lossy to run them, or remove them with `plan remove`.", lossy.iter().map(|s| s.title.as_str()).collect::<Vec<_>>().join(", "))));
    }
    if !perm.is_empty() && !a.flag("confirm-permanent") {
        return Err(Fail::Usage(format!("the plan has permanent step(s) ({}): they cannot be undone. Add --confirm-permanent to run them, or remove them with `plan remove`.", perm.iter().map(|s| s.title.as_str()).collect::<Vec<_>>().join(", "))));
    }
    let report = dry_run_report(ctx)?;
    print_dry_run(ctx, &report);
    if !a.flag("yes")
        && !confirm_typed(&format!("About to change files ({} saved).", bytes(report.total.saving())), "APPLY")
    {
        return Err(Fail::Error("cancelled: nothing was changed".into()));
    }
    let default_policy = match ctx.engine.settings_get()?.general.default_originals {
        OriginalsPolicy::Delete => OriginalsPolicy::RecycleBin,
        p => p,
    };
    let confirmations = plan
        .steps
        .iter()
        .filter(|s| s.safety.requires_confirmation())
        .map(|s| StepConfirmation {
            step_id: s.id,
            acknowledged: true,
            originals: policy.clone().unwrap_or_else(|| default_policy.clone()),
            synced_online_only: false,
        })
        .collect();
    let started = ctx.engine.plan_apply(PlanApplyRequest { confirmations })?;
    let jobs = ctx.wait_apply(&started)?;
    finish(ctx, started.run_id.0, &jobs)
}

/// Print what an apply did and map failures to the exit code.
pub fn finish(ctx: &Ctx, run_id: u64, jobs: &[Job]) -> Res {
    if ctx.json {
        println!("{}", json(&jobs));
    }
    let (mut ok, mut skipped, mut failed, mut saved) = (0, 0, 0, 0);
    for j in jobs {
        if let Some(o) = &j.outcome {
            ok += o.succeeded;
            skipped += o.skipped;
            failed += o.failed;
            saved += o.bytes_saved;
            if !ctx.json {
                for e in o.errors.iter().take(10) {
                    eprintln!(
                        "  failed: {}: {}",
                        mj_core::pathutil::display_lossy(&e.path),
                        e.error.as_ref().map(|x| x.message.as_str()).unwrap_or("unknown error")
                    );
                }
            }
        }
    }
    if !ctx.json {
        println!("Run {run_id}: {ok} done, {skipped} skipped, {failed} failed, {} saved. Undo with `myjanitor-cli undo --run {run_id}`.", bytes(saved));
    }
    if jobs.iter().any(|j| j.state == JobState::Failed) && ok == 0 {
        return Err(Fail::Error("the run failed; see `history` and `report --last`".into()));
    }
    if failed > 0 || jobs.iter().any(|j| j.state == JobState::CompletedWithErrors) {
        return Err(Fail::Partial(format!("{failed} item(s) failed; the others were done. See `report --last`.")));
    }
    Ok(())
}

/// `apply`.
pub fn apply(a: &Parsed) -> Res {
    let policy = originals(a)?;
    require_consent(a)?;
    let ctx = Ctx::open(a)?;
    let s = ctx.ensure_scanned()?;
    ctx.replay_plan(&s.plan, policy.as_ref())?;
    apply_loaded(&ctx, a, policy)
}

/// `clean <folders> --detectors a,b [--dry-run | --apply]`.
pub fn clean(a: &Parsed) -> Res {
    let wanted = a
        .value("detectors")
        .ok_or_else(|| Fail::Usage("`clean` needs --detectors <a,b>; see `list` after a scan for the ids".into()))?;
    if a.flag("apply") && a.flag("dry-run") {
        return Err(Fail::Usage("use either --dry-run or --apply, not both".into()));
    }
    let policy = originals(a)?;
    if a.flag("apply") {
        require_consent(a)?;
    }
    let ctx = Ctx::open(a)?;
    let status_scope = scope(&ctx, a, &a.positionals)?;
    ctx.run_scan(status_scope)?;
    ctx.engine.plan_clear()?;
    let mut templates = Vec::new();
    for w in wanted.split(',').map(str::trim).filter(|w| !w.is_empty()) {
        let id = if w.contains(':') { w.to_string() } else { format!("det:{w}") };
        ctx.engine
            .plan_add_step(PlanAddStepRequest { opportunity_id: OpportunityId(id.clone()), action: None, params: None })
            .map_err(|e| Fail::Error(format!("{id}: {e}")))?;
        templates.push(id);
    }
    if let Some(p) = &policy {
        let s = Session {
            scope: None,
            plan: templates
                .iter()
                .map(|id| Template { opportunity: OpportunityId(id.clone()), action: None, params: None })
                .collect(),
        };
        ctx.replay_plan(&s.plan, Some(p))?;
    }
    if a.flag("apply") {
        apply_loaded(&ctx, a, policy)
    } else {
        let r = dry_run_report(&ctx)?;
        print_dry_run(&ctx, &r);
        Ok(())
    }
}
