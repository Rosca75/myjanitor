//! In-house argument parsing (no `clap`). Owner: W3-E.
//!
//! Grammar: `myjanitor-cli [options] <command> [<subcommand>] [options] [args]`.
//! Options are long (`--name value`, `--name=value`, `--flag`) or short
//! (`-n value`, `-nvalue`, `-y`); `--` ends options; global options
//! (`--data-dir`, `--cache-dir`, `--help`, `--version`) work anywhere. Every
//! command is described once in [`COMMANDS`], and `--help` text, validation and
//! error messages all come from that table.

use std::collections::{BTreeMap, BTreeSet};

/// One option of a command.
#[derive(Debug, Clone, Copy)]
pub struct Opt {
    pub long: &'static str,
    pub short: Option<char>,
    /// Placeholder of the value (`None` = a flag).
    pub value: Option<&'static str>,
    /// May be given several times.
    pub repeat: bool,
    pub help: &'static str,
}

const fn flag(long: &'static str, short: Option<char>, help: &'static str) -> Opt {
    Opt { long, short, value: None, repeat: false, help }
}
const fn val(long: &'static str, short: Option<char>, value: &'static str, help: &'static str) -> Opt {
    Opt { long, short, value: Some(value), repeat: false, help }
}
const fn multi(long: &'static str, short: Option<char>, value: &'static str, help: &'static str) -> Opt {
    Opt { long, short, value: Some(value), repeat: true, help }
}

/// One command (or command + subcommand).
#[derive(Debug, Clone, Copy)]
pub struct CmdSpec {
    pub name: &'static str,
    pub sub: Option<&'static str>,
    pub about: &'static str,
    /// Positional arguments for the usage line, e.g. `<FOLDER>...`.
    pub usage: &'static str,
    pub min_pos: usize,
    pub max_pos: Option<usize>,
    pub opts: &'static [Opt],
}

/// Options every command accepts.
pub const GLOBAL_OPTS: &[Opt] = &[
    val("data-dir", None, "DIR", "Keep every MyJanitor folder (settings, journal, cache, quarantine) under DIR"),
    val("cache-dir", None, "DIR", "Scan cache folder (default: DIR/cache with --data-dir)"),
    flag("help", Some('h'), "Show help for this command"),
    flag("version", Some('V'), "Print the version"),
];

const JSON: Opt = flag("json", None, "Print the engine's JSON instead of text");
const YES: Opt =
    flag("yes", Some('y'), "Do not ask for the typed confirmation (lossy and permanent steps still need their flags)");
const ORIGINALS: Opt = val("originals", None, "POLICY", "What happens to originals: trash (default) or quarantine");
const LOSSY: Opt = flag("confirm-lossy", None, "Confirm the plan's lossy steps (they re-encode files)");
const PERM: Opt = flag("confirm-permanent", None, "Confirm the plan's permanent steps (they cannot be undone)");
const EXCLUDE: Opt = multi("exclude", Some('e'), "GLOB", "Skip paths matching GLOB (repeatable)");
const NOCACHE: Opt = flag("no-cache", None, "Do not use the scan cache for this scan");
const LIMIT: Opt = val("limit", Some('n'), "N", "Show at most N rows");

macro_rules! c {
    ($name:expr, $sub:expr, $about:expr, $usage:expr, $min:expr, $max:expr, $opts:expr) => {
        CmdSpec { name: $name, sub: $sub, about: $about, usage: $usage, min_pos: $min, max_pos: $max, opts: $opts }
    };
}

/// Every command.
pub const COMMANDS: &[CmdSpec] = &[
    c!(
        "scan",
        None,
        "Scan folders and print what could be reclaimed (read-only; remembers the folders)",
        "<FOLDER>...",
        1,
        None,
        &[EXCLUDE, NOCACHE, JSON]
    ),
    c!(
        "list",
        None,
        "List the opportunities of the last scan",
        "",
        0,
        Some(0),
        &[val("category", Some('c'), "CATEGORY", "clean, optimise, archive or review"), JSON]
    ),
    c!("show", None, "Show the files behind one opportunity", "<OPPORTUNITY-ID>", 1, Some(1), &[LIMIT, JSON]),
    c!(
        "search",
        None,
        "Find files with a quick query, e.g. \"ext:mp4 size:>1GB\"",
        "<QUERY> <FOLDER>...",
        2,
        None,
        &[LIMIT, EXCLUDE, NOCACHE, JSON]
    ),
    c!(
        "plan",
        Some("add"),
        "Add an opportunity to the plan",
        "<OPPORTUNITY-ID>",
        1,
        Some(1),
        &[
            val(
                "action",
                None,
                "ACTION",
                "trash, quarantine, hardlink, reflink, move or copy (default: the opportunity's own)"
            ),
            val("dest", None, "DIR", "Target folder for move and copy")
        ]
    ),
    c!("plan", Some("remove"), "Remove an opportunity from the plan", "<OPPORTUNITY-ID>", 1, Some(1), &[]),
    c!("plan", Some("show"), "Show the plan with estimates", "", 0, Some(0), &[JSON]),
    c!("plan", Some("clear"), "Empty the plan", "", 0, Some(0), &[]),
    c!("dry-run", None, "Estimate the plan exactly without changing anything", "", 0, Some(0), &[JSON]),
    c!(
        "apply",
        None,
        "Apply the plan (asks for a typed confirmation on a terminal)",
        "",
        0,
        Some(0),
        &[YES, ORIGINALS, LOSSY, PERM, JSON]
    ),
    c!(
        "clean",
        None,
        "Scan, plan and dry-run or apply in one invocation",
        "<FOLDER>...",
        1,
        None,
        &[
            val("detectors", None, "A,B", "Comma-separated opportunity ids or detector names (required)"),
            flag("dry-run", None, "Only estimate (default)"),
            flag("apply", None, "Apply the steps"),
            YES,
            ORIGINALS,
            LOSSY,
            PERM,
            EXCLUDE,
            NOCACHE,
            JSON
        ]
    ),
    c!("history", None, "List past runs", "", 0, Some(0), &[LIMIT, JSON]),
    c!(
        "undo",
        None,
        "Undo the last run, or the run given with --run",
        "",
        0,
        Some(0),
        &[flag("last", None, "Undo the newest undoable run (default)"), val("run", None, "ID", "Undo this run"), JSON]
    ),
    c!(
        "report",
        None,
        "Show or export the report of a run",
        "",
        0,
        Some(0),
        &[
            flag("last", None, "The newest run (default)"),
            val("run", None, "ID", "A specific run"),
            val("format", Some('f'), "FORMAT", "text (default), csv or html"),
            val("out", Some('o'), "FILE", "Write csv/html to FILE"),
            JSON
        ]
    ),
    c!(
        "run-profile",
        None,
        "Run a scheduled profile now (what the OS scheduler calls)",
        "<NAME-OR-ID>",
        1,
        Some(1),
        &[JSON]
    ),
    c!("schedule", Some("list"), "List scheduled profiles with their next run", "", 0, Some(0), &[JSON]),
    c!(
        "schedule",
        Some("from-plan"),
        "Save the current plan as a schedule",
        "<NAME>",
        1,
        Some(1),
        &[
            val("every", None, "daily|weekly|monthly", "Recurrence (default weekly)"),
            val("weekday", None, "1-7", "Weekly: 1 = Monday"),
            val("day", None, "1-28", "Monthly: day of the month"),
            val("at", None, "HH:MM", "Time of day (default 08:47)")
        ]
    ),
    c!(
        "schedule",
        Some("install"),
        "Register a schedule with the OS scheduler (explicit; per user, no admin)",
        "<NAME-OR-ID>",
        1,
        Some(1),
        &[]
    ),
    c!("schedule", Some("uninstall"), "Remove a schedule's OS registration", "<NAME-OR-ID>", 1, Some(1), &[]),
    c!("schedule", Some("delete"), "Delete a schedule", "<NAME-OR-ID>", 1, Some(1), &[]),
    c!("context-menu", Some("status"), "Show whether the file-manager menu is installed", "", 0, Some(0), &[]),
    c!(
        "context-menu",
        Some("install"),
        "Install the file-manager menu entries (explicit; per user)",
        "",
        0,
        Some(0),
        &[]
    ),
    c!("context-menu", Some("uninstall"), "Remove the file-manager menu entries", "", 0, Some(0), &[]),
    c!(
        "tools",
        None,
        "List optional external tools and whether they were found",
        "",
        0,
        Some(0),
        &[flag("refresh", None, "Detect again first"), JSON]
    ),
    c!(
        "settings",
        Some("get"),
        "Print settings (all, or one dotted key such as performance.threads)",
        "[KEY]",
        0,
        Some(1),
        &[]
    ),
    c!("settings", Some("set"), "Change one setting; VALUE is JSON or plain text", "<KEY> <VALUE>", 2, Some(2), &[]),
    c!("cache", Some("info"), "Show the scan cache size", "", 0, Some(0), &[JSON]),
    c!("cache", Some("clear"), "Clear the scan cache", "", 0, Some(0), &[JSON]),
    c!("info", None, "Version, platform and folders (JSON)", "", 0, Some(0), &[]),
    c!("commands", None, "List the engine commands", "", 0, Some(0), &[]),
];

/// Commands that take a subcommand.
const GROUPS: &[&str] = &["plan", "schedule", "context-menu", "settings", "cache"];

/// A command line the user got wrong (exit code 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageError(pub String);

/// A parsed command line.
#[derive(Debug, Clone)]
pub struct Parsed {
    pub cmd: &'static CmdSpec,
    values: BTreeMap<&'static str, Vec<String>>,
    flags: BTreeSet<&'static str>,
    pub positionals: Vec<String>,
}

impl Parsed {
    /// True when the flag was given.
    pub fn flag(&self, name: &str) -> bool {
        self.flags.contains(name)
    }
    /// The last value of an option.
    pub fn value(&self, name: &str) -> Option<&str> {
        self.values.get(name).and_then(|v| v.last()).map(String::as_str)
    }
    /// Every value of a repeatable option.
    pub fn values(&self, name: &str) -> &[String] {
        self.values.get(name).map(Vec::as_slice).unwrap_or(&[])
    }
    /// The option as a number.
    pub fn number(&self, name: &str) -> Result<Option<u64>, UsageError> {
        self.value(name)
            .map(|v| v.parse::<u64>().map_err(|_| UsageError(format!("--{name} needs a whole number, got `{v}`"))))
            .transpose()
    }
}

/// What the command line asks for.
#[derive(Debug, Clone)]
pub enum Outcome {
    Run(Parsed),
    /// Text to print on stdout (help, version), exit 0.
    Print(String),
}

fn find(name: &str, sub: Option<&str>) -> Option<&'static CmdSpec> {
    COMMANDS.iter().find(|c| c.name == name && c.sub == sub)
}

fn display(c: &CmdSpec) -> String {
    match c.sub {
        Some(s) => format!("{} {s}", c.name),
        None => c.name.to_string(),
    }
}

/// Usage and options of one command.
pub fn command_help(c: &CmdSpec) -> String {
    let mut s = format!(
        "{}\n\nUSAGE:\n    myjanitor-cli {}{}{}\n",
        c.about,
        display(c),
        if c.opts.is_empty() { "" } else { " [OPTIONS]" },
        if c.usage.is_empty() { String::new() } else { format!(" {}", c.usage) }
    );
    let rows: Vec<String> = c.opts.iter().chain(GLOBAL_OPTS).map(opt_line).collect();
    s.push_str("\nOPTIONS:\n");
    s.push_str(&rows.join("\n"));
    s.push('\n');
    s
}

fn opt_line(o: &Opt) -> String {
    let short = o.short.map(|c| format!("-{c}, ")).unwrap_or_else(|| "    ".into());
    let long = match o.value {
        Some(v) => format!("--{} <{v}>", o.long),
        None => format!("--{}", o.long),
    };
    format!("    {short}{long:<26} {}", o.help)
}

/// Top-level help.
pub fn general_help() -> String {
    let mut s = String::from("myjanitor-cli: clean files and make them smaller, safely and offline\n\nUSAGE:\n    myjanitor-cli [--data-dir DIR] <COMMAND> [OPTIONS]\n\nCOMMANDS:\n");
    let mut shown = BTreeSet::new();
    for c in COMMANDS {
        if c.sub.is_some() {
            if shown.insert(c.name) {
                let subs: Vec<_> = COMMANDS.iter().filter(|x| x.name == c.name).filter_map(|x| x.sub).collect();
                s.push_str(&format!("    {:<14} {}\n", c.name, subs.join("|")));
            }
        } else {
            s.push_str(&format!("    {:<14} {}\n", c.name, c.about));
        }
    }
    s.push_str("\nEXIT CODES:\n    0 ok   1 error   2 usage   3 finished with some failed items\n\nRun `myjanitor-cli <COMMAND> --help` for details. Changes are undoable with `undo` unless noted.\n");
    s
}

fn suggest(word: &str) -> String {
    let near = COMMANDS
        .iter()
        .map(|c| c.name)
        .filter(|n| n.starts_with(&word[..word.len().min(2)]) || word.starts_with(n))
        .collect::<BTreeSet<_>>();
    if near.is_empty() {
        String::new()
    } else {
        format!(" Did you mean {}?", near.into_iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(" or "))
    }
}

/// Parse the arguments (without the program name).
pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Outcome, UsageError> {
    let args: Vec<String> = args.into_iter().collect();
    // Pass 1: find the command words, skipping option values of global options.
    let mut words: Vec<String> = Vec::new();
    let mut i = 0;
    let mut help = false;
    let mut version = false;
    while i < args.len() {
        let a = &args[i];
        if a == "--" {
            break;
        }
        if a == "--help" || a == "-h" {
            help = true;
        } else if a == "--version" || a == "-V" {
            version = true;
        } else if a == "--data-dir" || a == "--cache-dir" {
            i += 1;
        } else if a.starts_with('-') && a.len() > 1 {
            // A command option: its value (if any) is not a command word, but we cannot know
            // which command yet; words are only taken before the first such option.
            if !words.is_empty() && !(words.len() == 1 && GROUPS.contains(&words[0].as_str())) {
                break;
            }
        } else if words.len() < 2 && (words.is_empty() || GROUPS.contains(&words[0].as_str())) {
            words.push(a.clone());
        } else {
            break;
        }
        i += 1;
    }
    if version && words.is_empty() {
        return Ok(Outcome::Print(format!("myjanitor-cli {}\n", env!("CARGO_PKG_VERSION"))));
    }
    if words.first().map(String::as_str) == Some("help") {
        let mut rest = args.iter().skip_while(|a| *a != "help").skip(1).filter(|a| !a.starts_with('-'));
        return match rest.next() {
            None => Ok(Outcome::Print(general_help())),
            Some(w) => help_for(w, rest.next().map(String::as_str)),
        };
    }
    if words.is_empty() {
        return if help || args.is_empty() {
            Ok(Outcome::Print(general_help()))
        } else {
            Err(UsageError("missing command; see `myjanitor-cli help`".into()))
        };
    }
    let name = words[0].clone();
    let sub = words.get(1).cloned();
    let spec = if GROUPS.contains(&name.as_str()) {
        match &sub {
            None if help => return help_for(&name, None),
            None => return Err(group_error(&name)),
            Some(s) => find(&name, Some(s)).ok_or_else(|| {
                let valid: Vec<_> = COMMANDS.iter().filter(|c| c.name == name).filter_map(|c| c.sub).collect();
                UsageError(format!("unknown subcommand `{s}` for `{name}`; expected one of: {}", valid.join(", ")))
            })?,
        }
    } else {
        find(&name, None).ok_or_else(|| {
            UsageError(format!("unknown command `{name}`; see `myjanitor-cli help`.{}", suggest(&name)))
        })?
    };
    if help {
        return Ok(Outcome::Print(command_help(spec)));
    }
    parse_options(spec, &args, words.len())
}

fn group_error(name: &str) -> UsageError {
    let valid: Vec<_> = COMMANDS.iter().filter(|c| c.name == name).filter_map(|c| c.sub).collect();
    UsageError(format!("`{name}` needs a subcommand: {}", valid.join(", ")))
}

fn help_for(name: &str, sub: Option<&str>) -> Result<Outcome, UsageError> {
    if let Some(c) = find(name, sub) {
        return Ok(Outcome::Print(command_help(c)));
    }
    let subs: Vec<&CmdSpec> = COMMANDS.iter().filter(|c| c.name == name).collect();
    if subs.is_empty() {
        return Err(UsageError(format!("no help for unknown command `{name}`")));
    }
    Ok(Outcome::Print(subs.iter().map(|c| format!("{}\n    {}\n", display(c), c.about)).collect::<Vec<_>>().join("")))
}

fn parse_options(spec: &'static CmdSpec, args: &[String], words: usize) -> Result<Outcome, UsageError> {
    let mut out = Parsed { cmd: spec, values: BTreeMap::new(), flags: BTreeSet::new(), positionals: Vec::new() };
    let shown = display(spec);
    let lookup = |long: Option<&str>, short: Option<char>| -> Option<&'static Opt> {
        spec.opts
            .iter()
            .chain(GLOBAL_OPTS)
            .find(|o| long.is_some_and(|l| o.long == l) || short.is_some_and(|s| o.short == Some(s)))
    };
    let mut skipped = 0;
    let mut only_pos = false;
    let mut it = args.iter().peekable();
    while let Some(a) = it.next() {
        if only_pos {
            out.positionals.push(a.clone());
            continue;
        }
        if skipped < words && !a.starts_with('-') {
            skipped += 1;
            continue;
        }
        if a == "--" {
            only_pos = true;
            continue;
        }
        let (opt, inline): (&Opt, Option<String>) = if let Some(body) = a.strip_prefix("--") {
            let (name, inline) = match body.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (body, None),
            };
            (
                lookup(Some(name), None).ok_or_else(|| {
                    UsageError(format!("unknown option `--{name}` for `{shown}`; see `myjanitor-cli {shown} --help`"))
                })?,
                inline,
            )
        } else if a.starts_with('-') && a.len() > 1 {
            let mut chars = a[1..].chars();
            let c = chars.next().unwrap_or('-');
            let rest: String = chars.collect();
            let o = lookup(None, Some(c)).ok_or_else(|| {
                UsageError(format!("unknown option `-{c}` for `{shown}`; see `myjanitor-cli {shown} --help`"))
            })?;
            let rest = rest.strip_prefix('=').map(str::to_string).unwrap_or(rest);
            if o.value.is_none() && !rest.is_empty() {
                return Err(UsageError(format!("-{c} does not take a value (combined short flags are not supported)")));
            }
            (o, (!rest.is_empty()).then_some(rest))
        } else {
            out.positionals.push(a.clone());
            continue;
        };
        match opt.value {
            None => {
                if inline.is_some() {
                    return Err(UsageError(format!("--{} does not take a value", opt.long)));
                }
                out.flags.insert(opt.long);
            }
            Some(ph) => {
                let v = match inline {
                    Some(v) => v,
                    None => {
                        it.next().cloned().ok_or_else(|| UsageError(format!("--{} needs a value <{ph}>", opt.long)))?
                    }
                };
                let slot = out.values.entry(opt.long).or_default();
                if !opt.repeat && !slot.is_empty() {
                    return Err(UsageError(format!("--{} was given twice", opt.long)));
                }
                slot.push(v);
            }
        }
    }
    let n = out.positionals.len();
    if n < spec.min_pos || spec.max_pos.is_some_and(|m| n > m) {
        let expect = if spec.usage.is_empty() { "no arguments".to_string() } else { spec.usage.to_string() };
        return Err(UsageError(format!(
            "`{shown}` expects {expect}, got {n} argument{}; see `myjanitor-cli {shown} --help`",
            if n == 1 { "" } else { "s" }
        )));
    }
    Ok(Outcome::Run(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(v: &[&str]) -> Result<Outcome, UsageError> {
        parse(v.iter().map(|s| s.to_string()))
    }
    fn run(v: &[&str]) -> Parsed {
        match p(v).unwrap() {
            Outcome::Run(r) => r,
            Outcome::Print(t) => panic!("expected a command, got text: {t}"),
        }
    }
    fn err(v: &[&str]) -> String {
        p(v).unwrap_err().0
    }
    fn text(v: &[&str]) -> String {
        match p(v).unwrap() {
            Outcome::Print(t) => t,
            Outcome::Run(r) => panic!("expected text, got {:?}", r.cmd.name),
        }
    }

    #[test]
    fn every_command_parses_with_its_minimum_arguments() {
        for c in COMMANDS {
            let mut v: Vec<String> = vec![c.name.into()];
            v.extend(c.sub.map(String::from));
            for _ in 0..c.min_pos {
                v.push("x".into());
            }
            match parse(v.clone()) {
                Ok(Outcome::Run(r)) => assert_eq!((r.cmd.name, r.cmd.sub), (c.name, c.sub)),
                other => panic!("{v:?}: {other:?}"),
            }
        }
    }

    #[test]
    fn options_flags_values_and_terminator() {
        let r =
            run(&["scan", "/a", "-e", "*.tmp", "--exclude=**/cache", "/b", "--no-cache", "--json", "--", "--weird"]);
        assert_eq!(r.cmd.name, "scan");
        assert_eq!(r.positionals, ["/a", "/b", "--weird"]);
        assert_eq!(r.values("exclude"), ["*.tmp", "**/cache"]);
        assert!(r.flag("no-cache") && r.flag("json") && !r.flag("yes"));
        let r = run(&["--data-dir", "/d", "show", "det:x", "-n5"]);
        assert_eq!(r.value("data-dir"), Some("/d"));
        assert_eq!(r.number("limit").unwrap(), Some(5));
        assert_eq!(r.positionals, ["det:x"]);
        let r = run(&["report", "--format=csv", "-o", "r.csv", "--data-dir=/x"]);
        assert_eq!((r.value("format"), r.value("out"), r.value("data-dir")), (Some("csv"), Some("r.csv"), Some("/x")));
    }

    #[test]
    fn subcommands() {
        let r = run(&["plan", "add", "det:temp_files", "--action", "quarantine"]);
        assert_eq!((r.cmd.name, r.cmd.sub), ("plan", Some("add")));
        assert_eq!(r.value("action"), Some("quarantine"));
        let r = run(&["settings", "set", "performance.threads", "4"]);
        assert_eq!(r.positionals, ["performance.threads", "4"]);
        assert_eq!(run(&["cache", "clear"]).cmd.sub, Some("clear"));
        assert_eq!(
            run(&["schedule", "from-plan", "Weekly", "--every", "daily", "--at", "07:30"]).value("at"),
            Some("07:30")
        );
        assert_eq!(run(&["--data-dir", "/d", "plan", "show"]).cmd.sub, Some("show"));
    }

    #[test]
    fn errors_are_specific_and_helpful() {
        assert!(err(&["frobnicate"]).contains("unknown command `frobnicate`"));
        assert!(err(&["scan"]).contains("expects <FOLDER>..."));
        assert!(err(&["list", "extra"]).contains("expects no arguments"));
        assert!(err(&["scan", "/a", "--bogus"]).contains("unknown option `--bogus` for `scan`"));
        assert!(err(&["scan", "/a", "-q"]).contains("unknown option `-q`"));
        assert!(err(&["show", "x", "--limit"]).contains("--limit needs a value"));
        assert!(err(&["scan", "/a", "--json=1"]).contains("does not take a value"));
        assert!(err(&["report", "--out", "a", "--out", "b"]).contains("twice"));
        assert!(err(&["plan"]).contains("needs a subcommand: add, remove, show, clear"));
        assert!(err(&["plan", "zap"]).contains("unknown subcommand `zap`"));
        assert!(err(&["settings", "set", "only-key"]).contains("expects <KEY> <VALUE>"));
        assert!(err(&["--json"]).contains("missing command"));
        assert!(p(&["show", "x", "--limit", "abc"])
            .and_then(|o| match o {
                Outcome::Run(r) => r.number("limit").map(|_| ()),
                _ => Ok(()),
            })
            .is_err());
    }

    #[test]
    fn help_and_version() {
        assert!(text(&[]).contains("COMMANDS:") && text(&[]).contains("EXIT CODES"));
        assert_eq!(text(&["--help"]), text(&["help"]));
        assert!(text(&["--version"]).starts_with("myjanitor-cli 0."));
        assert!(text(&["-V"]).starts_with("myjanitor-cli"));
        let h = text(&["apply", "--help"]);
        assert!(
            h.contains("USAGE:\n    myjanitor-cli apply [OPTIONS]")
                && h.contains("--confirm-lossy")
                && h.contains("-y, --yes")
                && h.contains("--data-dir")
        );
        assert_eq!(text(&["help", "apply"]), h);
        assert!(text(&["plan", "add", "--help"]).contains("plan add"));
        assert!(text(&["plan", "--help"]).contains("plan remove"));
        assert!(text(&["help", "plan"]).contains("plan clear"));
        assert!(text(&["scan", "-h"]).contains("--exclude"));
        assert!(p(&["help", "nope"]).is_err());
    }

    #[test]
    fn every_command_has_help_text_mentioning_its_options() {
        for c in COMMANDS {
            let h = command_help(c);
            for o in c.opts {
                assert!(h.contains(&format!("--{}", o.long)), "{} lacks --{}", display(c), o.long);
            }
        }
    }
}
