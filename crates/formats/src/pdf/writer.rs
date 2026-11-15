//! Serialising objects and writing a clean single-revision file, either with
//! a classic cross-reference table or with object streams plus a
//! cross-reference stream (PDF 1.5).

use std::collections::BTreeMap;

use super::filter;
use super::lexer::{is_delim, is_regular, is_ws};
use super::object::{Dict, Object, Stream};

/// Objects per object stream (as qpdf).
pub const OBJECTS_PER_STREAM: usize = 100;

/// Appends tokens, inserting a space only where two regular characters would touch.
pub struct TokenWriter<'a> {
    pub out: &'a mut Vec<u8>,
}

impl TokenWriter<'_> {
    fn raw(&mut self, bytes: &[u8]) {
        if let (Some(&last), Some(&first)) = (self.out.last(), bytes.first()) {
            if is_regular(last) && is_regular(first) {
                self.out.push(b' ');
            }
        }
        self.out.extend_from_slice(bytes);
    }

    /// Write one direct object. Streams are written by the file writer, never here.
    pub fn object(&mut self, o: &Object) {
        match o {
            Object::Null => self.raw(b"null"),
            Object::Bool(true) => self.raw(b"true"),
            Object::Bool(false) => self.raw(b"false"),
            Object::Int(i) => self.raw(i.to_string().as_bytes()),
            Object::Real(t) => self.raw(t),
            Object::Name(n) => self.name(n),
            Object::Str(s) => self.string(s),
            Object::Array(a) => {
                self.raw(b"[");
                for v in a {
                    self.object(v);
                }
                self.raw(b"]");
            }
            Object::Dict(d) => self.dict(d),
            Object::Ref(r) => self.raw(format!("{} {} R", r.num, r.gen).as_bytes()),
            // Unreachable for well-formed callers; written as its dictionary so nothing is lost silently.
            Object::Stream(s) => self.dict(&s.dict),
        }
    }

    /// Write a dictionary.
    pub fn dict(&mut self, d: &Dict) {
        self.raw(b"<<");
        for (k, v) in &d.0 {
            self.name(k);
            self.object(v);
        }
        self.raw(b">>");
    }

    fn name(&mut self, n: &[u8]) {
        let mut t = Vec::with_capacity(n.len() + 1);
        t.push(b'/');
        for &b in n {
            if b == b'#' || is_ws(b) || is_delim(b) || !(0x21..=0x7e).contains(&b) {
                t.extend_from_slice(format!("#{b:02X}").as_bytes());
            } else {
                t.push(b);
            }
        }
        self.raw(&t);
    }

    fn string(&mut self, s: &[u8]) {
        let mut lit = Vec::with_capacity(s.len() + 2);
        lit.push(b'(');
        for &b in s {
            match b {
                b'(' | b')' | b'\\' => {
                    lit.push(b'\\');
                    lit.push(b);
                }
                // A raw CR would be read back as LF.
                b'\r' => lit.extend_from_slice(b"\\r"),
                _ => lit.push(b),
            }
        }
        lit.push(b')');
        if lit.len() <= 2 * s.len() + 2 {
            self.raw(&lit);
        } else {
            let mut hex = Vec::with_capacity(2 * s.len() + 2);
            hex.push(b'<');
            for b in s {
                hex.extend_from_slice(format!("{b:02x}").as_bytes());
            }
            hex.push(b'>');
            self.raw(&hex);
        }
    }
}

/// Serialise a direct object on its own.
pub fn to_bytes(o: &Object) -> Vec<u8> {
    let mut out = Vec::new();
    TokenWriter { out: &mut out }.object(o);
    out
}

/// How to lay out the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// Classic `xref` table, every object at top level.
    Classic,
    /// Non-stream objects packed in object streams, cross-reference stream.
    ObjectStreams,
}

/// Write a complete file. Objects are numbered `1..=n` (generation 0) and must
/// only reference each other; `trailer` holds `/Root`, `/Info`, `/ID` and any
/// other kept trailer key (never `/Size`, `/Prev`).
pub fn write_file(version: (u8, u8), objects: &BTreeMap<u32, Object>, trailer: &Dict, layout: Layout) -> Vec<u8> {
    let mut out = Vec::new();
    // Header: `%PDF-x.y\n` followed by a binary comment (4 bytes >= 128).
    out.extend_from_slice(format!("%PDF-{}.{}\n", version.0, version.1).as_bytes());
    out.extend_from_slice(&[b'%', 0xE2, 0xE3, 0xCF, 0xD3, b'\n']);
    let max = objects.keys().next_back().copied().unwrap_or(0);
    match layout {
        Layout::Classic => {
            let mut offsets = vec![0u64; max as usize + 1];
            for (num, o) in objects {
                offsets[*num as usize] = out.len() as u64;
                write_indirect(&mut out, *num, o);
            }
            let xref_at = out.len();
            out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", max + 1).as_bytes());
            for num in 1..=max {
                if objects.contains_key(&num) {
                    out.extend_from_slice(format!("{:010} 00000 n \n", offsets[num as usize]).as_bytes());
                } else {
                    out.extend_from_slice(b"0000000000 00000 f \n");
                }
            }
            let mut t = trailer.clone();
            t.set(b"Size", Object::Int(max as i64 + 1));
            out.extend_from_slice(b"trailer\n");
            TokenWriter { out: &mut out }.dict(&t);
            out.extend_from_slice(format!("\nstartxref\n{xref_at}\n%%EOF\n").as_bytes());
        }
        Layout::ObjectStreams => {
            // entries[num] = (type, field2, field3)
            let packed: Vec<u32> =
                objects.iter().filter(|(_, o)| !matches!(o, Object::Stream(_))).map(|(n, _)| *n).collect();
            let n_streams = packed.len().div_ceil(OBJECTS_PER_STREAM) as u32;
            let total = max + n_streams + 1; // + object streams + xref stream
            let mut entries: Vec<(u8, u64, u64)> = vec![(0, 0, 0); total as usize + 1];
            entries[0] = (0, 0, 0xFFFF);
            for (num, o) in objects {
                if let Object::Stream(_) = o {
                    entries[*num as usize] = (1, out.len() as u64, 0);
                    write_indirect(&mut out, *num, o);
                }
            }
            for (k, chunk) in packed.chunks(OBJECTS_PER_STREAM).enumerate() {
                let stm_num = max + 1 + k as u32;
                let mut header = Vec::new();
                let mut body = Vec::new();
                for (idx, num) in chunk.iter().enumerate() {
                    if !header.is_empty() {
                        header.push(b' ');
                    }
                    header.extend_from_slice(format!("{} {}", num, body.len()).as_bytes());
                    TokenWriter { out: &mut body }.object(&objects[num]);
                    body.push(b'\n');
                    entries[*num as usize] = (2, stm_num as u64, idx as u64);
                }
                header.push(b'\n');
                let first = header.len();
                header.extend_from_slice(&body);
                let data = filter::deflate(&header, 10);
                let mut d = Dict::new();
                d.set(b"Type", Object::Name(b"ObjStm".to_vec()));
                d.set(b"N", Object::Int(chunk.len() as i64));
                d.set(b"First", Object::Int(first as i64));
                d.set(b"Filter", Object::Name(b"FlateDecode".to_vec()));
                entries[stm_num as usize] = (1, out.len() as u64, 0);
                write_indirect(&mut out, stm_num, &Object::Stream(Stream { dict: d, data }));
            }
            let xref_num = total;
            let xref_at = out.len() as u64;
            entries[xref_num as usize] = (1, xref_at, 0);
            let w2 = bytes_needed(xref_at.max(total as u64));
            let mut body = Vec::with_capacity(entries.len() * (3 + w2));
            for (t, f2, f3) in &entries {
                body.push(*t);
                body.extend_from_slice(&f2.to_be_bytes()[8 - w2..]);
                body.extend_from_slice(&(*f3 as u16).to_be_bytes());
            }
            let mut d = trailer.clone();
            d.set(b"Type", Object::Name(b"XRef".to_vec()));
            d.set(b"Size", Object::Int(total as i64 + 1));
            d.set(b"W", Object::Array(vec![Object::Int(1), Object::Int(w2 as i64), Object::Int(2)]));
            d.set(b"Filter", Object::Name(b"FlateDecode".to_vec()));
            let data = filter::deflate(&body, 10);
            write_indirect(&mut out, xref_num, &Object::Stream(Stream { dict: d, data }));
            out.extend_from_slice(format!("startxref\n{xref_at}\n%%EOF\n").as_bytes());
        }
    }
    out
}

fn bytes_needed(v: u64) -> usize {
    let mut n = 1;
    while n < 8 && v >> (8 * n) != 0 {
        n += 1;
    }
    n
}

/// `num 0 obj ... endobj`; a stream gets a direct `/Length`.
fn write_indirect(out: &mut Vec<u8>, num: u32, o: &Object) {
    out.extend_from_slice(format!("{num} 0 obj\n").as_bytes());
    match o {
        Object::Stream(s) => {
            let mut d = s.dict.clone();
            d.set(b"Length", Object::Int(s.data.len() as i64));
            TokenWriter { out: &mut *out }.dict(&d);
            out.extend_from_slice(b"stream\n");
            out.extend_from_slice(&s.data);
            out.extend_from_slice(b"\nendstream\nendobj\n");
        }
        other => {
            TokenWriter { out: &mut *out }.object(other);
            out.extend_from_slice(b"\nendobj\n");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::lexer::Lexer;
    use super::*;

    fn round_trip(o: &Object) {
        let b = to_bytes(o);
        let back = Lexer::new(&b, 0).object().unwrap();
        assert_eq!(&back, o, "{}", String::from_utf8_lossy(&b));
    }

    #[test]
    fn objects_round_trip() {
        let mut d = Dict::new();
        d.set(b"Type", Object::Name(b"Page".to_vec()));
        d.set(b"A B#()", Object::Name(b"x/y\x00\xff".to_vec()));
        d.set(b"N", Object::Array(vec![Object::Int(1), Object::Int(-2), Object::Real(b".5".to_vec()), Object::Null]));
        d.set(b"S", Object::Str(b"a(b)\\c\r\n\x00".to_vec()));
        d.set(b"H", Object::Str(vec![0x80, 0x81, 0x28]));
        d.set(b"R", Object::Ref(super::super::object::ObjRef::new(7)));
        d.set(b"T", Object::Bool(true));
        d.set(b"E", Object::Str(Vec::new()));
        round_trip(&Object::Dict(d));
        assert_eq!(to_bytes(&Object::Array(vec![Object::Int(1), Object::Int(2)])), b"[1 2]");
        assert_eq!(to_bytes(&Object::Array(vec![Object::Name(b"A".to_vec()), Object::Name(b"B".to_vec())])), b"[/A/B]");
    }

    #[test]
    fn bytes_needed_widths() {
        assert_eq!(bytes_needed(0), 1);
        assert_eq!(bytes_needed(255), 1);
        assert_eq!(bytes_needed(256), 2);
        assert_eq!(bytes_needed(1 << 40), 6);
    }
}
