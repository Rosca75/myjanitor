//! winapp2.ini parser and resolver (CLN-DET-11 compatibility): `[Section]`, `DetectFile=`,
//! `Detect=`, `LangSecRef=`, `Default=`, `Warning=`, `FileKeyN=path|pattern[;pattern][|RECURSE|REMOVESELF]`
//! and `ExcludeKeyN=`. In-house INI parsing (no crate). Registry keys (`Detect=`, `RegKey`) are
//! parsed but ignored: this app never reads the registry.
//!
//! The file is supplied by the user (never shipped, never downloaded). Parsing is hostile-input
//! safe: bounded lines, sections and keys, no panics, unknown keys ignored. Rules only ever
//! *propose* candidates from paths that are already in the scan index.
//! Owner: W2-D.

use std::collections::HashMap;
use std::path::Path;

use mj_core::glob::{Glob, GlobMode};
use mj_core::pathutil::{is_within, path_key};
use mj_core::{Diagnostics, FileIndex};

use crate::rules::Hit;

const MAX_LINES: usize = 2_000_000;
const MAX_LINE_CHARS: usize = 4096;
const MAX_SECTIONS: usize = 20_000;
const MAX_KEYS_PER_SECTION: usize = 512;

/// One `FileKeyN` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileKey {
    pub n: u32,
    /// Folder, possibly with `%Var%` and wildcards, as written.
    pub path: String,
    /// File name patterns (`*.*` normalised to `*`).
    pub patterns: Vec<String>,
    pub recurse: bool,
    pub remove_self: bool,
}

/// One `ExcludeKeyN` line (registry excludes are dropped at parse time).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExcludeKey {
    /// `FILE|folder|pattern`.
    File { path: String, pattern: String },
    /// `PATH|folder`.
    Path(String),
}

/// One cleaner section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Winapp2Entry {
    /// The section header as written.
    pub section: String,
    /// Display name (trailing ` *` removed).
    pub name: String,
    pub lang_sec_ref: Option<String>,
    pub detect_files: Vec<String>,
    /// Registry `Detect=` keys (kept for completeness, never evaluated).
    pub detects: Vec<String>,
    /// `Default=False` makes the cleaner opt-in.
    pub default: bool,
    pub warning: Option<String>,
    pub file_keys: Vec<FileKey>,
    pub exclude_keys: Vec<ExcludeKey>,
}

fn new_entry(section: &str) -> Winapp2Entry {
    Winapp2Entry {
        section: section.to_string(),
        name: section.trim().trim_end_matches('*').trim().to_string(),
        lang_sec_ref: None,
        detect_files: Vec::new(),
        detects: Vec::new(),
        default: true,
        warning: None,
        file_keys: Vec::new(),
        exclude_keys: Vec::new(),
    }
}

/// Trailing decimal number of a key (`FileKey12` -> 12), when the prefix matches.
fn numbered(key: &str, prefix: &str) -> Option<Option<u32>> {
    let rest = key.strip_prefix(prefix)?;
    if rest.is_empty() {
        return Some(None);
    }
    rest.chars().all(|c| c.is_ascii_digit()).then(|| Some(rest.parse().unwrap_or(0)))
}

fn parse_file_key(n: u32, value: &str) -> Option<FileKey> {
    let mut parts = value.split('|').map(str::trim);
    let path = parts.next().filter(|p| !p.is_empty())?.to_string();
    let rest: Vec<&str> = parts.collect();
    let is_flag = |s: &str| s.eq_ignore_ascii_case("RECURSE") || s.eq_ignore_ascii_case("REMOVESELF");
    // `path|pattern|FLAGS`, or `path|FLAGS` with the pattern omitted (= everything).
    let (pattern_field, flags): (&str, &[&str]) = match rest.split_first() {
        None => ("*.*", &[]),
        Some((first, tail)) if is_flag(first) => ("*.*", &rest[..tail.len() + 1]),
        Some((first, tail)) => (if first.is_empty() { "*.*" } else { first }, tail),
    };
    let patterns: Vec<String> = pattern_field
        .split(';')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| if p == "*.*" { "*".to_string() } else { p.to_string() })
        .collect();
    if patterns.is_empty() {
        return None;
    }
    let has = |f: &str| flags.iter().any(|x| x.eq_ignore_ascii_case(f));
    Some(FileKey { n, path, patterns, recurse: has("RECURSE"), remove_self: has("REMOVESELF") })
}

fn parse_exclude(value: &str) -> Option<ExcludeKey> {
    let mut parts = value.split('|').map(str::trim);
    match parts.next()?.to_ascii_uppercase().as_str() {
        "FILE" => {
            let path = parts.next().filter(|p| !p.is_empty())?.to_string();
            let pattern = parts.next().filter(|p| !p.is_empty()).unwrap_or("*").to_string();
            Some(ExcludeKey::File { path, pattern })
        }
        "PATH" => Some(ExcludeKey::Path(parts.next().filter(|p| !p.is_empty())?.to_string())),
        _ => None, // REG excludes and unknown kinds
    }
}

/// Parse winapp2.ini text. Sections without any `FileKey` (registry-only cleaners) are dropped.
/// Never panics, whatever the input.
pub fn parse(text: &str) -> Vec<Winapp2Entry> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut out: Vec<Winapp2Entry> = Vec::new();
    let mut cur: Option<Winapp2Entry> = None;
    let mut keys_in_section = 0usize;
    let finish = |cur: &mut Option<Winapp2Entry>, out: &mut Vec<Winapp2Entry>| {
        if let Some(e) = cur.take() {
            if !e.file_keys.is_empty() && out.len() < MAX_SECTIONS {
                out.push(e);
            }
        }
    };
    for line in text.lines().take(MAX_LINES) {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') || line.chars().count() > MAX_LINE_CHARS {
            continue;
        }
        if let Some(inner) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            finish(&mut cur, &mut out);
            keys_in_section = 0;
            cur = Some(new_entry(inner));
            continue;
        }
        let (Some(entry), Some((k, v))) = (cur.as_mut(), line.split_once('=')) else { continue };
        keys_in_section += 1;
        if keys_in_section > MAX_KEYS_PER_SECTION {
            continue;
        }
        let key = k.trim().to_ascii_lowercase();
        let value = v.trim();
        if let Some(n) = numbered(&key, "filekey") {
            if let Some(fk) = parse_file_key(n.unwrap_or(0), value) {
                entry.file_keys.push(fk);
            }
        } else if numbered(&key, "excludekey").is_some() {
            entry.exclude_keys.extend(parse_exclude(value));
        } else if numbered(&key, "detectfile").is_some() {
            if !value.is_empty() {
                entry.detect_files.push(value.to_string());
            }
        } else if numbered(&key, "detect").is_some() {
            if !value.is_empty() {
                entry.detects.push(value.to_string());
            }
        } else if key == "langsecref" {
            entry.lang_sec_ref = Some(value.to_string());
        } else if key == "default" {
            entry.default = !value.eq_ignore_ascii_case("false");
        } else if key == "warning" && !value.is_empty() {
            entry.warning = Some(value.to_string());
        }
    }
    finish(&mut cur, &mut out);
    out
}

/// Expand `%Var%` references with `env` (names matched case-insensitively). `None` when a variable
/// is unknown, because a half-expanded path could point somewhere unintended.
pub fn expand_vars(s: &str, env: &HashMap<String, String>) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after.find('%')?;
        let name = &after[..end];
        if name.is_empty() {
            out.push('%'); // `%%` is a literal percent sign
        } else {
            let upper = name.to_ascii_uppercase();
            let v = env.iter().find(|(k, _)| k.to_ascii_uppercase() == upper).map(|(_, v)| v)?;
            out.push_str(v.trim_end_matches(['\\', '/']));
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

/// An environment map from the process environment (the default when the caller supplies none).
pub fn process_env() -> HashMap<String, String> {
    std::env::vars().collect()
}

fn norm(p: &str) -> String {
    p.replace('\\', "/").trim_end_matches('/').to_string()
}

/// Leading literal folder of a possibly wildcarded path.
fn literal_prefix(dir: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in dir.split('/') {
        if seg.contains(['*', '?', '[']) {
            break;
        }
        parts.push(seg);
    }
    let joined = parts.join("/");
    if joined.is_empty() && dir.starts_with('/') {
        "/".to_string()
    } else {
        joined
    }
}

/// Resolve parsed sections against the index and return one hit per matching file (or per whole
/// folder for `RECURSE|REMOVESELF` with an all-files pattern and no excludes).
pub fn resolve(
    index: &FileIndex,
    entries: &[Winapp2Entry],
    env: &HashMap<String, String>,
    diag: &mut Diagnostics,
) -> Vec<Hit> {
    let mut hits = Vec::new();
    for entry in entries {
        if !detected(index, entry, env) {
            diag.skip("winapp2_app_not_detected");
            continue;
        }
        let excludes = compile_excludes(entry, env);
        let note = entry.warning.clone().unwrap_or_else(|| "from your winapp2.ini".into());
        for fk in &entry.file_keys {
            let Some(dir) = expand_vars(&fk.path, env).map(|d| norm(&d)) else {
                diag.skip("winapp2_unknown_variable");
                continue;
            };
            let whole_folder =
                fk.remove_self && fk.recurse && fk.patterns.iter().all(|p| p == "*") && excludes.is_empty();
            let key_hits = resolve_key(index, &dir, fk, &excludes, whole_folder);
            for pos in key_hits {
                hits.push(Hit {
                    pos,
                    app: entry.name.clone(),
                    what: "winapp2 rule".into(),
                    note: note.clone(),
                    select: entry.default,
                    generic: false,
                });
            }
        }
    }
    hits
}

/// `DetectFile` gating: an app is skipped only when one of its detect paths lies inside a scanned
/// root and none of them exists there. Registry detects and paths outside the roots never gate.
fn detected(index: &FileIndex, entry: &Winapp2Entry, env: &HashMap<String, String>) -> bool {
    let mut covered = false;
    for df in &entry.detect_files {
        let Some(p) = expand_vars(df, env).map(|d| norm(&d)) else { continue };
        if p.contains(['*', '?', '[']) {
            return true; // wildcard detects cannot be tested cheaply: do not gate
        }
        let path = Path::new(&p);
        if !index.roots.iter().any(|r| is_within(r, path)) {
            continue;
        }
        covered = true;
        if index.get(path).is_some() {
            return true;
        }
    }
    !covered
}

struct Exclude {
    dir: String,
    pattern: Option<Glob>,
}

fn compile_excludes(entry: &Winapp2Entry, env: &HashMap<String, String>) -> Vec<Exclude> {
    entry
        .exclude_keys
        .iter()
        .filter_map(|x| match x {
            ExcludeKey::File { path, pattern } => Some(Exclude {
                dir: norm(&expand_vars(path, env)?),
                pattern: Some(Glob::new(pattern, GlobMode::Name, true)),
            }),
            ExcludeKey::Path(path) => Some(Exclude { dir: norm(&expand_vars(path, env)?), pattern: None }),
        })
        .collect()
}

fn excluded(path: &Path, name: &str, ex: &[Exclude]) -> bool {
    ex.iter().any(|x| match &x.pattern {
        None => is_within(Path::new(&x.dir), path),
        Some(g) => path.parent().is_some_and(|p| path_key(p) == path_key(Path::new(&x.dir))) && g.matches(name),
    })
}

fn resolve_key(index: &FileIndex, dir: &str, fk: &FileKey, ex: &[Exclude], whole_folder: bool) -> Vec<usize> {
    let prefix = literal_prefix(dir);
    if prefix.is_empty() {
        return Vec::new();
    }
    let Some(root_pos) = index.position(Path::new(&prefix)) else { return Vec::new() };
    let literal = prefix == dir;
    if whole_folder && literal {
        return if index.entry(root_pos).is_dir() && index.entry(root_pos).depth > 0 {
            vec![root_pos]
        } else {
            Vec::new()
        };
    }
    let patterns: Vec<Glob> = fk.patterns.iter().map(|p| Glob::new(p, GlobMode::Name, true)).collect();
    let dir_glob =
        (!literal).then(|| Glob::new(&format!("{dir}{}", if fk.recurse { "/**" } else { "/*" }), GlobMode::Path, true));
    let dir_path = Path::new(dir);
    let dir_key = path_key(dir_path);
    let mut found = Vec::new();
    for pos in index.subtree(root_pos) {
        let e = index.entry(pos);
        if !e.is_file() || e.is_online_only() {
            continue;
        }
        let name = e.name();
        if !patterns.iter().any(|g| g.matches(&name)) {
            continue;
        }
        let in_dir = match &dir_glob {
            Some(g) => g.matches(&e.path.to_string_lossy()),
            None if fk.recurse => is_within(dir_path, &e.path),
            None => e.path.parent().is_some_and(|p| path_key(p) == dir_key),
        };
        if in_dir && !excluded(&e.path, &name, ex) {
            found.push(pos);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::*;
    use mj_core::testutil::TempDir;

    const SAMPLE: &str = "\u{feff}; winapp2 sample\r\n\
[Chrome Cache *]\r\n\
LangSecRef=3025\r\n\
Detect=HKCU\\Software\\Google\\Chrome\r\n\
DetectFile=%FIX%\\app\r\n\
Default=False\r\n\
Warning=Closes your browser first\r\n\
FileKey1=%FIX%\\app\\cache|*.*|RECURSE\r\n\
FileKey2=%FIX%\\app\\logs|*.log;*.old\r\n\
ExcludeKey1=FILE|%FIX%\\app\\logs|keep.log\r\n\
\r\n\
[Registry Only *]\r\n\
RegKey1=HKCU\\Software\\X\r\n\
\r\n\
[Whole *]\r\n\
FileKey1=%FIX%\\whole|*.*|RECURSE|REMOVESELF\r\n\
FileKey2=%MISSING%\\x|*.*\r\n";

    #[test]
    fn parses_sections_keys_and_flags() {
        let e = parse(SAMPLE);
        assert_eq!(e.len(), 2, "registry-only section is dropped");
        let c = &e[0];
        assert_eq!(c.name, "Chrome Cache");
        assert_eq!(c.lang_sec_ref.as_deref(), Some("3025"));
        assert_eq!(c.detect_files, vec!["%FIX%\\app"]);
        assert_eq!(c.detects.len(), 1);
        assert!(!c.default);
        assert_eq!(c.warning.as_deref(), Some("Closes your browser first"));
        assert_eq!(c.file_keys[0].patterns, vec!["*"]);
        assert!(c.file_keys[0].recurse && !c.file_keys[0].remove_self);
        assert_eq!(c.file_keys[1].patterns, vec!["*.log", "*.old"]);
        assert!(!c.file_keys[1].recurse);
        assert_eq!(c.exclude_keys.len(), 1);
        assert!(e[1].file_keys[0].remove_self);
    }

    #[test]
    fn variable_expansion() {
        let mut env = HashMap::new();
        env.insert("LocalAppData".to_string(), "C:\\Users\\me\\AppData\\Local\\".to_string());
        assert_eq!(expand_vars("%localappdata%\\Foo", &env).as_deref(), Some("C:\\Users\\me\\AppData\\Local\\Foo"));
        assert_eq!(expand_vars("100%% sure", &env).as_deref(), Some("100% sure"));
        assert_eq!(expand_vars("%Nope%\\x", &env), None);
        assert_eq!(expand_vars("dangling %LocalAppData", &env), None);
        assert_eq!(expand_vars("plain", &env).as_deref(), Some("plain"));
    }

    #[test]
    fn garbage_input_never_panics() {
        let mut x = 0x9e37_79b9u32;
        let alphabet: Vec<char> = "[]=|;*%\\/\r\n abcFileKeyDetectExcludeRECURSE0123456789\u{feff}é".chars().collect();
        for round in 0..200 {
            let mut s = String::new();
            for _ in 0..(round * 7 % 400) {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                s.push(alphabet[(x >> 16) as usize % alphabet.len()]);
            }
            let _ = parse(&s);
        }
        assert!(parse("").is_empty());
        assert!(parse("[Unclosed\nFileKey1=x|y").is_empty());
        assert!(parse("FileKey1=orphan|*.*").is_empty()); // key before any section
        assert!(parse("[S]\nFileKey1=|*.*\nFileKey2=\nFileKey3=|").is_empty());
        let long = format!("[S]\nFileKey1={}|*.*", "a".repeat(10_000));
        assert!(parse(&long).is_empty(), "over-long lines are ignored");
        let _ = expand_vars("%%%%%%", &HashMap::new());
    }

    #[test]
    fn resolves_against_the_index() {
        let t = TempDir::new("winapp2").unwrap();
        t.write("app/cache/a.bin", b"1").unwrap();
        t.write("app/cache/deep/b.bin", b"2").unwrap();
        t.write("app/logs/x.log", b"3").unwrap();
        t.write("app/logs/keep.log", b"4").unwrap();
        t.write("app/logs/sub/y.log", b"5").unwrap();
        t.write("whole/z.dat", b"6").unwrap();
        let ix = index_of(t.path());
        let mut env = HashMap::new();
        env.insert("FIX".into(), t.path().to_string_lossy().into_owned());
        let mut diag = Diagnostics::default();
        let hits = resolve(&ix, &parse(SAMPLE), &env, &mut diag);
        let mut names: Vec<String> = hits
            .iter()
            .map(|h| ix.entry(h.pos).path.strip_prefix(t.path()).unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        // Recursive cache files, non-recursive logs minus the excluded one, the whole folder.
        assert_eq!(names, vec!["app/cache/a.bin", "app/cache/deep/b.bin", "app/logs/x.log", "whole"]);
        assert!(!hits.iter().find(|h| h.app == "Chrome Cache").unwrap().select, "Default=False");
        assert_eq!(diag.skipped["winapp2_unknown_variable"], 1);
    }

    #[test]
    fn detectfile_inside_scanned_roots_gates_the_app() {
        let t = TempDir::new("winapp2_detect").unwrap();
        t.write("cache/a.bin", b"1").unwrap(); // `app` folder (the DetectFile) does not exist
        let ix = index_of(t.path());
        let mut env = HashMap::new();
        env.insert("FIX".into(), t.path().to_string_lossy().into_owned());
        let text = "[X *]\nDetectFile=%FIX%\\app\nFileKey1=%FIX%\\cache|*.*|RECURSE\n";
        let mut diag = Diagnostics::default();
        assert!(resolve(&ix, &parse(text), &env, &mut diag).is_empty());
        assert_eq!(diag.skipped["winapp2_app_not_detected"], 1);
        // A DetectFile outside every scanned root does not gate.
        let text = "[X *]\nDetectFile=/definitely/elsewhere\nFileKey1=%FIX%\\cache|*.*|RECURSE\n";
        assert_eq!(resolve(&ix, &parse(text), &env, &mut Diagnostics::default()).len(), 1);
    }
}
