//! Text matching for name and path conditions (CLN-FLT-01). Owner: W2-C.
//!
//! One [`TextMatcher`] is built per condition when a rule is compiled, so a
//! regex is compiled exactly once and an invalid one is reported before any
//! file is looked at. Paths are always matched with `/` separators, on every
//! platform, so one rule behaves the same on Windows and Linux.

use mj_core::glob::{Glob, GlobMode};
use mj_core::TextMode;
use regex::{Regex, RegexBuilder};

/// Longest pattern accepted for any mode (characters).
pub const MAX_PATTERN_CHARS: usize = 1024;
/// Compiled-regex size bound (bytes), so a hostile pattern cannot eat memory.
pub const REGEX_SIZE_LIMIT: usize = 1 << 20;

/// Compile a regex with the size, nesting and length limits applied.
/// The error is a one-line human message.
pub fn build_regex(pattern: &str, case_sensitive: bool) -> Result<Regex, String> {
    if pattern.is_empty() {
        return Err("the regular expression is empty".into());
    }
    if pattern.chars().count() > MAX_PATTERN_CHARS {
        return Err(format!("the regular expression is longer than {MAX_PATTERN_CHARS} characters"));
    }
    RegexBuilder::new(pattern)
        .case_insensitive(!case_sensitive)
        .size_limit(REGEX_SIZE_LIMIT)
        .dfa_size_limit(REGEX_SIZE_LIMIT * 4)
        .nest_limit(64)
        .build()
        .map_err(|e| {
            // The regex crate's message is multi-line; the last non-empty line is the reason.
            let text = e.to_string();
            let reason = text.lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or("invalid pattern");
            format!("invalid regular expression: {}", reason.trim_start_matches("error: "))
        })
}

/// A compiled name or path matcher.
#[derive(Debug)]
pub struct TextMatcher {
    kind: Kind,
    /// Normalise `\` to `/` in the text before matching.
    path_mode: bool,
}

#[derive(Debug)]
enum Kind {
    Glob(Glob),
    /// `needle` is already lower-cased when `case_sensitive` is false.
    Contains {
        needle: String,
        case_sensitive: bool,
    },
    Exact {
        value: String,
        case_sensitive: bool,
    },
    Regex(Regex),
}

impl TextMatcher {
    /// Build a matcher. `path_mode` selects path semantics (separators, `**`).
    pub fn new(mode: TextMode, pattern: &str, case_sensitive: bool, path_mode: bool) -> Result<TextMatcher, String> {
        if pattern.is_empty() {
            return Err("the pattern is empty".into());
        }
        if pattern.chars().count() > MAX_PATTERN_CHARS {
            return Err(format!("the pattern is longer than {MAX_PATTERN_CHARS} characters"));
        }
        let norm = |s: &str| {
            let s = if path_mode { s.replace('\\', "/") } else { s.to_string() };
            if case_sensitive {
                s
            } else {
                s.to_lowercase()
            }
        };
        let kind = match mode {
            TextMode::Wildcard => {
                let gm = if path_mode { GlobMode::Path } else { GlobMode::Name };
                Kind::Glob(Glob::new(pattern, gm, !case_sensitive))
            }
            TextMode::Contains => Kind::Contains { needle: norm(pattern), case_sensitive },
            TextMode::Exact => Kind::Exact { value: norm(pattern), case_sensitive },
            TextMode::Regex => Kind::Regex(build_regex(pattern, case_sensitive)?),
        };
        Ok(TextMatcher { kind, path_mode })
    }

    /// True when `text` matches.
    pub fn is_match(&self, text: &str) -> bool {
        // Cheap relative cost used to order predicates: regex and glob are dearer than substring tests.
        let normalised;
        let text = if self.path_mode && text.contains('\\') {
            normalised = text.replace('\\', "/");
            normalised.as_str()
        } else {
            text
        };
        match &self.kind {
            Kind::Glob(g) => g.matches(text),
            Kind::Contains { needle, case_sensitive } => {
                if *case_sensitive {
                    text.contains(needle.as_str())
                } else {
                    text.to_lowercase().contains(needle.as_str())
                }
            }
            Kind::Exact { value, case_sensitive } => {
                if *case_sensitive {
                    text == value
                } else {
                    text.to_lowercase() == *value
                }
            }
            Kind::Regex(r) => r.is_match(text),
        }
    }

    /// Relative evaluation cost (1 cheap .. 6 dear) for predicate ordering.
    pub fn cost(&self) -> u8 {
        match &self.kind {
            Kind::Exact { .. } => 2,
            Kind::Contains { .. } => 3,
            Kind::Glob(_) => 4,
            Kind::Regex(_) => 6,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(mode: TextMode, p: &str, cs: bool, path: bool) -> TextMatcher {
        TextMatcher::new(mode, p, cs, path).unwrap()
    }

    #[test]
    fn wildcard_name_and_path() {
        assert!(m(TextMode::Wildcard, "*.tmp", false, false).is_match("A.TMP"));
        assert!(!m(TextMode::Wildcard, "*.tmp", true, false).is_match("A.TMP"));
        assert!(m(TextMode::Wildcard, "**/Downloads/**", false, true).is_match("/home/u/downloads/a/b.zip"));
        assert!(m(TextMode::Wildcard, "**/Downloads/**", false, true).is_match("C:\\Users\\u\\Downloads\\a.zip"));
        assert!(!m(TextMode::Wildcard, "*.txt", false, true).is_match("/a/b.txt"));
    }

    #[test]
    fn contains_and_exact() {
        assert!(m(TextMode::Contains, "Final", false, false).is_match("my final v2.doc"));
        assert!(!m(TextMode::Contains, "Final", true, false).is_match("my final v2.doc"));
        assert!(m(TextMode::Exact, "node_modules", false, false).is_match("Node_Modules"));
        assert!(!m(TextMode::Exact, "node_modules", false, false).is_match("node_modules2"));
        assert!(m(TextMode::Contains, "a\\b", false, true).is_match("x/a/b/y"));
    }

    #[test]
    fn regex_is_validated_and_bounded() {
        let r = m(TextMode::Regex, r"^img_\d{4}\.jpe?g$", false, false);
        assert!(r.is_match("IMG_0042.JPG"));
        assert!(!r.is_match("IMG_42.jpg"));
        let e = TextMatcher::new(TextMode::Regex, "(", false, false).unwrap_err();
        assert!(e.starts_with("invalid regular expression"), "{e}");
        assert!(TextMatcher::new(TextMode::Regex, &"a".repeat(2000), false, false).is_err());
        // Compiled size above the limit is refused rather than allocated.
        assert!(TextMatcher::new(TextMode::Regex, r"(\w{200}){50}", false, false).is_err());
        assert!(TextMatcher::new(TextMode::Contains, "", false, false).is_err());
    }
}
