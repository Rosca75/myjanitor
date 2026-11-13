//! Full-text content search (CLN-CNT-01) and extension-mismatch checks
//! (CLN-CNT-02). Owner: W2-C.
//!
//! * Plain text is searched here: UTF-8 (with or without BOM), UTF-16 LE/BE
//!   (BOM or the NUL-interleaving heuristic) and 8-bit legacy text. At most
//!   [`ContentConfig::max_bytes`] of a file are read; binary files are skipped.
//! * PDF, Word, Excel, PowerPoint and OpenDocument text is reached through
//!   the [`TextExtractor`] extension point. Nobody registers extractors by
//!   default, so those files are reported as *unavailable* (never as "no
//!   match"): see [`ContentOutcome::Unavailable`] and
//!   `FilterDiagnostics::content_unavailable`.
//! * The mismatch check reads a bounded header and asks `mj_formats::sniff`.

use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use mj_core::Result;
use mj_formats::sniff::{self, Verdict};

/// Extensions whose text cannot be read without an extractor.
pub const DOCUMENT_EXTENSIONS: &[&str] =
    &["pdf", "docx", "docm", "xlsx", "xlsm", "pptx", "pptm", "doc", "xls", "ppt", "odt", "ods", "odp"];

/// Default cap on bytes read per file for a content search.
pub const DEFAULT_MAX_BYTES: u64 = 8 << 20;

/// Supplies the text of a document format (PDF, DOCX, ...). Implemented by
/// callers (the engine wires `mj-formats` parsers in); the filter crate never
/// parses those formats itself.
pub trait TextExtractor: Send + Sync {
    /// Lower-case extensions (no dot) this extractor handles.
    fn extensions(&self) -> &[&str];
    /// Extract the text of `path`, reading at most about `max_bytes` of source.
    fn extract_text(&self, path: &Path, max_bytes: u64) -> Result<String>;
}

/// The registered extractors.
#[derive(Clone, Default)]
pub struct ExtractorSet {
    list: Vec<Arc<dyn TextExtractor>>,
}

impl std::fmt::Debug for ExtractorSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ExtractorSet({} extractors)", self.list.len())
    }
}

impl ExtractorSet {
    /// No extractors.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an extractor (earlier registrations win for the same extension).
    pub fn register(&mut self, e: Arc<dyn TextExtractor>) {
        self.list.push(e);
    }

    /// The extractor for `ext`, if any.
    pub fn find(&self, ext: &str) -> Option<&Arc<dyn TextExtractor>> {
        self.list.iter().find(|e| e.extensions().iter().any(|x| x.eq_ignore_ascii_case(ext)))
    }
}

/// Limits and extension points for content conditions.
#[derive(Clone, Debug)]
pub struct ContentConfig {
    /// Read at most this many bytes of a file.
    pub max_bytes: u64,
    /// Document text extractors.
    pub extractors: ExtractorSet,
}

impl Default for ContentConfig {
    fn default() -> Self {
        ContentConfig { max_bytes: DEFAULT_MAX_BYTES, extractors: ExtractorSet::new() }
    }
}

/// A prepared search text.
#[derive(Debug, Clone)]
pub struct Needle {
    text: String,
    case_sensitive: bool,
}

impl Needle {
    /// Prepare `text` (lower-cased up front when the search ignores case).
    pub fn new(text: &str, case_sensitive: bool) -> Needle {
        Needle { text: if case_sensitive { text.to_string() } else { text.to_lowercase() }, case_sensitive }
    }

    /// Does `haystack` contain the text?
    pub fn found_in(&self, haystack: &str) -> bool {
        if self.case_sensitive {
            haystack.contains(&self.text)
        } else {
            haystack.to_lowercase().contains(&self.text)
        }
    }
}

/// What a content search of one file concluded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentOutcome {
    /// The text occurs.
    Found,
    /// It does not (within the bytes read; `truncated` when the file was longer than the cap).
    NotFound { truncated: bool },
    /// A document format with no registered extractor: not searched.
    Unavailable,
    /// Binary content: not searched.
    Binary,
    /// The file (or the extractor) failed.
    Unreadable,
}

/// Search one file for `needle`.
pub fn search_file(path: &Path, ext: &str, needle: &Needle, cfg: &ContentConfig) -> ContentOutcome {
    if let Some(ex) = cfg.extractors.find(ext) {
        return match ex.extract_text(path, cfg.max_bytes) {
            Ok(t) if needle.found_in(&t) => ContentOutcome::Found,
            Ok(t) => ContentOutcome::NotFound { truncated: t.len() as u64 >= cfg.max_bytes },
            Err(_) => ContentOutcome::Unreadable,
        };
    }
    if DOCUMENT_EXTENSIONS.contains(&ext) {
        return ContentOutcome::Unavailable;
    }
    let mut buf = Vec::new();
    let read = File::open(path).and_then(|f| f.take(cfg.max_bytes.saturating_add(1)).read_to_end(&mut buf));
    if read.is_err() {
        return ContentOutcome::Unreadable;
    }
    let truncated = buf.len() as u64 > cfg.max_bytes;
    if truncated {
        buf.truncate(cfg.max_bytes as usize);
    }
    match decode_text(&buf) {
        None => ContentOutcome::Binary,
        Some(text) if needle.found_in(&text) => ContentOutcome::Found,
        Some(_) => ContentOutcome::NotFound { truncated },
    }
}

/// Decode bytes as text, or `None` when they look binary.
pub fn decode_text(b: &[u8]) -> Option<String> {
    if let Some(rest) = b.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return Some(String::from_utf8_lossy(rest).into_owned());
    }
    if let Some(rest) = b.strip_prefix(&[0xFF, 0xFE]) {
        return Some(utf16(rest, false));
    }
    if let Some(rest) = b.strip_prefix(&[0xFE, 0xFF]) {
        return Some(utf16(rest, true));
    }
    let sample = &b[..b.len().min(8192)];
    if sample.contains(&0) {
        // BOM-less UTF-16: NULs sit at every other byte.
        let pairs = sample.len() / 2;
        if pairs >= 2 {
            let odd = sample.iter().skip(1).step_by(2).filter(|&&c| c == 0).count();
            let even = sample.iter().step_by(2).filter(|&&c| c == 0).count();
            if odd * 10 >= pairs * 3 && even == 0 {
                return Some(utf16(b, false));
            }
            if even * 10 >= pairs * 3 && odd == 0 {
                return Some(utf16(b, true));
            }
        }
        return None;
    }
    // Control characters other than whitespace and ESC mean binary.
    let controls = sample.iter().filter(|&&c| c < 0x20 && !matches!(c, b'\t' | b'\n' | b'\r' | 0x0C | 0x1B)).count();
    if controls * 20 > sample.len().max(1) {
        return None;
    }
    Some(String::from_utf8_lossy(b).into_owned())
}

fn utf16(b: &[u8], big_endian: bool) -> String {
    let units = b.chunks_exact(2).map(|c| {
        if big_endian {
            u16::from_be_bytes([c[0], c[1]])
        } else {
            u16::from_le_bytes([c[0], c[1]])
        }
    });
    char::decode_utf16(units).map(|r| r.unwrap_or('\u{FFFD}')).collect()
}

/// Result of an extension-mismatch check of one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MismatchOutcome {
    /// Content does not fit the extension.
    Mismatch,
    /// Fits, or the extension / file cannot be judged (empty, unchecked extension).
    Fine,
    /// The header could not be read.
    Unreadable,
}

/// True-type-versus-extension check (CLN-CNT-02) on a bounded header read.
pub fn extension_mismatch(path: &Path, ext: &str) -> MismatchOutcome {
    match sniff::check_file(path, ext) {
        Ok(Verdict::Mismatch { .. }) => MismatchOutcome::Mismatch,
        Ok(_) => MismatchOutcome::Fine,
        Err(_) => MismatchOutcome::Unreadable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::testutil::TempDir;

    fn utf16le(s: &str, bom: bool) -> Vec<u8> {
        let mut v = if bom { vec![0xFF, 0xFE] } else { vec![] };
        v.extend(s.encode_utf16().flat_map(|u| u.to_le_bytes()));
        v
    }

    #[test]
    fn decodes_utf8_utf16_and_rejects_binary() {
        assert_eq!(decode_text(b"\xEF\xBB\xBFhello").as_deref(), Some("hello"));
        assert_eq!(decode_text("héllo wörld".as_bytes()).as_deref(), Some("héllo wörld"));
        assert_eq!(decode_text(&utf16le("héllo", true)).as_deref(), Some("héllo"));
        assert_eq!(decode_text(&utf16le("hello world", false)).as_deref(), Some("hello world"));
        let be: Vec<u8> = "hello".encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
        assert_eq!(decode_text(&be).as_deref(), Some("hello"));
        assert_eq!(decode_text(&[0x89, b'P', b'N', b'G', 0, 0, 0, 13, 0, 1, 2]), None);
        assert_eq!(decode_text(&[1, 2, 3, 4, 5, 6, 7, 8]), None);
        assert!(decode_text(b"").is_some());
    }

    #[test]
    fn searches_plain_text_with_case_rules_and_cap() {
        let d = TempDir::new("content").unwrap();
        let a = d.write("a.txt", b"The Quick Brown Fox").unwrap();
        let w = d.write("w.txt", &utf16le("Invoice 42 paid", true)).unwrap();
        let cfg = ContentConfig::default();
        assert_eq!(search_file(&a, "txt", &Needle::new("quick brown", false), &cfg), ContentOutcome::Found);
        assert_eq!(
            search_file(&a, "txt", &Needle::new("quick brown", true), &cfg),
            ContentOutcome::NotFound { truncated: false }
        );
        assert_eq!(search_file(&w, "txt", &Needle::new("invoice 42", false), &cfg), ContentOutcome::Found);
        // Text after the cap is not seen, and that is reported.
        let big = d.write("big.txt", &[vec![b'a'; 100], b" needle".to_vec()].concat()).unwrap();
        let small = ContentConfig { max_bytes: 50, ..ContentConfig::default() };
        assert_eq!(
            search_file(&big, "txt", &Needle::new("needle", false), &small),
            ContentOutcome::NotFound { truncated: true }
        );
        let bin = d.write("b.bin", &[0, 1, 2, 3, 0, 0, 9]).unwrap();
        assert_eq!(search_file(&bin, "bin", &Needle::new("x", false), &cfg), ContentOutcome::Binary);
        assert_eq!(search_file(&d.join("nope.txt"), "txt", &Needle::new("x", false), &cfg), ContentOutcome::Unreadable);
    }

    struct FakePdf;
    impl TextExtractor for FakePdf {
        fn extensions(&self) -> &[&str] {
            &["pdf"]
        }
        fn extract_text(&self, path: &Path, _max: u64) -> Result<String> {
            if path.ends_with("broken.pdf") {
                Err(mj_core::Error::parse("pdf", "bad xref"))
            } else {
                Ok("Quarterly REPORT".into())
            }
        }
    }

    #[test]
    fn documents_are_unavailable_until_an_extractor_is_registered() {
        let d = TempDir::new("content-doc").unwrap();
        let p = d.write("a.pdf", b"%PDF-1.4").unwrap();
        let broken = d.write("broken.pdf", b"%PDF-1.4").unwrap();
        let n = Needle::new("report", false);
        let mut cfg = ContentConfig::default();
        assert_eq!(search_file(&p, "pdf", &n, &cfg), ContentOutcome::Unavailable);
        cfg.extractors.register(Arc::new(FakePdf));
        assert_eq!(search_file(&p, "pdf", &n, &cfg), ContentOutcome::Found);
        assert_eq!(search_file(&broken, "pdf", &n, &cfg), ContentOutcome::Unreadable);
        assert_eq!(search_file(&p, "docx", &n, &cfg), ContentOutcome::Unavailable);
    }

    #[test]
    fn mismatch_check_reads_a_header() {
        let d = TempDir::new("content-mm").unwrap();
        let ok = d.write("ok.png", b"\x89PNG\r\n\x1a\n....").unwrap();
        let bad = d.write("bad.png", b"<html>not an image</html>").unwrap();
        let empty = d.write("empty.png", b"").unwrap();
        assert_eq!(extension_mismatch(&ok, "png"), MismatchOutcome::Fine);
        assert_eq!(extension_mismatch(&bad, "png"), MismatchOutcome::Mismatch);
        assert_eq!(extension_mismatch(&empty, "png"), MismatchOutcome::Fine);
        assert_eq!(extension_mismatch(&d.join("gone.png"), "png"), MismatchOutcome::Unreadable);
        assert_eq!(extension_mismatch(&bad, "txt"), MismatchOutcome::Fine);
    }
}
