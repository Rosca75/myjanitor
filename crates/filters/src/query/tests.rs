//! Query parser tests: every key, units, dates, errors with positions,
//! round-trip into the rule tree and a deterministic fuzz.

use super::*;
use mj_core::time::{RelativeSpan, SpanUnit};
use mj_core::{Attributes, Cmp, Condition, DateField, DateValue, EntryKind, TextMode, Timestamp};

fn c(c: Condition) -> Rule {
    Rule::Cond(c)
}

fn ok(q: &str) -> Rule {
    parse(q).unwrap_or_else(|e| panic!("{q:?} should parse: {e:?}"))
}

fn errs(q: &str) -> Vec<QueryError> {
    parse(q).expect_err(&format!("{q:?} should fail"))
}

fn name(mode: TextMode, p: &str) -> Rule {
    c(Condition::Name { mode, pattern: p.into(), case_sensitive: false })
}

fn path(mode: TextMode, p: &str) -> Rule {
    c(Condition::Path { mode, pattern: p.into(), case_sensitive: false })
}

fn date(which: DateField, cmp: Cmp, value: DateValue) -> Rule {
    c(Condition::Date { which, cmp, value })
}

fn ts(y: i32, m: u32, d: u32) -> DateValue {
    DateValue::At(Timestamp::from_ymd_hms(y, m, d, 0, 0, 0))
}

fn ago(amount: u32, unit: SpanUnit) -> DateValue {
    DateValue::Ago(RelativeSpan { amount, unit })
}

#[test]
fn the_documented_example_round_trips_to_a_rule_tree() {
    let r = ok("ext:mp4 size:>1GB modified:<2024");
    assert_eq!(
        r,
        Rule::All(vec![
            c(Condition::Extension { values: vec!["mp4".into()] }),
            c(Condition::Size { cmp: Cmp::Gt, bytes: 1 << 30 }),
            date(DateField::Modified, Cmp::Lt, ts(2024, 1, 1)),
        ])
    );
    // And the tree survives JSON, the IPC transport.
    let json = serde_json::to_string(&r).unwrap();
    assert_eq!(serde_json::from_str::<Rule>(&json).unwrap(), r);
}

#[test]
fn ext_name_path_regex() {
    assert_eq!(
        ok("ext:jpg,.PNG,*.gif"),
        c(Condition::Extension { values: vec!["jpg".into(), "png".into(), "gif".into()] })
    );
    assert_eq!(ok("name:*.tmp"), name(TextMode::Wildcard, "*.tmp"));
    assert_eq!(ok("name:final"), name(TextMode::Contains, "final"));
    assert_eq!(ok("name:=Final.doc"), name(TextMode::Exact, "Final.doc"));
    assert_eq!(ok(r#"name:"final v2*""#), name(TextMode::Wildcard, "final v2*"));
    assert_eq!(ok(r"name:/^img_\d+/"), name(TextMode::Regex, r"^img_\d+"));
    assert_eq!(ok(r"regex:^img_\d+"), name(TextMode::Regex, r"^img_\d+"));
    assert_eq!(ok(r"regex:/a (b|c)/"), name(TextMode::Regex, "a (b|c)"));
    assert_eq!(
        ok("name:*.tmp,*.bak"),
        Rule::Any(vec![name(TextMode::Wildcard, "*.tmp"), name(TextMode::Wildcard, "*.bak")])
    );
    assert_eq!(ok("path:*/Photos/*"), path(TextMode::Wildcard, "*/Photos/*"));
    assert_eq!(ok("path:*Photos*"), path(TextMode::Wildcard, "**Photos**"));
    assert_eq!(ok("path:*.tmp"), path(TextMode::Wildcard, "**.tmp"));
    assert_eq!(ok("path:Photos"), path(TextMode::Contains, "Photos"));
    assert_eq!(ok("path:/home/user/docs/"), path(TextMode::Contains, "/home/user/docs/"));
    assert_eq!(ok(r"path:/Photos\d+/"), path(TextMode::Regex, r"Photos\d+"));
    assert_eq!(ok(r"pathregex:Photos\d+"), path(TextMode::Regex, r"Photos\d+"));
    // Free words are name conditions.
    assert_eq!(ok("invoice"), name(TextMode::Contains, "invoice"));
    assert_eq!(ok("*.tmp"), name(TextMode::Wildcard, "*.tmp"));
    assert_eq!(ok(r#""my file""#), name(TextMode::Contains, "my file"));
    assert_eq!(ok(r"/^a.*z$/"), name(TextMode::Regex, "^a.*z$"));
    assert_eq!(ok("or"), name(TextMode::Contains, "or"), "lower case keywords are words");
}

#[test]
fn sizes() {
    let sz = |cmp, bytes| c(Condition::Size { cmp, bytes });
    assert_eq!(ok("size:>1GB"), sz(Cmp::Gt, 1 << 30));
    assert_eq!(ok("size:<=500k"), sz(Cmp::Le, 500 << 10));
    assert_eq!(ok("size:>=1.5MB"), sz(Cmp::Ge, 3 << 19));
    assert_eq!(ok("size:2TB"), sz(Cmp::Eq, 2 << 40));
    assert_eq!(ok("size:=0"), c(Condition::ZeroByte));
    assert_eq!(ok("size:0"), c(Condition::ZeroByte));
    assert_eq!(ok("size:<1"), sz(Cmp::Lt, 1));
    assert_eq!(ok("size:1MB..10MB"), Rule::All(vec![sz(Cmp::Ge, 1 << 20), sz(Cmp::Le, 10 << 20)]));
    assert_eq!(ok("size:1MB,2MB"), Rule::Any(vec![sz(Cmp::Eq, 1 << 20), sz(Cmp::Eq, 2 << 20)]));
    assert!(parse("size:>1GB,<1KB").is_err(), "one operator per term");
}

#[test]
fn dates_absolute() {
    let m = DateField::Modified;
    assert_eq!(ok("modified:<2024"), date(m, Cmp::Lt, ts(2024, 1, 1)));
    assert_eq!(ok("modified:<=2024"), date(m, Cmp::Lt, ts(2025, 1, 1)));
    assert_eq!(ok("modified:>2024"), date(m, Cmp::Ge, ts(2025, 1, 1)));
    assert_eq!(ok("modified:>=2024-03"), date(m, Cmp::Ge, ts(2024, 3, 1)));
    assert_eq!(ok("created:<2024-03-15"), date(DateField::Created, Cmp::Lt, ts(2024, 3, 15)));
    assert_eq!(
        ok("accessed:2024-03-15"),
        Rule::All(vec![
            date(DateField::Accessed, Cmp::Ge, ts(2024, 3, 15)),
            date(DateField::Accessed, Cmp::Lt, ts(2024, 3, 16)),
        ])
    );
    assert_eq!(
        ok("modified:2024"),
        Rule::All(vec![date(m, Cmp::Ge, ts(2024, 1, 1)), date(m, Cmp::Lt, ts(2025, 1, 1))])
    );
    assert_eq!(
        ok("modified:2023..2024"),
        Rule::All(vec![date(m, Cmp::Ge, ts(2023, 1, 1)), date(m, Cmp::Lt, ts(2025, 1, 1))])
    );
    assert_eq!(
        ok("modified:2024-12"),
        Rule::All(vec![date(m, Cmp::Ge, ts(2024, 12, 1)), date(m, Cmp::Lt, ts(2025, 1, 1))])
    );
}

#[test]
fn dates_relative_are_ages() {
    let m = DateField::Modified;
    assert_eq!(ok("modified:>2y"), date(m, Cmp::Lt, ago(2, SpanUnit::Years)), "older than two years");
    assert_eq!(ok("modified:>=6m"), date(m, Cmp::Le, ago(6, SpanUnit::Months)));
    assert_eq!(ok("modified:<30d"), date(m, Cmp::Gt, ago(30, SpanUnit::Days)), "newer than 30 days");
    assert_eq!(ok("modified:30d"), date(m, Cmp::Ge, ago(30, SpanUnit::Days)), "within the last 30 days");
    assert_eq!(ok("accessed:>2w"), date(DateField::Accessed, Cmp::Lt, ago(2, SpanUnit::Weeks)));
    assert_eq!(ok("older:90d"), date(m, Cmp::Lt, ago(90, SpanUnit::Days)));
    assert_eq!(ok("newer:7d"), date(m, Cmp::Gt, ago(7, SpanUnit::Days)));
    assert_eq!(ok("older:2024"), date(m, Cmp::Lt, ts(2024, 1, 1)));
    assert_eq!(
        ok("modified:30d..90d"),
        Rule::All(vec![date(m, Cmp::Ge, ago(90, SpanUnit::Days)), date(m, Cmp::Le, ago(30, SpanUnit::Days)),])
    );
    assert_eq!(ok("modified:90d..30d"), ok("modified:30d..90d"));
}

#[test]
fn type_attr_is_kind_owner_depth_pathlen_content_top() {
    assert_eq!(ok("type:video"), c(Condition::TypeGroup { group: "video".into() }));
    assert_eq!(
        ok("type:Video,audio"),
        Rule::Any(vec![
            c(Condition::TypeGroup { group: "video".into() }),
            c(Condition::TypeGroup { group: "audio".into() }),
        ])
    );
    assert_eq!(ok("attr:hidden"), c(Condition::Attribute { attr: Attributes::HIDDEN, set: true }));
    assert_eq!(ok("attr:!hidden"), c(Condition::Attribute { attr: Attributes::HIDDEN, set: false }));
    assert_eq!(ok("attr:read-only"), c(Condition::Attribute { attr: Attributes::READONLY, set: true }));
    assert_eq!(ok("is:empty"), c(Condition::ZeroByte));
    assert_eq!(ok("is:hidden"), c(Condition::Attribute { attr: Attributes::HIDDEN, set: true }));
    assert_eq!(ok("is:invalid"), c(Condition::InvalidName));
    assert_eq!(ok("is:mismatch"), c(Condition::ExtensionMismatch));
    assert_eq!(ok("is:dir"), c(Condition::Kind { kind: EntryKind::Dir }));
    assert_eq!(ok("kind:link"), c(Condition::Kind { kind: EntryKind::Link }));
    assert_eq!(ok("owner:alice"), c(Condition::Owner { value: "alice".into() }));
    assert_eq!(ok("owner:uid:1000"), c(Condition::Owner { value: "uid:1000".into() }));
    assert_eq!(ok("depth:>3"), c(Condition::Depth { cmp: Cmp::Gt, value: 3 }));
    assert_eq!(
        ok("depth:2..5"),
        Rule::All(
            vec![c(Condition::Depth { cmp: Cmp::Ge, value: 2 }), c(Condition::Depth { cmp: Cmp::Le, value: 5 }),]
        )
    );
    assert_eq!(ok("pathlen:>240"), c(Condition::PathLength { cmp: Cmp::Gt, value: 240 }));
    assert_eq!(
        ok(r#"content:"hello, world""#),
        c(Condition::ContentContains { text: "hello, world".into(), case_sensitive: false })
    );
    assert_eq!(ok("top:50"), c(Condition::LargestN { n: 50 }));
    assert_eq!(
        ok("ext:mp4 top:5"),
        Rule::All(vec![c(Condition::Extension { values: vec!["mp4".into()] }), c(Condition::LargestN { n: 5 })])
    );
}

#[test]
fn boolean_structure() {
    let a = || c(Condition::Extension { values: vec!["a".into()] });
    let b = || c(Condition::Extension { values: vec!["b".into()] });
    let d = || c(Condition::Extension { values: vec!["d".into()] });
    assert_eq!(ok("-ext:a"), Rule::Not(Box::new(a())));
    assert_eq!(ok("NOT ext:a"), Rule::Not(Box::new(a())));
    assert_eq!(ok("--ext:a"), Rule::Not(Box::new(Rule::Not(Box::new(a())))));
    assert_eq!(ok("ext:a OR ext:b"), Rule::Any(vec![a(), b()]));
    assert_eq!(ok("ext:a AND ext:b"), Rule::All(vec![a(), b()]));
    assert_eq!(ok("ext:a ext:b OR ext:d"), Rule::Any(vec![Rule::All(vec![a(), b()]), d()]), "AND binds tighter");
    assert_eq!(ok("ext:a (ext:b OR ext:d)"), Rule::All(vec![a(), Rule::Any(vec![b(), d()])]));
    assert_eq!(ok("(ext:a)"), a());
    assert_eq!(ok("-(ext:a OR ext:b)"), Rule::Not(Box::new(Rule::Any(vec![a(), b()]))));
    assert_eq!(ok("ext:a (ext:b ext:d)"), Rule::All(vec![a(), b(), d()]), "nested AND is flattened");
    assert_eq!(ok("ext:a OR ext:b OR ext:d"), Rule::Any(vec![a(), b(), d()]));
    assert_eq!(
        ok("ext:jpg,png -path:*/Photos/*"),
        Rule::All(vec![
            c(Condition::Extension { values: vec!["jpg".into(), "png".into()] }),
            Rule::Not(Box::new(path(TextMode::Wildcard, "*/Photos/*"))),
        ])
    );
}

fn first(q: &str) -> QueryError {
    errs(q).remove(0)
}

#[test]
fn errors_carry_positions() {
    let e = first("ext:mp4 size:>1XB");
    assert_eq!((e.offset, e.len), (14, 3));
    assert!(e.message.contains("unknown size unit"), "{}", e.message);
    let e = first("exte:mp4");
    assert_eq!((e.offset, e.len), (0, 5));
    assert!(e.message.contains("did you mean 'ext:'"), "{}", e.message);
    let e = first("ext:mp4 (name:a");
    assert_eq!((e.offset, e.len), (8, 1));
    assert!(e.message.contains("no matching ')'"));
    let e = first("name:a)");
    assert_eq!(e.offset, 6);
    let e = first("OR ext:a");
    assert_eq!((e.offset, e.len), (0, 2));
    let e = first("ext:a OR");
    assert_eq!((e.offset, e.len), (6, 2));
    let e = first("ext:a -");
    assert_eq!(e.offset, 6);
    let e = first("()");
    assert_eq!(e.offset, 1);
    let e = first("modified:<2024-13");
    assert_eq!(e.offset, 10);
    let e = first("size:");
    assert!(e.message.contains("needs a value"));
    let e = first(r"name:/(/");
    assert_eq!(e.offset, 5);
    assert!(e.message.starts_with("invalid regular expression"));
    assert!(first("name:*.tmp ext:a,,b").message.contains("missing"));
    assert!(first("ext:").message.contains("ext:mp4"));
}

#[test]
fn several_bad_terms_are_all_reported() {
    let e = errs("size:>1XB ext: kind:blob");
    assert_eq!(e.len(), 3, "{e:?}");
    assert!(e.windows(2).all(|w| w[0].offset <= w[1].offset), "sorted by position");
}

#[test]
fn rejected_forms() {
    assert!(first("").message.contains("empty"));
    assert!(first("   ").message.contains("empty"));
    assert!(first("ext:>mp4").message.contains("operator"));
    assert!(first("name:<a").message.contains("operator"));
    assert!(first("size:>1MB..2MB").message.contains("range"));
    assert!(first("size:10MB..1MB").message.contains("range"));
    assert!(first("modified:2020..30d").message.contains("range"));
    assert!(first("modified:12").message.contains("unit"));
    assert!(first("modified:9999-12").message.contains("range"));
    assert!(first("modified:100000y").message.contains("further"));
    assert!(first("attr:bogus").message.contains("attributes:"));
    assert!(first("top:abc").message.contains("whole number"));
    assert!(first("top:1,2").message.contains("single"));
    assert!(first("C:\\Users").message.contains("unknown key"));
    assert!(first("name:(1)").message.contains("space before"));
    assert!(first("ext:a OR top:5").message.contains("top:N"));
    assert!(first("-top:5").message.contains("top:N"));
    assert!(first("ext:a top:5 OR ext:b").message.contains("top:N"));
    assert!(first("ext:*").message.contains("not a file extension"));
    assert!(first(&"x".repeat(9000)).message.contains("longer"));
}

#[test]
fn nesting_is_bounded() {
    let deep = format!("{}ext:a{}", "(".repeat(100), ")".repeat(100));
    assert!(first(&deep).message.contains("nested"));
    let negs = format!("{}ext:a", "-".repeat(200));
    assert!(first(&negs).message.contains("nested"));
    let fine = format!("{}ext:a{}", "(".repeat(20), ")".repeat(20));
    assert!(parse(&fine).is_ok());
}

#[test]
fn groups_are_validated_when_provided() {
    let g = crate::groups::builtin();
    assert_eq!(parse_with_groups("type:VIDEO", &g).unwrap(), c(Condition::TypeGroup { group: "video".into() }));
    let e = parse_with_groups("type:videoo", &g).unwrap_err();
    assert!(e[0].message.contains("known groups"));
    assert_eq!(e[0].offset, 5);
}

#[test]
fn parsed_rules_compile_and_select() {
    use mj_core::{FileEntry, FileIndex, ScanStats};
    let g = crate::groups::builtin();
    let ctx = crate::FilterContext { now: Timestamp::from_ymd_hms(2026, 11, 13, 0, 0, 0), type_groups: &g };
    let mut old = FileEntry::new("/r/Movies/big.mp4", EntryKind::File, 3 << 30);
    old.modified = Some(Timestamp::from_ymd_hms(2019, 1, 1, 0, 0, 0));
    let mut recent = FileEntry::new("/r/Movies/new.mp4", EntryKind::File, 3 << 30);
    recent.modified = Some(Timestamp::from_ymd_hms(2026, 10, 1, 0, 0, 0));
    let small = FileEntry::new("/r/Movies/small.mp4", EntryKind::File, 10);
    let index = FileIndex::new(vec!["/r".into()], vec![old, recent, small], ScanStats::default(), Timestamp::default());
    let rule = ok("type:video size:>1GB modified:>2y -name:new*");
    let sel = crate::select(&index, &rule, &ctx, false).unwrap();
    assert_eq!(sel, vec![0]);
    let rule = ok("ext:mp4 top:1");
    assert_eq!(crate::select(&index, &rule, &ctx, false).unwrap().len(), 1);
}

#[test]
fn suggestions() {
    let s = suggest("ext:mp4 si", 10);
    assert_eq!(
        s[0],
        Suggestion { insert: "size:>".into(), replace_len: 2, label: "size:>1GB  bigger than 1 GB".into() }
    );
    assert_eq!(suggest("", 0).len(), KEYS_LEN);
    assert!(suggest("m", 1).iter().any(|s| s.insert == "modified:<"));
    let v = suggest("size:>1", 7);
    assert!(v.iter().any(|s| s.insert == ">1GB" && s.replace_len == 2), "{v:?}");
    assert!(v.iter().all(|s| !s.insert.contains("..")), "no range after an operator");
    let t = suggest("type:vi", 7);
    assert_eq!(t[0].insert, "video");
    assert_eq!(t[0].replace_len, 2);
    assert!(suggest("attr:h", 6).iter().any(|s| s.insert == "hidden"));
    assert!(suggest("is:", 3).iter().any(|s| s.insert == "empty"));
    // After a space the token is empty again; inside quotes nothing is offered.
    assert_eq!(suggest("ext:mp4 ", 8).len(), KEYS_LEN);
    assert!(suggest("name:\"si", 8).is_empty());
    assert!(suggest("a O", 3).iter().any(|s| s.insert == "OR"));
    assert!(suggest("-si", 3).iter().any(|s| s.insert == "size:>" && s.replace_len == 2));
    assert!(suggest("(ext:a OR si", 12).iter().any(|s| s.insert == "size:>"));
    assert!(suggest("bogus:x", 7).is_empty());
    // Any cursor value is safe, including past the end and inside a multi-byte character.
    for cur in 0..40 {
        let _ = suggest("名前:ñ size:>1", cur);
    }
}

const KEYS_LEN: usize = 19;

/// Deterministic fuzz: random and mutated queries must never panic, and every
/// error must point inside the query on a character boundary.
#[test]
fn fuzz_garbage_never_panics() {
    let mut s: u64 = 0xD1B5_4A32_D192_ED03;
    let mut next = move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    let atoms = [
        "ext:",
        "size:",
        ">",
        "<=",
        "..",
        "1GB",
        "-",
        "(",
        ")",
        "OR",
        "AND",
        "NOT",
        "\"",
        "'",
        "/",
        ",",
        ":",
        " ",
        "name:",
        "modified:",
        "2024",
        "30d",
        "top:",
        "regex:",
        "名",
        "é",
        "\u{0}",
        "\\",
        "*",
        "?",
        "[",
        "attr:!",
        "path:",
        "depth:",
        "is:",
        "type:",
        "9999999999999999999999",
        "99999y",
        "=",
        "!",
        "\t",
        "\u{1F600}",
    ];
    for _ in 0..20_000 {
        let n = (next() % 14) as usize;
        let mut q = String::new();
        for _ in 0..n {
            q.push_str(atoms[(next() % atoms.len() as u64) as usize]);
        }
        match parse(&q) {
            Ok(_) => {}
            Err(es) => {
                assert!(!es.is_empty());
                for e in es {
                    assert!(e.offset + e.len <= q.len(), "{q:?}: {e:?}");
                    assert!(q.is_char_boundary(e.offset) && q.is_char_boundary(e.offset + e.len), "{q:?}: {e:?}");
                }
            }
        }
        let cur = (next() % (q.len() as u64 + 3)) as usize;
        let _ = suggest(&q, cur);
    }
    // Raw random bytes interpreted as (lossy) text.
    for _ in 0..5000 {
        let len = (next() % 64) as usize;
        let bytes: Vec<u8> = (0..len).map(|_| next() as u8).collect();
        let q = String::from_utf8_lossy(&bytes).into_owned();
        let _ = parse(&q);
        let _ = suggest(&q, (next() % 70) as usize);
    }
}
