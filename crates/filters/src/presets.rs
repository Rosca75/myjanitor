//! Rule presets (CLN-ENG-02): the built-in set, JSON (de)serialisation of user
//! presets through the contract type `RulePreset`, and the pure list operations
//! behind `preset_save` / `preset_delete`. Owner: W2-C (may add more; ids are stable).

use mj_core::time::{RelativeSpan, SpanUnit};
use mj_core::{Attributes, Cmp, Condition, DateField, DateValue, EntryKind, Error, Result, Rule, RulePreset, TextMode};

use crate::eval::{compile, FilterContext};

fn older_than(amount: u32, unit: SpanUnit, which: DateField) -> Rule {
    Rule::Cond(Condition::Date { which, cmp: Cmp::Lt, value: DateValue::Ago(RelativeSpan { amount, unit }) })
}

fn group(g: &str) -> Rule {
    Rule::Cond(Condition::TypeGroup { group: g.into() })
}

fn kind(k: EntryKind) -> Rule {
    Rule::Cond(Condition::Kind { kind: k })
}

fn name(mode: TextMode, pattern: &str) -> Rule {
    Rule::Cond(Condition::Name { mode, pattern: pattern.into(), case_sensitive: false })
}

fn names(mode: TextMode, patterns: &[&str]) -> Rule {
    Rule::Any(patterns.iter().map(|p| name(mode, p)).collect())
}

fn preset(id: &str, title: &str, description: &str, rule: Rule, topmost_only: bool) -> RulePreset {
    RulePreset {
        id: id.into(),
        name: title.into(),
        description: Some(description.into()),
        rule,
        topmost_only,
        builtin: true,
    }
}

/// Presets shipped with the app.
pub fn builtin() -> Vec<RulePreset> {
    let size = |cmp, bytes| Rule::Cond(Condition::Size { cmp, bytes });
    vec![
        preset(
            "downloads_older_90d",
            "Downloads older than 90 days",
            "Anything in a Downloads folder not modified for 90 days",
            Rule::All(vec![
                Rule::Cond(Condition::Path {
                    mode: TextMode::Wildcard,
                    pattern: "**/Downloads/**".into(),
                    case_sensitive: false,
                }),
                older_than(90, SpanUnit::Days, DateField::Modified),
            ]),
            false,
        ),
        preset(
            "large_old_media",
            "Large media older than 2 years",
            "Video and audio over 500 MB not modified for 2 years",
            Rule::All(vec![
                Rule::Any(vec![group("video"), group("audio")]),
                size(Cmp::Gt, 500 << 20),
                older_than(2, SpanUnit::Years, DateField::Modified),
            ]),
            false,
        ),
        preset(
            "consulting_cleanup",
            "Consulting cleanup",
            "Office files and PDFs not opened for a year",
            Rule::All(vec![
                Rule::Any(vec![group("office"), group("pdf")]),
                older_than(1, SpanUnit::Years, DateField::Accessed),
            ]),
            false,
        ),
        preset(
            "dev_junk",
            "Dev junk",
            "Rebuildable developer folders (node_modules, caches) and compiled leftovers; reports the topmost folder",
            Rule::Any(vec![
                Rule::All(vec![
                    kind(EntryKind::Dir),
                    Rule::Any(
                        [
                            "node_modules",
                            "__pycache__",
                            ".pytest_cache",
                            ".mypy_cache",
                            ".tox",
                            ".gradle",
                            ".parcel-cache",
                            ".next",
                        ]
                        .iter()
                        .map(|n| name(TextMode::Exact, n))
                        .collect(),
                    ),
                ]),
                Rule::All(vec![
                    kind(EntryKind::File),
                    names(TextMode::Wildcard, &["*.pyc", "*.pyo", "*.class", "*.o", "*.obj"]),
                ]),
            ]),
            true,
        ),
        preset(
            "large_videos",
            "Large videos",
            "Video files over 1 GB, biggest first in the results",
            Rule::All(vec![group("video"), size(Cmp::Gt, 1 << 30)]),
            false,
        ),
        preset(
            "old_installers",
            "Old installers",
            "Installers and disk images not modified for 6 months",
            Rule::All(vec![
                group("installers"),
                kind(EntryKind::File),
                older_than(6, SpanUnit::Months, DateField::Modified),
            ]),
            false,
        ),
        preset(
            "temp_and_backup_files",
            "Temporary and backup files",
            "*.tmp, *.bak, *.old, Office lock files (~$*) and similar",
            Rule::All(vec![
                kind(EntryKind::File),
                names(TextMode::Wildcard, &["*.tmp", "*.temp", "*.bak", "*.old", "~$*", "*~"]),
            ]),
            false,
        ),
        preset("zero_byte_files", "Zero-byte files", "Empty files", Rule::Cond(Condition::ZeroByte), false),
        preset(
            "extension_mismatch",
            "Files whose content does not match their extension",
            "Reads each file's header: a .jpg that is really a PNG or an HTML page. Slower than name filters",
            Rule::Cond(Condition::ExtensionMismatch),
            false,
        ),
        preset(
            "invalid_windows_names",
            "Names Windows cannot handle",
            "Reserved names (CON, NUL...), trailing dots or spaces and characters < > : \" | ? *",
            Rule::All(vec![Rule::Cond(Condition::InvalidName)]),
            false,
        ),
        preset(
            "long_paths",
            "Very long paths",
            "Full paths over 240 characters, which break many Windows tools",
            Rule::Cond(Condition::PathLength { cmp: Cmp::Gt, value: 240 }),
            false,
        ),
        preset(
            "largest_100_files",
            "100 largest files",
            "The hundred biggest files in the scan",
            Rule::All(vec![kind(EntryKind::File), Rule::Cond(Condition::LargestN { n: 100 })]),
            false,
        ),
        preset(
            "hidden_system_files",
            "Hidden system files",
            "Files with both the hidden and the system attribute",
            Rule::All(vec![
                Rule::Cond(Condition::Attribute { attr: Attributes::HIDDEN, set: true }),
                Rule::Cond(Condition::Attribute { attr: Attributes::SYSTEM, set: true }),
            ]),
            false,
        ),
    ]
}

/// Built-in presets followed by user presets.
pub fn all(user: &[RulePreset]) -> Vec<RulePreset> {
    let mut v = builtin();
    v.extend(user.iter().filter(|p| !p.builtin).cloned());
    v
}

/// Serialise user presets as pretty JSON (an array of `RulePreset`).
pub fn to_json(presets: &[RulePreset]) -> Result<String> {
    serde_json::to_string_pretty(presets).map_err(|e| Error::Internal(format!("could not serialise presets: {e}")))
}

/// Parse an array of presets. A syntax or shape error is `Error::Parse`
/// (whole-file failure; the caller keeps its previous list).
pub fn from_json(json: &str) -> Result<Vec<RulePreset>> {
    serde_json::from_str(json).map_err(|e| Error::parse("presets file", e.to_string()))
}

/// Check that a preset's rule compiles (regexes, groups, `LargestN` placement).
pub fn validate(p: &RulePreset, ctx: &FilterContext<'_>) -> Result<()> {
    if p.name.trim().is_empty() {
        return Err(Error::InvalidInput("a preset needs a name".into()));
    }
    compile(&p.rule, ctx).map(|_| ())
}

fn slug(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect();
    let s = s.split('_').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("_");
    if s.is_empty() {
        "preset".into()
    } else {
        s
    }
}

/// Insert or replace a user preset by id (an empty id creates a new one) and
/// return its id. Built-in ids cannot be overwritten (`InvalidInput`); the
/// `builtin` flag of the input is ignored. The caller validates the rule with
/// [`validate`] first.
pub fn upsert(user: &mut Vec<RulePreset>, mut p: RulePreset) -> Result<String> {
    p.builtin = false;
    if builtin().iter().any(|b| b.id == p.id) {
        return Err(Error::InvalidInput(format!(
            "'{}' is a built-in preset and cannot be overwritten; save a copy under a new name",
            p.id
        )));
    }
    if p.id.trim().is_empty() {
        let base = format!("user_{}", slug(&p.name));
        let taken = |id: &str| user.iter().any(|u| u.id == id) || builtin().iter().any(|b| b.id == id);
        let mut id = base.clone();
        let mut n = 2;
        while taken(&id) {
            id = format!("{base}_{n}");
            n += 1;
        }
        p.id = id;
    }
    let id = p.id.clone();
    match user.iter_mut().find(|u| u.id == id) {
        Some(slot) => *slot = p,
        None => user.push(p),
    }
    Ok(id)
}

/// Remove a user preset. Built-ins cannot be deleted; an unknown id is `NotFound`-style `InvalidInput`.
pub fn remove(user: &mut Vec<RulePreset>, id: &str) -> Result<()> {
    if builtin().iter().any(|b| b.id == id) {
        return Err(Error::InvalidInput(format!("'{id}' is a built-in preset and cannot be deleted")));
    }
    let before = user.len();
    user.retain(|u| u.id != id);
    if user.len() == before {
        return Err(Error::InvalidInput(format!("no preset with id '{id}'")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn ctx(g: &std::collections::BTreeMap<String, Vec<String>>) -> FilterContext<'_> {
        FilterContext { now: mj_core::Timestamp::from_ymd_hms(2026, 11, 13, 0, 0, 0), type_groups: g }
    }

    #[test]
    fn builtin_ids_unique_and_all_compile() {
        let p = builtin();
        let ids: BTreeSet<_> = p.iter().map(|x| x.id.clone()).collect();
        assert_eq!(ids.len(), p.len());
        assert!(p.iter().all(|x| x.builtin));
        let groups = crate::groups::builtin();
        for x in &p {
            validate(x, &ctx(&groups)).unwrap_or_else(|e| panic!("{}: {e}", x.id));
        }
        assert!(ids.contains("dev_junk") && ids.contains("large_videos") && ids.contains("old_installers"));
    }

    #[test]
    fn json_round_trip_through_contract_type() {
        let mut p = builtin();
        p.push(RulePreset {
            id: "u".into(),
            name: "Mine".into(),
            description: None,
            rule: crate::parse("ext:mp4 size:>1GB modified:<2024").unwrap(),
            topmost_only: true,
            builtin: false,
        });
        let json = to_json(&p).unwrap();
        assert_eq!(from_json(&json).unwrap(), p);
        // The IPC example shape deserialises as is.
        let ipc = r#"[{"id":"","name":"Old screen recordings","rule":{"all":[{"cond":{"field":"path","mode":"wildcard","pattern":"**/Screen Recordings/**","case_sensitive":false}},{"cond":{"field":"date","which":"modified","cmp":"lt","value":{"ago":{"amount":6,"unit":"months"}}}}]},"topmost_only":false,"builtin":false}]"#;
        assert_eq!(from_json(ipc).unwrap()[0].name, "Old screen recordings");
        assert!(matches!(from_json("{nope"), Err(Error::Parse { .. })));
        assert!(from_json(r#"[{"id":"x"}]"#).is_err());
    }

    #[test]
    fn upsert_and_remove_rules() {
        let mut user = Vec::new();
        let mk = |id: &str, name: &str| RulePreset {
            id: id.into(),
            name: name.into(),
            description: None,
            rule: Rule::everything(),
            topmost_only: false,
            builtin: true,
        };
        let a = upsert(&mut user, mk("", "Old screen recordings")).unwrap();
        assert_eq!(a, "user_old_screen_recordings");
        assert!(!user[0].builtin, "the builtin flag of user input is ignored");
        let b = upsert(&mut user, mk("", "Old screen recordings")).unwrap();
        assert_eq!(b, "user_old_screen_recordings_2");
        upsert(&mut user, mk(&a, "Renamed")).unwrap();
        assert_eq!(user.len(), 2);
        assert_eq!(user[0].name, "Renamed");
        assert!(upsert(&mut user, mk("dev_junk", "x")).is_err());
        assert!(remove(&mut user, "dev_junk").is_err());
        assert!(remove(&mut user, "nope").is_err());
        remove(&mut user, &a).unwrap();
        assert_eq!(all(&user).len(), builtin().len() + 1);
    }

    #[test]
    fn validate_rejects_bad_rules_and_names() {
        let groups = crate::groups::builtin();
        let mut p = builtin().remove(0);
        p.rule = Rule::Cond(Condition::Name { mode: TextMode::Regex, pattern: "(".into(), case_sensitive: false });
        assert!(validate(&p, &ctx(&groups)).is_err());
        p.rule = Rule::everything();
        p.name = "  ".into();
        assert!(validate(&p, &ctx(&groups)).is_err());
    }
}
