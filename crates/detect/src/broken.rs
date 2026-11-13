//! Broken / corrupted files (CLN-DET-08) by cheap structural checks only: a handful of bytes at the
//! head and tail of each file (PNG signature and IEND, JPEG SOI/EOI, PDF header and `%%EOF`, ZIP
//! end-of-central-directory) plus a streaming-free gzip completeness check (see `inflate`).
//!
//! These are heuristics for *obvious* damage (truncated downloads, interrupted copies). A file that
//! passes is not proven healthy, and everything flagged is review-only and never pre-selected.
//! Owner: W2-D.

use std::path::Path;

use mj_core::pool::parallel_map;
use mj_core::{
    ActionKind, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, FileEntry,
    PlatformSupport, Result, SafetyClass,
};

use crate::common::{self, Ticker};
use crate::fsio;
use crate::inflate::{check_gzip, GzipVerdict};

/// Largest tail window needed: ZIP EOCD (22 bytes) plus a 64 KiB comment.
const ZIP_TAIL: usize = 65_557 + 16;

/// What a structural check found.
#[derive(Debug, PartialEq, Eq)]
pub enum Check {
    Healthy,
    /// Obvious damage, with a one-line reason.
    Broken(String),
    /// Not evaluated, counted under this diagnostics key.
    Skipped(&'static str),
    /// I/O failure while reading.
    Failed,
}

/// Broken files: Damaged images, documents and archives
#[derive(Debug, Default, Clone, Copy)]
pub struct BrokenFiles;

impl Detector for BrokenFiles {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::BrokenFiles,
            title: "Broken files".into(),
            description: "Damaged images, documents and archives".into(),
            category: Category::Clean,
            safety: SafetyClass::ReviewOnly,
            default_action: ActionKind::None,
            available_actions: vec![ActionKind::None, ActionKind::Trash, ActionKind::Quarantine],
            settings: vec![common::number_setting(
                "max_gzip_mb",
                "Largest .gz file to verify",
                "Larger gzip files are only checked for a valid header.",
                32.0,
                1.0,
                4096.0,
                "MB",
            )],
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-08".into()],
            unit: "files".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let max_gz = (ctx.params.number("max_gzip_mb", 32.0).max(1.0) * 1_048_576.0) as u64;
        let mut out = DetectorOutput::default();
        let mut tick = Ticker::new(ctx, ctx.index.len() as u64);
        let mut work: Vec<(&FileEntry, Kind)> = Vec::new();
        for e in ctx.index.entries() {
            tick.tick()?;
            if !e.is_file() || e.size == 0 || !common::usable(e) {
                continue;
            }
            if let Some(kind) = Kind::of(&e.extension()) {
                work.push((e, kind));
            }
        }
        let results = parallel_map(&work, ctx.threads, ctx.cancel, |_, (e, k)| check(&e.path, e.size, *k, max_gz));
        ctx.cancel.checkpoint()?;
        for ((e, kind), r) in work.iter().zip(results) {
            match r.unwrap_or(Check::Failed) {
                Check::Healthy => {}
                Check::Skipped(why) => out.diagnostics.skip(why),
                Check::Failed => out.diagnostics.unreadable(&e.path),
                Check::Broken(reason) => {
                    tick.found(e.reclaimable());
                    let mut c = common::file_candidate(e, DetectorId::BrokenFiles, reason, false);
                    c.columns.insert("type".into(), kind.label().into());
                    out.candidates.push(c);
                }
            }
        }
        tick.finish();
        Ok(out)
    }
}

/// File families that get a structural check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Png,
    Jpeg,
    Pdf,
    Zip,
    Gzip,
}

impl Kind {
    /// Family for a lower-case extension.
    pub fn of(ext: &str) -> Option<Kind> {
        Some(match ext {
            "png" => Kind::Png,
            "jpg" | "jpeg" | "jpe" => Kind::Jpeg,
            "pdf" => Kind::Pdf,
            "zip" | "docx" | "xlsx" | "pptx" | "odt" | "ods" | "odp" | "epub" | "jar" | "apk" | "docm" | "xlsm" => {
                Kind::Zip
            }
            "gz" | "tgz" => Kind::Gzip,
            _ => return None,
        })
    }

    fn label(self) -> &'static str {
        match self {
            Kind::Png => "PNG image",
            Kind::Jpeg => "JPEG image",
            Kind::Pdf => "PDF document",
            Kind::Zip => "ZIP archive",
            Kind::Gzip => "gzip archive",
        }
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// Run the structural check for `kind` on a file of `size` bytes.
pub fn check(path: &Path, size: u64, kind: Kind, max_gz: u64) -> Check {
    match check_io(path, size, kind, max_gz) {
        Ok(c) => c,
        Err(_) => Check::Failed,
    }
}

fn check_io(path: &Path, size: u64, kind: Kind, max_gz: u64) -> std::io::Result<Check> {
    let broken = |s: &str| Ok(Check::Broken(s.to_string()));
    match kind {
        Kind::Png => {
            let head = fsio::read_head(path, 8)?;
            if head != [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a] {
                return broken("Not a valid PNG: the file signature is wrong");
            }
            let (tail, _) = fsio::read_tail(path, 32)?;
            if !contains(&tail, b"IEND") {
                return broken("PNG has no IEND end marker; the file is cut short");
            }
        }
        Kind::Jpeg => {
            let head = fsio::read_head(path, 2)?;
            if head != [0xff, 0xd8] {
                return broken("Not a valid JPEG: it does not start with the SOI marker");
            }
            let (tail, _) = fsio::read_tail(path, 4096)?;
            if !contains(&tail, &[0xff, 0xd9]) {
                return broken("JPEG has no end-of-image marker; the file is cut short");
            }
        }
        Kind::Pdf => {
            let head = fsio::read_head(path, 1024)?;
            if !contains(&head, b"%PDF-") {
                return broken("Not a valid PDF: the %PDF header is missing");
            }
            let (tail, _) = fsio::read_tail(path, 2048)?;
            if !contains(&tail, b"%%EOF") {
                return broken("PDF has no %%EOF marker; the file is cut short");
            }
        }
        Kind::Zip => {
            let head = fsio::read_head(path, 8)?;
            if head.starts_with(&[0xd0, 0xcf, 0x11, 0xe0]) {
                return Ok(Check::Skipped("encrypted_or_legacy_office")); // password-protected OOXML is a CFB file
            }
            if !(head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") || head.starts_with(b"PK\x07\x08"))
            {
                // Self-extracting and prefixed archives start with other bytes; only the tail decides.
                let (tail, _) = fsio::read_tail(path, ZIP_TAIL)?;
                return Ok(if contains(&tail, b"PK\x05\x06") {
                    Check::Healthy
                } else {
                    Check::Broken("Not a valid ZIP: no ZIP signature and no central directory".into())
                });
            }
            let (tail, _) = fsio::read_tail(path, ZIP_TAIL)?;
            if !contains(&tail, b"PK\x05\x06") {
                return broken("ZIP has no end-of-central-directory record; the archive is cut short");
            }
        }
        Kind::Gzip => {
            if size > max_gz {
                let head = fsio::read_head(path, 3)?;
                return Ok(if head.starts_with(&[0x1f, 0x8b, 8]) {
                    Check::Skipped("too_large_to_verify")
                } else {
                    Check::Broken("Not a valid gzip file: the header is wrong".into())
                });
            }
            let data = fsio::read_head(path, size as usize)?;
            return Ok(match check_gzip(&data) {
                GzipVerdict::Complete => Check::Healthy,
                GzipVerdict::BadHeader => Check::Broken("Not a valid gzip file: the header is wrong".into()),
                GzipVerdict::Broken(why) => Check::Broken(format!("gzip archive is truncated or corrupt: {why}")),
            });
        }
    }
    Ok(Check::Healthy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inflate::tests::DYNAMIC;
    use crate::testkit::*;
    use mj_core::testutil::TempDir;

    fn png_ok() -> Vec<u8> {
        let mut v = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        v.extend_from_slice(&[
            0, 0, 0, 0x0d, b'I', b'H', b'D', b'R', 0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0, 1, 2, 3, 4,
        ]);
        v.extend_from_slice(&[0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82]);
        v
    }

    fn jpeg_ok() -> Vec<u8> {
        let mut v = vec![0xff, 0xd8, 0xff, 0xe0, 0, 4, 0, 0];
        v.extend(std::iter::repeat_n(0x11, 500));
        v.extend_from_slice(&[0xff, 0xd9]);
        v
    }

    fn pdf_ok() -> Vec<u8> {
        let mut v = b"%PDF-1.7\n".to_vec();
        v.extend(std::iter::repeat_n(b'x', 3000));
        v.extend_from_slice(b"\ntrailer\n%%EOF\n");
        v
    }

    fn zip_ok() -> Vec<u8> {
        let mut v = b"PK\x03\x04".to_vec();
        v.extend(std::iter::repeat_n(7, 300));
        v.extend_from_slice(b"PK\x05\x06\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0");
        v
    }

    fn truncated(v: &[u8], n: usize) -> Vec<u8> {
        v[..v.len() - n].to_vec()
    }

    fn fixture() -> TempDir {
        let t = TempDir::new("broken").unwrap();
        t.write("good.png", &png_ok()).unwrap();
        t.write("cut.png", &truncated(&png_ok(), 12)).unwrap();
        t.write("sigless.png", b"this is not a png at all").unwrap();
        t.write("good.jpg", &jpeg_ok()).unwrap();
        t.write("cut.jpg", &truncated(&jpeg_ok(), 100)).unwrap();
        t.write("padded.jpeg", &[jpeg_ok(), vec![0; 200]].concat()).unwrap();
        t.write("good.pdf", &pdf_ok()).unwrap();
        t.write("cut.pdf", &truncated(&pdf_ok(), 40)).unwrap();
        t.write("good.zip", &zip_ok()).unwrap();
        t.write("cut.docx", &truncated(&zip_ok(), 22)).unwrap();
        t.write("encrypted.docx", &[0xd0, 0xcf, 0x11, 0xe0, 1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
        t.write("good.gz", &DYNAMIC).unwrap();
        t.write("cut.gz", &DYNAMIC[..200]).unwrap();
        t.write("notes.txt", b"plain").unwrap();
        t
    }

    #[test]
    fn flags_only_damaged_files_and_never_selects_them() {
        let t = fixture();
        let out = run(&BrokenFiles, &index_of(t.path()), &Default::default());
        assert_eq!(rel(&out, t.path()), vec!["cut.docx", "cut.gz", "cut.jpg", "cut.pdf", "cut.png", "sigless.png"]);
        assert!(out.candidates.iter().all(|c| !c.default_selected));
        assert_eq!(out.diagnostics.skipped["encrypted_or_legacy_office"], 1);
        let gz = out.candidates.iter().find(|c| c.path.ends_with("cut.gz")).unwrap();
        assert_eq!(gz.columns["type"], "gzip archive");
    }

    #[test]
    fn large_gzip_is_not_verified() {
        let t = fixture();
        let out = run(&BrokenFiles, &index_of(t.path()), &p(&[("max_gzip_mb", 1.0)]));
        // Under the default cap the cut .gz is found; with a 1 MB cap it still is (file is tiny).
        assert!(rel(&out, t.path()).contains(&"cut.gz".to_string()));
        assert_eq!(check(&t.join("good.gz"), 5 << 20, Kind::Gzip, 1 << 20), Check::Skipped("too_large_to_verify"));
    }

    #[test]
    fn unreadable_is_reported() {
        let t = fixture();
        let ix = index_of(t.path());
        std::fs::remove_file(t.join("good.png")).unwrap();
        let out = run(&BrokenFiles, &ix, &Default::default());
        assert_eq!(out.diagnostics.unreadable, 1);
    }
}
