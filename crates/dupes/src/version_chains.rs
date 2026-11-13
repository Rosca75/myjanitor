//! Version chains (CLN-DET-18): v1..vN, final, final_v2, date prefixes, initials suffixes; keep latest, review others.
//! Owner: W2-B.
//!
//! Purely name-based (no content is read). Files chain when they are in the
//! **same folder**, share an **extension family** (Word, Excel, PowerPoint,
//! OpenDocument kinds; otherwise the exact extension) and reduce to the same
//! **base name** once version markers are removed.
//!
//! ## Tokenisation
//! The stem (file name without its last extension) is split into tokens at
//! every run of non-alphanumeric characters (`_ - . space ( ) [ ] { } + , ~`);
//! a token remembers whether it was written in parentheses. The base key is the
//! remaining tokens, lower-cased, joined by one space, so `Client-Proposal`,
//! `client_proposal` and `Client Proposal` are the same base.
//!
//! ## Markers removed (repeatedly, from either end)
//! | Marker | Examples | Where |
//! |---|---|---|
//! | copy number | `(1)`, `(12)` (parenthesised 1-99) | end |
//! | copy word | `- Copy`, `_copy`, `Copy of ...` | end / start |
//! | version | `v2`, `V10`, `v2.1`, `v_3`, `ver 2`, `version 3`, `rev4`, `rev 4`, `final2` | end |
//! | status word | `final`, `draft`, `latest`, `updated`, `revised`, `edited`, `clean`, `old` | end / start |
//! | full date | `2024-05-01`, `2024_05_01`, `2024.05.01`, `20240501`, `01-05-2024`, `01.05.2024` | end / start |
//! | initials | `_JD`, `-ABC` (2-3 upper-case letters) | only the **last** token of the stem, and only when the name carries another marker |
//!
//! Deliberately **not** markers, so the corresponding files never chain: bare
//! years (`Budget 2023` / `Budget 2024`), quarters and months (`Q1` / `Q2`,
//! `2024-05`), bare numbers (`Part 1`, `Proposal_2`), six-digit dates
//! (`240501` is as often a project number), and upper-case suffixes without
//! another marker (`Report_EU` / `Report_US`).
//!
//! ## Fuzzy stems
//! Two bases of the same folder and family also chain when they have the same
//! number of tokens, differ in exactly **one** token, that pair differs by a
//! single edit (Levenshtein distance 1), both tokens are at least 5 letters long
//! and contain no digit, and both chains carry at least one marker (a typo such
//! as `Propsal_v1` / `Proposal_v2`). Names that differ in a digit never chain.
//!
//! The keeper is chosen by the keep rule (default: latest modified); every
//! other member is a review candidate that is never pre-selected.

use std::collections::BTreeMap;

use mj_core::{
    ActionKind, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, FileEntry, GroupId,
    GroupKind, KeepRule, ParamValue, PlatformSupport, Result, SafetyClass, SettingKind, SettingSpec,
};

use crate::keep::{self, GroupSpec, Selection};
use crate::perceptual::UnionFind;

/// Parameter key: fuzzy stem matching on/off.
pub const FUZZY: &str = "fuzzy";
/// Folders with more distinct bases than this skip fuzzy matching (quadratic step).
pub const FUZZY_LIMIT: usize = 2000;

/// Version chains: Older versions of the same document
#[derive(Debug, Default, Clone, Copy)]
pub struct VersionChains;

impl Detector for VersionChains {
    fn info(&self) -> DetectorInfo {
        let mut settings = keep::settings("newest", false);
        settings.push(SettingSpec {
            key: FUZZY.into(),
            label: "Tolerate one-letter typos in names".into(),
            help: Some("Proposal_v1 and Propsal_v2 chain; digits must always match".into()),
            kind: SettingKind::Toggle { default: true },
            advanced: true,
            lossy: false,
            disabled_reason: None,
        });
        DetectorInfo {
            id: DetectorId::VersionChains,
            title: "Version chains".into(),
            description: "Older versions of the same document".into(),
            category: Category::Clean,
            safety: SafetyClass::ReviewOnly,
            default_action: ActionKind::None,
            available_actions: vec![
                ActionKind::None,
                ActionKind::Trash,
                ActionKind::Quarantine,
                ActionKind::Move,
                ActionKind::Archive,
            ],
            settings,
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-18".into()],
            unit: "files".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let rule = keep::rule_from_params(ctx.params, KeepRule::Newest);
        let fuzzy = !matches!(ctx.params.0.get(FUZZY), Some(ParamValue::Bool(false)));
        let files: Vec<&FileEntry> = ctx.index.files().collect();
        let mut out = DetectorOutput::default();
        let chains = find_chains(&files, fuzzy, &mut out.diagnostics.notes);
        for (n, chain) in chains.iter().enumerate() {
            if n % 256 == 0 {
                ctx.cancel.checkpoint()?;
            }
            let members: Vec<&FileEntry> = chain.iter().map(|m| files[m.index]).collect();
            let keeper = keep::pick(&rule, &members, &[]);
            let columns = chain
                .iter()
                .map(|m| BTreeMap::from([("version".to_string(), m.markers.join(", "))]))
                .collect();
            let label = format!("{} versions of {}", members.len(), members[keeper].name());
            keep::emit(
                GroupSpec {
                    id: GroupId(n as u64 + 1),
                    kind: GroupKind::VersionChain,
                    source: DetectorId::VersionChains,
                    label,
                    confidence: 100.0,
                    members,
                    keeper,
                    selection: Selection::None,
                    badges: mj_core::Badges::empty(),
                    reason: "Older version of a document in the same folder".into(),
                    keeper_reason: "Latest version (kept)".into(),
                    columns,
                },
                &mut out,
            );
        }
        Ok(out)
    }
}

/// One member of a chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// Index into the slice given to [`find_chains`].
    pub index: usize,
    /// Markers found in the name, e.g. `["v2", "final"]`.
    pub markers: Vec<String>,
}

/// A parsed file name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedName {
    /// Base tokens, lower case.
    pub base: Vec<String>,
    pub markers: Vec<String>,
}

impl ParsedName {
    /// The base key (`"client proposal"`).
    pub fn key(&self) -> String {
        self.base.join(" ")
    }
}

#[derive(Debug, Clone)]
struct Token {
    text: String,
    lower: String,
    paren: bool,
}

fn tokenize(stem: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0i32;
    let mut cur_paren = false;
    for ch in stem.chars() {
        if ch.is_alphanumeric() {
            if cur.is_empty() {
                cur_paren = depth > 0;
            }
            cur.push(ch);
            continue;
        }
        if !cur.is_empty() {
            out.push(Token { lower: cur.to_lowercase(), text: std::mem::take(&mut cur), paren: cur_paren });
        }
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = (depth - 1).max(0),
            _ => {}
        }
    }
    if !cur.is_empty() {
        out.push(Token { lower: cur.to_lowercase(), text: cur, paren: cur_paren });
    }
    out
}

const STATUS_WORDS: &[&str] = &["final", "draft", "latest", "updated", "revised", "edited", "clean", "old"];
const VERSION_WORDS: &[&str] = &["v", "ver", "version", "rev", "revision"];

fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// `v2`, `V10`, `rev4`, `final2`.
fn is_version_token(lower: &str) -> bool {
    ["v", "rev", "final"].iter().any(|p| lower.strip_prefix(p).is_some_and(|r| is_digits(r) && r.len() <= 4))
}

fn valid_date(y: u32, m: u32, d: u32) -> bool {
    (1990..=2100).contains(&y) && (1..=12).contains(&m) && d >= 1 && d <= mj_core::time::days_in_month(y as i32, m)
}

/// Length (in tokens) of a full date at the start of `t`, if any.
fn date_len(t: &[Token]) -> Option<usize> {
    let num = |i: usize| t.get(i).filter(|x| is_digits(&x.lower)).map(|x| (x.lower.len(), x.lower.parse::<u32>().ok()));
    if let Some((8, Some(v))) = num(0) {
        if valid_date(v / 10000, v / 100 % 100, v % 100) {
            return Some(1);
        }
    }
    if let (Some((la, Some(a))), Some((2, Some(b))), Some((lc, Some(c)))) = (num(0), num(1), num(2)) {
        if la == 4 && lc == 2 && valid_date(a, b, c) {
            return Some(3);
        }
        if la == 2 && lc == 4 && valid_date(c, b, a) {
            return Some(3);
        }
    }
    None
}

/// Strip one marker from the end of `t`; returns its display text.
fn strip_end(t: &mut Vec<Token>) -> Option<String> {
    let n = t.len();
    if n < 2 {
        return None;
    }
    let last = &t[n - 1];
    if last.paren && is_digits(&last.lower) && last.lower.len() <= 2 && last.lower != "0" {
        let m = format!("({})", last.text);
        t.pop();
        return Some(m);
    }
    if last.lower == "copy" || STATUS_WORDS.contains(&last.lower.as_str()) || is_version_token(&last.lower) {
        return t.pop().map(|x| x.lower);
    }
    // `v2.1`, `v 2`, `version 3`, `rev 4`.
    if is_digits(&last.lower) && last.lower.len() <= 4 && n >= 3 {
        let prev = &t[n - 2].lower;
        if VERSION_WORDS.contains(&prev.as_str()) || is_version_token(prev) {
            let m = format!("{}.{}", prev, last.lower);
            t.truncate(n - 2);
            return Some(m);
        }
    }
    for k in [3usize, 1] {
        if n > k && date_len(&t[n - k..]) == Some(k) {
            let m = t[n - k..].iter().map(|x| x.lower.as_str()).collect::<Vec<_>>().join("-");
            t.truncate(n - k);
            return Some(m);
        }
    }
    None
}

/// Strip one marker from the start of `t`.
fn strip_start(t: &mut Vec<Token>) -> Option<String> {
    if t.len() < 2 {
        return None;
    }
    if t.len() >= 3 && t[0].lower == "copy" && t[1].lower == "of" {
        t.drain(..2);
        return Some("copy of".into());
    }
    if STATUS_WORDS.contains(&t[0].lower.as_str()) {
        return Some(t.remove(0).lower);
    }
    if let Some(k) = date_len(t) {
        if t.len() > k {
            let m = t[..k].iter().map(|x| x.lower.as_str()).collect::<Vec<_>>().join("-");
            t.drain(..k);
            return Some(m);
        }
    }
    None
}

fn is_initials(tok: &Token) -> bool {
    (2..=3).contains(&tok.text.len()) && tok.text.bytes().all(|b| b.is_ascii_uppercase()) && !tok.paren
}

/// Parse a file stem into base tokens and version markers.
pub fn parse_stem(stem: &str) -> ParsedName {
    let mut t = tokenize(stem);
    let mut markers = Vec::new();
    // A trailing initials token, tentatively removed; kept only if another marker exists.
    let initials = if t.len() >= 2 && is_initials(&t[t.len() - 1]) { t.pop() } else { None };
    loop {
        if let Some(m) = strip_end(&mut t) {
            markers.push(m);
            continue;
        }
        if let Some(m) = strip_start(&mut t) {
            markers.push(m);
            continue;
        }
        break;
    }
    match initials {
        Some(i) if !markers.is_empty() => markers.push(i.text),
        Some(_) => {
            // No other marker (so none was stripped): the suffix is part of the name.
            return ParsedName { base: tokenize(stem).into_iter().map(|x| x.lower).collect(), markers };
        }
        None => {}
    }
    ParsedName { base: t.into_iter().map(|x| x.lower).collect(), markers }
}

/// Extension family: Office kinds share one; everything else is its own extension.
pub fn family(ext: &str) -> &str {
    match ext {
        "doc" | "docx" | "docm" | "dot" | "dotx" | "rtf" | "odt" => "word",
        "xls" | "xlsx" | "xlsm" | "xlsb" | "ods" => "excel",
        "ppt" | "pptx" | "pptm" | "pps" | "ppsx" | "odp" => "powerpoint",
        "jpg" | "jpeg" | "jpe" => "jpeg",
        "tif" | "tiff" => "tiff",
        "htm" | "html" => "html",
        other => other,
    }
}

fn stem_and_ext(e: &FileEntry) -> Option<(String, String)> {
    let stem = e.path.file_stem()?.to_string_lossy().into_owned();
    Some((stem, e.extension()))
}

fn levenshtein_is_one(a: &str, b: &str) -> bool {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > 1 || a == b {
        return false;
    }
    let (s, l) = if a.len() <= b.len() { (&a, &b) } else { (&b, &a) };
    let mut i = 0;
    while i < s.len() && s[i] == l[i] {
        i += 1;
    }
    if s.len() == l.len() {
        s[i + 1..] == l[i + 1..]
    } else {
        s[i..] == l[i + 1..]
    }
}

/// True when two bases are a one-letter typo apart (see module docs).
pub fn fuzzy_match(a: &[String], b: &[String]) -> bool {
    if a.len() != b.len() || a == b {
        return false;
    }
    let diffs: Vec<usize> = (0..a.len()).filter(|&i| a[i] != b[i]).collect();
    if diffs.len() != 1 {
        return false;
    }
    let (x, y) = (&a[diffs[0]], &b[diffs[0]]);
    let ok = |s: &str| s.chars().count() >= 5 && !s.chars().any(|c| c.is_numeric());
    ok(x) && ok(y) && levenshtein_is_one(x, y)
}

/// Find chains among `files` (any order). Chains are sorted by folder then base;
/// members by path.
pub fn find_chains(files: &[&FileEntry], fuzzy: bool, notes: &mut Vec<String>) -> Vec<Vec<Member>> {
    // (folder key, family) -> base key -> members
    type Bases = BTreeMap<String, (Vec<String>, Vec<Member>)>;
    let mut folders: BTreeMap<(String, String), Bases> = BTreeMap::new();
    for (i, e) in files.iter().enumerate() {
        if !e.is_file() {
            continue;
        }
        let Some((stem, ext)) = stem_and_ext(e) else { continue };
        let parsed = parse_stem(&stem);
        let letters = parsed.base.iter().map(|t| t.chars().filter(|c| c.is_alphanumeric()).count()).sum::<usize>();
        if letters < 2 {
            continue;
        }
        let folder = e.path.parent().map(mj_core::pathutil::path_key).unwrap_or_default();
        let slot = folders.entry((folder, family(&ext).to_string())).or_default();
        let entry = slot.entry(parsed.key()).or_insert_with(|| (parsed.base.clone(), Vec::new()));
        entry.1.push(Member { index: i, markers: parsed.markers });
    }
    let mut chains = Vec::new();
    for ((folder, _), bases) in folders {
        let list: Vec<(Vec<String>, Vec<Member>)> = bases.into_values().collect();
        let mut uf = UnionFind::new(list.len());
        if fuzzy && list.len() > 1 {
            if list.len() > FUZZY_LIMIT {
                notes.push(format!("Version chains: fuzzy name matching skipped in a folder with {} names ({folder})", list.len()));
            } else {
                let marked = |m: &[Member]| m.iter().any(|x| !x.markers.is_empty());
                for i in 0..list.len() {
                    for j in i + 1..list.len() {
                        if marked(&list[i].1) && marked(&list[j].1) && fuzzy_match(&list[i].0, &list[j].0) {
                            uf.union(i, j);
                        }
                    }
                }
            }
        }
        let mut merged: BTreeMap<usize, Vec<Member>> = BTreeMap::new();
        for (i, (_, members)) in list.into_iter().enumerate() {
            merged.entry(uf.find(i)).or_default().extend(members);
        }
        for (_, mut members) in merged {
            if members.len() >= 2 {
                members.sort_by(|a, b| files[a.index].path.cmp(&files[b.index].path));
                chains.push(members);
            }
        }
    }
    chains
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &str) -> String {
        parse_stem(s).key()
    }

    #[test]
    fn markers_are_removed() {
        for (name, base) in [
            ("Client Proposal v1", "client proposal"),
            ("Client_Proposal_v2", "client proposal"),
            ("client-proposal_V10", "client proposal"),
            ("Client Proposal_final", "client proposal"),
            ("Client Proposal_final_v2", "client proposal"),
            ("Client Proposal FINAL FINAL", "client proposal"),
            ("Client Proposal (1)", "client proposal"),
            ("Client Proposal - Copy", "client proposal"),
            ("Client Proposal - Copy (2)", "client proposal"),
            ("Copy of Client Proposal", "client proposal"),
            ("2024-05-01 Client Proposal", "client proposal"),
            ("20240501_Client Proposal", "client proposal"),
            ("Client Proposal 2024.05.01", "client proposal"),
            ("Client Proposal_01-05-2024", "client proposal"),
            ("Client Proposal_v2_JD", "client proposal"),
            ("Client Proposal v2.1", "client proposal"),
            ("Client Proposal version 3", "client proposal"),
            ("Client Proposal rev 4", "client proposal"),
            ("Client Proposal_draft", "client proposal"),
            ("DRAFT - Client Proposal", "client proposal"),
        ] {
            assert_eq!(key(name), base, "{name}");
        }
        assert_eq!(parse_stem("Deck_v3_final_JD").markers, vec!["final", "v3", "JD"]);
    }

    #[test]
    fn semantic_differences_are_kept() {
        for (a, b) in [
            ("Q1 Report", "Q2 Report"),
            ("Budget 2023", "Budget 2024"),
            ("Budget_2023_v2", "Budget_2024_v1"),
            ("Report_EU", "Report_US"),
            ("Report_EU_v2", "Report_US_v2"),
            ("Monthly report 2024-05", "Monthly report 2024-06"),
            ("Part 1", "Part 2"),
            ("Proposal_2", "Proposal"),
            ("Invoice 240501", "Invoice 240502"),
            ("Meeting notes Anna_v1", "Meeting notes Anne_v2"),
        ] {
            assert_ne!(key(a), key(b), "{a} vs {b}");
            assert!(!fuzzy_match(&parse_stem(a).base, &parse_stem(b).base), "{a} vs {b}");
        }
        // A suffix without another marker is part of the name.
        assert_eq!(parse_stem("Report_EU").markers, Vec::<String>::new());
        assert_eq!(key("Report_EU"), "report eu");
    }

    #[test]
    fn fuzzy_tolerates_one_letter_typos_only() {
        let b = |s: &str| parse_stem(s).base;
        assert!(fuzzy_match(&b("Client Propsal_v1"), &b("Client Proposal_v2")));
        assert!(fuzzy_match(&b("Strategy Reviw v1"), &b("Strategy Review v2")));
        assert!(!fuzzy_match(&b("Client Proposal"), &b("Client Proposal")));
        assert!(!fuzzy_match(&b("Q1 Reprot"), &b("Q2 Report")), "two tokens differ");
        assert!(!fuzzy_match(&b("Plan A1234"), &b("Plan A1235")), "digits never fuzz");
        assert!(!fuzzy_match(&b("Client Prop"), &b("Client Prob")), "short tokens");
        assert!(levenshtein_is_one("abcde", "abde") && levenshtein_is_one("abcde", "abXde"));
        assert!(!levenshtein_is_one("abcde", "edcba"));
    }

    #[test]
    fn families() {
        assert_eq!(family("docx"), family("doc"));
        assert_eq!(family("xlsx"), "excel");
        assert_ne!(family("pdf"), family("docx"));
        assert_eq!(family("txt"), "txt");
    }
}
