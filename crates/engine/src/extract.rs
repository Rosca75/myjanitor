//! CLN-CNT-01: text extractors for content search, registered through the filter crate's
//! `TextExtractor` extension point. Best effort, read-only and bounded:
//!
//! - **PDF**: the in-house reader loads the file (at most `max_bytes`); every page's decoded
//!   content streams are scanned for the literal and hex strings shown by `Tj`, `TJ`, `'`
//!   and `"`. Simple single-byte fonts read well; CID/Identity-H fonts give no text (no
//!   match, never a false one). Encrypted files give an error (reported as unreadable).
//! - **DOCX/DOCM** `word/document.xml` text runs (`w:t`), **XLSX/XLSM** shared strings and
//!   inline strings (`t`), **PPTX/PPTM** slide text runs (`a:t`), through the in-house OOXML
//!   reader with zip-bomb caps scaled to `max_bytes`.
//!
//! Output is capped at `max_bytes` characters of text.

use std::path::Path;
use std::sync::Arc;

use mj_core::{Error, Result};
use mj_filters::content::{ExtractorSet, TextExtractor};

/// Every built-in extractor.
pub(crate) fn builtin() -> ExtractorSet {
    let mut s = ExtractorSet::new();
    s.register(Arc::new(PdfText));
    s.register(Arc::new(OoxmlText));
    s
}

/// Decode the XML entities a text node can hold.
fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';').filter(|e| *e <= 12) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..end];
        let c = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32)
            }
            _ if ent.starts_with('#') => ent[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The text of every element whose qualified name is exactly `tag` (`w:t`, `a:t`, `t`),
/// appended to `out` with a space between runs, until `out` holds `cap` bytes.
pub(crate) fn element_text(xml: &str, tag: &str, out: &mut String, cap: usize) {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut rest = xml;
    while let Some(i) = rest.find(&open) {
        if out.len() >= cap {
            return;
        }
        rest = &rest[i + open.len()..];
        // `<w:t>` or `<w:t xml:space=...>`, never `<w:tab/>` or `<w:tbl>`.
        let Some(c) = rest.chars().next() else { return };
        if c != '>' && !c.is_whitespace() && c != '/' {
            continue;
        }
        let Some(gt) = rest.find('>') else { return };
        if rest[..gt].ends_with('/') {
            rest = &rest[gt + 1..];
            continue; // empty element
        }
        rest = &rest[gt + 1..];
        let Some(end) = rest.find(&close) else { return };
        out.push_str(&unescape(&rest[..end]));
        out.push(' ');
        rest = &rest[end + close.len()..];
    }
}

/// PDF text (best effort, simple fonts).
struct PdfText;

impl TextExtractor for PdfText {
    fn extensions(&self) -> &[&str] {
        &["pdf"]
    }

    fn extract_text(&self, path: &Path, max_bytes: u64) -> Result<String> {
        let len = std::fs::metadata(path).map_err(|e| Error::io(path, e))?.len();
        if len > max_bytes.max(1 << 20) {
            return Err(Error::unsupported("Content search", "this PDF is larger than the content-search limit"));
        }
        let data = std::fs::read(path).map_err(|e| Error::io(path, e))?;
        let doc = mj_formats::pdf::document::Document::load(&data)?;
        if doc.xref.encrypted {
            return Err(Error::unsupported("Content search", "the PDF is encrypted"));
        }
        let cap = max_bytes as usize;
        let mut out = String::new();
        for page in doc.pages()? {
            if out.len() >= cap {
                break;
            }
            let Some(p) = doc.objects.get(&page).and_then(mj_formats::pdf::object::Object::as_dict) else { continue };
            let Some(c) = p.get(b"Contents") else { continue };
            let streams: Vec<&mj_formats::pdf::object::Object> = match doc.resolve(c) {
                mj_formats::pdf::object::Object::Array(a) => a.iter().map(|x| doc.resolve(x)).collect(),
                other => vec![other],
            };
            for s in streams {
                if let mj_formats::pdf::object::Object::Stream(s) = s {
                    if let Ok(bytes) = mj_formats::pdf::filter::decode_full(&s.dict, &s.data) {
                        content_strings(&bytes, &mut out, cap);
                    }
                }
            }
        }
        Ok(out)
    }
}

/// Literal `( ... )` and hex `< ... >` strings of a content stream, decoded as Latin-1 (what
/// simple fonts with standard encodings mostly are), separated by spaces.
pub(crate) fn content_strings(b: &[u8], out: &mut String, cap: usize) {
    let mut i = 0;
    while i < b.len() && out.len() < cap {
        match b[i] {
            b'%' => {
                while i < b.len() && b[i] != b'\n' && b[i] != b'\r' {
                    i += 1;
                }
            }
            b'(' => {
                let mut depth = 1;
                i += 1;
                while i < b.len() && depth > 0 {
                    let c = b[i];
                    match c {
                        b'\\' if i + 1 < b.len() => {
                            i += 1;
                            let e = b[i];
                            let ch = match e {
                                b'n' => Some('\n'),
                                b'r' | b't' | b'b' | b'f' => Some(' '),
                                b'0'..=b'7' => {
                                    let mut v = u32::from(e - b'0');
                                    for _ in 0..2 {
                                        if i + 1 < b.len() && (b'0'..=b'7').contains(&b[i + 1]) {
                                            i += 1;
                                            v = v * 8 + u32::from(b[i] - b'0');
                                        }
                                    }
                                    char::from_u32(v & 0xff)
                                }
                                b'\n' | b'\r' => None,
                                other => Some(other as char),
                            };
                            if let Some(ch) = ch {
                                out.push(ch);
                            }
                        }
                        b'(' => {
                            depth += 1;
                            out.push('(');
                        }
                        b')' => {
                            depth -= 1;
                            if depth > 0 {
                                out.push(')');
                            }
                        }
                        _ => out.push(c as char),
                    }
                    i += 1;
                }
                out.push(' ');
                continue;
            }
            b'<' if b.get(i + 1) != Some(&b'<') => {
                let start = i + 1;
                let Some(len) = b[start..].iter().position(|&c| c == b'>') else { return };
                let hex: Vec<u8> = b[start..start + len].iter().copied().filter(u8::is_ascii_hexdigit).collect();
                for pair in hex.chunks(2) {
                    let s = std::str::from_utf8(pair).unwrap_or("0");
                    let v = u8::from_str_radix(&format!("{s:0<2}"), 16).unwrap_or(b' ');
                    out.push(v as char);
                }
                out.push(' ');
                i = start + len + 1;
                continue;
            }
            b'<' => i += 1, // `<<` of an inline dictionary
            _ => {}
        }
        i += 1;
    }
}

/// Word, Excel and PowerPoint text.
struct OoxmlText;

impl TextExtractor for OoxmlText {
    fn extensions(&self) -> &[&str] {
        &["docx", "docm", "xlsx", "xlsm", "pptx", "pptm"]
    }

    fn extract_text(&self, path: &Path, max_bytes: u64) -> Result<String> {
        let ext = mj_core::pathutil::extension_lower(path);
        let caps = mj_formats::zip::write::repack::Caps {
            max_entries: 20_000,
            max_entry: max_bytes.max(1 << 20) * 4,
            max_total: max_bytes.max(1 << 20) * 16,
        };
        let mut pkg = mj_formats::ooxml::Package::open_file(path, caps)?;
        let cap = max_bytes as usize;
        let mut out = String::new();
        let (parts, tag): (Vec<String>, &str) = match ext.as_str() {
            "docx" | "docm" => (vec!["word/document.xml".into()], "w:t"),
            "xlsx" | "xlsm" => {
                let mut v = vec!["xl/sharedStrings.xml".to_string()];
                v.extend(pkg.parts().into_iter().filter(|p| {
                    let l = p.to_ascii_lowercase();
                    l.starts_with("xl/worksheets/sheet") && l.ends_with(".xml")
                }));
                (v, "t")
            }
            _ => {
                let mut v: Vec<String> = pkg
                    .parts()
                    .into_iter()
                    .filter(|p| {
                        let l = p.to_ascii_lowercase();
                        l.starts_with("ppt/slides/slide") && l.ends_with(".xml")
                    })
                    .collect();
                // Slide order: slide2 before slide10.
                v.sort_by_key(|p| {
                    p.trim_start_matches(|c: char| !c.is_ascii_digit())
                        .trim_end_matches(".xml")
                        .parse::<u64>()
                        .unwrap_or(0)
                });
                (v, "a:t")
            }
        };
        for part in parts {
            if out.len() >= cap {
                break;
            }
            let Some(name) = pkg.canonical(&part) else { continue };
            let bytes = pkg.read(&name)?;
            let text = String::from_utf8_lossy(&bytes);
            element_text(&text, tag, &mut out, cap);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_runs_and_entities() {
        let mut out = String::new();
        element_text(
            r#"<w:p><w:r><w:t>Hello</w:t><w:tab/><w:t xml:space="preserve"> &amp; caf&#233;</w:t><w:t/></w:r><w:tbl/></w:p>"#,
            "w:t",
            &mut out,
            1000,
        );
        assert_eq!(out, "Hello  & café ");
        let mut capped = String::new();
        element_text("<t>aaaa</t><t>bbbb</t>", "t", &mut capped, 3);
        assert_eq!(capped, "aaaa ");
    }

    #[test]
    fn pdf_content_strings() {
        let mut out = String::new();
        content_strings(
            b"BT /F1 9 Tf (Invoice \\(2024\\)) Tj [(Tot) -20 (al)] TJ <48656c6c6f> Tj ET % (comment)",
            &mut out,
            1000,
        );
        assert!(out.contains("Invoice (2024)"), "{out}");
        assert!(out.contains("Tot al"), "{out}");
        assert!(out.contains("Hello"), "{out}");
        assert!(!out.contains("comment"));
    }

    #[test]
    fn extractors_read_real_files() {
        let t = mj_core::testutil::TempDir::new("extract").unwrap();
        let rows = vec![vec!["needle-in-pdf".to_string()]];
        let pdf = mj_formats::pdf::table::table_pdf("Title", "sub", &["Col"], &[1.0], &rows);
        let p = t.write("a.pdf", &pdf.bytes).unwrap();
        assert!(PdfText.extract_text(&p, 8 << 20).unwrap().contains("needle-in-pdf"));
        assert!(PdfText.extract_text(&t.write("bad.pdf", b"not a pdf").unwrap(), 8 << 20).is_err());
    }
}
