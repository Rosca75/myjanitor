//! `myjanitor-cli`: the command-line front end (AUT-01) over the same engine as
//! the GUI. Owner: W3-E. Wave 1 commands: `info`, `commands`, `scan`.
//! Planned (W3-E): `list`, `dry-run`, `apply`, `undo`, `history`, `export`.
#![forbid(unsafe_code)]

mod args;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use mj_engine::api::{OpportunitiesRequest, ScanRunState, ScanStartRequest, ScopeSetRequest};
use mj_engine::{Engine, EngineConfig};

const HELP: &str = "myjanitor-cli: clean files and make them smaller, safely and offline

USAGE:
    myjanitor-cli [--data-dir DIR] [--cache-dir DIR] <COMMAND> [ARGS]

COMMANDS:
    info                 Version, platform and folders (JSON)
    commands             List the engine commands
    scan <PATH>...       Scan folders and print the opportunities (read-only)
    help                 This text

OPTIONS:
    --json               JSON output where supported
    --version            Print the version";

fn engine(a: &args::Args) -> Result<Arc<Engine>, String> {
    let (mut data, mut cache, portable) = mj_engine::paths::default_dirs();
    if let Some(d) = a.options.get("data-dir") {
        data = PathBuf::from(d);
    }
    if let Some(c) = a.options.get("cache-dir") {
        cache = PathBuf::from(c);
    }
    let mut cfg = EngineConfig::new(data, cache);
    cfg.portable = portable;
    Engine::new(cfg).map_err(|e| e.to_string())
}

fn json<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
}

fn scan(a: &args::Args) -> Result<(), String> {
    if a.positionals.is_empty() {
        return Err("scan needs at least one folder".into());
    }
    let e = engine(a)?;
    let roots = a
        .positionals
        .iter()
        .map(|p| std::fs::canonicalize(p).map_err(|err| format!("{p}: {err}")))
        .collect::<Result<Vec<_>, _>>()?;
    let mut scope = e.scope_get().map_err(|x| x.to_string())?;
    scope.roots = roots;
    e.scope_set(ScopeSetRequest { scope }).map_err(|x| x.to_string())?;
    e.scan_start(ScanStartRequest::default()).map_err(|x| x.to_string())?;
    let status = loop {
        let s = e.scan_status().map_err(|x| x.to_string())?;
        if !matches!(s.state, ScanRunState::Scanning | ScanRunState::Paused | ScanRunState::Stopping) {
            break s;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    if let Some(err) = &status.error {
        return Err(format!("{} ({})", err.message, err.hint));
    }
    let list = e.opportunities_list(OpportunitiesRequest::default()).map_err(|x| x.to_string())?;
    if a.flag("json") {
        println!("{}", json(&list));
    } else {
        let stats = status.stats.unwrap_or_default();
        println!("Scanned {} files, {} folders, {}", stats.files, stats.dirs, mj_core::size::format_bytes(stats.bytes));
        if stats.unreadable > 0 {
            println!("Could not read {} entries", stats.unreadable);
        }
        println!(
            "Reclaimable: {} in {} opportunities",
            mj_core::size::format_bytes(list.summary.total),
            list.items.len()
        );
        for o in &list.items {
            println!("  {:<28} {:>10}  {} {}", o.title, mj_core::size::format_bytes(o.saving), o.count, o.unit);
        }
    }
    Ok(())
}

fn run() -> Result<(), String> {
    let a = args::parse(std::env::args().skip(1))?;
    if a.flag("version") {
        println!("myjanitor-cli {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    match a.command.as_deref() {
        None | Some("help") => {
            println!("{HELP}");
            Ok(())
        }
        Some("info") => {
            println!("{}", json(&engine(&a)?.app_info().map_err(|e| e.to_string())?));
            Ok(())
        }
        Some("commands") => {
            for c in mj_engine::api::COMMANDS {
                println!("{}", c.name);
            }
            Ok(())
        }
        Some("scan") => scan(&a),
        Some(other) => Err(format!("unknown command `{other}`; see `myjanitor-cli help`")),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
    }
}
