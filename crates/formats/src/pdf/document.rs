//! Reading a PDF file: header, cross-reference tables and streams, incremental
//! updates (`/Prev`), hybrid files (`/XRefStm`), object streams (`/ObjStm`),
//! indirect objects and streams with direct or indirect `/Length`.
//!
//! Only objects reachable from the trailer are loaded ([`Document::load`]).
//! Anything inconsistent (an offset that does not point at the object it
//! claims, a `/Length` that does not end at `endstream`, a loop in the
//! `/Prev` chain) is an error: the caller refuses the file rather than guess.

use std::collections::{BTreeMap, HashMap, HashSet};

use mj_core::{Error, Result};

use super::filter;
use super::lexer::{is_regular, Lexer};
use super::object::{for_each_ref, Dict, ObjRef, Object, Stream};

/// Largest file accepted (the whole file is held in memory).
pub const MAX_FILE: usize = 512 * 1024 * 1024;
/// Largest object number accepted (PDF implementation limit is 8,388,607).
pub const MAX_OBJECTS: usize = 8_388_607;
/// Most reachable objects loaded.
pub const MAX_LOADED: usize = 2_000_000;
/// Most cross-reference sections followed through `/Prev`.
pub const MAX_SECTIONS: usize = 4096;

fn err(msg: impl Into<String>) -> Error {
    Error::parse("PDF", msg)
}

/// One cross-reference entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XrefEntry {
    Free,
    /// Byte offset and generation.
    InFile(u64, u16),
    /// Object stream number and index inside it.
    InStream(u32, u32),
}

/// The merged cross-reference information of a file.
#[derive(Debug, Clone, Default)]
pub struct Xref {
    pub entries: HashMap<u32, XrefEntry>,
    /// The newest trailer, with `/Root`, `/Info`, `/ID`, `/Encrypt` filled from older ones when absent.
    pub trailer: Dict,
    /// Number of sections read (1 = no incremental update).
    pub sections: usize,
    /// Any section was a cross-reference stream.
    pub has_xref_stream: bool,
    /// Any trailer of the chain carries `/Encrypt`.
    pub encrypted: bool,
}

/// A loaded document: the trailer and every object reachable from it.
#[derive(Debug, Clone)]
pub struct Document {
    /// Header version, e.g. `(1, 7)`.
    pub version: (u8, u8),
    pub trailer: Dict,
    /// Reachable objects by number (generation dropped: numbers are unique once loaded).
    pub objects: BTreeMap<u32, Object>,
    pub xref: Xref,
}

impl Document {
    /// Parse `data` and load every object reachable from the trailer.
    pub fn load(data: &[u8]) -> Result<Document> {
        if data.len() > MAX_FILE {
            return Err(err("file too large"));
        }
        let version = header_version(data)?;
        let xref = read_xref(data)?;
        let trailer = xref.trailer.clone();
        if xref.encrypted {
            // Objects are encrypted: do not even try to parse their strings.
            return Ok(Document { version, trailer, objects: BTreeMap::new(), xref });
        }
        let mut loader = Loader { data, xref: &xref, objstm: HashMap::new() };
        let mut objects = BTreeMap::new();
        let mut queue: Vec<ObjRef> = Vec::new();
        for (_, v) in &trailer.0 {
            for_each_ref(v, |r| queue.push(r));
        }
        let mut seen: HashSet<u32> = HashSet::new();
        while let Some(r) = queue.pop() {
            if !seen.insert(r.num) {
                continue;
            }
            if seen.len() > MAX_LOADED {
                return Err(err("too many objects"));
            }
            if let Some(obj) = loader.load(r.num, 0)? {
                for_each_ref(&obj, |c| {
                    if !seen.contains(&c.num) {
                        queue.push(c)
                    }
                });
                objects.insert(r.num, obj);
            }
        }
        Ok(Document { version, trailer, objects, xref })
    }

    /// Resolve a value: follows one level of reference; dangling references are `null`.
    pub fn resolve<'a>(&'a self, o: &'a Object) -> &'a Object {
        match o {
            Object::Ref(r) => self.objects.get(&r.num).unwrap_or(&Object::Null),
            other => other,
        }
    }

    /// Resolved dictionary entry.
    pub fn get<'a>(&'a self, d: &'a Dict, key: &[u8]) -> Option<&'a Object> {
        d.get(key).map(|o| self.resolve(o)).filter(|o| **o != Object::Null)
    }

    /// The document catalog.
    pub fn catalog(&self) -> Option<&Dict> {
        self.trailer.get(b"Root").map(|o| self.resolve(o)).and_then(|o| match o {
            Object::Dict(d) => Some(d),
            _ => None,
        })
    }

    /// Object numbers of the page leaves, in document order.
    /// Iterative; cycles and depth beyond 256 levels are errors.
    pub fn pages(&self) -> Result<Vec<u32>> {
        let cat = self.catalog().ok_or_else(|| err("no document catalog"))?;
        let root = cat.get(b"Pages").and_then(Object::as_ref).ok_or_else(|| err("no page tree"))?;
        let mut out = Vec::new();
        let mut visited = HashSet::new();
        // Stack of (node, depth) processed in reverse so order is preserved.
        let mut stack = vec![(root.num, 0usize)];
        while let Some((num, depth)) = stack.pop() {
            if depth > 256 {
                return Err(err("page tree too deep"));
            }
            if !visited.insert(num) {
                return Err(err("cycle in the page tree"));
            }
            let node = self.objects.get(&num).and_then(Object::as_dict).ok_or_else(|| err("missing page tree node"))?;
            let is_pages = node.type_name() == Some(b"Pages") || (node.type_name().is_none() && node.contains(b"Kids"));
            if is_pages {
                let kids = match node.get(b"Kids").map(|k| self.resolve(k)) {
                    Some(Object::Array(a)) => a,
                    _ => return Err(err("page tree node without /Kids")),
                };
                for k in kids.iter().rev() {
                    let r = k.as_ref().ok_or_else(|| err("page tree kid is not a reference"))?;
                    stack.push((r.num, depth + 1));
                }
            } else {
                out.push(num);
            }
        }
        Ok(out)
    }
}

/// `%PDF-x.y` within the first 1024 bytes; garbage before it is refused
/// (offsets would be ambiguous).
pub fn header_version(data: &[u8]) -> Result<(u8, u8)> {
    if !data.starts_with(b"%PDF-") {
        return Err(err("not a PDF file, or data before the %PDF header"));
    }
    let maj = data.get(5).copied().filter(u8::is_ascii_digit).ok_or_else(|| err("bad header version"))?;
    let min = data.get(7).copied().filter(u8::is_ascii_digit).ok_or_else(|| err("bad header version"))?;
    if data.get(6) != Some(&b'.') {
        return Err(err("bad header version"));
    }
    Ok((maj - b'0', min - b'0'))
}

fn rfind(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > hay.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).rev().find(|&i| &hay[i..i + needle.len()] == needle)
}

/// Offset of the last cross-reference section (`startxref` near the end).
fn startxref(data: &[u8]) -> Result<u64> {
    let tail_start = data.len().saturating_sub(4096);
    let pos =
        rfind(&data[tail_start..], b"startxref").ok_or_else(|| err("no startxref (truncated file?)"))? + tail_start;
    let mut lx = Lexer::new(data, pos + 9);
    let off = lx.uint()?;
    // Not `skip_ws`: `%%EOF` is itself a comment.
    let mut p = lx.pos;
    while data.get(p).is_some_and(|b| super::lexer::is_ws(*b)) {
        p += 1;
    }
    if !data[p..].starts_with(b"%%EOF") {
        return Err(err("no %%EOF after startxref"));
    }
    Ok(off)
}

/// Read and merge every cross-reference section of the file.
pub fn read_xref(data: &[u8]) -> Result<Xref> {
    let mut x = Xref::default();
    let mut visited: HashSet<u64> = HashSet::new();
    let mut next = Some(startxref(data)?);
    let mut first = true;
    while let Some(off) = next.take() {
        if !visited.insert(off) {
            return Err(err("loop in the cross-reference chain"));
        }
        if visited.len() > MAX_SECTIONS {
            return Err(err("too many cross-reference sections"));
        }
        let o = usize::try_from(off).ok().filter(|o| *o < data.len()).ok_or_else(|| err("xref offset out of range"))?;
        let trailer = if data[o..].starts_with(b"xref") {
            let (entries, trailer) = classic_section(data, o)?;
            // Hybrid file: the hidden xref stream overrides this table.
            if let Some(stm) = trailer.get(b"XRefStm").and_then(Object::as_int) {
                let so = usize::try_from(stm).ok().filter(|s| *s < data.len()).ok_or_else(|| err("bad /XRefStm"))?;
                let (sentries, sdict) = stream_section(data, so)?;
                if sdict.contains(b"Encrypt") {
                    x.encrypted = true;
                }
                x.has_xref_stream = true;
                merge(&mut x.entries, sentries);
            }
            merge(&mut x.entries, entries);
            trailer
        } else {
            let (entries, dict) = stream_section(data, o)?;
            x.has_xref_stream = true;
            merge(&mut x.entries, entries);
            dict
        };
        x.sections += 1;
        if trailer.contains(b"Encrypt") {
            x.encrypted = true;
        }
        if first {
            x.trailer = trailer.clone();
            first = false;
        } else {
            for key in [&b"Root"[..], b"Info", b"ID", b"Encrypt"] {
                if !x.trailer.contains(key) {
                    if let Some(v) = trailer.get(key) {
                        x.trailer.set(key, v.clone());
                    }
                }
            }
        }
        next = match trailer.get(b"Prev") {
            None => None,
            Some(Object::Int(p)) if *p >= 0 => Some(*p as u64),
            Some(_) => return Err(err("bad /Prev")),
        };
    }
    if x.entries.len() > MAX_OBJECTS {
        return Err(err("too many objects"));
    }
    Ok(x)
}

/// Older sections never override newer ones.
fn merge(into: &mut HashMap<u32, XrefEntry>, from: Vec<(u32, XrefEntry)>) {
    for (n, e) in from {
        into.entry(n).or_insert(e);
    }
}

fn classic_section(data: &[u8], off: usize) -> Result<(Vec<(u32, XrefEntry)>, Dict)> {
    let mut lx = Lexer::new(data, off);
    lx.expect_keyword(b"xref")?;
    let mut out = Vec::new();
    loop {
        lx.skip_ws();
        if lx.try_keyword(b"trailer") {
            break;
        }
        let start = lx.uint()?;
        let count = lx.uint()?;
        if start.saturating_add(count) > MAX_OBJECTS as u64 {
            return Err(err("xref subsection out of range"));
        }
        for i in 0..count {
            let offset = lx.uint()?;
            let gen = lx.uint()?;
            lx.skip_ws();
            let kind = lx.word();
            let num = (start + i) as u32;
            let gen = u16::try_from(gen).map_err(|_| err("bad generation"))?;
            match kind {
                b"n" => out.push((num, XrefEntry::InFile(offset, gen))),
                b"f" => out.push((num, XrefEntry::Free)),
                _ => return Err(err("bad xref entry")),
            }
        }
    }
    match lx.object()? {
        Object::Dict(d) => Ok((out, d)),
        _ => Err(err("trailer is not a dictionary")),
    }
}

fn be(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0u64, |acc, b| (acc << 8) | *b as u64)
}

fn stream_section(data: &[u8], off: usize) -> Result<(Vec<(u32, XrefEntry)>, Dict)> {
    // A cross-reference stream's /Length is required to be direct.
    let (_, obj) = parse_indirect(data, off, &|_| Err(err("indirect /Length in an xref stream")))?;
    let Object::Stream(s) = obj else {
        return Err(err("xref offset does not point at an xref table or stream"));
    };
    if s.dict.type_name() != Some(b"XRef") {
        return Err(err("xref stream without /Type /XRef"));
    }
    let size = s.dict.get(b"Size").and_then(Object::as_int).ok_or_else(|| err("xref stream without /Size"))?;
    if !(0..=MAX_OBJECTS as i64).contains(&size) {
        return Err(err("bad xref /Size"));
    }
    let w: Vec<usize> = match s.dict.get(b"W") {
        Some(Object::Array(a)) if a.len() == 3 => a
            .iter()
            .map(|v| v.as_int().filter(|n| (0..=8).contains(n)).map(|n| n as usize))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| err("bad /W"))?,
        _ => return Err(err("bad /W")),
    };
    let index: Vec<(u64, u64)> = match s.dict.get(b"Index") {
        None => vec![(0, size as u64)],
        Some(Object::Array(a)) if a.len() % 2 == 0 => a
            .chunks(2)
            .map(|p| Some((u64::try_from(p[0].as_int()?).ok()?, u64::try_from(p[1].as_int()?).ok()?)))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| err("bad /Index"))?,
        _ => return Err(err("bad /Index")),
    };
    let body = filter::decode_full(&s.dict, &s.data)?;
    let entry_len: usize = w.iter().sum();
    if entry_len == 0 {
        return Err(err("bad /W"));
    }
    let mut out = Vec::new();
    let mut pos = 0usize;
    for (start, count) in index {
        if start.saturating_add(count) > MAX_OBJECTS as u64 {
            return Err(err("xref stream /Index out of range"));
        }
        for i in 0..count {
            let rec = body.get(pos..pos + entry_len).ok_or_else(|| err("xref stream too short"))?;
            pos += entry_len;
            let f1 = if w[0] == 0 { 1 } else { be(&rec[..w[0]]) };
            let f2 = be(&rec[w[0]..w[0] + w[1]]);
            let f3 = be(&rec[w[0] + w[1]..]);
            let num = (start + i) as u32;
            let e = match f1 {
                0 => XrefEntry::Free,
                1 => XrefEntry::InFile(f2, u16::try_from(f3).map_err(|_| err("bad generation"))?),
                2 => XrefEntry::InStream(u32::try_from(f2).map_err(|_| err("bad object stream number"))?, f3 as u32),
                // Unknown types are to be treated as null references (spec): free.
                _ => XrefEntry::Free,
            };
            out.push((num, e));
        }
    }
    Ok((out, s.dict))
}

/// Parse `num gen obj ... endobj` at `off`. `length` resolves an indirect `/Length`.
pub fn parse_indirect(data: &[u8], off: usize, length: &dyn Fn(ObjRef) -> Result<i64>) -> Result<(ObjRef, Object)> {
    let mut lx = Lexer::new(data, off);
    let num = lx.uint()?;
    let gen = lx.uint()?;
    lx.expect_keyword(b"obj")?;
    let r = ObjRef {
        num: u32::try_from(num).map_err(|_| err("bad object number"))?,
        gen: u16::try_from(gen).map_err(|_| err("bad generation"))?,
    };
    let obj = lx.object()?;
    lx.skip_ws();
    let is_stream = data[lx.pos..].starts_with(b"stream") && data.get(lx.pos + 6).is_none_or(|c| !is_regular(*c));
    if !is_stream {
        // `endobj` is not required: some writers omit it, and the object is complete.
        return Ok((r, obj));
    }
    let Object::Dict(dict) = obj else {
        return Err(err("stream without a dictionary"));
    };
    let mut p = lx.pos + 6;
    // The keyword is followed by CRLF or LF (a lone CR is tolerated).
    if data.get(p) == Some(&b'\r') {
        p += 1;
    }
    if data.get(p) == Some(&b'\n') {
        p += 1;
    }
    let len = match dict.get(b"Length") {
        Some(Object::Int(n)) => *n,
        Some(Object::Ref(lr)) => length(*lr)?,
        _ => return Err(err(format!("stream {} without a usable /Length", r.num))),
    };
    let len = usize::try_from(len).map_err(|_| err("negative /Length"))?;
    let end = p.checked_add(len).filter(|e| *e <= data.len()).ok_or_else(|| err("stream runs past the end of file"))?;
    let mut lx2 = Lexer::new(data, end);
    lx2.skip_ws();
    if !data[lx2.pos..].starts_with(b"endstream") {
        return Err(err(format!("stream {} /Length does not end at endstream", r.num)));
    }
    Ok((r, Object::Stream(Stream { dict, data: data[p..end].to_vec() })))
}

/// A decoded object stream: `(object number, offset)` per slot, and the decoded bytes.
type DecodedObjStm = (Vec<(u32, usize)>, Vec<u8>);

/// Lazily resolves objects through the cross-reference table.
struct Loader<'a> {
    data: &'a [u8],
    xref: &'a Xref,
    /// Decoded object streams: `(object table, decoded bytes, offset of first object)`.
    objstm: HashMap<u32, DecodedObjStm>,
}

impl Loader<'_> {
    /// Load object `num`. `None` = free or absent (a dangling reference reads as `null`).
    fn load(&mut self, num: u32, depth: usize) -> Result<Option<Object>> {
        if depth > 2 {
            return Err(err("indirect /Length chain"));
        }
        match self.xref.entries.get(&num) {
            None | Some(XrefEntry::Free) => Ok(None),
            Some(XrefEntry::InFile(off, gen)) => {
                let off = usize::try_from(*off)
                    .ok()
                    .filter(|o| *o < self.data.len())
                    .ok_or_else(|| err("object offset out of range"))?;
                let data = self.data;
                let xref = self.xref;
                let length = |lr: ObjRef| -> Result<i64> {
                    // The /Length object is a plain integer stored in the file or an object stream.
                    let mut sub = Loader { data, xref, objstm: HashMap::new() };
                    match sub.load(lr.num, depth + 1)? {
                        Some(Object::Int(n)) => Ok(n),
                        _ => Err(err("indirect /Length is not an integer")),
                    }
                };
                let (r, obj) = parse_indirect(self.data, off, &length)?;
                if r.num != num || r.gen != *gen {
                    return Err(err(format!("xref entry for object {num} points at object {}", r.num)));
                }
                Ok(Some(obj))
            }
            Some(XrefEntry::InStream(stm, idx)) => {
                let (stm, idx) = (*stm, *idx as usize);
                if !self.objstm.contains_key(&stm) {
                    let decoded = self.decode_objstm(stm, depth)?;
                    self.objstm.insert(stm, decoded);
                }
                let (table, body) = &self.objstm[&stm];
                let (n, off) = *table.get(idx).ok_or_else(|| err("object stream index out of range"))?;
                if n != num {
                    return Err(err(format!("object stream {stm} slot {idx} holds object {n}, not {num}")));
                }
                let mut lx = Lexer::new(body, off);
                let obj = lx.object()?;
                if matches!(obj, Object::Stream(_)) {
                    return Err(err("stream inside an object stream"));
                }
                Ok(Some(obj))
            }
        }
    }

    fn decode_objstm(&mut self, stm: u32, depth: usize) -> Result<DecodedObjStm> {
        // The object stream itself must be stored directly in the file.
        if !matches!(self.xref.entries.get(&stm), Some(XrefEntry::InFile(..))) {
            return Err(err("object stream is not stored in the file"));
        }
        let Some(Object::Stream(s)) = self.load(stm, depth)? else {
            return Err(err("object stream reference is not a stream"));
        };
        if s.dict.type_name() != Some(b"ObjStm") {
            return Err(err("object stream without /Type /ObjStm"));
        }
        let n = s
            .dict
            .get(b"N")
            .and_then(Object::as_int)
            .filter(|n| (0..=1_000_000).contains(n))
            .ok_or_else(|| err("bad /N"))?;
        let first =
            s.dict.get(b"First").and_then(Object::as_int).filter(|f| *f >= 0).ok_or_else(|| err("bad /First"))?;
        let body = filter::decode_full(&s.dict, &s.data)?;
        let first = first as usize;
        if first > body.len() {
            return Err(err("/First past the end of the object stream"));
        }
        let mut lx = Lexer::new(&body[..first], 0);
        let mut table = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let num = lx.uint()?;
            let off = lx.uint()?;
            let num = u32::try_from(num).map_err(|_| err("bad object number in object stream"))?;
            let off = first
                .checked_add(off as usize)
                .filter(|o| *o <= body.len())
                .ok_or_else(|| err("bad offset in object stream"))?;
            table.push((num, off));
        }
        Ok((table, body))
    }
}

/// Quick checks a caller can make without loading the document.
pub fn contains(hay: &[u8], needle: &[u8]) -> bool {
    let Some((&first, rest)) = needle.split_first() else {
        return false;
    };
    let mut i = 0;
    while let Some(p) = hay[i..].iter().position(|b| *b == first) {
        let at = i + p;
        if hay.len() - at > rest.len() && &hay[at + 1..at + 1 + rest.len()] == rest {
            return true;
        }
        i = at + 1;
    }
    false
}
