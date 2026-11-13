//! In-house argument parsing (no `clap`). Owner: W3-E.
//!
//! Grammar: `myjanitor-cli [global options] <command> [options] [positionals]`.
//! Options are `--name value`, `--name=value` or boolean `--flag`; `--` ends
//! options. Unknown options are errors.

use std::collections::BTreeMap;

/// Parsed command line.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Args {
    pub command: Option<String>,
    pub options: BTreeMap<String, String>,
    pub flags: Vec<String>,
    pub positionals: Vec<String>,
}

/// Options that take a value (all others are flags).
pub const VALUE_OPTIONS: &[&str] = &["data-dir", "cache-dir", "format", "tab", "out"];

/// Parse arguments (without the program name).
pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Args, String> {
    let mut out = Args::default();
    let mut it = args.into_iter();
    let mut only_positionals = false;
    while let Some(a) = it.next() {
        if only_positionals || !a.starts_with("--") || a == "-" {
            if out.command.is_none() && !only_positionals {
                out.command = Some(a);
            } else {
                out.positionals.push(a);
            }
            continue;
        }
        if a == "--" {
            only_positionals = true;
            continue;
        }
        let body = &a[2..];
        let (name, inline) = match body.split_once('=') {
            Some((n, v)) => (n.to_string(), Some(v.to_string())),
            None => (body.to_string(), None),
        };
        if VALUE_OPTIONS.contains(&name.as_str()) {
            let v = match inline {
                Some(v) => v,
                None => it.next().ok_or_else(|| format!("--{name} needs a value"))?,
            };
            out.options.insert(name, v);
        } else if inline.is_some() {
            return Err(format!("--{name} does not take a value"));
        } else {
            out.flags.push(name);
        }
    }
    Ok(out)
}

impl Args {
    /// True when `--name` was given.
    pub fn flag(&self, name: &str) -> bool {
        self.flags.iter().any(|f| f == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(v: &[&str]) -> Result<Args, String> {
        parse(v.iter().map(|s| s.to_string()))
    }

    #[test]
    fn parses_commands_options_flags() {
        let a = p(&["scan", "--format", "json", "/a", "--json", "--out=x.csv", "--", "--weird"]).unwrap();
        assert_eq!(a.command.as_deref(), Some("scan"));
        assert_eq!(a.options["format"], "json");
        assert_eq!(a.options["out"], "x.csv");
        assert!(a.flag("json"));
        assert_eq!(a.positionals, vec!["/a", "--weird"]);
        assert!(p(&["scan", "--format"]).is_err());
        assert!(p(&["--json=1"]).is_err());
    }
}
