//! `scan`, `list`, `show`, `search`. Owner: W3-E.

use std::path::PathBuf;

use mj_core::pathutil::display_lossy;
use mj_core::{Availability, OpportunityId, ScanScope};
use mj_engine::api::{
    CandidatesPageRequest, OpportunitiesRequest, OpportunityTab, SearchParseRequest, SearchRunRequest,
};

use crate::args::Parsed;
use crate::ctx::{Ctx, Fail, Res, Session};
use crate::out::{bytes, json, table};
use crate::safety::check_root;

/// Roots from the command line: existing folders, canonicalised, safe to act on.
pub fn roots(ctx: &Ctx, paths: &[String]) -> Res<Vec<PathBuf>> {
    let canonical = |p: &std::path::Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let home = mj_engine::paths::home_dir().map(|h| canonical(&h));
    // The app's own folders, as given and with links resolved.
    let own: Vec<PathBuf> =
        [&ctx.dirs.data, &ctx.dirs.cache].iter().flat_map(|d| [d.to_path_buf(), canonical(d)]).collect();
    let own_refs: Vec<&std::path::Path> = own.iter().map(PathBuf::as_path).collect();
    let mut out = Vec::new();
    for p in paths {
        let root = std::fs::canonicalize(p).map_err(|e| Fail::Error(format!("{p}: {e}; give an existing folder")))?;
        if !root.is_dir() {
            return Err(Fail::Error(format!("{p} is not a folder")));
        }
        check_root(&root, home.as_deref(), &own_refs).map_err(Fail::Error)?;
        out.push(root);
    }
    Ok(out)
}

/// The scope for a scan: the user's scan defaults from settings plus the flags.
pub fn scope(ctx: &Ctx, a: &Parsed, paths: &[String]) -> Res<ScanScope> {
    let settings = ctx.engine.settings_get()?;
    let mut options = settings.scan.clone();
    if a.flag("no-cache") {
        options.use_cache = false;
    }
    Ok(ScanScope { roots: roots(ctx, paths)?, include: vec![], exclude: a.values("exclude").to_vec(), options })
}

/// `scan <folders>`.
pub fn scan(a: &Parsed) -> Res {
    let ctx = Ctx::open(a)?;
    let scope = scope(&ctx, a, &a.positionals)?;
    let status = ctx.run_scan(scope.clone())?;
    let mut session = ctx.session();
    session = Session { scope: Some(scope), plan: if session.scope.is_some() { vec![] } else { session.plan } };
    ctx.save_session(&session)?;
    let list = ctx.engine.opportunities_list(OpportunitiesRequest::default())?;
    if ctx.json {
        println!("{}", json(&list));
        return Ok(());
    }
    let stats = status.stats.unwrap_or_default();
    println!("Scanned {} files in {} folders, {}", stats.files, stats.dirs, bytes(stats.bytes));
    if stats.unreadable > 0 {
        println!("Could not read {} entries (they are not part of the results)", stats.unreadable);
    }
    print_opportunities(&list);
    Ok(())
}

fn print_opportunities(list: &mj_engine::api::OpportunitiesList) {
    println!(
        "Reclaimable: {} in {} opportunities ({} reversible)",
        bytes(list.summary.total),
        list.items.len(),
        bytes(list.summary.reversible)
    );
    if list.items.is_empty() {
        return;
    }
    let mut rows = vec![["ID", "WHAT", "SAVING", "FILES", "SAFETY"].map(String::from).to_vec()];
    for o in &list.items {
        let what = match &o.availability {
            Availability::Available => o.title.clone(),
            Availability::Disabled { reason } => format!("{} (unavailable: {reason})", o.title),
        };
        rows.push(vec![
            o.id.0.clone(),
            what,
            bytes(o.saving),
            format!("{} {}", o.count, o.unit),
            o.safety.label().to_string(),
        ]);
    }
    print!("{}", table(&rows, &[2]));
}

/// `list [--category] [--json]`.
pub fn list(a: &Parsed) -> Res {
    let ctx = Ctx::open(a)?;
    let tab = match a.value("category") {
        None => OpportunityTab::All,
        Some("clean") => OpportunityTab::Clean,
        Some("optimise") | Some("optimize") => OpportunityTab::Optimise,
        Some("archive") => OpportunityTab::Archive,
        Some("review") | Some("review_only") => OpportunityTab::ReviewOnly,
        Some(other) => {
            return Err(Fail::Usage(format!("unknown category `{other}`; use clean, optimise, archive or review")))
        }
    };
    ctx.ensure_scanned()?;
    let list = ctx.engine.opportunities_list(OpportunitiesRequest { tab, ..Default::default() })?;
    if ctx.json {
        println!("{}", json(&list));
    } else {
        print_opportunities(&list);
    }
    Ok(())
}

/// Print one page of candidates of `id`.
pub fn print_candidates(ctx: &Ctx, id: &OpportunityId, limit: u32) -> Res {
    let page = ctx.engine.candidates_page(CandidatesPageRequest {
        opportunity_id: id.clone(),
        sub_group: None,
        offset: 0,
        limit: limit.clamp(1, 500),
        sort: Default::default(),
        descending: false,
        filter: None,
    })?;
    if ctx.json {
        println!("{}", json(&page));
        return Ok(());
    }
    let mut rows = vec![["", "FILE", "SIZE", "SAVING", "WHY"].map(String::from).to_vec()];
    for r in &page.rows {
        let c = &r.candidate;
        let mark = if !r.actionable {
            "!"
        } else if r.selected {
            "x"
        } else {
            " "
        };
        rows.push(vec![
            format!("[{mark}]"),
            display_lossy(&c.path),
            bytes(c.size),
            bytes(c.size.saturating_sub(c.size_after)),
            c.reason.clone(),
        ]);
    }
    print!("{}", table(&rows, &[2, 3]));
    println!(
        "{} of {} shown. [x] selected by default, [ ] not selected, [!] protected or blocked.",
        page.rows.len(),
        page.total
    );
    Ok(())
}

/// `show <id> [--limit N]`.
pub fn show(a: &Parsed) -> Res {
    let ctx = Ctx::open(a)?;
    let limit = a.number("limit")?.unwrap_or(50) as u32;
    ctx.ensure_scanned()?;
    print_candidates(&ctx, &OpportunityId(a.positionals[0].clone()), limit)
}

/// `search "<query>" <folders>`.
pub fn search(a: &Parsed) -> Res {
    let ctx = Ctx::open(a)?;
    let query = a.positionals[0].clone();
    let parsed = ctx.engine.search_parse(SearchParseRequest { query: query.clone() })?;
    let Some(rule) = parsed.rule else {
        let msgs: Vec<String> =
            parsed.errors.iter().map(|e| format!("  at character {}: {}", e.offset + 1, e.message)).collect();
        return Err(Fail::Usage(format!("the query `{query}` has a problem:\n{}", msgs.join("\n"))));
    };
    let scope = scope(&ctx, a, &a.positionals[1..])?;
    ctx.run_scan(scope)?;
    let opp =
        ctx.engine.search_run(SearchRunRequest { rule, title: Some(query), topmost_only: false, action: None })?;
    if !ctx.json {
        println!("{}: {} files, {}", opp.title, opp.count, bytes(opp.size_before));
    }
    print_candidates(&ctx, &opp.id, a.number("limit")?.unwrap_or(50) as u32)
}
