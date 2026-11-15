//! CLN-ACT-11: a minimal, valid, text-only PDF holding a paginated table (exports of results
//! and run reports). One standard Type 1 font (Helvetica, never embedded) with WinAnsiEncoding;
//! text strings are escaped (`\`, `(`, `)`), characters WinAnsi cannot encode are replaced by
//! `?` and counted, and the count is stated on the last page. Written through the in-house
//! writer ([`super::writer::write_file`], classic cross-reference table), so the in-house
//! reader can parse it back.

use std::collections::BTreeMap;

use super::object::{Dict, ObjRef, Object, Stream};
use super::writer::{write_file, Layout};

/// A4 portrait, in points.
const PAGE_W: f64 = 595.0;
const PAGE_H: f64 = 842.0;
const MARGIN: f64 = 40.0;
const FONT_SIZE: f64 = 8.0;
const LEADING: f64 = 11.0;
/// Rough Helvetica advance per character at 1 pt (used only to cut long cells).
const AVG_ADVANCE: f64 = 0.52;
/// Hard caps (hostile inputs): rows and pages.
const MAX_ROWS: usize = 200_000;

/// What the export wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TablePdf {
    pub bytes: Vec<u8>,
    pub pages: usize,
    /// Characters replaced by `?` because WinAnsi cannot encode them.
    pub replaced: u64,
}

/// WinAnsi (Windows-1252) byte for a character, if it has one.
pub fn winansi(c: char) -> Option<u8> {
    let u = c as u32;
    if (0x20..0x7f).contains(&u) || (0xa0..=0xff).contains(&u) {
        return Some(u as u8);
    }
    const HIGH: [(char, u8); 27] = [
        ('€', 0x80),
        ('‚', 0x82),
        ('ƒ', 0x83),
        ('„', 0x84),
        ('…', 0x85),
        ('†', 0x86),
        ('‡', 0x87),
        ('ˆ', 0x88),
        ('‰', 0x89),
        ('Š', 0x8a),
        ('‹', 0x8b),
        ('Œ', 0x8c),
        ('Ž', 0x8e),
        ('‘', 0x91),
        ('’', 0x92),
        ('“', 0x93),
        ('”', 0x94),
        ('•', 0x95),
        ('–', 0x96),
        ('—', 0x97),
        ('˜', 0x98),
        ('™', 0x99),
        ('š', 0x9a),
        ('›', 0x9b),
        ('œ', 0x9c),
        ('ž', 0x9e),
        ('Ÿ', 0x9f),
    ];
    HIGH.iter().find(|(h, _)| *h == c).map(|(_, b)| *b)
}

/// `text` as WinAnsi bytes, at most `max_chars` characters (cut with `...`), escaped for a
/// literal string; `replaced` counts characters that could not be encoded.
fn encode(text: &str, max_chars: usize, replaced: &mut u64) -> Vec<u8> {
    let chars: Vec<char> = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let cut = chars.len() > max_chars;
    let keep = if cut { max_chars.saturating_sub(3) } else { chars.len() };
    let mut out = Vec::with_capacity(keep + 3);
    for &c in &chars[..keep] {
        let b = winansi(c).unwrap_or_else(|| {
            *replaced += 1;
            b'?'
        });
        if matches!(b, b'\\' | b'(' | b')') {
            out.push(b'\\');
        }
        out.push(b);
    }
    if cut {
        out.extend_from_slice(b"...");
    }
    out
}

/// One text run at `(x, y)`.
fn show(content: &mut Vec<u8>, x: f64, y: f64, s: &[u8]) {
    content.extend_from_slice(format!("BT /F1 {FONT_SIZE} Tf {x:.1} {y:.1} Td (").as_bytes());
    content.extend_from_slice(s);
    content.extend_from_slice(b") Tj ET\n");
}

/// Write `rows` under `headers` (column widths as fractions of the usable width) with a title
/// and a subtitle line on every page.
pub fn table_pdf(title: &str, subtitle: &str, headers: &[&str], widths: &[f64], rows: &[Vec<String>]) -> TablePdf {
    let mut replaced = 0u64;
    let usable = PAGE_W - 2.0 * MARGIN;
    let total: f64 = widths.iter().sum::<f64>().max(f64::EPSILON);
    let cols: Vec<(f64, usize)> = {
        let mut x = MARGIN;
        widths
            .iter()
            .map(|w| {
                let width = usable * w / total;
                let col = (x, ((width - 4.0) / (FONT_SIZE * AVG_ADVANCE)).max(3.0) as usize);
                x += width;
                col
            })
            .collect()
    };
    let per_page = ((PAGE_H - 2.0 * MARGIN - 3.0 * LEADING) / LEADING).floor().max(1.0) as usize;
    let rows = &rows[..rows.len().min(MAX_ROWS)];
    let chunks: Vec<&[Vec<String>]> = if rows.is_empty() { vec![&rows[..0]] } else { rows.chunks(per_page).collect() };
    let n_pages = chunks.len();
    let mut contents = Vec::new();
    for (i, chunk) in chunks.iter().enumerate() {
        let mut c = Vec::new();
        let mut y = PAGE_H - MARGIN;
        show(&mut c, MARGIN, y, &encode(title, 110, &mut replaced));
        y -= LEADING;
        let sub = format!("{subtitle}  -  page {} of {n_pages}", i + 1);
        show(&mut c, MARGIN, y, &encode(&sub, 110, &mut replaced));
        y -= LEADING * 1.5;
        for ((x, max), h) in cols.iter().zip(headers) {
            show(&mut c, *x, y, &encode(h, *max, &mut replaced));
        }
        c.extend_from_slice(
            format!("{MARGIN:.1} {:.1} m {:.1} {:.1} l S\n", y - 3.0, PAGE_W - MARGIN, y - 3.0).as_bytes(),
        );
        y -= LEADING;
        for row in chunk.iter() {
            for ((x, max), cell) in cols.iter().zip(row) {
                show(&mut c, *x, y, &encode(cell, *max, &mut replaced));
            }
            y -= LEADING;
        }
        contents.push(c);
    }
    if replaced > 0 {
        let note = format!(
            "Note: {replaced} character(s) cannot be shown in this PDF font and were replaced by ?. \
             The CSV and HTML exports keep every character."
        );
        let last = contents.last_mut().expect("at least one page");
        let mut ignored = 0;
        show(last, MARGIN, MARGIN - 12.0, &encode(&note, 130, &mut ignored));
    }
    // Objects: 1 catalog, 2 pages, 3 font, then (page, content) pairs.
    let mut objects: BTreeMap<u32, Object> = BTreeMap::new();
    let r = |n: u32| Object::Ref(ObjRef::new(n));
    let name = |s: &str| Object::Name(s.as_bytes().to_vec());
    let mut kids = Vec::new();
    for (i, c) in contents.into_iter().enumerate() {
        let page = 4 + 2 * i as u32;
        let content = page + 1;
        let mut res = Dict::new();
        let mut fonts = Dict::new();
        fonts.set(b"F1", r(3));
        res.set(b"Font", Object::Dict(fonts));
        let mut p = Dict::new();
        p.set(b"Type", name("Page"));
        p.set(b"Parent", r(2));
        p.set(
            b"MediaBox",
            Object::Array(vec![Object::Int(0), Object::Int(0), Object::Int(PAGE_W as i64), Object::Int(PAGE_H as i64)]),
        );
        p.set(b"Resources", Object::Dict(res));
        p.set(b"Contents", r(content));
        objects.insert(page, Object::Dict(p));
        objects.insert(content, Object::Stream(Stream { dict: Dict::new(), data: c }));
        kids.push(r(page));
    }
    let mut pages = Dict::new();
    pages.set(b"Type", name("Pages"));
    pages.set(b"Count", Object::Int(kids.len() as i64));
    pages.set(b"Kids", Object::Array(kids));
    objects.insert(2, Object::Dict(pages));
    let mut font = Dict::new();
    font.set(b"Type", name("Font"));
    font.set(b"Subtype", name("Type1"));
    font.set(b"BaseFont", name("Helvetica"));
    font.set(b"Encoding", name("WinAnsiEncoding"));
    objects.insert(3, Object::Dict(font));
    let mut cat = Dict::new();
    cat.set(b"Type", name("Catalog"));
    cat.set(b"Pages", r(2));
    objects.insert(1, Object::Dict(cat));
    let mut trailer = Dict::new();
    trailer.set(b"Root", r(1));
    TablePdf { bytes: write_file((1, 4), &objects, &trailer, Layout::Classic), pages: n_pages, replaced }
}

#[cfg(test)]
mod tests {
    use super::super::document::Document;
    use super::*;

    fn page_text(doc: &Document, page: u32) -> Vec<u8> {
        let p = doc.objects.get(&page).and_then(Object::as_dict).unwrap();
        let c = doc.resolve(p.get(b"Contents").unwrap());
        match c {
            Object::Stream(s) => s.data.clone(),
            _ => panic!("no content stream"),
        }
    }

    #[test]
    fn cln_act_11_table_pdf_is_valid_paginated_and_escaped() {
        let mut rows: Vec<Vec<String>> = (0..150)
            .map(|i| vec![format!("/home/m/f{i} (copy).txt"), "1.2 MB".into(), "Trash".into(), "done".into()])
            .collect();
        rows.push(vec!["C:\\Users\\Mèl\\résumé 日本.pdf".into(), "3 KB".into(), "Move".into(), "skipped".into()]);
        let t = table_pdf("Report", "run 1", &["Path", "Size", "Action", "Status"], &[6.0, 1.0, 1.5, 1.0], &rows);
        assert!(t.pages >= 2, "paginated");
        assert_eq!(t.replaced, 2, "the two CJK characters");
        let doc = Document::load(&t.bytes).expect("the in-house reader parses it");
        let pages = doc.pages().unwrap();
        assert_eq!(pages.len(), t.pages);
        let first = page_text(&doc, pages[0]);
        assert!(super::super::document::contains(&first, b"f0 \\(copy\\).txt"), "parentheses escaped");
        let last = page_text(&doc, *pages.last().unwrap());
        assert!(
            super::super::document::contains(&last, b"C:\\\\Users\\\\M\xe8l\\\\r\xe9sum\xe9 ??.pdf"),
            "WinAnsi + ?"
        );
        assert!(super::super::document::contains(&last, b"replaced by ?"), "the replacement is noted");
        let font = doc.objects.values().filter_map(Object::as_dict).find(|d| d.get(b"BaseFont").is_some()).unwrap();
        assert!(font.get(b"Encoding").unwrap().is_name(b"WinAnsiEncoding"));
        let empty = table_pdf("Empty", "", &["Path"], &[1.0], &[]);
        assert_eq!(Document::load(&empty.bytes).unwrap().pages().unwrap().len(), 1);
    }

    #[test]
    fn winansi_mapping() {
        assert_eq!(winansi('A'), Some(b'A'));
        assert_eq!(winansi('é'), Some(0xe9));
        assert_eq!(winansi('€'), Some(0x80));
        assert_eq!(winansi('日'), None);
        assert_eq!(winansi('\u{0081}'), None);
    }
}
