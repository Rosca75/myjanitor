//! Safety inspection (SAF-07/09/10). **Owner: W2-E.**
//!
//! - path-only, cheap: synced-folder membership (`sync_root_of`), protected;
//! - content, bounded reads: signed (OOXML `_xmlsignatures/`, ODF
//!   `META-INF/documentsignatures.xml`, legacy Office `_signatures`, PDF
//!   `/ByteRange` and `/Type /Sig`), sensitivity-labelled (OOXML
//!   `docProps/custom.xml` and legacy `DocumentSummaryInformation` holding
//!   `MSIP_Label_*`), encrypted (CFB `EncryptionInfo` / `EncryptedPackage` /
//!   legacy flags, PDF `/Encrypt`, ZIP flag bit 0).
//!
//! When sync roots are unknown (detection not available), `sync_status_known`
//! is false and the engine shows a checklist warning instead of claiming
//! "nothing is synced".
//!
//! Every read is bounded: ZIP central directories are parsed from the file
//! (the reader caps sizes), `custom.xml` is read up to 1 MiB, PDFs are scanned
//! completely up to 64 MiB and as head + tail windows (4 MiB each) beyond.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use mj_core::{Badges, Error, FileEntry, Result, Settings};
use mj_formats::cfb::{CompoundFile, EntryType};
use mj_formats::zip::ZipArchive;
use mj_platform::{ActionPlatform, SyncRoot};

use crate::protected::ProtectedPaths;

/// Largest `custom.xml` / summary stream read for label detection.
const MAX_PROPS: u64 = 1 << 20;
/// PDFs up to this size are scanned completely.
const PDF_FULL_SCAN: u64 = 64 << 20;
/// Head and tail window for larger PDFs.
const PDF_WINDOW: u64 = 4 << 20;

/// Inspects candidates before they are offered.
#[derive(Debug, Clone)]
pub struct SafetyInspector {
    sync_roots: Option<Vec<SyncRoot>>,
    /// Roots supplied by the caller (Settings, tests); used whether or not detection worked.
    extra: Vec<SyncRoot>,
    protected: ProtectedPaths,
}

impl SafetyInspector {
    /// Build from the platform's sync roots (unknown when not detectable).
    pub fn new(platform: &dyn ActionPlatform, protected: ProtectedPaths) -> Self {
        SafetyInspector { sync_roots: platform.sync_roots().ok(), extra: Vec::new(), protected }
    }

    /// Build from explicit sync roots (`None` = unknown).
    pub fn with_sync_roots(sync_roots: Option<Vec<SyncRoot>>, protected: ProtectedPaths) -> Self {
        SafetyInspector { sync_roots, extra: Vec::new(), protected }
    }

    /// Add roots the user or the caller knows about (they count even when detection failed,
    /// but do not make the status "known" on their own).
    pub fn add_sync_roots(&mut self, extra: &[SyncRoot]) {
        self.extra.extend(extra.iter().cloned());
    }

    /// True when synced-folder detection worked.
    pub fn sync_status_known(&self) -> bool {
        self.sync_roots.is_some()
    }

    /// The sync root containing `path`.
    pub fn sync_root_of(&self, path: &Path) -> Option<&SyncRoot> {
        self.sync_roots
            .iter()
            .flatten()
            .chain(self.extra.iter())
            .filter(|r| mj_core::pathutil::is_within(&r.path, path))
            .max_by_key(|r| r.path.components().count())
    }

    /// Badges from metadata and path only (no I/O).
    pub fn path_badges(&self, entry: &FileEntry) -> Badges {
        let mut b = Badges::empty();
        if self.protected.is_protected(&entry.path) {
            b |= Badges::PROTECTED;
        }
        if self.sync_root_of(&entry.path).is_some() {
            b |= Badges::SYNCED;
        }
        if entry.is_online_only() {
            b |= Badges::ONLINE_ONLY;
        }
        if entry.attrs.contains(mj_core::Attributes::ENCRYPTED) {
            b |= Badges::ENCRYPTED;
        }
        b
    }

    /// Badges that need a bounded read of the file (signed, labelled, encrypted).
    pub fn content_badges(&self, path: &Path) -> Result<Badges> {
        content_badges(path)
    }
}

/// Content badges of one file (see the module docs). Unknown formats give no badge;
/// a malformed container of a known format gives no badge either (it cannot be acted
/// on by optimisers anyway, which validate their input), but an I/O error is returned.
pub fn content_badges(path: &Path) -> Result<Badges> {
    let mut f = File::open(path).map_err(|e| Error::io(path, e))?;
    let len = f.metadata().map_err(|e| Error::io(path, e))?.len();
    let mut head = [0u8; 1024];
    let n = read_up_to(&mut f, &mut head).map_err(|e| Error::io(path, e))?;
    let head = &head[..n];
    if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") {
        f.seek(SeekFrom::Start(0)).map_err(|e| Error::io(path, e))?;
        return Ok(zip_badges(BufReader::new(f)).unwrap_or(Badges::empty()));
    }
    if mj_formats::cfb::is_compound_file(head) {
        f.seek(SeekFrom::Start(0)).map_err(|e| Error::io(path, e))?;
        return Ok(cfb_badges(BufReader::new(f)).unwrap_or(Badges::empty()));
    }
    if head.windows(5).any(|w| w == b"%PDF-") {
        return pdf_badges(&mut f, len).map_err(|e| Error::io(path, e));
    }
    Ok(Badges::empty())
}

fn read_up_to(f: &mut File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        let k = f.read(&mut buf[n..])?;
        if k == 0 {
            break;
        }
        n += k;
    }
    Ok(n)
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
}

fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
}

/// ZIP-based documents: encrypted entries, signatures, sensitivity labels.
pub fn zip_badges<R: Read + Seek>(r: R) -> Result<Badges> {
    let mut z = ZipArchive::new(r)?;
    let mut b = Badges::empty();
    if z.any_encrypted() {
        b |= Badges::ENCRYPTED;
    }
    let names: Vec<String> = z.entries().iter().map(|e| e.name.clone()).collect();
    if names.iter().any(|n| {
        let l = n.to_ascii_lowercase();
        l.starts_with("_xmlsignatures/") || l == "meta-inf/documentsignatures.xml"
    }) {
        b |= Badges::SIGNED;
    }
    if let Some(i) = names.iter().position(|n| n.eq_ignore_ascii_case("docProps/custom.xml")) {
        if !z.entries()[i].is_encrypted() {
            if let Ok(xml) = z.read(i, MAX_PROPS) {
                if contains(&xml, b"MSIP_Label_") {
                    b |= Badges::LABELLED | Badges::CONFIDENTIAL;
                }
            }
        }
    }
    Ok(b)
}

/// Compound files: encryption / IRM, legacy signatures and labels.
pub fn cfb_badges<R: Read + Seek>(r: R) -> Result<Badges> {
    let mut cf = CompoundFile::open(r)?;
    let mut b = Badges::empty();
    if cf.protection()?.is_some() {
        b |= Badges::ENCRYPTED;
    }
    let roots: Vec<(String, EntryType)> = cf.root_entries()?.iter().map(|e| (e.name.clone(), e.kind)).collect();
    if roots.iter().any(|(n, _)| n.eq_ignore_ascii_case("_signatures") || n.eq_ignore_ascii_case("_xmlsignatures")) {
        b |= Badges::SIGNED;
    }
    if let Ok(Some(dsi)) = cf.read_stream("\u{5}DocumentSummaryInformation", MAX_PROPS) {
        if contains(&dsi, b"MSIP_Label_") || contains(&dsi, &utf16("MSIP_Label_")) {
            b |= Badges::LABELLED | Badges::CONFIDENTIAL;
        }
    }
    Ok(b)
}

/// PDFs: `/Encrypt` in a trailer or xref-stream dictionary; `/ByteRange` or a
/// `/Type /Sig` dictionary for signatures.
fn pdf_badges(f: &mut File, len: u64) -> std::io::Result<Badges> {
    let mut b = Badges::empty();
    let scan = |chunk: &[u8], b: &mut Badges| {
        if contains(chunk, b"/Encrypt") {
            *b |= Badges::ENCRYPTED;
        }
        if contains(chunk, b"/ByteRange") || sig_type(chunk) {
            *b |= Badges::SIGNED;
        }
    };
    let windows: Vec<(u64, u64)> =
        if len <= PDF_FULL_SCAN { vec![(0, len)] } else { vec![(0, PDF_WINDOW), (len - PDF_WINDOW, PDF_WINDOW)] };
    const CHUNK: usize = 1 << 20;
    const OVERLAP: usize = 64;
    for (start, size) in windows {
        f.seek(SeekFrom::Start(start))?;
        let mut left = size;
        let mut carry: Vec<u8> = Vec::new();
        let mut buf = vec![0u8; CHUNK];
        while left > 0 {
            let want = (left as usize).min(CHUNK);
            let n = read_up_to(f, &mut buf[..want])?;
            if n == 0 {
                break;
            }
            let mut joined = std::mem::take(&mut carry);
            joined.extend_from_slice(&buf[..n]);
            scan(&joined, &mut b);
            let keep = joined.len().min(OVERLAP);
            carry = joined[joined.len() - keep..].to_vec();
            left -= n as u64;
        }
    }
    Ok(b)
}

/// `/Type /Sig` with any whitespace (or none) between the two names, not `/Type /SigFieldLock`.
fn sig_type(chunk: &[u8]) -> bool {
    let needle = b"/Type";
    let mut i = 0;
    while let Some(p) = chunk[i..].windows(needle.len()).position(|w| w == needle) {
        let mut j = i + p + needle.len();
        while j < chunk.len() && matches!(chunk[j], b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | b'\0') {
            j += 1;
        }
        if chunk[j..].starts_with(b"/Sig") {
            let after = chunk.get(j + 4).copied();
            if !after.is_some_and(|c| c.is_ascii_alphanumeric()) {
                return true;
            }
        }
        i = i + p + 1;
    }
    false
}

/// Which content badges make the engine skip an item, according to the settings.
pub fn skip_badges(settings: &Settings) -> Badges {
    let mut b = Badges::empty();
    if settings.safety.skip_signed {
        b |= Badges::SIGNED;
    }
    if settings.safety.skip_labelled {
        b |= Badges::LABELLED;
    }
    if settings.safety.skip_encrypted {
        b |= Badges::ENCRYPTED;
    }
    b
}

/// True when MyJanitor would need elevation to act on `path`: the OS refused access and
/// the path is a protected system location on Windows. MyJanitor never elevates (SAF-05);
/// the report lists such paths so the user can decide. Always false on Linux.
pub fn needs_elevation(err: &Error, protected: &ProtectedPaths) -> bool {
    cfg!(windows) && matches!(err, Error::PermissionDenied(p) if protected.is_protected(p))
}
