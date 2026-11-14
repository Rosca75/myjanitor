//! Archiving (OPT-ARC-01..05): cold folders to ZIP (in-house) or 7z/zstd (external), integrity test before removing sources; recompress existing archives.
//! Owner: W3-B.
//!
//! The action engine runs `Action::Archive` as `Archiver::create` then
//! `Archiver::verify`, and removes the sources only when `verify` returns
//! `Ok` (and deletes the archive otherwise). So:
//!
//! - `create` never leaves anything out silently: a file it cannot store
//!   (too large for native ZIP, special file, unrepresentable name, changed
//!   while archiving) fails the whole archive. Links are never followed;
//!   they are listed in the index as "not archived".
//! - `verify` re-walks and re-reads the sources and checks every entry
//!   against them (OPT-ARC-02). It is not bypassable: there is no option.
//! - On any failure or cancellation the partial archive is removed here too.
//!
//! Every archive starts with `INDEX.csv` (path, size, mtime, SHA-256) and gets
//! a sidecar `<archive>.index.csv` (OPT-ARC-04).

mod ext;
mod folders;
mod index;
mod recompress;
mod tar;
mod walk;
mod zipfmt;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use mj_core::params::ChoiceOption;
use mj_core::{
    ActionKind, ArchiveFormat, ArchiveJob, Availability, CancelToken, Category, DetectContext, DetectorOutput, Error,
    Estimate, ExternalTool, FileEntry, OptimiseJob, OptimiseOutput, Optimiser, OptimiserId, OptimiserInfo, ParamValues,
    PlatformSupport, Result, SafetyClass, SettingKind, SettingSpec,
};

pub use ext::{seven_zip_add_args, zstd_compress_args};
pub use folders::{estimate_files, level_from};
pub use index::{sidecar_path, INDEX_NAME};
pub use zipfmt::MAX_ZIP_FILE;

use walk::{Item, Kind};

/// Why encryption is not offered.
pub const ENCRYPTION_REASON: &str =
    "Not available: 7z only accepts a password on its command line, where other programs can read it";
/// Why split volumes are not offered.
pub const SPLIT_REASON: &str = "Not available yet: the archive step has no volume size setting";

/// Archive cold folders: Folders not used for months, packed into one archive
#[derive(Debug, Default, Clone, Copy)]
pub struct ArchiveFolders;

/// Extensions (lower case) this optimiser targets (folders: none).
pub const ARCHIVEFOLDERS_EXTENSIONS: &[&str] = &[];

fn choice(key: &str, label: &str, default: &str, options: &[(&str, &str, &str)]) -> SettingSpec {
    SettingSpec {
        key: key.into(),
        label: label.into(),
        help: None,
        kind: SettingKind::Choice {
            default: default.into(),
            options: options
                .iter()
                .map(|(v, l, h)| ChoiceOption { value: (*v).into(), label: (*l).into(), help: Some((*h).into()) })
                .collect(),
        },
        advanced: false,
        lossy: false,
        disabled_reason: None,
    }
}

fn disabled_toggle(key: &str, label: &str, reason: &str) -> SettingSpec {
    SettingSpec {
        key: key.into(),
        label: label.into(),
        help: None,
        kind: SettingKind::Toggle { default: false },
        advanced: true,
        lossy: false,
        disabled_reason: Some(reason.into()),
    }
}

fn number(key: &str, label: &str, default: f64, max: f64, unit: &str) -> SettingSpec {
    SettingSpec {
        key: key.into(),
        label: label.into(),
        help: None,
        kind: SettingKind::Number { default, min: 0.0, max, step: 1.0, unit: Some(unit.into()) },
        advanced: false,
        lossy: false,
        disabled_reason: None,
    }
}

/// Step settings of [`ArchiveFolders`].
pub fn folder_settings(tools: &[ExternalTool]) -> Vec<SettingSpec> {
    let mut format = choice(
        "format",
        "Format",
        "zip",
        &[
            ("zip", "ZIP", "Opens everywhere; files up to 512 MB each"),
            ("7z", "7z", "Smaller, solid; requires 7z"),
            ("tar_zst", "tar.zst", "Fast and strong; requires zstd"),
        ],
    );
    let missing: Vec<&str> = ["7z", "zstd"].into_iter().filter(|t| mj_core::tools::find(tools, t).is_none()).collect();
    if !missing.is_empty() {
        format.help = Some(format!("Requires {} on PATH for the other formats", missing.join(" and ")));
    }
    vec![
        format,
        choice(
            "level",
            "Compression",
            "best",
            &[
                ("stored", "Stored", "No compression"),
                ("fast", "Fast", "Quick, larger"),
                ("best", "Best", "Slowest, smallest"),
            ],
        ),
        number("min_age_months", "Not modified for at least", 12.0, 600.0, "months"),
        number("min_size_mib", "Minimum folder size", 10.0, 1_048_576.0, "MiB"),
        disabled_toggle("encrypt", "Encrypt (AES-256)", ENCRYPTION_REASON),
        disabled_toggle("split", "Split into volumes", SPLIT_REASON),
    ]
}

impl Optimiser for ArchiveFolders {
    fn info(&self) -> OptimiserInfo {
        OptimiserInfo {
            id: OptimiserId::ArchiveFolders,
            title: "Archive cold folders".into(),
            description: "Folders not used for months, packed into one archive".into(),
            category: Category::Archive,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Archive,
            available_actions: vec![ActionKind::Archive],
            presets: Vec::new(),
            settings: folder_settings(&[]),
            platform: PlatformSupport::Both,
            tools: vec![],
            feature_ids: vec!["OPT-ARC-01".into(), "OPT-ARC-02".into(), "OPT-ARC-04".into(), "OPT-ARC-05".into()],
            unit: "folders".into(),
        }
    }

    /// ZIP is in-house, so the card is always available.
    fn availability(&self, tools: &[ExternalTool]) -> Availability {
        let _ = tools;
        Availability::Available
    }

    fn applies_to(&self, entry: &FileEntry) -> bool {
        entry.is_dir()
    }

    /// Sample-based: never archives (see `folders`).
    fn estimate(&self, entry: &FileEntry, params: &ParamValues) -> Result<Estimate> {
        let _ = params;
        Ok(folders::estimate_folder(&entry.path))
    }

    fn optimise(&self, job: &OptimiseJob<'_>) -> Result<OptimiseOutput> {
        let _ = job;
        Err(Error::InvalidInput("Folders are archived by the Archive action (Archiver::create + verify)".into()))
    }

    fn analyse(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        folders::analyse(ctx)
    }
}

/// Recompress archives: Existing archives repacked with a stronger setting
#[derive(Debug, Default, Clone, Copy)]
pub struct ArchiveRecompress;

/// Extensions (lower case) this optimiser targets.
pub const ARCHIVERECOMPRESS_EXTENSIONS: &[&str] = &["zip"];

/// Why ZIP -> 7z is not offered.
pub const TO_7Z_REASON: &str =
    "Not available: a recompressed file keeps its name, so it cannot change to the 7z format";

impl Optimiser for ArchiveRecompress {
    fn info(&self) -> OptimiserInfo {
        OptimiserInfo {
            id: OptimiserId::ArchiveRecompress,
            title: "Recompress archives".into(),
            description: "Existing archives repacked with a stronger setting".into(),
            category: Category::Archive,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Optimise,
            available_actions: vec![ActionKind::Optimise],
            presets: Vec::new(),
            settings: vec![disabled_toggle("to_7z", "Convert to solid 7z", TO_7Z_REASON)],
            platform: PlatformSupport::Both,
            tools: vec![],
            feature_ids: vec!["OPT-ARC-03".into()],
            unit: "files".into(),
        }
    }

    fn availability(&self, tools: &[ExternalTool]) -> Availability {
        let _ = tools;
        Availability::Available
    }

    fn applies_to(&self, entry: &FileEntry) -> bool {
        entry.is_file() && ARCHIVERECOMPRESS_EXTENSIONS.contains(&entry.extension().as_str())
    }

    fn estimate(&self, entry: &FileEntry, params: &ParamValues) -> Result<Estimate> {
        let _ = params;
        Ok(recompress::estimate(entry))
    }

    fn optimise(&self, job: &OptimiseJob<'_>) -> Result<OptimiseOutput> {
        job.cancel.checkpoint()?;
        recompress::recompress(job.src, job.dst, job.cancel)
    }

    fn analyse(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let entries: Vec<&FileEntry> =
            ctx.index.files().filter(|e| self.applies_to(e) && !e.is_online_only()).collect();
        let est = mj_core::pool::parallel_map(&entries, ctx.threads, ctx.cancel, |_, e| recompress::estimate(e));
        ctx.cancel.checkpoint()?;
        let mut out = DetectorOutput::default();
        for (e, r) in entries.iter().zip(est) {
            match r {
                None => out.diagnostics.unreadable(&e.path),
                Some(est) if est.skipped > 0 => out.diagnostics.skip("encrypted_or_unreadable"),
                Some(est) if est.saving() == 0 => out.diagnostics.skip("not_smaller"),
                Some(est) => out.candidates.push(crate::pdf::file_candidate(
                    e,
                    OptimiserId::ArchiveRecompress,
                    est.bytes_after,
                    "Repack with the strongest deflate".into(),
                )),
            }
        }
        Ok(out)
    }
}

/// ZIP (in-house) and 7z / tar+zstd (external) archive writer and verifier.
#[derive(Debug, Default, Clone, Copy)]
pub struct Zip7Archiver;

/// What `create` found: the items, their hashes and the index.
struct Prepared {
    items: Vec<Item>,
    hashes: Vec<Option<String>>,
    index_bytes: Vec<u8>,
    index_name: String,
}

fn prepare(sources: &[PathBuf], base: &Path, cancel: &CancelToken) -> Result<Prepared> {
    if sources.is_empty() {
        return Err(Error::InvalidInput("nothing to archive".into()));
    }
    let items = walk::collect(sources, base, &|| cancel.checkpoint())?;
    let hashes = index::hash_all(&items, cancel)?;
    let index_bytes = index::build(&items, &hashes);
    let index_name = index::index_name(&items);
    Ok(Prepared { items, hashes, index_bytes, index_name })
}

/// Container format from the first bytes of a file.
fn sniff_format(path: &Path) -> Result<ArchiveFormat> {
    use std::io::Read;
    let mut head = [0u8; 6];
    let mut f = std::fs::File::open(path).map_err(|e| Error::io(path, e))?;
    let n = f.read(&mut head).map_err(|e| Error::io(path, e))?;
    match &head[..n] {
        [b'P', b'K', 3, 4, ..] | [b'P', b'K', 5, 6, ..] => Ok(ArchiveFormat::Zip),
        [b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C] => Ok(ArchiveFormat::SevenZip),
        [0x28, 0xB5, 0x2F, 0xFD, ..] => Ok(ArchiveFormat::TarZstd),
        _ => Err(Error::parse("archive integrity test", "not a ZIP, 7z or zstd file")),
    }
}

/// `verify` has no tool list in the contract: find the tool by the same detection.
fn locate_tool(id: &str) -> Result<PathBuf> {
    let tools = crate::tools::detect(&Default::default());
    crate::tools::require(&tools, id).map(Path::to_path_buf)
}

/// Compare a tar listing with the sources (index first, every folder and file, nothing else).
fn check_tar(entries: &[tar::TarEntry], p: &Prepared) -> Result<()> {
    let bad = |m: String| Error::parse("archive integrity test", m);
    let first = entries.first().ok_or_else(|| bad("empty archive".into()))?;
    if first.name != p.index_name || first.data.as_deref() != Some(&p.index_bytes[..]) {
        return Err(bad("index missing or different from the sources".into()));
    }
    let mut by_name = std::collections::HashMap::new();
    for e in &entries[1..] {
        if by_name.insert(e.name.as_str(), e).is_some() {
            return Err(bad(format!("duplicate entry {}", e.name)));
        }
    }
    let mut expected = 0;
    for (it, h) in p.items.iter().zip(&p.hashes) {
        match &it.kind {
            Kind::Link { .. } => {}
            Kind::Dir => {
                expected += 1;
                let e = by_name
                    .get(format!("{}/", it.rel).as_str())
                    .ok_or_else(|| bad(format!("folder {} missing", it.rel)))?;
                if !e.is_dir {
                    return Err(bad(format!("{} is not a folder", it.rel)));
                }
            }
            Kind::File { size } => {
                expected += 1;
                let e = by_name.get(it.rel.as_str()).ok_or_else(|| bad(format!("{} missing", it.rel)))?;
                if e.size != *size || Some(&e.sha256) != h.as_ref() {
                    return Err(bad(format!("{}: content differs from the source", it.rel)));
                }
            }
        }
    }
    if by_name.len() != expected {
        return Err(bad(format!("{} entries, expected {expected}", by_name.len())));
    }
    Ok(())
}

/// Write the sidecar index beside the archive (never over an existing file).
fn write_sidecar(dst: &Path, bytes: &[u8]) -> Option<PathBuf> {
    use std::io::Write;
    let p = index::sidecar_path(dst);
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&p).ok()?;
    if f.write_all(bytes).and_then(|_| f.sync_all()).is_err() {
        drop(f);
        let _ = std::fs::remove_file(&p);
        return None;
    }
    Some(p)
}

/// Remove our sidecar after a failed verification (only when it is ours: same bytes).
fn remove_sidecar_if(archive: &Path, bytes: Option<&[u8]>) {
    let p = index::sidecar_path(archive);
    if let (Ok(have), Some(want)) = (std::fs::read(&p), bytes) {
        if have == want {
            let _ = std::fs::remove_file(&p);
        }
    }
}

impl Zip7Archiver {
    fn create_inner(&self, job: &ArchiveJob<'_>, p: &Prepared) -> Result<u64> {
        match job.format {
            ArchiveFormat::Zip => zipfmt::create(
                &p.items,
                &p.hashes,
                &p.index_bytes,
                &p.index_name,
                job.dst,
                job.level,
                job.cancel,
                job.progress,
            ),
            ArchiveFormat::SevenZip => {
                let tool = crate::tools::require(job.tools, "7z")?;
                ext::create_7z(
                    tool,
                    &p.items,
                    &p.index_bytes,
                    &p.index_name,
                    job.base,
                    job.dst,
                    job.level,
                    job.cancel,
                )?;
                Ok(std::fs::metadata(job.dst).map_err(|e| Error::io(job.dst, e))?.len())
            }
            ArchiveFormat::TarZstd => {
                let tool = crate::tools::require(job.tools, "zstd")?;
                let partial = ext::partial_tar(job.dst);
                let r =
                    tar::create(&p.items, &p.hashes, &p.index_bytes, &p.index_name, &partial, job.cancel, job.progress);
                if let Err(e) = r {
                    let _ = std::fs::remove_file(&partial);
                    return Err(e);
                }
                ext::zstd_compress(tool, &partial, job.dst, job.level, job.cancel)?;
                Ok(std::fs::metadata(job.dst).map_err(|e| Error::io(job.dst, e))?.len())
            }
        }
    }

    fn verify_inner(&self, archive: &Path, p: &Prepared, cancel: &CancelToken) -> Result<()> {
        match sniff_format(archive)? {
            ArchiveFormat::Zip => zipfmt::verify(archive, &p.items, &p.hashes, cancel),
            ArchiveFormat::SevenZip => {
                ext::verify_7z(&locate_tool("7z")?, archive, &p.items, &p.index_bytes, &p.index_name, cancel)
            }
            ArchiveFormat::TarZstd => {
                let tarp = ext::zstd_decompress(&locate_tool("zstd")?, archive, cancel)?;
                let r = tar::read(&tarp, &p.index_name, cancel).and_then(|e| check_tar(&e, p));
                let _ = std::fs::remove_file(&tarp);
                r
            }
        }
    }
}

impl mj_core::Archiver for Zip7Archiver {
    fn formats(&self, tools: &[ExternalTool]) -> Vec<ArchiveFormat> {
        let mut f = vec![ArchiveFormat::Zip];
        if mj_core::tools::find(tools, "7z").is_some() {
            f.push(ArchiveFormat::SevenZip);
        }
        if mj_core::tools::find(tools, "zstd").is_some() {
            f.push(ArchiveFormat::TarZstd);
        }
        f
    }

    fn create(&self, job: &ArchiveJob<'_>) -> Result<u64> {
        if job.dst.exists() {
            return Err(Error::Conflict(format!("{} already exists", job.dst.display())));
        }
        let res = prepare(job.sources, job.base, job.cancel).and_then(|p| self.create_inner(job, &p).map(|n| (n, p)));
        match res {
            Ok((n, p)) => {
                write_sidecar(job.dst, &p.index_bytes);
                Ok(n)
            }
            Err(e) => {
                let _ = std::fs::remove_file(job.dst);
                let _ = std::fs::remove_file(ext::partial_tar(job.dst));
                Err(e)
            }
        }
    }

    fn verify(&self, archive: &Path, sources: &[PathBuf], base: &Path, cancel: &CancelToken) -> Result<()> {
        let p = match prepare(sources, base, cancel) {
            Ok(p) => p,
            Err(e) => {
                remove_sidecar_if(archive, None);
                return Err(e);
            }
        };
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.verify_inner(archive, &p, cancel)))
            .unwrap_or_else(|_| Err(Error::Internal("archive verification panicked".into())));
        if r.is_err() {
            remove_sidecar_if(archive, Some(&p.index_bytes));
        }
        r
    }
}

/// Links that an archive of `sources` would not contain (for reports).
pub fn skipped_links(sources: &[PathBuf], base: &Path) -> Result<Vec<PathBuf>> {
    let items = walk::collect(sources, base, &|| Ok(()))?;
    Ok(index::links(&items).into_iter().map(|i| i.abs.clone()).collect())
}
