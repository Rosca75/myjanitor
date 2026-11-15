//! Minimal, hardened XML pull parser and byte-range editor for OOXML parts. Owner: W3-A.
//!
//! Two layers:
//!
//! - [`Tokenizer`] is a lossless pull tokenizer: every token carries the byte
//!   span it was read from, and the spans of all tokens tile the input exactly,
//!   so callers can re-emit untouched parts of a document byte-for-byte and
//!   change only what they mean to change ([`Edits`] applies byte-range
//!   replacements). It checks well-formedness as it goes (matching tags, one
//!   root, unique attributes, valid names, characters and references) and
//!   tracks namespace declarations so OOXML elements can be matched by
//!   namespace URI rather than by prefix.
//! - [`XmlReader`] / [`XmlEvent`] is the simple event API on top of it
//!   (decoded attribute values and text).
//!
//! Hostile input rules: no DTD at all (`<!DOCTYPE` is rejected, so there is no
//! entity expansion beyond the five predefined entities and numeric character
//! references), depth / size / attribute-count limits ([`Limits`]), every
//! index access is bounds-checked and nothing panics. Parsing is linear in the
//! input size (each byte is examined a bounded number of times).

use std::borrow::Cow;
use std::ops::Range;

use mj_core::{Error, Result};

/// Resource limits applied while tokenizing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Maximum element nesting depth.
    pub max_depth: usize,
    /// Maximum input size in bytes.
    pub max_input: usize,
    /// Maximum attributes on one element.
    pub max_attributes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_depth: 256, max_input: 512 * 1024 * 1024, max_attributes: 4096 }
    }
}

fn perr(msg: impl Into<String>) -> Error {
    Error::parse("XML document", msg)
}

/// Kind of a [`Token`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// `<?xml ...?>` at the very start.
    Declaration,
    /// `<?target ...?>`.
    ProcessingInstruction,
    /// `<!-- ... -->`.
    Comment,
    /// `<![CDATA[ ... ]]>`.
    CData,
    /// Character data (raw, references not yet decoded).
    Text,
    /// `<name ...>`.
    StartTag,
    /// `<name .../>`.
    EmptyTag,
    /// `</name>`.
    EndTag,
}

/// One attribute of a start or empty tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute<'a> {
    /// Qualified name as written (`r:id`).
    pub name: &'a str,
    /// Value as written between the quotes (references not decoded).
    pub raw_value: &'a str,
    /// Byte span of the value (between the quotes) in the input.
    pub value_span: Range<usize>,
    /// Byte span of the whole attribute, from the name to the closing quote.
    pub span: Range<usize>,
}

impl<'a> Attribute<'a> {
    /// Decoded value (references resolved, whitespace normalised as XML requires).
    pub fn value(&self) -> Cow<'a, str> {
        // The tokenizer validated the references, so decoding cannot fail here.
        unescape(self.raw_value, true).unwrap_or(Cow::Borrowed(self.raw_value))
    }

    /// Prefix part of the name (`r` in `r:id`), empty when unprefixed.
    pub fn prefix(&self) -> &'a str {
        split_qname(self.name).0
    }

    /// Local part of the name (`id` in `r:id`).
    pub fn local_name(&self) -> &'a str {
        split_qname(self.name).1
    }
}

/// One token. `span` indexes the input given to the [`Tokenizer`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token<'a> {
    pub kind: TokenKind,
    pub span: Range<usize>,
    /// Element name (tags) or PI target; empty otherwise.
    pub name: &'a str,
    /// Attributes (start and empty tags only).
    pub attrs: Vec<Attribute<'a>>,
    /// Nesting depth of the element this token opens or closes (root = 1);
    /// for other tokens, the depth of the enclosing element (0 outside the root).
    pub depth: usize,
    raw: &'a str,
}

impl<'a> Token<'a> {
    /// The exact source text of the token.
    pub fn raw(&self) -> &'a str {
        self.raw
    }

    /// Prefix of the element name (empty when unprefixed).
    pub fn prefix(&self) -> &'a str {
        split_qname(self.name).0
    }

    /// Local part of the element name.
    pub fn local_name(&self) -> &'a str {
        split_qname(self.name).1
    }

    /// The attribute with qualified name `name`.
    pub fn attr(&self, name: &str) -> Option<&Attribute<'a>> {
        self.attrs.iter().find(|a| a.name == name)
    }

    /// True for start and empty tags.
    pub fn is_open(&self) -> bool {
        matches!(self.kind, TokenKind::StartTag | TokenKind::EmptyTag)
    }

    /// Decoded character data for `Text` and `CData` tokens (empty otherwise).
    pub fn text(&self) -> Cow<'a, str> {
        match self.kind {
            TokenKind::Text => unescape(self.raw, false).unwrap_or(Cow::Borrowed(self.raw)),
            TokenKind::CData => Cow::Borrowed(self.raw.get(9..self.raw.len().saturating_sub(3)).unwrap_or_default()),
            _ => Cow::Borrowed(""),
        }
    }
}

/// Split `p:name` into (`p`, `name`).
pub fn split_qname(name: &str) -> (&str, &str) {
    match name.split_once(':') {
        Some((p, l)) => (p, l),
        None => ("", name),
    }
}

/// Namespace URI of the `xml` prefix (always bound).
pub const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

#[derive(Debug, Clone)]
struct NsBinding<'a> {
    depth: usize,
    prefix: &'a str,
    uri: Cow<'a, str>,
}

/// Lossless, well-formedness-checking pull tokenizer.
pub struct Tokenizer<'a> {
    input: &'a str,
    pos: usize,
    stack: Vec<&'a str>,
    limits: Limits,
    seen_root: bool,
    ns: Vec<NsBinding<'a>>,
    /// Depth whose namespace bindings must be dropped before the next token
    /// (after an end tag or an empty tag).
    pending_pop: Option<usize>,
    failed: bool,
}

impl<'a> Tokenizer<'a> {
    /// Tokenizer with default [`Limits`]. A leading byte-order mark is skipped
    /// (it never appears in a token span; callers that re-emit keep it themselves).
    pub fn new(input: &'a str) -> Result<Self> {
        Self::with_limits(input, Limits::default())
    }

    /// Tokenizer with explicit limits.
    pub fn with_limits(input: &'a str, limits: Limits) -> Result<Self> {
        if input.len() > limits.max_input {
            return Err(perr(format!("document larger than the {}-byte limit", limits.max_input)));
        }
        let pos = if input.starts_with('\u{feff}') { 3 } else { 0 };
        Ok(Tokenizer {
            input,
            pos,
            stack: Vec::new(),
            limits,
            seen_root: false,
            ns: Vec::new(),
            pending_pop: None,
            failed: false,
        })
    }

    /// The input.
    pub fn input(&self) -> &'a str {
        self.input
    }

    /// Current element depth (number of open elements).
    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    /// Namespace URI bound to `prefix` at the current position (`""` = default namespace).
    pub fn resolve_prefix(&self, prefix: &str) -> Option<&str> {
        if prefix == "xml" {
            return Some(XML_NS);
        }
        self.ns.iter().rev().find(|b| b.prefix == prefix).map(|b| b.uri.as_ref()).filter(|u| !u.is_empty())
    }

    /// Namespace URI of a tag token returned by the last call.
    pub fn element_namespace(&self, t: &Token<'_>) -> Option<&str> {
        self.resolve_prefix(t.prefix())
    }

    /// Namespace URI of an attribute of the last tag token (unprefixed attributes have none).
    pub fn attr_namespace(&self, a: &Attribute<'_>) -> Option<&str> {
        let p = a.prefix();
        if p.is_empty() {
            None
        } else {
            self.resolve_prefix(p)
        }
    }

    /// Next token, `Ok(None)` at a well-formed end of input.
    pub fn next_token(&mut self) -> Result<Option<Token<'a>>> {
        if self.failed {
            return Err(perr("tokenizer already failed"));
        }
        let r = self.next_inner();
        if r.is_err() {
            self.failed = true;
        }
        r
    }

    fn next_inner(&mut self) -> Result<Option<Token<'a>>> {
        if let Some(d) = self.pending_pop.take() {
            self.ns.retain(|b| b.depth < d);
        }
        let input = self.input;
        let bytes = input.as_bytes();
        let start = self.pos;
        if start >= bytes.len() {
            if let Some(open) = self.stack.last() {
                return Err(perr(format!("element <{open}> is not closed")));
            }
            if !self.seen_root {
                return Err(perr("no root element"));
            }
            return Ok(None);
        }
        let depth = self.stack.len();
        if bytes[start] != b'<' {
            let end = input[start..].find('<').map(|i| start + i).unwrap_or(bytes.len());
            let raw = &input[start..end];
            if depth == 0 {
                if !raw.bytes().all(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n')) {
                    return Err(perr(format!("text outside the root element at byte {start}")));
                }
            } else {
                if raw.contains("]]>") {
                    return Err(perr(format!("']]>' in character data at byte {start}")));
                }
                check_chars(raw, start)?;
                unescape(raw, false)?;
            }
            self.pos = end;
            return Ok(Some(self.token(TokenKind::Text, start..end, "", Vec::new(), depth)));
        }
        let rest = &input[start..];
        if rest.starts_with("<?") {
            let close = rest.find("?>").ok_or_else(|| perr("unterminated processing instruction"))?;
            let end = start + close + 2;
            let name_end = scan_name(bytes, start + 2);
            let target = &input[start + 2..name_end];
            if target.is_empty() {
                return Err(perr(format!("processing instruction without a target at byte {start}")));
            }
            check_chars(&input[start..end], start)?;
            let kind = if target.eq_ignore_ascii_case("xml") {
                let first = if input.starts_with('\u{feff}') { 3 } else { 0 };
                if target != "xml" || start != first {
                    return Err(perr("misplaced XML declaration"));
                }
                TokenKind::Declaration
            } else {
                if name_end < end - 2 && !is_space(bytes[name_end]) {
                    return Err(perr(format!("bad processing instruction target at byte {start}")));
                }
                TokenKind::ProcessingInstruction
            };
            self.pos = end;
            return Ok(Some(self.token(kind, start..end, target, Vec::new(), depth)));
        }
        if let Some(after) = rest.strip_prefix("<!--") {
            let close = after.find("-->").ok_or_else(|| perr("unterminated comment"))?;
            let body = &rest[4..4 + close];
            if body.contains("--") || body.ends_with('-') {
                return Err(perr(format!("'--' inside a comment at byte {start}")));
            }
            let end = start + 4 + close + 3;
            check_chars(body, start)?;
            self.pos = end;
            return Ok(Some(self.token(TokenKind::Comment, start..end, "", Vec::new(), depth)));
        }
        if let Some(after) = rest.strip_prefix("<![CDATA[") {
            if depth == 0 {
                return Err(perr("CDATA section outside the root element"));
            }
            let close = after.find("]]>").ok_or_else(|| perr("unterminated CDATA section"))?;
            let end = start + 9 + close + 3;
            check_chars(&rest[9..9 + close], start)?;
            self.pos = end;
            return Ok(Some(self.token(TokenKind::CData, start..end, "", Vec::new(), depth)));
        }
        if rest.starts_with("<!") {
            if rest.len() >= 9 && rest[2..9].eq_ignore_ascii_case("DOCTYPE") {
                return Err(perr("document type declarations (DOCTYPE) are not accepted"));
            }
            return Err(perr(format!("unsupported markup declaration at byte {start}")));
        }
        if rest.starts_with("</") {
            let name_end = scan_name(bytes, start + 2);
            let name = &input[start + 2..name_end];
            if name.is_empty() {
                return Err(perr(format!("end tag without a name at byte {start}")));
            }
            let mut p = skip_space(bytes, name_end);
            if bytes.get(p) != Some(&b'>') {
                return Err(perr(format!("malformed end tag </{name}>")));
            }
            p += 1;
            match self.stack.pop() {
                Some(open) if open == name => {}
                Some(open) => return Err(perr(format!("</{name}> closes <{open}>"))),
                None => return Err(perr(format!("</{name}> without a start tag"))),
            }
            self.pending_pop = Some(depth);
            self.pos = p;
            return Ok(Some(self.token(TokenKind::EndTag, start..p, name, Vec::new(), depth)));
        }
        // Start or empty tag.
        let name_end = scan_name(bytes, start + 1);
        let name = &input[start + 1..name_end];
        if name.is_empty() || !valid_qname(name) {
            return Err(perr(format!("invalid element name at byte {start}")));
        }
        if depth == 0 && self.seen_root {
            return Err(perr("more than one root element"));
        }
        let mut attrs: Vec<Attribute<'a>> = Vec::new();
        let mut p = name_end;
        let empty;
        loop {
            let q = skip_space(bytes, p);
            match bytes.get(q) {
                None => return Err(perr(format!("unterminated tag <{name}>"))),
                Some(b'>') => {
                    p = q + 1;
                    empty = false;
                    break;
                }
                Some(b'/') => {
                    if bytes.get(q + 1) != Some(&b'>') {
                        return Err(perr(format!("malformed tag <{name}>")));
                    }
                    p = q + 2;
                    empty = true;
                    break;
                }
                Some(_) => {}
            }
            if q == p {
                return Err(perr(format!("missing space between attributes in <{name}>")));
            }
            let an_end = scan_name(bytes, q);
            let an = &input[q..an_end];
            if an.is_empty() || !valid_qname(an) {
                return Err(perr(format!("invalid attribute name in <{name}>")));
            }
            let mut r = skip_space(bytes, an_end);
            if bytes.get(r) != Some(&b'=') {
                return Err(perr(format!("attribute {an} without a value in <{name}>")));
            }
            r = skip_space(bytes, r + 1);
            let quote = match bytes.get(r) {
                Some(&c @ (b'"' | b'\'')) => c,
                _ => return Err(perr(format!("unquoted attribute {an} in <{name}>"))),
            };
            let vstart = r + 1;
            let vlen = input[vstart..]
                .find(quote as char)
                .ok_or_else(|| perr(format!("unterminated attribute {an} in <{name}>")))?;
            let vend = vstart + vlen;
            let raw_value = &input[vstart..vend];
            if raw_value.contains('<') {
                return Err(perr(format!("'<' in attribute {an} of <{name}>")));
            }
            check_chars(raw_value, vstart)?;
            unescape(raw_value, true)?;
            if attrs.iter().any(|a| a.name == an) {
                return Err(perr(format!("duplicate attribute {an} in <{name}>")));
            }
            if attrs.len() >= self.limits.max_attributes {
                return Err(perr(format!("too many attributes in <{name}>")));
            }
            attrs.push(Attribute { name: an, raw_value, value_span: vstart..vend, span: q..vend + 1 });
            p = vend + 1;
        }
        let new_depth = depth + 1;
        if new_depth > self.limits.max_depth {
            return Err(perr(format!("element nesting deeper than {}", self.limits.max_depth)));
        }
        self.seen_root = true;
        // Namespace declarations bind at this element's depth.
        for a in &attrs {
            if a.name == "xmlns" {
                self.ns.push(NsBinding { depth: new_depth, prefix: "", uri: a.value() });
            } else if let Some(pfx) = a.name.strip_prefix("xmlns:") {
                let uri = a.value();
                if uri.is_empty() {
                    return Err(perr(format!("prefix {pfx} bound to an empty namespace")));
                }
                self.ns.push(NsBinding { depth: new_depth, prefix: pfx, uri });
            }
        }
        let kind = if empty {
            self.pending_pop = Some(new_depth);
            TokenKind::EmptyTag
        } else {
            self.stack.push(name);
            TokenKind::StartTag
        };
        self.pos = p;
        Ok(Some(self.token(kind, start..p, name, attrs, new_depth)))
    }

    fn token(
        &self,
        kind: TokenKind,
        span: Range<usize>,
        name: &'a str,
        attrs: Vec<Attribute<'a>>,
        depth: usize,
    ) -> Token<'a> {
        let raw = &self.input[span.clone()];
        Token { kind, span, name, attrs, depth, raw }
    }
}

fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n')
}

fn skip_space(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && is_space(b[p]) {
        p += 1;
    }
    p
}

/// End of a name starting at `p` (ASCII name characters, `:` and any non-ASCII byte).
fn scan_name(b: &[u8], mut p: usize) -> usize {
    while p < b.len() {
        let c = b[p];
        if c.is_ascii_alphanumeric() || matches!(c, b'_' | b':' | b'-' | b'.') || c >= 0x80 {
            p += 1;
        } else {
            break;
        }
    }
    p
}

/// A qualified name: does not start with a digit, `-` or `.`, at most one `:`, no empty part.
fn valid_qname(n: &str) -> bool {
    let ok_part = |s: &str| match s.chars().next() {
        Some(c) => !(c.is_ascii_digit() || c == '-' || c == '.'),
        None => false,
    };
    match n.split_once(':') {
        Some((p, l)) => ok_part(p) && ok_part(l) && !l.contains(':'),
        None => ok_part(n),
    }
}

/// True for characters allowed in XML 1.0 documents.
fn is_xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}')
}

fn check_chars(s: &str, at: usize) -> Result<()> {
    if s.bytes().all(|b| b >= 0x20 || matches!(b, b'\t' | b'\n' | b'\r')) && !s.contains(['\u{FFFE}', '\u{FFFF}']) {
        return Ok(());
    }
    match s.char_indices().find(|(_, c)| !is_xml_char(*c)) {
        Some((i, c)) => Err(perr(format!("invalid character U+{:04X} at byte {}", c as u32, at + i))),
        None => Ok(()),
    }
}

/// Decode references (`&lt;`, `&#38;`...). With `attr`, literal tabs and line
/// breaks become spaces (attribute-value normalisation). Unknown entities and
/// bare `&` are errors.
pub fn unescape(s: &str, attr: bool) -> Result<Cow<'_, str>> {
    let needs_ws = attr && s.contains(['\t', '\n', '\r']);
    if !s.contains('&') && !needs_ws && !s.contains('\r') {
        return Ok(Cow::Borrowed(s));
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(['&', '\t', '\n', '\r']) {
        out.push_str(&rest[..i]);
        let c = rest.as_bytes()[i];
        if c != b'&' {
            if attr {
                // "\r\n" counts as one line break.
                out.push(' ');
                if c == b'\r' && rest.as_bytes().get(i + 1) == Some(&b'\n') {
                    rest = &rest[i + 2..];
                    continue;
                }
            } else if c == b'\r' {
                out.push('\n');
                if rest.as_bytes().get(i + 1) == Some(&b'\n') {
                    rest = &rest[i + 2..];
                    continue;
                }
            } else {
                out.push(c as char);
            }
            rest = &rest[i + 1..];
            continue;
        }
        let semi = rest[i..].find(';').ok_or_else(|| perr("'&' without a reference"))?;
        let ent = &rest[i + 1..i + semi];
        let ch = match ent {
            "lt" => '<',
            "gt" => '>',
            "amp" => '&',
            "quot" => '"',
            "apos" => '\'',
            _ => {
                let code = if let Some(h) = ent.strip_prefix("#x") {
                    if h.is_empty() || h.len() > 8 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
                        return Err(perr(format!("bad character reference &{ent};")));
                    }
                    u32::from_str_radix(h, 16).ok()
                } else if let Some(d) = ent.strip_prefix('#') {
                    if d.is_empty() || d.len() > 10 || !d.bytes().all(|b| b.is_ascii_digit()) {
                        return Err(perr(format!("bad character reference &{ent};")));
                    }
                    d.parse::<u32>().ok()
                } else {
                    return Err(perr(format!("unknown entity &{};", ent.chars().take(32).collect::<String>())));
                };
                match code.and_then(char::from_u32).filter(|c| is_xml_char(*c)) {
                    Some(c) => c,
                    None => return Err(perr(format!("character reference &{ent}; is not an XML character"))),
                }
            }
        };
        out.push(ch);
        rest = &rest[i + semi + 1..];
    }
    out.push_str(rest);
    Ok(Cow::Owned(out))
}

/// Escape text for an attribute value delimited by `"`.
pub fn escape_attr(s: &str) -> Cow<'_, str> {
    if !s.contains(['&', '<', '>', '"', '\'', '\t', '\n', '\r']) {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            c => out.push(c),
        }
    }
    Cow::Owned(out)
}

/// Escape character data.
pub fn escape_text(s: &str) -> Cow<'_, str> {
    if !s.contains(['&', '<', '>', '\r']) {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#13;"),
            c => out.push(c),
        }
    }
    Cow::Owned(out)
}

/// A UTF-8 view of an XML part's bytes: the BOM (if any) and the text after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlText<'a> {
    /// Leading UTF-8 byte-order mark, kept for byte-identical re-emission.
    pub bom: &'a [u8],
    pub text: &'a str,
}

/// View `bytes` as UTF-8 XML (optionally with a UTF-8 BOM). UTF-16 documents are
/// refused here (they can be checked with [`decode_any`] but are never edited).
pub fn as_utf8(bytes: &[u8]) -> Result<XmlText<'_>> {
    let (bom, rest) = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) { bytes.split_at(3) } else { (&bytes[..0], bytes) };
    if rest.starts_with(&[0xFF, 0xFE]) || rest.starts_with(&[0xFE, 0xFF]) {
        return Err(perr("UTF-16 document (only UTF-8 parts are edited)"));
    }
    let text = std::str::from_utf8(rest).map_err(|e| perr(format!("not UTF-8: {e}")))?;
    Ok(XmlText { bom, text })
}

/// Decode an XML document in UTF-8 (with or without BOM) or UTF-16 (with BOM).
pub fn decode_any(bytes: &[u8]) -> Result<Cow<'_, str>> {
    let utf16 = |b: &[u8], le: bool| -> Result<String> {
        if !b.len().is_multiple_of(2) {
            return Err(perr("odd-length UTF-16 document"));
        }
        let units =
            b.chunks_exact(2)
                .map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) });
        char::decode_utf16(units).collect::<std::result::Result<String, _>>().map_err(|_| perr("invalid UTF-16"))
    };
    if let Some(b) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(b, true).map(Cow::Owned);
    }
    if let Some(b) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(b, false).map(Cow::Owned);
    }
    let b = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    std::str::from_utf8(b).map(Cow::Borrowed).map_err(|e| perr(format!("not UTF-8: {e}")))
}

/// Check that `bytes` is a well-formed XML document (UTF-8 or UTF-16 with BOM).
pub fn check_well_formed(bytes: &[u8]) -> Result<()> {
    let text = decode_any(bytes)?;
    let mut t = Tokenizer::new(&text)?;
    while t.next_token()?.is_some() {}
    Ok(())
}

/// A set of non-overlapping byte-range replacements, applied in one pass so
/// that every byte outside the ranges is copied unchanged.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Edits {
    list: Vec<(Range<usize>, String)>,
}

impl Edits {
    /// No edits.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace `range` with `with`.
    pub fn replace(&mut self, range: Range<usize>, with: impl Into<String>) {
        self.list.push((range, with.into()));
    }

    /// Delete `range`.
    pub fn delete(&mut self, range: Range<usize>) {
        self.list.push((range, String::new()));
    }

    /// Insert `text` at byte `at`.
    pub fn insert(&mut self, at: usize, text: impl Into<String>) {
        self.list.push((at..at, text.into()));
    }

    /// True when nothing is to be changed.
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// Number of edits.
    pub fn len(&self) -> usize {
        self.list.len()
    }

    /// Apply to `input`. Overlapping ranges, ranges outside the input or not on
    /// character boundaries are refused.
    pub fn apply(mut self, input: &str) -> Result<String> {
        self.list.sort_by_key(|(r, _)| (r.start, r.end));
        let mut out = String::with_capacity(input.len());
        let mut at = 0usize;
        for (r, s) in &self.list {
            if r.start < at || r.end < r.start || r.end > input.len() {
                return Err(Error::Internal(format!("overlapping or invalid XML edit {r:?}")));
            }
            let before = input.get(at..r.start).ok_or_else(|| Error::Internal("edit not on a char boundary".into()))?;
            out.push_str(before);
            out.push_str(s);
            at = r.end;
        }
        out.push_str(input.get(at..).ok_or_else(|| Error::Internal("edit not on a char boundary".into()))?);
        Ok(out)
    }
}

/// Apply `edits` to an XML part's bytes, keeping its BOM.
pub fn apply_edits(x: &XmlText<'_>, edits: Edits) -> Result<Vec<u8>> {
    let body = edits.apply(x.text)?;
    let mut out = Vec::with_capacity(x.bom.len() + body.len());
    out.extend_from_slice(x.bom);
    out.extend_from_slice(body.as_bytes());
    Ok(out)
}

/// One parse event of the simple API. Borrowed from the input where possible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XmlEvent<'a> {
    /// A start tag; `self_closing` for `<x/>` (no matching `End` follows).
    Start {
        name: &'a str,
        attrs: Vec<(&'a str, String)>,
        self_closing: bool,
    },
    End {
        name: &'a str,
    },
    /// Character data (references decoded) or a CDATA section.
    Text(String),
    Comment(&'a str),
    /// Processing instruction or the XML declaration (content between `<?` and `?>`).
    ProcessingInstruction(&'a str),
    Eof,
}

/// Simple pull parser over a UTF-8 document (see the module docs for the rules).
pub struct XmlReader<'a> {
    inner: Option<Tokenizer<'a>>,
    error: Option<Error>,
}

impl<'a> XmlReader<'a> {
    /// Parser over `input`.
    pub fn new(input: &'a str) -> Self {
        match Tokenizer::new(input) {
            Ok(t) => XmlReader { inner: Some(t), error: None },
            Err(e) => XmlReader { inner: None, error: Some(e) },
        }
    }

    /// Next event. Whitespace-only text outside the root element is skipped.
    pub fn next_event(&mut self) -> Result<XmlEvent<'a>> {
        let Some(t) = self.inner.as_mut() else {
            return Err(self.error.clone().unwrap_or_else(|| perr("parser failed")));
        };
        loop {
            let Some(tok) = t.next_token()? else { return Ok(XmlEvent::Eof) };
            return Ok(match tok.kind {
                TokenKind::Text if tok.depth == 0 => continue,
                TokenKind::Text | TokenKind::CData => XmlEvent::Text(tok.text().into_owned()),
                TokenKind::Comment => XmlEvent::Comment(&tok.raw()[4..tok.raw().len() - 3]),
                TokenKind::Declaration | TokenKind::ProcessingInstruction => {
                    XmlEvent::ProcessingInstruction(&tok.raw()[2..tok.raw().len() - 2])
                }
                TokenKind::StartTag | TokenKind::EmptyTag => XmlEvent::Start {
                    name: tok.name,
                    attrs: tok.attrs.iter().map(|a| (a.name, a.value().into_owned())).collect(),
                    self_closing: tok.kind == TokenKind::EmptyTag,
                },
                TokenKind::EndTag => XmlEvent::End { name: tok.name },
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(s: &str) -> Result<Vec<Token<'_>>> {
        let mut t = Tokenizer::new(s)?;
        let mut out = Vec::new();
        while let Some(x) = t.next_token()? {
            out.push(x);
        }
        Ok(out)
    }

    const DOC: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n\
<!-- c --><r:root xmlns:r=\"urn:r\" xmlns=\"urn:d\" a='1 &amp; 2'>\n  <x r:id=\"rId1\"/>\
<y>t&lt;&#65;&#x42;<![CDATA[<raw>&]]></y><?pi data?></r:root>\n";

    #[test]
    fn spans_tile_the_input() {
        let toks = tokens(DOC).unwrap();
        let joined: String = toks.iter().map(|t| t.raw()).collect();
        assert_eq!(joined, DOC);
        let mut at = 0;
        for t in &toks {
            assert_eq!(t.span.start, at);
            at = t.span.end;
        }
        assert_eq!(at, DOC.len());
    }

    #[test]
    fn conformance_basics() {
        let toks = tokens(DOC).unwrap();
        assert_eq!(toks[0].kind, TokenKind::Declaration);
        let root = toks.iter().find(|t| t.name == "r:root").unwrap();
        assert_eq!(root.kind, TokenKind::StartTag);
        assert_eq!(root.attr("a").unwrap().value(), "1 & 2");
        assert_eq!(root.depth, 1);
        let x = toks.iter().find(|t| t.name == "x").unwrap();
        assert_eq!(x.kind, TokenKind::EmptyTag);
        assert_eq!(x.attr("r:id").unwrap().local_name(), "id");
        let text = toks.iter().find(|t| t.kind == TokenKind::Text && t.depth == 2).unwrap();
        assert_eq!(text.text(), "t<AB");
        let cdata = toks.iter().find(|t| t.kind == TokenKind::CData).unwrap();
        assert_eq!(cdata.text(), "<raw>&");
        assert!(toks.iter().any(|t| t.kind == TokenKind::ProcessingInstruction && t.name == "pi"));
    }

    #[test]
    fn namespaces_resolve_by_uri() {
        let s = "<a xmlns='urn:d' xmlns:p='urn:p'><p:b p:x='1'/><c xmlns='urn:e'><d/></c><e/></a>";
        let mut t = Tokenizer::new(s).unwrap();
        let mut seen = Vec::new();
        while let Some(tok) = t.next_token().unwrap() {
            if tok.is_open() {
                let ns = t.element_namespace(&tok).map(str::to_string);
                let ans =
                    tok.attrs.iter().find(|a| a.name == "p:x").and_then(|a| t.attr_namespace(a)).map(str::to_string);
                seen.push((tok.name.to_string(), ns, ans));
            }
        }
        assert_eq!(seen[0].1.as_deref(), Some("urn:d"));
        assert_eq!(seen[1], ("p:b".into(), Some("urn:p".into()), Some("urn:p".into())));
        assert_eq!(seen[2].1.as_deref(), Some("urn:e"));
        assert_eq!(seen[3].1.as_deref(), Some("urn:e"));
        assert_eq!(seen[4].1.as_deref(), Some("urn:d"), "binding of <c> must not leak");
    }

    #[test]
    fn event_api() {
        let mut r = XmlReader::new("<?xml version='1.0'?><a b=\"&quot;\"><c/>x</a>");
        assert_eq!(r.next_event().unwrap(), XmlEvent::ProcessingInstruction("xml version='1.0'"));
        assert_eq!(
            r.next_event().unwrap(),
            XmlEvent::Start { name: "a", attrs: vec![("b", "\"".into())], self_closing: false }
        );
        assert_eq!(r.next_event().unwrap(), XmlEvent::Start { name: "c", attrs: vec![], self_closing: true });
        assert_eq!(r.next_event().unwrap(), XmlEvent::Text("x".into()));
        assert_eq!(r.next_event().unwrap(), XmlEvent::End { name: "a" });
        assert_eq!(r.next_event().unwrap(), XmlEvent::Eof);
    }

    #[test]
    fn attribute_whitespace_normalisation() {
        let toks = tokens("<a v='x\ty\r\nz&#10;'/>").unwrap();
        assert_eq!(toks[0].attr("v").unwrap().value(), "x y z\n");
    }

    #[test]
    fn bom_is_skipped() {
        let s = "\u{feff}<?xml version='1.0'?><a/>";
        let toks = tokens(s).unwrap();
        assert_eq!(toks[0].kind, TokenKind::Declaration);
        assert_eq!(toks[0].span.start, 3);
        assert!(check_well_formed(s.as_bytes()).is_ok());
        let x = as_utf8(s.as_bytes()).unwrap();
        assert_eq!(x.bom.len(), 3);
    }

    #[test]
    fn utf16_is_checked_not_edited() {
        let mut b = vec![0xFF, 0xFE];
        for u in "<a>é</a>".encode_utf16() {
            b.extend_from_slice(&u.to_le_bytes());
        }
        assert!(check_well_formed(&b).is_ok());
        assert!(as_utf8(&b).is_err());
    }

    #[test]
    fn hostile_inputs_are_refused() {
        let bad = [
            "",
            "   ",
            "<!DOCTYPE lolz [<!ENTITY lol \"lol\"><!ENTITY lol2 \"&lol;&lol;\">]><a>&lol2;</a>",
            "<a>&lol;</a>",
            "<a>&#0;</a>",
            "<a>&#xD800;</a>",
            "<a>&#x110000;</a>",
            "<a>&#99999999999;</a>",
            "<a>& b</a>",
            "<a><b></a></b>",
            "<a>",
            "<a></a><b/>",
            "x<a/>",
            "<a/>x",
            "<a b='<'/>",
            "<a b='1' b='2'/>",
            "<a b=1/>",
            "<a b='1'c='2'/>",
            "<1a/>",
            "<a:b:c/>",
            "<a><!-- x -- y --></a>",
            "<a>]]></a>",
            "<a>\u{1}</a>",
            "<a/><?xml version='1.0'?>",
            "<![CDATA[x]]><a/>",
            "</a>",
            "<a xmlns:p=''/>",
            "<a><![CDATA[x</a>",
            "<a b='1",
            "<?pi",
        ];
        for s in bad {
            assert!(tokens(s).is_err(), "accepted: {s:?}");
        }
    }

    #[test]
    fn depth_and_size_limits() {
        let deep = "<a>".repeat(300) + &"</a>".repeat(300);
        assert!(tokens(&deep).is_err());
        let ok = "<a>".repeat(100) + &"</a>".repeat(100);
        assert!(tokens(&ok).is_ok());
        let lim = Limits { max_input: 10, ..Limits::default() };
        assert!(Tokenizer::with_limits("<a>0123456789</a>", lim).is_err());
        let lim = Limits { max_attributes: 2, ..Limits::default() };
        let mut t = Tokenizer::with_limits("<a x='1' y='2' z='3'/>", lim).unwrap();
        assert!(t.next_token().is_err());
        assert!(t.next_token().is_err(), "a failed tokenizer stays failed");
    }

    #[test]
    fn every_prefix_and_mutation_is_safe() {
        // No panics and no infinite loops on truncated or byte-mutated input.
        for i in 0..=DOC.len() {
            if let Some(p) = DOC.get(..i) {
                let _ = tokens(p);
            }
        }
        let bytes = DOC.as_bytes();
        for i in 0..bytes.len() {
            for b in [b'<', b'>', b'&', b'"', b'/', b'?', b'!', b'-', b';', 0] {
                let mut m = bytes.to_vec();
                m[i] = b;
                if let Ok(s) = std::str::from_utf8(&m) {
                    let _ = tokens(s);
                }
            }
        }
    }

    #[test]
    fn edits_keep_untouched_bytes() {
        let s = "<a x=\"1\">\n  <b/>\n  <c y='2'/>\n</a>";
        let toks = tokens(s).unwrap();
        let b = toks.iter().find(|t| t.name == "b").unwrap();
        let c = toks.iter().find(|t| t.name == "c").unwrap();
        let mut e = Edits::new();
        e.delete(b.span.clone());
        e.replace(c.attr("y").unwrap().value_span.clone(), escape_attr("<3>").into_owned());
        let out = e.apply(s).unwrap();
        assert_eq!(out, "<a x=\"1\">\n  \n  <c y='&lt;3&gt;'/>\n</a>");
        let mut bad = Edits::new();
        bad.delete(0..5);
        bad.delete(3..7);
        assert!(bad.apply(s).is_err());
        let x = as_utf8(b"\xEF\xBB\xBF<a/>").unwrap();
        let mut e = Edits::new();
        e.insert(2, " k='v'");
        assert_eq!(apply_edits(&x, e).unwrap(), b"\xEF\xBB\xBF<a k='v'/>".to_vec());
    }

    #[test]
    fn escaping_round_trips() {
        let v = "a&b<c>\"d'e\tf\ng";
        let doc = format!("<a v=\"{}\">{}</a>", escape_attr(v), escape_text(v));
        let toks = tokens(&doc).unwrap();
        assert_eq!(toks[0].attr("v").unwrap().value(), v);
        assert_eq!(toks[1].text(), v);
    }
}
