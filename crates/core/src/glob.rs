//! Wildcard / glob matching (in-house, no `glob` crate). Used by scope
//! includes/excludes, protected paths, filters and detectors.
//!
//! Syntax: `?` one character, `*` any run (not crossing `/` in path mode),
//! `**` any run including `/`, `**/` zero or more whole directories,
//! `[abc]`, `[a-z]`, `[!x]` / `[^x]` classes, `\` escapes the next character
//! in name mode. In path mode `\` is a separator (it is converted to `/`
//! in both pattern and text), so Windows patterns work unchanged.
//!
//! Matching is O(pattern x text) by dynamic programming: no exponential blow-up
//! on hostile patterns.

/// How separators are treated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobMode {
    /// Match a single file name: `*` matches anything.
    Name,
    /// Match a path: `*` stops at `/`, `**` crosses it, `\` is a separator.
    Path,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Lit(char),
    One,
    Star,
    DoubleStar,
    /// `**/`: zero or more complete directory components.
    DirStar,
    Class {
        negated: bool,
        items: Vec<(char, char)>,
    },
}

/// A compiled glob pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Glob {
    pattern: String,
    toks: Vec<Tok>,
    mode: GlobMode,
    case_insensitive: bool,
    has_separator: bool,
}

fn fold(c: char, ci: bool) -> char {
    if ci {
        c.to_lowercase().next().unwrap_or(c)
    } else {
        c
    }
}

impl Glob {
    /// Compile `pattern`. Never fails: an unterminated `[` is a literal.
    pub fn new(pattern: &str, mode: GlobMode, case_insensitive: bool) -> Glob {
        let src: String = if mode == GlobMode::Path { pattern.replace('\\', "/") } else { pattern.to_string() };
        let chars: Vec<char> = src.chars().collect();
        let mut toks = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            match c {
                '?' => toks.push(Tok::One),
                '*' => {
                    if i + 1 < chars.len() && chars[i + 1] == '*' {
                        if mode == GlobMode::Path && i + 2 < chars.len() && chars[i + 2] == '/' {
                            toks.push(Tok::DirStar);
                            i += 2;
                        } else {
                            toks.push(Tok::DoubleStar);
                            i += 1;
                        }
                        while i + 1 < chars.len() && chars[i + 1] == '*' {
                            i += 1;
                        }
                    } else {
                        toks.push(Tok::Star);
                    }
                }
                '[' => match parse_class(&chars, i, case_insensitive) {
                    Some((tok, next)) => {
                        toks.push(tok);
                        i = next;
                        continue;
                    }
                    None => toks.push(Tok::Lit(fold('[', case_insensitive))),
                },
                '\\' if mode == GlobMode::Name && i + 1 < chars.len() => {
                    i += 1;
                    toks.push(Tok::Lit(fold(chars[i], case_insensitive)));
                }
                c => toks.push(Tok::Lit(fold(c, case_insensitive))),
            }
            i += 1;
        }
        Glob { has_separator: src.contains('/'), pattern: pattern.to_string(), toks, mode, case_insensitive }
    }

    /// Compile with the platform's default case rule (insensitive on Windows).
    pub fn platform(pattern: &str, mode: GlobMode) -> Glob {
        Glob::new(pattern, mode, cfg!(windows))
    }

    /// The source pattern.
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// True when the pattern contains wildcards (otherwise it is a literal).
    pub fn is_wildcard(&self) -> bool {
        self.toks.iter().any(|t| !matches!(t, Tok::Lit(_)))
    }

    /// Match the whole `text`.
    pub fn matches(&self, text: &str) -> bool {
        let text: Vec<char> = if self.mode == GlobMode::Path {
            text.chars().map(|c| if c == '\\' { '/' } else { fold(c, self.case_insensitive) }).collect()
        } else {
            text.chars().map(|c| fold(c, self.case_insensitive)).collect()
        };
        self.match_chars(&text)
    }

    /// Path-mode convenience: a pattern without any separator is matched
    /// against the last component only (gitignore-style), otherwise against
    /// the whole path.
    pub fn matches_path(&self, path: &std::path::Path) -> bool {
        let s = path.to_string_lossy();
        if self.has_separator {
            self.matches(&s)
        } else {
            let norm = s.replace('\\', "/");
            let name = norm.rsplit('/').next().unwrap_or("");
            self.matches(name)
        }
    }

    fn match_chars(&self, t: &[char]) -> bool {
        let (p, n) = (self.toks.len(), t.len());
        let path = self.mode == GlobMode::Path;
        // dp[i * (n + 1) + j]: toks[i..] matches t[j..]
        let w = n + 1;
        let mut dp = vec![false; (p + 1) * w];
        dp[p * w + n] = true;
        for i in (0..p).rev() {
            for j in (0..=n).rev() {
                let here = |ii: usize, jj: usize| dp[ii * w + jj];
                let v = match &self.toks[i] {
                    Tok::Lit(c) => j < n && t[j] == *c && here(i + 1, j + 1),
                    Tok::One => j < n && !(path && t[j] == '/') && here(i + 1, j + 1),
                    Tok::Star => here(i + 1, j) || (j < n && !(path && t[j] == '/') && here(i, j + 1)),
                    Tok::DoubleStar => here(i + 1, j) || (j < n && here(i, j + 1)),
                    Tok::DirStar => {
                        let boundary = j == 0 || t[j - 1] == '/';
                        (boundary && here(i + 1, j)) || (j < n && here(i, j + 1))
                    }
                    Tok::Class { negated, items } => {
                        j < n
                            && !(path && t[j] == '/')
                            && (items.iter().any(|&(a, b)| a <= t[j] && t[j] <= b) != *negated)
                            && here(i + 1, j + 1)
                    }
                };
                dp[i * w + j] = v;
            }
        }
        dp[0]
    }
}

fn parse_class(chars: &[char], start: usize, ci: bool) -> Option<(Tok, usize)> {
    let mut i = start + 1;
    let negated = i < chars.len() && (chars[i] == '!' || chars[i] == '^');
    if negated {
        i += 1;
    }
    let mut items = Vec::new();
    let mut first = true;
    while i < chars.len() {
        let c = chars[i];
        if c == ']' && !first {
            return Some((Tok::Class { negated, items }, i + 1));
        }
        first = false;
        if i + 2 < chars.len() && chars[i + 1] == '-' && chars[i + 2] != ']' {
            let (a, b) = (fold(c, ci), fold(chars[i + 2], ci));
            items.push((a.min(b), a.max(b)));
            i += 3;
        } else {
            let f = fold(c, ci);
            items.push((f, f));
            i += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(p: &str, t: &str) -> bool {
        Glob::new(p, GlobMode::Name, false).matches(t)
    }

    fn path(p: &str, t: &str) -> bool {
        Glob::new(p, GlobMode::Path, false).matches(t)
    }

    #[test]
    fn name_mode() {
        assert!(name("*.tmp", "a.tmp"));
        assert!(!name("*.tmp", "a.tmpx"));
        assert!(name("~$*", "~$Report.docx"));
        assert!(name("file?.txt", "file1.txt"));
        assert!(!name("file?.txt", "file.txt"));
        assert!(name("[abc]*", "beta"));
        assert!(!name("[!abc]*", "beta"));
        assert!(name("[a-c]x", "bx"));
        assert!(name("\\*lit", "*lit"));
        assert!(!name("\\*lit", "xlit"));
        assert!(name("[unterminated", "[unterminated"));
        assert!(Glob::new("*.JPG", GlobMode::Name, true).matches("a.jpg"));
    }

    #[test]
    fn path_mode() {
        assert!(path("/home/*/Downloads", "/home/me/Downloads"));
        assert!(!path("/home/*/Downloads", "/home/me/x/Downloads"));
        assert!(path("/home/**/node_modules", "/home/me/x/y/node_modules"));
        assert!(path("**/node_modules/**", "/a/node_modules/b/c"));
        assert!(path("/a/**/b", "/a/b"));
        assert!(path("/a/**/b", "/a/x/y/b"));
        assert!(!path("/a/**/b", "/a/xb"));
        assert!(path("C:\\Users\\*\\AppData", "C:/Users/me/AppData"));
    }

    #[test]
    fn matches_path_uses_basename_without_separator() {
        let g = Glob::new("*.log", GlobMode::Path, false);
        assert!(g.matches_path(std::path::Path::new("/var/x/app.log")));
        let g = Glob::new("/var/*.log", GlobMode::Path, false);
        assert!(!g.matches_path(std::path::Path::new("/var/x/app.log")));
    }

    #[test]
    fn hostile_pattern_is_fast() {
        let p = "*a*a*a*a*a*a*a*a*a*a*a*a*b";
        let t = "a".repeat(200);
        assert!(!name(p, &t));
    }
}
