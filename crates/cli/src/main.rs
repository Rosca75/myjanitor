//! `myjanitor-cli`: the command-line front end (AUT-01) over the same engine as
//! the GUI. Owner: W3-E.
//!
//! Exit codes: 0 ok, 1 error, 2 usage, 3 finished with some failed items.
//! Human output is plain aligned text; `--json` prints the engine's serde types.
//! `--data-dir DIR` moves every application folder under DIR (tests always use it).
#![forbid(unsafe_code)]

mod args;
mod cmd_admin;
mod cmd_hist;
mod cmd_plan;
mod cmd_scan;
mod ctx;
mod out;
mod safety;

use std::process::ExitCode;

use args::{Outcome, Parsed};
use ctx::{Fail, Res};

/// Run the parsed command.
fn dispatch(a: &Parsed) -> Res {
    match (a.cmd.name, a.cmd.sub) {
        ("scan", _) => cmd_scan::scan(a),
        ("list", _) => cmd_scan::list(a),
        ("show", _) => cmd_scan::show(a),
        ("search", _) => cmd_scan::search(a),
        ("plan", _) => cmd_plan::plan(a),
        ("dry-run", _) => cmd_plan::dry_run(a),
        ("apply", _) => cmd_plan::apply(a),
        ("clean", _) => cmd_plan::clean(a),
        ("history", _) => cmd_hist::history(a),
        ("undo", _) => cmd_hist::undo(a),
        ("report", _) => cmd_hist::report(a),
        ("run-profile", _) => cmd_admin::run_profile(a),
        ("schedule", _) => cmd_admin::schedule(a),
        ("context-menu", _) => cmd_admin::context_menu_cmd(a),
        ("tools", _) => cmd_admin::tools(a),
        ("settings", _) => cmd_admin::settings(a),
        ("cache", _) => cmd_admin::cache(a),
        ("info", _) => cmd_admin::info(a),
        ("commands", _) => cmd_admin::commands(a),
        (other, _) => Err(Fail::Usage(format!("`{other}` is not implemented"))),
    }
}

fn main() -> ExitCode {
    let result = match args::parse(std::env::args().skip(1)) {
        Ok(Outcome::Print(text)) => {
            print!("{text}");
            return ExitCode::SUCCESS;
        }
        Ok(Outcome::Run(a)) => dispatch(&a),
        Err(e) => Err(Fail::from(e)),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(f) => {
            eprintln!("error: {}", f.message());
            ExitCode::from(f.code())
        }
    }
}
