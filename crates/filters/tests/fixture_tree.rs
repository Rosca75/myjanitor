//! Integration test: a generated fixture tree on disk, indexed by a tiny
//! walker, queried through the quick-query parser, presets, content search,
//! mismatch detection, exclusions, topmost and `top:N` together.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::SystemTime;

use mj_core::testutil::{set_mtime, TempDir};
use mj_core::{EntryKind, FileEntry, FileIndex, Rule, ScanStats, Timestamp};
use mj_filters::{parse, presets, select, select_with, Exclusions, FilterContext, SelectOptions};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR....";
const JPG: &[u8] = b"\xFF\xD8\xFF\xE0\0\x10JFIF....";

fn now() -> Timestamp {
    Timestamp::from_ymd_hms(2026, 11, 13, 12, 0, 0)
}

struct Fixture {
    dir: TempDir,
    index: FileIndex,
    groups: BTreeMap<String, Vec<String>>,
}

fn walk(dir: &Path, root_depth: usize, out: &mut Vec<FileEntry>) {
    for e in fs::read_dir(dir).unwrap() {
        let e = e.unwrap();
        let md = e.metadata().unwrap();
        let kind = if md.is_dir() { EntryKind::Dir } else { EntryKind::File };
        let mut fe = FileEntry::new(e.path(), kind, if md.is_dir() { 0 } else { md.len() });
        fe.modified = md.modified().ok().map(Timestamp::from_system_time);
        fe.depth = (e.path().components().count() - root_depth) as u32;
        let is_dir = md.is_dir();
        out.push(fe);
        if is_dir {
            walk(&e.path(), root_depth, out);
        }
    }
}

fn build() -> Fixture {
    let d = TempDir::new("filters-fixture").unwrap();
    let w = |rel: &str, bytes: &[u8], mtime: Option<Timestamp>| {
        let p = d.write(rel, bytes).unwrap();
        if let Some(t) = mtime {
            set_mtime(&p, t.to_system_time()).unwrap();
        }
    };
    let old = Some(Timestamp::from_ymd_hms(2023, 1, 1, 0, 0, 0));
    let recent = Some(Timestamp::from_ymd_hms(2026, 11, 1, 0, 0, 0));
    w("Downloads/setup.exe", b"MZ\x90\0\x03\0\0\0 padding", old);
    w("Downloads/tool.msi", &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0], old);
    w("Downloads/fresh.zip", b"PK\x03\x04 stuff", recent);
    w("Photos/a.jpg", JPG, old);
    w("Photos/b.png", PNG, recent);
    w("Photos/fake.jpg", b"<html><body>404 not found</body></html>", old);
    w("Docs/notes.txt", b"Meeting notes about Invoice 42", old);
    w("Docs/todo.txt", b"buy milk", recent);
    w("Docs/report.pdf", b"%PDF-1.7 Invoice inside a pdf", old);
    w("Docs/empty.txt", b"", old);
    w("proj/node_modules/pkg/index.js", b"module.exports = 1;", old);
    w("proj/node_modules/pkg/big.bin", &vec![7u8; 5000], old);
    w("proj/src/main.rs", b"fn main() {}", old);
    w("proj/__pycache__/x.pyc", b"\x00\x01", old);
    let mut entries = Vec::new();
    walk(d.path(), d.path().components().count(), &mut entries);
    let index = FileIndex::new(vec![d.path().to_path_buf()], entries, ScanStats::default(), now());
    Fixture { dir: d, index, groups: mj_filters::groups::builtin() }
}

impl Fixture {
    fn ctx(&self) -> FilterContext<'_> {
        FilterContext { now: now(), type_groups: &self.groups }
    }

    fn names(&self, rule: &Rule, topmost: bool) -> Vec<String> {
        let root = self.dir.path();
        select(&self.index, rule, &self.ctx(), topmost)
            .unwrap()
            .into_iter()
            .map(|i| {
                let p = self.index.entry(i).path.strip_prefix(root).unwrap().to_path_buf();
                p.to_string_lossy().replace('\\', "/")
            })
            .collect()
    }

    fn q(&self, query: &str) -> Vec<String> {
        self.names(&parse(query).unwrap_or_else(|e| panic!("{query}: {e:?}")), false)
    }
}

#[test]
fn metadata_filters_combine_through_the_query_parser() {
    let f = build();
    // Old installers in Downloads: extension + path + age.
    assert_eq!(f.q("type:installers path:*Downloads* older:90d"), ["Downloads/setup.exe", "Downloads/tool.msi"]);
    // OR, grouping and negation.
    assert_eq!(f.q("(ext:jpg OR ext:png) -name:fake*"), ["Photos/a.jpg", "Photos/b.png"]);
    // Dates: modified in 2023, not a directory, bigger than 1 byte.
    assert_eq!(f.q("modified:2023 kind:file size:>1 ext:txt,pdf"), ["Docs/notes.txt", "Docs/report.pdf"]);
    // Zero-byte and depth.
    assert_eq!(f.q("is:empty"), ["Docs/empty.txt"]);
    assert_eq!(f.q("depth:>=3 ext:js,bin"), ["proj/node_modules/pkg/big.bin", "proj/node_modules/pkg/index.js"]);
    // Regex and a size range.
    assert_eq!(f.q(r"name:/^(a|b)\.(jpg|png)$/"), ["Photos/a.jpg", "Photos/b.png"]);
    assert_eq!(f.q("size:1KB..10KB"), ["proj/node_modules/pkg/big.bin"]);
}

#[test]
fn true_type_versus_extension() {
    let f = build();
    // fake.jpg is HTML; the empty txt is not judged; .txt files are not checked at all.
    assert_eq!(f.q("is:mismatch"), ["Photos/fake.jpg"]);
    let rule = parse("is:mismatch").unwrap();
    let sel = select_with(&f.index, &rule, &f.ctx(), SelectOptions::default()).unwrap();
    assert_eq!(sel.diagnostics.header_reads, 7, "only non-empty files with checked extensions are opened");
    assert_eq!(sel.diagnostics.header_unreadable, 0);
    // Cheap filters first: restricting to .jpg means only two headers are read.
    let rule = parse("ext:jpg is:mismatch").unwrap();
    let sel = select_with(&f.index, &rule, &f.ctx(), SelectOptions::default()).unwrap();
    assert_eq!(sel.diagnostics.header_reads, 2);
    assert_eq!(sel.positions.len(), 1);
}

#[test]
fn content_search_reports_what_it_could_not_read() {
    let f = build();
    let rule = parse("content:invoice").unwrap();
    let sel = select_with(&f.index, &rule, &f.ctx(), SelectOptions::default()).unwrap();
    let hit: Vec<_> = sel.positions.iter().map(|&i| f.index.entry(i).name()).collect();
    assert_eq!(hit, ["notes.txt"], "the PDF has no extractor and is not reported as a match");
    assert_eq!(sel.diagnostics.content_unavailable.get("pdf"), Some(&1));
    let core = sel.diagnostics.to_core();
    assert!(core.notes.iter().any(|n| n.contains("pdf")));
    // "Files without the word": the unsearched PDF must not appear either.
    let rule = parse("ext:txt,pdf -content:invoice").unwrap();
    let names = f.names(&rule, false);
    assert_eq!(
        names,
        ["Docs/empty.txt", "Docs/todo.txt"],
        "an empty file really lacks the word; the unsearched PDF is unknown, not a match"
    );
}

#[test]
fn presets_and_topmost() {
    let f = build();
    let dev = presets::builtin().into_iter().find(|p| p.id == "dev_junk").unwrap();
    assert_eq!(f.names(&dev.rule, dev.topmost_only), ["proj/__pycache__", "proj/node_modules"]);
    let all = f.names(&dev.rule, false);
    assert!(all.contains(&"proj/__pycache__/x.pyc".to_string()), "without topmost the child is listed too");
    let old = presets::builtin().into_iter().find(|p| p.id == "old_installers").unwrap();
    assert_eq!(f.names(&old.rule, false), ["Downloads/setup.exe", "Downloads/tool.msi"]);
    let empty = presets::builtin().into_iter().find(|p| p.id == "zero_byte_files").unwrap();
    assert_eq!(f.names(&empty.rule, false), ["Docs/empty.txt"]);
}

#[test]
fn exclusions_and_largest_n() {
    let f = build();
    let rule = parse("kind:file top:2").unwrap();
    let top2 = f.names(&rule, false);
    assert_eq!(top2.len(), 2);
    assert!(top2.contains(&"proj/node_modules/pkg/big.bin".to_string()));
    // Excluding node_modules removes it before the limiter runs, so another file takes the slot.
    let ex = Exclusions::from_entries(&[f.dir.join("proj/node_modules").to_string_lossy().as_ref(), "**/*.pyc"]);
    let sel =
        select_with(&f.index, &rule, &f.ctx(), SelectOptions { exclusions: Some(&ex), ..SelectOptions::default() })
            .unwrap();
    let names: Vec<_> = sel.positions.iter().map(|&i| f.index.entry(i).name()).collect();
    assert!(!names.contains(&"big.bin".to_string()) && !names.contains(&"x.pyc".to_string()), "{names:?}");
    assert_eq!(names.len(), 2);
    assert!(sel.diagnostics.excluded >= 4);
}

#[test]
fn a_compiled_rule_matches_a_foreign_entry_too() {
    // `compile` + `matches` work on any FileEntry, e.g. the one the UI is hovering.
    let f = build();
    let rule = parse("ext:mp4 size:>1GB").unwrap();
    let c = mj_filters::compile(&rule, &f.ctx()).unwrap();
    assert!(c.matches(&FileEntry::new("/elsewhere/a.mp4", EntryKind::File, 2 << 30)));
    assert!(!c.matches(&FileEntry::new("/elsewhere/a.mp4", EntryKind::File, 5)));
    let _ = SystemTime::now();
}
