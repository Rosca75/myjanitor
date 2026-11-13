//! Tokenizer and direct-object parser (ISO 32000-1 §7.2, §7.3).
//!
//! Hostile-input rules: nesting is limited to [`MAX_DEPTH`], every index is
//! bounds-checked, nothing panics; malformed input is an `Error::Parse`.

use mj_core::{Error, Result};

use super::object::{Dict, ObjRef, Object};

/// Maximum nesting of arrays and dictionaries (the PDF implementation limit is 28).
pub const MAX_DEPTH: usize = 64;

/// True for PDF white-space characters.
pub fn is_ws(b: u8) -> bool {
    matches!(b, 0 | 9 | 10 | 12 | 13 | 32)
}

/// True for PDF delimiters.
pub fn is_delim(b: u8) -> bool {
    matches!(b, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
}

/// True for regular characters (neither white space nor delimiter).
pub fn is_regular(b: u8) -> bool {
    !is_ws(b) && !is_delim(b)
}

fn err(msg: impl Into<String>) -> Error {
    Error::parse("PDF", msg)
}

/// A cursor over PDF bytes.
pub struct Lexer<'a> {
    pub data: &'a [u8],
    pub pos: usize,
}

impl<'a> Lexer<'a> {
    /// Cursor at `pos`.
    pub fn new(data: &'a [u8], pos: usize) -> Self {
        Lexer { data, pos }
    }

    fn peek(&self) -> Option<u8> {
        self.data.get(self.pos).copied()
    }

    /// Skip white space and comments.
    pub fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if is_ws(b) {
                self.pos += 1;
            } else if b == b'%' {
                while let Some(c) = self.peek() {
                    if c == b'\r' || c == b'\n' {
                        break;
                    }
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    /// Read a run of regular characters (a keyword or number token).
    pub fn word(&mut self) -> &'a [u8] {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if !is_regular(b) {
                break;
            }
            self.pos += 1;
        }
        &self.data[start..self.pos]
    }

    /// Skip white space then expect the keyword `kw` (followed by a non-regular byte).
    pub fn expect_keyword(&mut self, kw: &[u8]) -> Result<()> {
        self.skip_ws();
        let save = self.pos;
        let w = self.word();
        if w == kw {
            Ok(())
        } else {
            self.pos = save;
            Err(err(format!("expected '{}' at offset {}", String::from_utf8_lossy(kw), save)))
        }
    }

    /// True (and consumed) when the next token is keyword `kw`.
    pub fn try_keyword(&mut self, kw: &[u8]) -> bool {
        let save = self.pos;
        self.skip_ws();
        if self.word() == kw {
            true
        } else {
            self.pos = save;
            false
        }
    }

    /// Parse an unsigned decimal integer token.
    pub fn uint(&mut self) -> Result<u64> {
        self.skip_ws();
        let start = self.pos;
        let w = self.word();
        parse_uint(w).ok_or_else(|| err(format!("expected an unsigned integer at offset {start}")))
    }

    /// Parse one direct object (references `n g R` included).
    pub fn object(&mut self) -> Result<Object> {
        self.object_at_depth(0)
    }

    fn object_at_depth(&mut self, depth: usize) -> Result<Object> {
        if depth > MAX_DEPTH {
            return Err(err("objects nested too deeply"));
        }
        self.skip_ws();
        let start = self.pos;
        let b = self.peek().ok_or_else(|| err("unexpected end of data"))?;
        match b {
            b'/' => {
                self.pos += 1;
                Ok(Object::Name(self.name_body()))
            }
            b'(' => {
                self.pos += 1;
                Ok(Object::Str(self.literal_string()?))
            }
            b'<' => {
                if self.data.get(self.pos + 1) == Some(&b'<') {
                    self.pos += 2;
                    Ok(Object::Dict(self.dict_body(depth)?))
                } else {
                    self.pos += 1;
                    Ok(Object::Str(self.hex_string()?))
                }
            }
            b'[' => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_ws();
                    match self.peek() {
                        None => return Err(err("unterminated array")),
                        Some(b']') => {
                            self.pos += 1;
                            return Ok(Object::Array(items));
                        }
                        Some(_) => items.push(self.object_at_depth(depth + 1)?),
                    }
                }
            }
            _ if is_regular(b) => {
                let w = self.word();
                match w {
                    b"true" => Ok(Object::Bool(true)),
                    b"false" => Ok(Object::Bool(false)),
                    b"null" => Ok(Object::Null),
                    _ => self.number_or_ref(w, start),
                }
            }
            _ => Err(err(format!("unexpected byte 0x{b:02x} at offset {start}"))),
        }
    }

    fn number_or_ref(&mut self, w: &[u8], start: usize) -> Result<Object> {
        if !is_number(w) {
            return Err(err(format!("unexpected token '{}' at offset {start}", String::from_utf8_lossy(w))));
        }
        let is_int = w.iter().all(|c| c.is_ascii_digit() || *c == b'+' || *c == b'-');
        if !is_int {
            return Ok(Object::Real(w.to_vec()));
        }
        let Some(value) = parse_int(w) else {
            // Integer too large for i64: keep it textually.
            return Ok(Object::Real(w.to_vec()));
        };
        // Look ahead for `gen R`.
        if w[0].is_ascii_digit() && value <= u32::MAX as i64 {
            let save = self.pos;
            self.skip_ws();
            let g = self.word();
            if let Some(gen) = parse_uint(g).filter(|g| *g <= u16::MAX as u64) {
                self.skip_ws();
                if self.peek() == Some(b'R') && self.data.get(self.pos + 1).is_none_or(|c| !is_regular(*c)) {
                    self.pos += 1;
                    return Ok(Object::Ref(ObjRef { num: value as u32, gen: gen as u16 }));
                }
            }
            self.pos = save;
        }
        Ok(Object::Int(value))
    }

    fn name_body(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        while let Some(b) = self.peek() {
            if !is_regular(b) {
                break;
            }
            if b == b'#' {
                let h = self.data.get(self.pos + 1).and_then(|c| hex_val(*c));
                let l = self.data.get(self.pos + 2).and_then(|c| hex_val(*c));
                if let (Some(h), Some(l)) = (h, l) {
                    out.push(h << 4 | l);
                    self.pos += 3;
                    continue;
                }
            }
            out.push(b);
            self.pos += 1;
        }
        out
    }

    fn literal_string(&mut self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        let mut nest = 1usize;
        loop {
            let b = self.peek().ok_or_else(|| err("unterminated string"))?;
            self.pos += 1;
            match b {
                b'(' => {
                    nest += 1;
                    out.push(b);
                }
                b')' => {
                    nest -= 1;
                    if nest == 0 {
                        return Ok(out);
                    }
                    out.push(b);
                }
                b'\r' => {
                    // An unescaped end of line is read as a single LF.
                    if self.peek() == Some(b'\n') {
                        self.pos += 1;
                    }
                    out.push(b'\n');
                }
                b'\\' => {
                    let c = self.peek().ok_or_else(|| err("unterminated string"))?;
                    self.pos += 1;
                    match c {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'(' | b')' | b'\\' => out.push(c),
                        b'\r' => {
                            if self.peek() == Some(b'\n') {
                                self.pos += 1;
                            }
                        }
                        b'\n' => {}
                        b'0'..=b'7' => {
                            let mut v = (c - b'0') as u32;
                            for _ in 0..2 {
                                match self.peek() {
                                    Some(d @ b'0'..=b'7') => {
                                        v = v * 8 + (d - b'0') as u32;
                                        self.pos += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push((v & 0xff) as u8);
                        }
                        other => out.push(other),
                    }
                }
                _ => out.push(b),
            }
        }
    }

    fn hex_string(&mut self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        let mut hi: Option<u8> = None;
        loop {
            let b = self.peek().ok_or_else(|| err("unterminated hex string"))?;
            self.pos += 1;
            if b == b'>' {
                if let Some(h) = hi {
                    out.push(h << 4);
                }
                return Ok(out);
            }
            if is_ws(b) {
                continue;
            }
            let v = hex_val(b).ok_or_else(|| err("invalid character in hex string"))?;
            match hi.take() {
                None => hi = Some(v),
                Some(h) => out.push(h << 4 | v),
            }
        }
    }

    fn dict_body(&mut self, depth: usize) -> Result<Dict> {
        let mut d = Dict::new();
        loop {
            self.skip_ws();
            match self.peek() {
                None => return Err(err("unterminated dictionary")),
                Some(b'>') => {
                    if self.data.get(self.pos + 1) == Some(&b'>') {
                        self.pos += 2;
                        return Ok(d);
                    }
                    return Err(err("unexpected '>' in dictionary"));
                }
                Some(b'/') => {
                    self.pos += 1;
                    let key = self.name_body();
                    let value = self.object_at_depth(depth + 1)?;
                    d.0.push((key, value));
                }
                Some(_) => return Err(err(format!("dictionary key expected at offset {}", self.pos))),
            }
        }
    }
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Unsigned decimal without sign.
pub fn parse_uint(w: &[u8]) -> Option<u64> {
    if w.is_empty() || w.len() > 19 || !w.iter().all(u8::is_ascii_digit) {
        return None;
    }
    w.iter().try_fold(0u64, |acc, d| acc.checked_mul(10)?.checked_add((d - b'0') as u64))
}

fn parse_int(w: &[u8]) -> Option<i64> {
    let (neg, digits) = match w.first() {
        Some(b'-') => (true, &w[1..]),
        Some(b'+') => (false, &w[1..]),
        _ => (false, w),
    };
    let v = parse_uint(digits)?;
    let v = i64::try_from(v).ok()?;
    Some(if neg { -v } else { v })
}

/// `[+-]? digits* (. digits*)?` with at least one digit.
fn is_number(w: &[u8]) -> bool {
    let body = match w.first() {
        Some(b'+') | Some(b'-') => &w[1..],
        _ => w,
    };
    let mut digits = 0;
    let mut dots = 0;
    for &c in body {
        match c {
            b'0'..=b'9' => digits += 1,
            b'.' => dots += 1,
            _ => return false,
        }
    }
    digits > 0 && dots <= 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &[u8]) -> Object {
        Lexer::new(s, 0).object().unwrap()
    }

    #[test]
    fn scalars() {
        assert_eq!(parse(b"  42 "), Object::Int(42));
        assert_eq!(parse(b"-17"), Object::Int(-17));
        assert_eq!(parse(b"+.5"), Object::Real(b"+.5".to_vec()));
        assert_eq!(parse(b"4."), Object::Real(b"4.".to_vec()));
        assert_eq!(parse(b"99999999999999999999"), Object::Real(b"99999999999999999999".to_vec()));
        assert_eq!(parse(b"true"), Object::Bool(true));
        assert_eq!(parse(b"null"), Object::Null);
        assert_eq!(parse(b"12 0 R"), Object::Ref(ObjRef { num: 12, gen: 0 }));
        assert_eq!(parse(b"%c\n 12 3 R"), Object::Ref(ObjRef { num: 12, gen: 3 }));
    }

    #[test]
    fn ints_not_refs() {
        assert_eq!(parse(b"[1 2 3]"), Object::Array(vec![Object::Int(1), Object::Int(2), Object::Int(3)]));
        // `R` must stand alone: `RG` is an (invalid) keyword, not a reference.
        assert!(Lexer::new(b"[1 2 RG]", 0).object().is_err());
    }

    #[test]
    fn names_and_strings() {
        assert_eq!(parse(b"/A#20B#2"), Object::Name(b"A B#2".to_vec()));
        assert_eq!(parse(b"/"), Object::Name(Vec::new()));
        assert_eq!(parse(b"(a(b)c\\)\\n\\101\\7x\\\r\ny\r\nz)"), Object::Str(b"a(b)c)\nA\x07xy\nz".to_vec()));
        assert_eq!(parse(b"<48 65 6c6C 6>"), Object::Str(b"Hell\x60".to_vec()));
    }

    #[test]
    fn dicts() {
        let o = parse(b"<</Type/Page/Kids[1 0 R 2 0 R]/N -3.25/S<</X(y)>>>>");
        let d = o.as_dict().unwrap();
        assert!(d.get(b"Type").unwrap().is_name(b"Page"));
        assert_eq!(
            d.get(b"Kids"),
            Some(&Object::Array(vec![Object::Ref(ObjRef::new(1)), Object::Ref(ObjRef::new(2))]))
        );
        assert_eq!(d.get(b"N"), Some(&Object::Real(b"-3.25".to_vec())));
    }

    #[test]
    fn malformed_is_an_error_not_a_panic() {
        for bad in [&b"<<"[..], b"[1 2", b"(abc", b"<4G>", b"<</A>>", b">>", b"--5", b"1.2.3", b"", b"<</A 1 /B"] {
            assert!(Lexer::new(bad, 0).object().is_err(), "{:?}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn depth_is_limited() {
        let mut s = Vec::new();
        s.extend(std::iter::repeat_n(b'[', 10_000));
        assert!(Lexer::new(&s, 0).object().is_err());
        let mut ok = Vec::new();
        ok.extend(std::iter::repeat_n(b'[', 20));
        ok.extend(std::iter::repeat_n(b']', 20));
        assert!(Lexer::new(&ok, 0).object().is_ok());
    }
}
