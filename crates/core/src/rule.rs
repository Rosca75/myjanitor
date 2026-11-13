//! Filter rules: an AND/OR/NOT tree of typed conditions (CLN-FLT-*, CLN-ENG-01).
//!
//! The quick-query syntax (`ext:mp4 size:>1GB modified:<2024`, CLN-ENG-05) parses
//! into this same tree (in `mj-filters`), so the rule builder and the search box
//! share one evaluator. JSON example:
//!
//! ```json
//! {"all":[{"cond":{"field":"extension","values":["mp4"]}},
//!         {"cond":{"field":"size","cmp":"gt","bytes":1073741824}}]}
//! ```

use serde::{Deserialize, Serialize};

use crate::entry::{Attributes, EntryKind};
use crate::time::{RelativeSpan, Timestamp};

/// A rule tree node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    /// Every child matches (empty = matches everything).
    All(Vec<Rule>),
    /// At least one child matches (empty = matches nothing).
    Any(Vec<Rule>),
    /// The child does not match.
    Not(Box<Rule>),
    /// A leaf condition.
    Cond(Condition),
}

impl Rule {
    /// The rule that matches everything.
    pub fn everything() -> Rule {
        Rule::All(Vec::new())
    }
}

/// Numeric comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cmp {
    Lt,
    Le,
    Eq,
    Ge,
    Gt,
}

impl Cmp {
    /// Apply to two ordered values: `lhs <cmp> rhs`.
    pub fn test<T: PartialOrd>(self, lhs: T, rhs: T) -> bool {
        match self {
            Cmp::Lt => lhs < rhs,
            Cmp::Le => lhs <= rhs,
            Cmp::Eq => lhs == rhs,
            Cmp::Ge => lhs >= rhs,
            Cmp::Gt => lhs > rhs,
        }
    }
}

/// How a text pattern is matched (CLN-FLT-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextMode {
    /// Glob wildcards (`*.tmp`).
    Wildcard,
    /// Substring.
    Contains,
    /// Whole string equality.
    Exact,
    /// Regular expression (`regex` crate syntax).
    Regex,
}

/// Which timestamp a date condition reads (CLN-FLT-04).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DateField {
    Created,
    Modified,
    Accessed,
}

/// A date bound: absolute or relative to "now" at evaluation time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DateValue {
    /// A fixed instant.
    At(Timestamp),
    /// `now - span` ("older than 2 years" = `cmp: lt, value: {ago: {amount: 2, unit: years}}`).
    Ago(RelativeSpan),
}

/// A leaf condition. Tagged by `field`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "field", rename_all = "snake_case")]
pub enum Condition {
    /// File name (last component).
    Name {
        mode: TextMode,
        pattern: String,
        #[serde(default)]
        case_sensitive: bool,
    },
    /// Full path.
    Path {
        mode: TextMode,
        pattern: String,
        #[serde(default)]
        case_sensitive: bool,
    },
    /// Extension in the list (lower case, no dot).
    Extension { values: Vec<String> },
    /// Member of a file-type group (`audio`, `video`, `office`, `archives`, `installers`, `system`, ...;
    /// groups are defined and editable in `Settings::type_groups`, CLN-FLT-02).
    TypeGroup { group: String },
    /// Logical size.
    Size { cmp: Cmp, bytes: u64 },
    /// Zero-byte files.
    ZeroByte,
    /// Keep only the N largest files of the result. Evaluated after the rest of the tree;
    /// only valid as a direct child of the top-level `all`.
    LargestN { n: u32 },
    /// Date comparison; a missing timestamp never matches.
    Date { which: DateField, cmp: Cmp, value: DateValue },
    /// Attribute is set (`set: true`) or clear (`set: false`) (CLN-FLT-05).
    Attribute { attr: Attributes, set: bool },
    /// Owner equals (`uid:1000`, a SID, or a resolved user name) (CLN-FLT-06).
    Owner { value: String },
    /// Depth below the scan root (CLN-FLT-07).
    Depth { cmp: Cmp, value: u32 },
    /// Full path length in characters (CLN-FLT-08).
    PathLength { cmp: Cmp, value: u32 },
    /// Name not portable to Windows (CLN-FLT-08).
    InvalidName,
    /// Entry kind.
    Kind { kind: EntryKind },
    /// Content type differs from what the extension claims (CLN-CNT-02; reads the file header).
    ExtensionMismatch,
    /// Full-text content contains the text (CLN-CNT-01; reads the file, slow).
    ContentContains {
        text: String,
        #[serde(default)]
        case_sensitive: bool,
    },
}

/// A saved rule preset ("Downloads older than 90 days", CLN-ENG-02).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RulePreset {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub rule: Rule,
    /// Report the topmost matching folder instead of each child (CLN-ENG-04).
    #[serde(default)]
    pub topmost_only: bool,
    /// Shipped with the app (cannot be deleted, can be copied).
    #[serde(default)]
    pub builtin: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::SpanUnit;

    #[test]
    fn json_shape_is_stable() {
        let r = Rule::All(vec![
            Rule::Cond(Condition::Extension { values: vec!["mp4".into()] }),
            Rule::Cond(Condition::Size { cmp: Cmp::Gt, bytes: 1 << 30 }),
            Rule::Not(Box::new(Rule::Cond(Condition::Date {
                which: DateField::Modified,
                cmp: Cmp::Ge,
                value: DateValue::Ago(RelativeSpan { amount: 2, unit: SpanUnit::Years }),
            }))),
            Rule::Cond(Condition::Attribute { attr: Attributes::HIDDEN, set: true }),
        ]);
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.starts_with(r#"{"all":[{"cond":{"field":"extension","values":["mp4"]}}"#), "{json}");
        assert!(json.contains(r#""value":{"ago":{"amount":2,"unit":"years"}}"#), "{json}");
        assert!(json.contains(r#""attr":["hidden"]"#), "{json}");
        let back: Rule = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
        assert!(Cmp::Lt.test(1, 2) && !Cmp::Gt.test(1, 2));
    }
}
