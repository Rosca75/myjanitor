//! Unit tests of the rule engine: one per condition, tree semantics, evaluation
//! order, `LargestN`, topmost, exclusions, cancel and validation.

use super::*;
use mj_core::time::{RelativeSpan, SpanUnit};
use mj_core::{ScanStats, Timestamp};

fn now() -> Timestamp {
    Timestamp::from_ymd_hms(2026, 11, 13, 12, 0, 0)
}

fn groups() -> BTreeMap<String, Vec<String>> {
    crate::groups::builtin()
}

fn matches(rule: &Rule, e: &FileEntry) -> bool {
    let g = groups();
    let ctx = FilterContext { now: now(), type_groups: &g };
    compile(rule, &ctx).unwrap().matches(e)
}

fn file(path: &str, size: u64) -> FileEntry {
    FileEntry::new(path, EntryKind::File, size)
}

fn cond(c: Condition) -> Rule {
    Rule::Cond(c)
}

fn name(mode: TextMode, p: &str, cs: bool) -> Rule {
    cond(Condition::Name { mode, pattern: p.into(), case_sensitive: cs })
}

fn date(which: DateField, cmp: Cmp, value: DateValue) -> Rule {
    cond(Condition::Date { which, cmp, value })
}

fn ago(amount: u32, unit: SpanUnit) -> DateValue {
    DateValue::Ago(RelativeSpan { amount, unit })
}

fn index_of(entries: Vec<FileEntry>) -> FileIndex {
    FileIndex::new(vec!["/r".into()], entries, ScanStats::default(), Timestamp::default())
}

fn picked(index: &FileIndex, pos: &[usize]) -> Vec<String> {
    pos.iter().map(|&i| index.entry(i).path.to_string_lossy().into_owned()).collect()
}

#[test]
fn name_and_path_conditions() {
    let e = file("/r/Photos/IMG_0042.JPG", 10);
    assert!(matches(&name(TextMode::Wildcard, "*.jpg", false), &e));
    assert!(!matches(&name(TextMode::Wildcard, "*.jpg", true), &e));
    assert!(matches(&name(TextMode::Contains, "img_00", false), &e));
    assert!(matches(&name(TextMode::Exact, "img_0042.jpg", false), &e));
    assert!(matches(&name(TextMode::Regex, r"^IMG_\d{4}\.JPG$", true), &e));
    assert!(!matches(&name(TextMode::Regex, r"^img_\d{4}\.jpg$", true), &e));
    let p = |mode, pat: &str| cond(Condition::Path { mode, pattern: pat.into(), case_sensitive: false });
    assert!(matches(&p(TextMode::Wildcard, "**/photos/*.jpg"), &e));
    assert!(matches(&p(TextMode::Contains, "/photos/"), &e));
    assert!(!matches(&p(TextMode::Exact, "/r/Photos"), &e));
    assert!(matches(&p(TextMode::Regex, r"photos/img_\d+"), &e));
}

#[test]
fn extension_and_type_groups_apply_to_files_only() {
    let ext = cond(Condition::Extension { values: vec![".MP4".into(), "mkv".into()] });
    assert!(matches(&ext, &file("/r/a.Mp4", 1)));
    assert!(!matches(&ext, &file("/r/a.avi", 1)));
    assert!(!matches(&ext, &FileEntry::new("/r/dir.mp4", EntryKind::Dir, 0)), "a folder named x.mp4 is not a video");
    let tg = cond(Condition::TypeGroup { group: "Video".into() });
    assert!(matches(&tg, &file("/r/a.mkv", 1)));
    assert!(!matches(&tg, &file("/r/a.txt", 1)));
    // A user override replaces the built-in group.
    let mut g = groups();
    g.insert("video".into(), vec!["xyz".into()]);
    let ctx = FilterContext { now: now(), type_groups: &g };
    let c = compile(&tg, &ctx).unwrap();
    assert!(c.matches(&file("/r/a.xyz", 1)) && !c.matches(&file("/r/a.mkv", 1)));
}

#[test]
fn size_zero_byte() {
    let big = file("/r/a", 2 << 30);
    assert!(matches(&cond(Condition::Size { cmp: Cmp::Gt, bytes: 1 << 30 }), &big));
    assert!(matches(&cond(Condition::Size { cmp: Cmp::Le, bytes: 2 << 30 }), &big));
    assert!(!matches(&cond(Condition::Size { cmp: Cmp::Lt, bytes: 1 }), &big));
    assert!(matches(&cond(Condition::ZeroByte), &file("/r/e", 0)));
    assert!(!matches(&cond(Condition::ZeroByte), &big));
    assert!(!matches(&cond(Condition::ZeroByte), &FileEntry::new("/r/d", EntryKind::Dir, 0)));
}

#[test]
fn dates_absolute_relative_and_missing() {
    let mut e = file("/r/a", 1);
    e.modified = Some(Timestamp::from_ymd_hms(2024, 6, 1, 0, 0, 0));
    let before_2025 = date(DateField::Modified, Cmp::Lt, DateValue::At(Timestamp::from_ymd_hms(2025, 1, 1, 0, 0, 0)));
    assert!(matches(&before_2025, &e));
    // now = 2026-11-13: two years ago = 2024-11-13, so 2024-06 is older.
    assert!(matches(&date(DateField::Modified, Cmp::Lt, ago(2, SpanUnit::Years)), &e));
    assert!(!matches(&date(DateField::Modified, Cmp::Lt, ago(3, SpanUnit::Years)), &e));
    assert!(matches(&date(DateField::Modified, Cmp::Ge, ago(90, SpanUnit::Weeks)), &{
        let mut r = e.clone();
        r.modified = Some(Timestamp::from_ymd_hms(2026, 10, 1, 0, 0, 0));
        r
    }));
    // Missing timestamps never satisfy a date condition, in any direction.
    for cmp in [Cmp::Lt, Cmp::Le, Cmp::Eq, Cmp::Ge, Cmp::Gt] {
        assert!(!matches(&date(DateField::Created, cmp, ago(1, SpanUnit::Days)), &e));
    }
    e.accessed = Some(Timestamp::from_ymd_hms(2020, 1, 1, 0, 0, 0));
    assert!(matches(&date(DateField::Accessed, Cmp::Lt, ago(1, SpanUnit::Years)), &e));
}

#[test]
fn attributes_are_tri_state() {
    let mut e = file("/r/a", 1);
    e.attrs = Attributes::HIDDEN | Attributes::READONLY;
    let a = |attr, set| cond(Condition::Attribute { attr, set });
    assert!(matches(&a(Attributes::HIDDEN, true), &e));
    assert!(!matches(&a(Attributes::HIDDEN, false), &e));
    assert!(matches(&a(Attributes::SYSTEM, false), &e));
    assert!(!matches(&a(Attributes::SYSTEM, true), &e));
    assert!(matches(&a(Attributes::HIDDEN | Attributes::READONLY, true), &e), "a set of attributes requires all");
    assert!(!matches(&a(Attributes::HIDDEN | Attributes::SYSTEM, true), &e));
    assert!(!matches(&a(Attributes::HIDDEN | Attributes::SYSTEM, false), &e), "clear means none of them set");
}

#[test]
fn owner_depth_path_length_kind_and_invalid_names() {
    let mut e = file("/r/a/b/c.txt", 1);
    e.owner = Some("Alice".into());
    e.depth = 3;
    assert!(matches(&cond(Condition::Owner { value: "alice".into() }), &e));
    assert!(!matches(&cond(Condition::Owner { value: "bob".into() }), &e));
    assert!(!matches(&cond(Condition::Owner { value: "alice".into() }), &file("/r/x", 1)), "no owner never matches");
    assert!(matches(&cond(Condition::Depth { cmp: Cmp::Ge, value: 3 }), &e));
    assert!(!matches(&cond(Condition::Depth { cmp: Cmp::Gt, value: 3 }), &e));
    assert!(matches(&cond(Condition::PathLength { cmp: Cmp::Eq, value: 12 }), &e));
    let long = file(&format!("/r/{}", "x".repeat(300)), 1);
    assert!(matches(&cond(Condition::PathLength { cmp: Cmp::Gt, value: 260 }), &long));
    assert!(matches(&cond(Condition::Kind { kind: EntryKind::File }), &e));
    assert!(!matches(&cond(Condition::Kind { kind: EntryKind::Dir }), &e));
    for bad in
        ["/r/CON", "/r/nul.txt", "/r/a:b.txt", "/r/what?.txt", "/r/trailing.", "/r/trailing ", "/r/q\"x", "/r/a|b"]
    {
        assert!(matches(&cond(Condition::InvalidName), &file(bad, 1)), "{bad}");
    }
    for ok in ["/r/console.txt", "/r/a.b.c", "/r/.hidden", "/r/normal name.txt"] {
        assert!(!matches(&cond(Condition::InvalidName), &file(ok, 1)), "{ok}");
    }
}

#[test]
fn tree_semantics_and_or_not() {
    let e = file("/r/a.mp4", 100);
    let t = || cond(Condition::Extension { values: vec!["mp4".into()] });
    let f = || cond(Condition::Extension { values: vec!["avi".into()] });
    assert!(matches(&Rule::All(vec![]), &e), "all([]) matches everything");
    assert!(!matches(&Rule::Any(vec![]), &e), "any([]) matches nothing");
    assert!(matches(&Rule::All(vec![t(), t()]), &e));
    assert!(!matches(&Rule::All(vec![t(), f()]), &e));
    assert!(matches(&Rule::Any(vec![f(), t()]), &e));
    assert!(!matches(&Rule::Any(vec![f(), f()]), &e));
    assert!(matches(&Rule::Not(Box::new(f())), &e));
    assert!(!matches(&Rule::Not(Box::new(t())), &e));
    // (mp4 AND NOT big) OR avi
    let rule = Rule::Any(vec![
        Rule::All(vec![t(), Rule::Not(Box::new(cond(Condition::Size { cmp: Cmp::Gt, bytes: 1000 })))]),
        f(),
    ]);
    assert!(matches(&rule, &e));
    assert!(!matches(&rule, &file("/r/a.mp4", 5000)));
    assert!(matches(&rule, &file("/r/a.avi", 5000)));
}

#[test]
fn cheap_predicates_run_before_content_reads() {
    let dir = mj_core::testutil::TempDir::new("order").unwrap();
    let mut entries = Vec::new();
    for i in 0..10 {
        let ext = if i < 3 { "txt" } else { "log" };
        let p = dir.write(format!("f{i}.{ext}"), b"hello needle world").unwrap();
        entries.push(FileEntry::new(p, EntryKind::File, 18));
    }
    let g = groups();
    let ctx = FilterContext { now: now(), type_groups: &g };
    // The content condition is listed first, yet only the 3 .txt files are ever read.
    let rule = Rule::All(vec![
        cond(Condition::ContentContains { text: "NEEDLE".into(), case_sensitive: false }),
        cond(Condition::Extension { values: vec!["txt".into()] }),
    ]);
    let c = compile(&rule, &ctx).unwrap();
    assert!(c.reads_files());
    let hits = entries.iter().filter(|e| c.matches(e)).count();
    assert_eq!(hits, 3);
    assert_eq!(c.diagnostics().content_reads, 3, "content is read last, only for survivors");
    // Same for an Any: the cheap side decides first.
    let any = Rule::Any(vec![
        cond(Condition::ContentContains { text: "zzz".into(), case_sensitive: false }),
        cond(Condition::Size { cmp: Cmp::Eq, bytes: 18 }),
    ]);
    let c = compile(&any, &ctx).unwrap();
    assert!(entries.iter().all(|e| c.matches(e)));
    assert_eq!(c.diagnostics().content_reads, 0);
}

#[test]
fn unavailable_content_is_unknown_not_false() {
    let dir = mj_core::testutil::TempDir::new("unknown").unwrap();
    let pdf = dir.write("a.pdf", b"%PDF-1.4 whatever").unwrap();
    let txt = dir.write("a.txt", b"plain words").unwrap();
    let g = groups();
    let ctx = FilterContext { now: now(), type_groups: &g };
    let has = cond(Condition::ContentContains { text: "invoice".into(), case_sensitive: false });
    let negated = Rule::Not(Box::new(has.clone()));
    let c = compile(&negated, &ctx).unwrap();
    let pdf_e = FileEntry::new(pdf, EntryKind::File, 17);
    let txt_e = FileEntry::new(txt, EntryKind::File, 11);
    assert!(c.matches(&txt_e), "searched and absent");
    assert!(!c.matches(&pdf_e), "never searched: must not be reported as 'does not contain'");
    assert!(!compile(&has, &ctx).unwrap().matches(&pdf_e));
    let d = c.diagnostics();
    assert_eq!(d.content_unavailable.get("pdf"), Some(&1));
    assert!(d.to_core().notes[0].contains("pdf"));
}

#[test]
fn online_only_files_are_never_read() {
    let dir = mj_core::testutil::TempDir::new("online").unwrap();
    let p = dir.write("cloud.txt", b"needle").unwrap();
    let mut e = FileEntry::new(p, EntryKind::File, 6);
    e.attrs = Attributes::ONLINE_ONLY;
    let g = groups();
    let ctx = FilterContext { now: now(), type_groups: &g };
    let c = compile(&cond(Condition::ContentContains { text: "needle".into(), case_sensitive: false }), &ctx).unwrap();
    assert!(!c.matches(&e));
    let d = c.diagnostics();
    assert_eq!((d.content_reads, d.skipped_online_only), (0, 1));
}

#[test]
fn validation_errors_are_invalid_input() {
    let g = groups();
    let ctx = FilterContext { now: now(), type_groups: &g };
    let bad = |r: Rule| match compile(&r, &ctx) {
        Err(Error::InvalidInput(m)) => m,
        other => panic!("expected InvalidInput, got {other:?}"),
    };
    assert!(bad(name(TextMode::Regex, "(", false)).contains("invalid regular expression"));
    assert!(bad(name(TextMode::Contains, "", false)).contains("empty"));
    assert!(bad(cond(Condition::TypeGroup { group: "nope".into() })).contains("known groups"));
    assert!(bad(cond(Condition::Extension { values: vec![] })).contains("no extensions"));
    assert!(bad(cond(Condition::Attribute { attr: Attributes::empty(), set: true })).contains("attribute"));
    assert!(bad(cond(Condition::Owner { value: " ".into() })).contains("owner"));
    assert!(bad(cond(Condition::ContentContains { text: String::new(), case_sensitive: false })).contains("text"));
    assert!(bad(date(DateField::Created, Cmp::Lt, ago(100_000, SpanUnit::Years))).contains("date"));
    // LargestN only as a direct child of the top-level all.
    assert!(bad(Rule::Any(vec![cond(Condition::LargestN { n: 5 })])).contains("largest"));
    assert!(bad(Rule::All(vec![Rule::Not(Box::new(cond(Condition::LargestN { n: 5 })))])).contains("largest"));
    assert!(bad(Rule::All(vec![Rule::All(vec![cond(Condition::LargestN { n: 5 })])])).contains("largest"));
    assert!(compile(&Rule::All(vec![cond(Condition::LargestN { n: 5 })]), &ctx).is_ok());
    assert!(compile(&cond(Condition::LargestN { n: 5 }), &ctx).is_ok());
    // A hostile, deeply nested rule is refused instead of overflowing the stack.
    let mut r = Rule::everything();
    for _ in 0..500 {
        r = Rule::Not(Box::new(r));
    }
    assert!(bad(r).contains("nested"));
}

#[test]
fn largest_n_applies_after_the_rest_of_the_tree() {
    let sizes = [("a.mp4", 50u64), ("b.mp4", 500), ("c.mp4", 300), ("d.txt", 9999), ("e.mp4", 400)];
    let index = index_of(sizes.iter().map(|(n, s)| file(&format!("/r/{n}"), *s)).collect());
    let g = groups();
    let ctx = FilterContext { now: now(), type_groups: &g };
    let rule =
        Rule::All(vec![cond(Condition::LargestN { n: 2 }), cond(Condition::Extension { values: vec!["mp4".into()] })]);
    let sel = select(&index, &rule, &ctx, false).unwrap();
    // The 9999-byte .txt is not an mp4, so it does not use up a slot.
    assert_eq!(picked(&index, &sel), ["/r/b.mp4", "/r/e.mp4"]);
    let zero = Rule::All(vec![cond(Condition::LargestN { n: 0 })]);
    assert!(select(&index, &zero, &ctx, false).unwrap().is_empty());
    let more = Rule::All(vec![cond(Condition::LargestN { n: 50 })]);
    assert_eq!(select(&index, &more, &ctx, false).unwrap().len(), 5);
}

fn tree() -> FileIndex {
    let d = |p: &str| FileEntry::new(p, EntryKind::Dir, 0);
    // Sorted keys put `a.d` and `a.txt` between `a` and `a/x` ('.' < '/').
    index_of(vec![
        d("/r/a"),
        d("/r/a.d"),
        file("/r/a.d/x", 5),
        file("/r/a.txt", 5),
        file("/r/a/x", 5),
        d("/r/a/sub"),
        file("/r/a/sub/y", 5),
        d("/r/b"),
        file("/r/b/z", 5),
    ])
}

#[test]
fn topmost_keeps_only_the_highest_matching_folder() {
    let index = tree();
    let g = groups();
    let ctx = FilterContext { now: now(), type_groups: &g };
    let any = Rule::Any(vec![
        cond(Condition::Kind { kind: EntryKind::Dir }),
        cond(Condition::Kind { kind: EntryKind::File }),
    ]);
    let all = select(&index, &any, &ctx, false).unwrap();
    assert_eq!(all.len(), index.len());
    let top = select(&index, &any, &ctx, true).unwrap();
    assert_eq!(picked(&index, &top), ["/r/a", "/r/a.d", "/r/a.txt", "/r/b"]);
    // Only some folders match: folders named "sub" and "b"; files stay where no folder matches above.
    let rule = Rule::Any(vec![
        Rule::All(vec![cond(Condition::Kind { kind: EntryKind::Dir }), name(TextMode::Exact, "sub", false)]),
        name(TextMode::Exact, "y", false),
        name(TextMode::Exact, "x", false),
    ]);
    let top = select(&index, &rule, &ctx, true).unwrap();
    assert_eq!(picked(&index, &top), ["/r/a.d/x", "/r/a/sub", "/r/a/x"]);
    // Files only: nothing to collapse.
    let files = cond(Condition::Kind { kind: EntryKind::File });
    assert_eq!(select(&index, &files, &ctx, true).unwrap().len(), 5);
}

#[test]
fn exclusions_apply_before_the_rule_and_cover_subtrees() {
    let index = tree();
    let g = groups();
    let ctx = FilterContext { now: now(), type_groups: &g };
    let files = cond(Condition::Kind { kind: EntryKind::File });
    let ex = Exclusions::from_entries(&["/r/a", "**/z"]);
    let sel = select_with(&index, &files, &ctx, SelectOptions { exclusions: Some(&ex), ..Default::default() }).unwrap();
    assert_eq!(picked(&index, &sel.positions), ["/r/a.d/x", "/r/a.txt"]);
    assert_eq!(sel.diagnostics.excluded, 5, "a, a/x, a/sub, a/sub/y and b/z");
    // Pattern excluding a directory name also excludes everything inside it.
    let ex = Exclusions::from_entries(&["**/sub"]);
    let sel = select_with(&index, &files, &ctx, SelectOptions { exclusions: Some(&ex), ..Default::default() }).unwrap();
    assert!(!picked(&index, &sel.positions).contains(&"/r/a/sub/y".to_string()));
    assert_eq!(sel.positions.len(), 4);
}

#[test]
fn cancel_stops_evaluation() {
    let index = tree();
    let g = groups();
    let ctx = FilterContext { now: now(), type_groups: &g };
    let token = CancelToken::new();
    token.cancel();
    let r =
        select_with(&index, &Rule::everything(), &ctx, SelectOptions { cancel: Some(&token), ..Default::default() });
    assert!(matches!(r, Err(Error::Cancelled)));
    let ok = CancelToken::new();
    let r = select_with(&index, &Rule::everything(), &ctx, SelectOptions { cancel: Some(&ok), ..Default::default() });
    assert_eq!(r.unwrap().positions.len(), index.len());
}

#[test]
fn compiled_rule_is_shareable_across_threads() {
    fn assert_sync<T: Send + Sync>() {}
    assert_sync::<CompiledRule>();
    assert_sync::<Exclusions>();
}
