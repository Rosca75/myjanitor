//! Tokeniser of the quick-query syntax. Owner: W2-C.
//!
//! Works on `char_indices` only, so every span is a valid UTF-8 boundary and no
//! input can make it panic. Quoted values (`"a b"` or `'a b'`, no escapes) are
//! recognised only at the start of a value, so apostrophes inside words are
//! ordinary text. A value written `/…/` (closing slash followed by a value
//! terminator) is kept whole, spaces and parentheses included, so regular
//! expressions can be typed unquoted.

use super::QueryError;

/// Longest query accepted (bytes).
pub const MAX_QUERY_BYTES: usize = 8192;
/// Most tokens accepted.
pub const MAX_TOKENS: usize = 2000;
/// Most comma-separated values in one term.
pub const MAX_ITEMS: usize = 256;

/// Byte range in the query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    /// The error at this span.
    pub fn err(self, message: impl Into<String>) -> QueryError {
        QueryError { offset: self.start, len: self.end.saturating_sub(self.start), message: message.into() }
    }
}

/// A comparison operator written before a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
}

impl Op {
    /// Its source text.
    pub fn text(self) -> &'static str {
        match self {
            Op::Lt => "<",
            Op::Le => "<=",
            Op::Gt => ">",
            Op::Ge => ">=",
            Op::Eq => "=",
        }
    }
}

/// One comma-separated value of a term.
#[derive(Debug, Clone)]
pub struct Item {
    /// The text without quotes (the raw `/…/` text for a regex candidate).
    pub text: String,
    pub span: Span,
    pub quoted: bool,
    /// For an unquoted `/inner/` value: the text between the slashes.
    pub regex: Option<String>,
}

/// A term: optional `key:`, optional operator, one or more values.
#[derive(Debug, Clone)]
pub struct Word {
    pub span: Span,
    pub key: Option<(String, Span)>,
    pub op: Option<(Op, Span)>,
    pub items: Vec<Item>,
}

impl Word {
    /// `Some("OR")` etc. when this is a bare unquoted keyword.
    pub fn keyword(&self) -> Option<&'static str> {
        if self.key.is_some() || self.items.len() != 1 || self.items[0].quoted {
            return None;
        }
        match self.items[0].text.as_str() {
            "OR" => Some("OR"),
            "AND" => Some("AND"),
            "NOT" => Some("NOT"),
            _ => None,
        }
    }
}

/// A token.
#[derive(Debug, Clone)]
pub enum Tok {
    LParen(Span),
    RParen(Span),
    /// A `-` directly in front of a term.
    Neg(Span),
    Word(Word),
}

impl Tok {
    /// Where the token is.
    pub fn span(&self) -> Span {
        match self {
            Tok::LParen(s) | Tok::RParen(s) | Tok::Neg(s) => *s,
            Tok::Word(w) => w.span,
        }
    }
}

struct Lexer<'a> {
    q: &'a str,
    chars: Vec<(usize, char)>,
}

impl Lexer<'_> {
    fn off(&self, i: usize) -> usize {
        self.chars.get(i).map(|c| c.0).unwrap_or(self.q.len())
    }

    fn ch(&self, i: usize) -> Option<char> {
        self.chars.get(i).map(|c| c.1)
    }

    fn span(&self, a: usize, b: usize) -> Span {
        Span { start: self.off(a), end: self.off(b) }
    }

    /// Does a value end before index `i`?
    fn value_end(&self, i: usize, has_key: bool) -> bool {
        match self.ch(i) {
            None => true,
            Some(c) => c.is_whitespace() || c == ')' || (has_key && c == ','),
        }
    }

    fn item(&self, i: &mut usize, has_key: bool) -> Result<Item, QueryError> {
        let s = *i;
        let n = self.chars.len();
        if let Some(q @ ('"' | '\'')) = self.ch(s) {
            let close = (s + 1..n).find(|&j| self.chars[j].1 == q);
            let Some(j) = close else {
                return Err(self.span(s, n).err(format!("this {q} has no closing {q}")));
            };
            *i = j + 1;
            if !self.value_end(*i, has_key) {
                return Err(self.span(j, *i + 1).err("unexpected text after the closing quote; add a space"));
            }
            let text = self.q[self.off(s + 1)..self.off(j)].to_string();
            return Ok(Item { text, span: self.span(s, *i), quoted: true, regex: None });
        }
        if self.ch(s) == Some('/') {
            // A regex candidate: the first `/` that is followed by a value terminator closes it.
            let close = (s + 2..n).find(|&j| self.chars[j].1 == '/' && self.value_end(j + 1, has_key));
            if let Some(j) = close {
                *i = j + 1;
                let inner = self.q[self.off(s + 1)..self.off(j)].to_string();
                let text = self.q[self.off(s)..self.off(j + 1)].to_string();
                return Ok(Item { text, span: self.span(s, *i), quoted: false, regex: Some(inner) });
            }
        }
        let mut j = s;
        while let Some(c) = self.ch(j) {
            if c.is_whitespace() || c == '(' || c == ')' || (has_key && c == ',') {
                break;
            }
            j += 1;
        }
        *i = j;
        Ok(Item {
            text: self.q[self.off(s)..self.off(j)].to_string(),
            span: self.span(s, j),
            quoted: false,
            regex: None,
        })
    }

    fn word(&self, i: &mut usize) -> Result<Word, QueryError> {
        let start = *i;
        let mut j = start;
        while self.ch(j).is_some_and(|c| c.is_ascii_alphabetic() || c == '_') {
            j += 1;
        }
        let mut key = None;
        if j > start && self.ch(j) == Some(':') {
            let name = self.q[self.off(start)..self.off(j)].to_ascii_lowercase();
            key = Some((name, self.span(start, j + 1)));
            *i = j + 1;
        }
        let mut op = None;
        if key.is_some() {
            let (c0, c1) = (self.ch(*i), self.ch(*i + 1));
            let found = match (c0, c1) {
                (Some('<'), Some('=')) => Some((Op::Le, 2)),
                (Some('>'), Some('=')) => Some((Op::Ge, 2)),
                (Some('<'), _) => Some((Op::Lt, 1)),
                (Some('>'), _) => Some((Op::Gt, 1)),
                (Some('='), _) => Some((Op::Eq, 1)),
                _ => None,
            };
            if let Some((o, len)) = found {
                op = Some((o, self.span(*i, *i + len)));
                *i += len;
            }
        }
        let has_key = key.is_some();
        let mut items = Vec::new();
        loop {
            items.push(self.item(i, has_key)?);
            if has_key && self.ch(*i) == Some(',') {
                *i += 1;
                if items.len() >= MAX_ITEMS {
                    return Err(self.span(start, *i).err(format!("more than {MAX_ITEMS} values in one term")));
                }
                continue;
            }
            break;
        }
        Ok(Word { span: self.span(start, *i), key, op, items })
    }
}

/// Tokenise a query.
pub fn lex(q: &str) -> Result<Vec<Tok>, QueryError> {
    if q.len() > MAX_QUERY_BYTES {
        return Err(Span { start: MAX_QUERY_BYTES.min(q.len()), end: q.len() }
            .err(format!("the query is longer than {MAX_QUERY_BYTES} bytes")));
    }
    let lx = Lexer { q, chars: q.char_indices().collect() };
    let n = lx.chars.len();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < n {
        let c = lx.chars[i].1;
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if toks.len() >= MAX_TOKENS {
            return Err(lx.span(i, n).err(format!("more than {MAX_TOKENS} terms in the query")));
        }
        match c {
            '(' => {
                let glued = i > 0 && !matches!(lx.chars[i - 1].1, c if c.is_whitespace() || c == '(' || c == '-');
                if glued && !matches!(toks.last(), Some(Tok::RParen(_))) {
                    return Err(lx.span(i, i + 1).err("put a space before '(' or quote the value, e.g. name:\"a(1)\""));
                }
                toks.push(Tok::LParen(lx.span(i, i + 1)));
                i += 1;
            }
            ')' => {
                toks.push(Tok::RParen(lx.span(i, i + 1)));
                i += 1;
            }
            '-' => {
                toks.push(Tok::Neg(lx.span(i, i + 1)));
                i += 1;
            }
            _ => toks.push(Tok::Word(lx.word(&mut i)?)),
        }
    }
    Ok(toks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(q: &str) -> Vec<Word> {
        lex(q).unwrap().into_iter().filter_map(|t| if let Tok::Word(w) = t { Some(w) } else { None }).collect()
    }

    #[test]
    fn keys_ops_lists_and_spans() {
        let w = &words("size:>=1GB ext:jpg,png")[..];
        assert_eq!(w[0].key.as_ref().unwrap().0, "size");
        assert_eq!(w[0].op.unwrap().0, Op::Ge);
        assert_eq!(w[0].items[0].text, "1GB");
        assert_eq!(w[1].items.iter().map(|i| i.text.as_str()).collect::<Vec<_>>(), ["jpg", "png"]);
        assert_eq!((w[1].span.start, w[1].span.end), (11, 22));
    }

    #[test]
    fn quotes_and_regex_values() {
        let w = &words(r#"name:"final v2*" 'a b' don't name:/^a (b|c)$/ x"#)[..];
        assert!(w[0].items[0].quoted && w[0].items[0].text == "final v2*");
        assert_eq!(w[1].items[0].text, "a b");
        assert_eq!(w[2].items[0].text, "don't");
        assert_eq!(w[3].items[0].regex.as_deref(), Some("^a (b|c)$"));
        assert_eq!(w[4].items[0].text, "x");
    }

    #[test]
    fn slash_paths_are_not_regexes_without_a_closing_slash() {
        let w = &words("path:/home/user/*")[..];
        assert!(w[0].items[0].regex.is_none());
        assert_eq!(w[0].items[0].text, "/home/user/*");
    }

    #[test]
    fn errors_have_positions() {
        let e = lex(r#"ext:mp4 name:"oops"#).unwrap_err();
        assert_eq!(e.offset, 13);
        assert!(lex("a(b)").is_err());
        assert!(lex("a (b)").is_ok());
        assert!(lex(&"a ".repeat(5000)).is_err());
        assert!(lex("name:\"a\"b").is_err());
    }

    #[test]
    fn keywords_and_unicode_do_not_panic() {
        assert_eq!(words("OR")[0].keyword(), Some("OR"));
        assert_eq!(words("or")[0].keyword(), None);
        assert_eq!(words("\"OR\"")[0].keyword(), None);
        let _ = lex("名前:ñ ünï \u{200b}:");
    }
}
