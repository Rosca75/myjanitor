//! Generic ZIP-based repack at maximum deflate (OPT-OFC-01/03): EPUB, CBZ, JAR, ODF.
//! Owner: W3-A.
//!
//! Every deflate entry is recompressed at miniz level 10 and the smaller of the old
//! and the new form is kept per entry (a stored entry may become deflated when that
//! helps); entry order, names, timestamps, attributes, extra fields and comments are
//! kept. Special rules:
//!
//! - EPUB / ODF: a first entry named `mimetype` stays first and stored (OCF / ODF);
//! - OOXML packages are left to the Office optimisers (`[Content_Types].xml` order
//!   is preserved there by the same writer);
//! - JAR: v1 signatures (`META-INF/*.SF|RSA|DSA|EC`) sign entry *content* digests
//!   and the manifest bytes, which a recompression does not change, so signed JARs
//!   are repacked; APKs are never repacked (v2+ signatures cover the whole file and
//!   `zipalign` alignment of stored entries would be lost);
//! - encrypted archives or entries, signed ODF documents, archives with data outside
//!   their entries (prefixes, gaps, APK signing blocks, overlaps) and unsupported
//!   compression methods are skipped with the reason.
//!
//! After writing, the output is verified: same entry list in the same order, every
//! entry decodes with the CRC-32 and size of the original (or of its intended
//! replacement, for the Office optimisers that share [`repack_file`]).

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufWriter, Write};
use std::path::Path;

use mj_core::{
    ActionKind, Availability, CancelToken, Category, Confidence, DetectContext, DetectorOutput, Error, Estimate,
    ExternalTool, FileEntry, OptimiseJob, OptimiseOutput, Optimiser, OptimiserId, OptimiserInfo, ParamValues,
    PlatformSupport, Result, SafetyClass,
};
use mj_formats::sniff::FileKind;
use mj_formats::zip::write::repack::{self, Caps, EntryRule, Expected, SourceArchive};
use mj_formats::zip::write::MAX_LEVEL;
use mj_formats::zip::{ZipArchive, ZipEntry, ZipWriter, METHOD_DEFLATE, METHOD_STORED};

use crate::framework::{self, MinGain};

/// ZIP-based files: Repack EPUB, CBZ, JAR and ODF files losslessly
#[derive(Debug, Default, Clone, Copy)]
pub struct ZipRepack;

/// Extensions (lower case) this optimiser targets.
pub const ZIPREPACK_EXTENSIONS: &[&str] = &["epub", "cbz", "jar", "apk", "odt", "ods", "odp"];

/// Extensions whose content is already compressed (deflate gains nothing worth estimating).
const COMPRESSED_EXTS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "avif", "heic", "mp3", "mp4", "m4a", "m4v", "mov", "ogg", "opus", "webm",
    "zip", "gz", "7z", "rar", "xz", "zst", "woff", "woff2", "jar", "apk", "docx", "xlsx", "pptx", "odt", "ods", "odp",
    "jxl", "wmv", "wma", "flac", "class",
];

/// Changes a repack applies on top of recompression (used by the Office optimisers).
#[derive(Debug, Clone, Default)]
pub struct RepackPlan {
    /// New content for entries, by name.
    pub replace: BTreeMap<String, Vec<u8>>,
    /// Entries left out of the output.
    pub remove: BTreeSet<String>,
    /// Entries always written stored.
    pub stored: BTreeSet<String>,
}

/// What [`repack_file`] wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repacked {
    pub bytes_after: u64,
    pub entries: usize,
}

fn mime_first(e: &[ZipEntry]) -> bool {
    e.first().is_some_and(|x| x.name == "mimetype")
}

/// Write `src` repacked (and changed by `plan`) to `dst`, then verify it. On any
/// error `dst` is removed. Cancel is checked between entries.
pub fn repack_file(src: &Path, dst: &Path, plan: &RepackPlan, caps: Caps, cancel: &CancelToken) -> Result<Repacked> {
    let res = repack_inner(src, dst, plan, caps, cancel);
    if res.is_err() {
        let _ = std::fs::remove_file(dst);
    }
    res
}

fn repack_inner(src: &Path, dst: &Path, plan: &RepackPlan, caps: Caps, cancel: &CancelToken) -> Result<Repacked> {
    let mut s = SourceArchive::open_file(src, caps)?;
    let f = std::fs::OpenOptions::new().write(true).create_new(true).open(dst).map_err(|e| Error::io(dst, e))?;
    let mut w = ZipWriter::new(BufWriter::new(f));
    w.set_comment(&s.info.comment)?;
    let entries = s.entries().to_vec();
    let mut expected = Vec::with_capacity(entries.len());
    for (i, e) in entries.iter().enumerate() {
        cancel.checkpoint()?;
        if plan.remove.contains(&e.name) {
            continue;
        }
        if e.method != METHOD_STORED && e.method != METHOD_DEFLATE {
            return Err(Error::unsupported("ZIP repack", format!("compression method {} in {}", e.method, e.name)));
        }
        let rule = if plan.stored.contains(&e.name) { EntryRule::Stored } else { EntryRule::Best };
        let data = match plan.replace.get(&e.name) {
            Some(d) => d.clone(),
            None => s.read(i)?,
        };
        let out = repack::copy_entry(&mut w, &mut s, i, &data, rule, MAX_LEVEL)?;
        expected.push(Expected { name: e.name.clone(), crc32: out.crc32, size: out.uncompressed_size });
    }
    let mut bw = w.finish()?;
    bw.flush().map_err(|e| Error::io(dst, e))?;
    let file = bw.into_inner().map_err(|e| Error::io(dst, e.into_error()))?;
    file.sync_all().map_err(|e| Error::io(dst, e))?;
    drop(file);
    let r = std::fs::File::open(dst).map_err(|e| Error::io(dst, e))?;
    repack::verify_entries(std::io::BufReader::new(r), &expected, caps)
        .map_err(|e| Error::parse("repacked archive", format!("verification failed: {e}")))?;
    let bytes_after = std::fs::metadata(dst).map_err(|e| Error::io(dst, e))?.len();
    Ok(Repacked { bytes_after, entries: expected.len() })
}

/// Why an archive must not be repacked, from its directory alone.
fn refusal(path: &Path, entries: &[ZipEntry], apk_block: bool) -> Option<String> {
    let names: Vec<String> = entries.iter().map(|e| e.name.to_ascii_lowercase()).collect();
    let has = |n: &str| names.iter().any(|x| x == n);
    if entries.iter().any(ZipEntry::is_encrypted) {
        return Some("encrypted archive".into());
    }
    if mj_core::pathutil::extension_lower(path) == "apk"
        || apk_block
        || (has("androidmanifest.xml") && has("classes.dex"))
    {
        return Some("APK files are not repacked: their signature and alignment cover the whole file".into());
    }
    if has("[content_types].xml") {
        return Some("Office documents are handled by the Office optimisers".into());
    }
    if has("meta-inf/documentsignatures.xml")
        || has("meta-inf/macrosignatures.xml")
        || names.iter().any(|n| n.starts_with("_xmlsignatures/"))
    {
        return Some("digitally signed document".into());
    }
    if let Some(e) = entries.iter().find(|e| e.method != METHOD_STORED && e.method != METHOD_DEFLATE) {
        return Some(format!("unsupported compression method {} in {}", e.method, e.name));
    }
    None
}

fn skipped(before: u64, reason: impl Into<String>) -> OptimiseOutput {
    OptimiseOutput {
        bytes_before: before,
        bytes_after: before,
        lossy: false,
        skipped: Some(reason.into()),
        notes: Vec::new(),
    }
}

fn accepted_kind(kind: Option<FileKind>, ext: &str) -> bool {
    match kind {
        Some(FileKind::Epub | FileKind::Odf | FileKind::Jar) => true,
        Some(FileKind::Zip) => matches!(ext, "cbz" | "apk" | "jar"),
        _ => false,
    }
}

/// Estimated stored size of one entry after a level-10 repack (a model, not a measurement).
fn entry_estimate(e: &ZipEntry) -> u64 {
    let ext = e.name.rsplit_once('.').map(|(_, x)| x.to_ascii_lowercase()).unwrap_or_default();
    if e.is_dir() || COMPRESSED_EXTS.contains(&ext.as_str()) || e.name == "mimetype" {
        return e.compressed_size;
    }
    match e.method {
        // Text stored uncompressed typically deflates to well under half.
        METHOD_STORED if e.uncompressed_size > 256 => e.uncompressed_size / 3,
        // Re-deflating at the strongest level typically wins a few percent.
        METHOD_DEFLATE => e.compressed_size - e.compressed_size / 25,
        _ => e.compressed_size,
    }
}

impl Optimiser for ZipRepack {
    fn info(&self) -> OptimiserInfo {
        OptimiserInfo {
            id: OptimiserId::ZipRepack,
            title: "ZIP-based files".into(),
            description: "Repack EPUB, CBZ, JAR and ODF files losslessly".into(),
            category: Category::Optimise,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Optimise,
            available_actions: vec![ActionKind::Optimise],
            presets: Vec::new(),
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            tools: vec![],
            feature_ids: vec!["OPT-OFC-01".into(), "OPT-OFC-03".into()],
            unit: "files".into(),
        }
    }

    fn availability(&self, tools: &[ExternalTool]) -> Availability {
        let _ = tools;
        Availability::Available
    }

    fn applies_to(&self, entry: &FileEntry) -> bool {
        entry.is_file() && ZIPREPACK_EXTENSIONS.contains(&entry.extension().as_str())
    }

    fn estimate(&self, entry: &FileEntry, params: &ParamValues) -> Result<Estimate> {
        let _ = params;
        let f = std::fs::File::open(&entry.path).map_err(|e| Error::io(&entry.path, e))?;
        let mut r = std::io::BufReader::new(f);
        let apk = mj_formats::zip::write::source::archive_info(&mut r).map(|i| i.apk_signing_block).unwrap_or(false);
        let z = ZipArchive::new(r)?;
        let mut est = Estimate {
            files: 1,
            bytes_before: entry.size,
            bytes_after: entry.size,
            confidence: Confidence::Estimated,
            skipped: 0,
        };
        if refusal(&entry.path, z.entries(), apk).is_some() {
            est.skipped = 1;
            return Ok(est);
        }
        let saving: u64 = z.entries().iter().map(|e| e.compressed_size.saturating_sub(entry_estimate(e))).sum();
        est.bytes_after = entry.size.saturating_sub(saving);
        if !MinGain::default().accepts(est.bytes_before, est.bytes_after) {
            est.bytes_after = est.bytes_before;
        }
        Ok(est)
    }

    fn optimise(&self, job: &OptimiseJob<'_>) -> Result<OptimiseOutput> {
        let before = std::fs::metadata(job.src).map_err(|e| Error::io(job.src, e))?.len();
        let ext = mj_core::pathutil::extension_lower(job.src);
        let kind = framework::content_kind(job.src)?;
        if !accepted_kind(kind, &ext) {
            let what = kind.map(|k| k.label()).unwrap_or("unrecognised content");
            return Ok(skipped(before, format!("content is {what}, not an EPUB, CBZ, JAR or ODF file")));
        }
        let mut f = std::io::BufReader::new(std::fs::File::open(job.src).map_err(|e| Error::io(job.src, e))?);
        let info = match mj_formats::zip::write::source::archive_info(&mut f) {
            Ok(i) => i,
            Err(e) => return Ok(skipped(before, format!("not a readable ZIP archive: {e}"))),
        };
        let z = match ZipArchive::new(f) {
            Ok(z) => z,
            Err(e) => return Ok(skipped(before, format!("not a readable ZIP archive: {e}"))),
        };
        if let Some(r) = refusal(job.src, z.entries(), info.apk_signing_block) {
            return Ok(skipped(before, r));
        }
        let mut plan = RepackPlan::default();
        if mime_first(z.entries()) && matches!(kind, Some(FileKind::Epub | FileKind::Odf)) {
            plan.stored.insert("mimetype".into());
        }
        drop(z);
        let out = match repack_file(job.src, job.dst, &plan, Caps::default(), job.cancel) {
            Ok(o) => o,
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(e @ (Error::Unsupported { .. } | Error::Parse { .. })) => {
                return Ok(skipped(before, format!("not repacked: {e}")))
            }
            Err(e) => return Err(e),
        };
        if let Some(reason) = MinGain::default().reject_reason(before, out.bytes_after) {
            let _ = std::fs::remove_file(job.dst);
            return Ok(skipped(before, reason));
        }
        Ok(OptimiseOutput {
            bytes_before: before,
            bytes_after: out.bytes_after,
            lossy: false,
            skipped: None,
            notes: vec![format!("{} entries recompressed at the maximum level", out.entries)],
        })
    }

    fn analyse(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        framework::analyse_by_content(
            self,
            ctx,
            &[FileKind::Epub, FileKind::Odf, FileKind::Jar, FileKind::Zip],
            &[],
            |_, _| "Can be repacked losslessly at the maximum compression level".into(),
            |_| Default::default(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::testutil::TempDir;
    use mj_core::NullProgress;

    fn build(entries: &[(&str, &[u8], u8)]) -> Vec<u8> {
        let mut w = ZipWriter::new(Vec::new());
        for (n, d, l) in entries {
            w.add(n, d, *l, 0x6000, 0x5821).unwrap();
        }
        w.set_comment(b"kept comment").unwrap();
        w.finish().unwrap()
    }

    fn run(src: &Path, dst: &Path) -> Result<OptimiseOutput> {
        let cancel = CancelToken::new();
        let params = ParamValues::new();
        let tmp = dst.parent().unwrap().to_path_buf();
        ZipRepack.optimise(&OptimiseJob {
            src,
            dst,
            params: &params,
            tools: &[],
            temp_dir: &tmp,
            cancel: &cancel,
            progress: &NullProgress,
        })
    }

    fn xhtml() -> Vec<u8> {
        "<html><body><p>Lorem ipsum dolor sit amet, consectetur.</p></body></html>\n".repeat(400).into_bytes()
    }

    #[test]
    fn epub_repack_keeps_crcs_order_and_mimetype_rule() {
        let t = TempDir::new("epub").unwrap();
        let body = xhtml();
        let src = t
            .write(
                "book.epub",
                &build(&[
                    ("mimetype", b"application/epub+zip", 0),
                    ("META-INF/container.xml", b"<container/>", 1),
                    ("OEBPS/ch1.xhtml", &body, 1),
                    ("OEBPS/ch2.xhtml", &body, 0),
                ]),
            )
            .unwrap();
        let dst = t.join("out.epub");
        let o = run(&src, &dst).unwrap();
        assert!(o.skipped.is_none(), "{o:?}");
        assert!(o.bytes_after < o.bytes_before);
        let a = ZipArchive::new(std::io::Cursor::new(std::fs::read(&src).unwrap())).unwrap();
        let mut b = ZipArchive::new(std::io::Cursor::new(std::fs::read(&dst).unwrap())).unwrap();
        let names = |z: &ZipArchive<_>| {
            z.entries()
                .iter()
                .map(|e: &ZipEntry| (e.name.clone(), e.crc32, e.uncompressed_size, e.dos_time))
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&a), names(&b));
        assert_eq!(b.entries()[0].method, METHOD_STORED);
        assert_eq!(b.entries()[3].method, METHOD_DEFLATE, "stored text becomes deflated");
        assert_eq!(b.read(2, 1 << 20).unwrap(), body);
        let info =
            mj_formats::zip::write::source::archive_info(&mut std::io::Cursor::new(std::fs::read(&dst).unwrap()))
                .unwrap();
        assert_eq!(info.comment, b"kept comment");
        if let Ok(out) = std::process::Command::new("unzip").arg("-t").arg(&dst).output() {
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stdout));
        }
    }

    #[test]
    fn signed_jar_is_repacked_apk_and_signed_odf_are_not() {
        let t = TempDir::new("jar").unwrap();
        let class = xhtml();
        let jar = build(&[
            ("META-INF/MANIFEST.MF", b"Manifest-Version: 1.0\r\n", 1),
            ("META-INF/CERT.SF", b"sig", 1),
            ("META-INF/CERT.RSA", b"rsa", 0),
            ("a/B.txt", &class, 0),
        ]);
        let src = t.write("lib.jar", &jar).unwrap();
        let o = run(&src, &t.join("o1.jar")).unwrap();
        assert!(o.skipped.is_none(), "{o:?}");
        let src = t.write("app.apk", &jar).unwrap();
        let o = run(&src, &t.join("o2.apk")).unwrap();
        assert!(o.skipped.unwrap().contains("APK"));
        assert!(!t.join("o2.apk").exists());
        let odf = build(&[
            ("mimetype", b"application/vnd.oasis.opendocument.text", 0),
            ("content.xml", &class, 1),
            ("META-INF/manifest.xml", b"<m/>", 1),
            ("META-INF/documentsignatures.xml", b"<s/>", 1),
        ]);
        let src = t.write("signed.odt", &odf).unwrap();
        assert!(run(&src, &t.join("o3.odt")).unwrap().skipped.unwrap().contains("signed"));
    }

    #[test]
    fn negative_cases_write_nothing() {
        let t = TempDir::new("neg").unwrap();
        // Already maximal: not smaller.
        let src = t.write("tight.cbz", &build(&[("p1.jpg", &mj_formats_noise(5000), 0)])).unwrap();
        let o = run(&src, &t.join("o.cbz")).unwrap();
        assert!(o.skipped.unwrap().contains("not smaller"));
        assert!(!t.join("o.cbz").exists());
        // Not a ZIP at all.
        let src = t.write("fake.epub", b"plain text").unwrap();
        assert!(run(&src, &t.join("o2.epub")).unwrap().skipped.is_some());
        // Data hidden after the entries.
        let mut z = build(&[("mimetype", b"application/epub+zip", 0), ("META-INF/container.xml", &xhtml(), 1)]);
        let a = ZipArchive::new(std::io::Cursor::new(z.clone())).unwrap();
        let cd = mj_formats::zip::write::source::archive_info(&mut std::io::Cursor::new(z.clone())).unwrap().cd_offset
            as usize;
        drop(a);
        let mut hidden = z[..cd].to_vec();
        hidden.extend_from_slice(b"SECRET PAYLOAD");
        let shift = 14u32;
        let mut tail = z[cd..].to_vec();
        let eocd = tail.len() - 22 - 12;
        let off = u32::from_le_bytes([tail[eocd + 16], tail[eocd + 17], tail[eocd + 18], tail[eocd + 19]]) + shift;
        tail[eocd + 16..eocd + 20].copy_from_slice(&off.to_le_bytes());
        hidden.extend_from_slice(&tail);
        z = hidden;
        let src = t.write("hidden.epub", &z).unwrap();
        let o = run(&src, &t.join("o3.epub")).unwrap();
        assert!(o.skipped.unwrap().contains("not repacked"), "hidden data must block the repack");
        // OOXML goes to the Office optimisers.
        let src = t
            .write("x.jar", &build(&[("[Content_Types].xml", b"<Types/>", 1), ("META-INF/MANIFEST.MF", b"x", 1)]))
            .unwrap();
        assert!(run(&src, &t.join("o4.jar")).unwrap().skipped.is_some());
    }

    #[test]
    fn cancel_removes_partial_output() {
        let t = TempDir::new("zc").unwrap();
        let src =
            t.write("a.epub", &build(&[("mimetype", b"application/epub+zip", 0), ("x.xhtml", &xhtml(), 1)])).unwrap();
        let cancel = CancelToken::new();
        cancel.cancel();
        let dst = t.join("o.epub");
        assert_eq!(repack_file(&src, &dst, &RepackPlan::default(), Caps::default(), &cancel), Err(Error::Cancelled));
        assert!(!dst.exists());
    }

    #[test]
    fn estimate_is_cheap_and_honest() {
        let t = TempDir::new("ze").unwrap();
        let p =
            t.write("a.epub", &build(&[("mimetype", b"application/epub+zip", 0), ("x.xhtml", &xhtml(), 0)])).unwrap();
        let e = FileEntry::new(&p, mj_core::EntryKind::File, std::fs::metadata(&p).unwrap().len());
        let est = ZipRepack.estimate(&e, &ParamValues::new()).unwrap();
        assert_eq!(est.confidence, Confidence::Estimated);
        assert!(est.saving() > 0);
        let p = t.write("b.apk", &build(&[("x.xhtml", &xhtml(), 0)])).unwrap();
        let e = FileEntry::new(&p, mj_core::EntryKind::File, 10);
        assert_eq!(ZipRepack.estimate(&e, &ParamValues::new()).unwrap().skipped, 1);
    }

    fn mj_formats_noise(n: usize) -> Vec<u8> {
        let mut x = 0x2545_F491_4F6C_DD1Du64;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x >> 24) as u8
            })
            .collect()
    }
}
