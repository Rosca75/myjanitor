//! Built-in rule presets (CLN-ENG-02). Owner: W2-C (may add more; ids are stable).

use mj_core::time::{RelativeSpan, SpanUnit};
use mj_core::{Cmp, Condition, DateField, DateValue, Rule, RulePreset};

fn older_than(amount: u32, unit: SpanUnit, which: DateField) -> Rule {
    Rule::Cond(Condition::Date { which, cmp: Cmp::Lt, value: DateValue::Ago(RelativeSpan { amount, unit }) })
}

/// Presets shipped with the app.
pub fn builtin() -> Vec<RulePreset> {
    vec![
        RulePreset {
            id: "downloads_older_90d".into(),
            name: "Downloads older than 90 days".into(),
            description: Some("Anything in a Downloads folder not modified for 90 days".into()),
            rule: Rule::All(vec![
                Rule::Cond(Condition::Path {
                    mode: mj_core::TextMode::Wildcard,
                    pattern: "**/Downloads/**".into(),
                    case_sensitive: false,
                }),
                older_than(90, SpanUnit::Days, DateField::Modified),
            ]),
            topmost_only: false,
            builtin: true,
        },
        RulePreset {
            id: "large_old_media".into(),
            name: "Large media older than 2 years".into(),
            description: Some("Video and audio over 500 MB not modified for 2 years".into()),
            rule: Rule::All(vec![
                Rule::Any(vec![
                    Rule::Cond(Condition::TypeGroup { group: "video".into() }),
                    Rule::Cond(Condition::TypeGroup { group: "audio".into() }),
                ]),
                Rule::Cond(Condition::Size { cmp: Cmp::Gt, bytes: 500 << 20 }),
                older_than(2, SpanUnit::Years, DateField::Modified),
            ]),
            topmost_only: false,
            builtin: true,
        },
        RulePreset {
            id: "consulting_cleanup".into(),
            name: "Consulting cleanup".into(),
            description: Some("Office files and PDFs not opened for a year".into()),
            rule: Rule::All(vec![
                Rule::Any(vec![
                    Rule::Cond(Condition::TypeGroup { group: "office".into() }),
                    Rule::Cond(Condition::TypeGroup { group: "pdf".into() }),
                ]),
                older_than(1, SpanUnit::Years, DateField::Accessed),
            ]),
            topmost_only: false,
            builtin: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    #[test]
    fn builtin_ids_unique() {
        let p = super::builtin();
        let mut ids: Vec<_> = p.iter().map(|x| x.id.clone()).collect();
        ids.dedup();
        assert_eq!(ids.len(), p.len());
        assert!(p.iter().all(|x| x.builtin));
    }
}
