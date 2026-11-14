//! GUI launch arguments (W4-INT): `--data-dir <dir>` (every app folder below one root;
//! smoke tests and power users), and the context-menu entry points `--optimise <path>`
//! and `--duplicates <path>` (E-06), which only preselect the scope. Unknown arguments
//! are ignored (the webview runtime may add its own); a flag without a value is ignored.

use std::path::PathBuf;

use mj_engine::api::{LaunchIntent, LaunchMode};

/// Parsed launch arguments.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LaunchArgs {
    pub data_dir: Option<PathBuf>,
    pub intent: Option<LaunchIntent>,
}

/// Parse `args` (without the program name). The last entry point wins; relative paths are
/// made absolute against `cwd` (a context menu passes absolute paths anyway).
pub fn parse(args: &[String], cwd: &std::path::Path) -> LaunchArgs {
    let abs = |p: &str| {
        let p = PathBuf::from(p);
        if p.is_absolute() {
            p
        } else {
            cwd.join(p)
        }
    };
    let mut out = LaunchArgs::default();
    let mut it = args.iter().peekable();
    while let Some(a) = it.next() {
        let (flag, inline) = match a.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v.to_string())),
            _ => (a.as_str(), None),
        };
        let mode = match flag {
            "--optimise" | "--optimize" => Some(LaunchMode::Optimise),
            "--duplicates" => Some(LaunchMode::Duplicates),
            "--data-dir" => None,
            _ => continue,
        };
        let value = match inline {
            Some(v) => Some(v),
            None => match it.peek() {
                Some(v) if !v.starts_with("--") => it.next().cloned(),
                _ => None,
            },
        };
        let Some(v) = value.filter(|v| !v.is_empty()) else { continue };
        match mode {
            Some(mode) => out.intent = Some(LaunchIntent { mode, path: abs(&v) }),
            None => out.data_dir = Some(abs(&v)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(a: &[&str]) -> LaunchArgs {
        parse(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>(), std::path::Path::new("/work"))
    }

    #[test]
    fn entry_points_and_data_dir() {
        let a = p(&["--optimise", "/home/m/Docs"]);
        assert_eq!(a.intent, Some(LaunchIntent { mode: LaunchMode::Optimise, path: "/home/m/Docs".into() }));
        assert_eq!(a.data_dir, None);
        let a = p(&["--data-dir=/tmp/x", "--duplicates", "Pictures"]);
        assert_eq!(a.data_dir, Some("/tmp/x".into()));
        assert_eq!(a.intent, Some(LaunchIntent { mode: LaunchMode::Duplicates, path: "/work/Pictures".into() }));
    }

    #[test]
    fn unknown_and_valueless_flags_are_ignored() {
        assert_eq!(p(&["--foo", "bar", "--optimise"]), LaunchArgs::default());
        assert_eq!(p(&["--optimise", "--duplicates"]), LaunchArgs::default());
        assert_eq!(p(&[]), LaunchArgs::default());
    }
}
