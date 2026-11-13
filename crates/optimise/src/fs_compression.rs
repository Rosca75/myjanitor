//! Transparent filesystem compression (OPT-FSC-01..06): WOF on NTFS, Btrfs zstd on Linux; skip poorly compressible files; DirectStorage warning.
//! Owner: W3-B.
//!
//! This optimiser does not transform `src -> dst`: compression is applied in
//! place, reversibly, by `Action::FsCompress` / `Action::FsDecompress`
//! through `ActionPlatform` (`mj_platform::action::fs_compress`). Here we
//! only find candidates and estimate: cold files, not already compressed,
//! on a supporting volume, whose sampled blocks actually compress.
//!
//! OPT-FSC-03 (background watcher) is not built: visible but disabled.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use mj_core::params::ChoiceOption;
use mj_core::{
    ActionKind, Availability, Category, Confidence, DetectContext, DetectorOutput, Error, Estimate, ExternalTool,
    FileEntry, FsCompressAlgo, OptimiseJob, OptimiseOutput, Optimiser, OptimiserId, OptimiserInfo, ParamValues,
    PlatformSupport, Result, SafetyClass, SettingKind, SettingSpec,
};
use mj_formats::pdf::filter::deflate_raw;
use mj_platform::action::fs_compress as platform;

/// Transparent compression: Compress cold folders, files stay usable
#[derive(Debug, Default, Clone, Copy)]
pub struct FsCompression;

/// Extensions (lower case) of formats that are already compressed (OPT-FSC-02):
/// skipped without reading when "skip poorly compressible files" is on.
pub const FSCOMPRESSION_EXTENSIONS: &[&str] = &[
    "7z", "aac", "apk", "avi", "avif", "br", "bz2", "cab", "cbz", "docx", "dng", "epub", "flac", "gif", "gz", "heic",
    "heif", "jar", "jpeg", "jpg", "jxl", "lz", "lz4", "lzma", "m4a", "m4v", "mkv", "mov", "mp3", "mp4", "msi", "odp",
    "ods", "odt", "ogg", "opus", "png", "pptx", "rar", "tgz", "txz", "webm", "webp", "wma", "wmv", "xlsx", "xz", "zip",
    "zst",
];

/// Sampled block size and count of the compressibility probe.
pub const PROBE_BLOCK: usize = 64 * 1024;
pub const PROBE_BLOCKS: u64 = 4;
/// Skip a file whose sampled blocks compress to more than this fraction.
pub const SKIP_RATIO: f64 = 0.9;
/// Allocation unit assumed for rounding (NTFS and Btrfs default).
pub const CLUSTER: u64 = 4096;

/// The OPT-FSC-04 warning.
pub const DIRECTSTORAGE_WARNING: &str =
    "Game folder: games that stream assets with DirectStorage can load slower from compressed files";

/// Why the background watcher is disabled (OPT-FSC-03).
pub const WATCHER_REASON: &str = "Not built yet: re-run this step after large updates instead";

/// Default algorithm for this platform.
pub fn default_algorithm() -> FsCompressAlgo {
    if cfg!(windows) {
        FsCompressAlgo::Xpress8k
    } else {
        FsCompressAlgo::BtrfsZstd
    }
}

/// Wire value of an algorithm (the step parameter `algorithm`).
pub fn algo_key(a: FsCompressAlgo) -> &'static str {
    match a {
        FsCompressAlgo::Xpress4k => "xpress4k",
        FsCompressAlgo::Xpress8k => "xpress8k",
        FsCompressAlgo::Xpress16k => "xpress16k",
        FsCompressAlgo::Lzx => "lzx",
        FsCompressAlgo::BtrfsZstd => "btrfs_zstd",
    }
}

/// Parse the `algorithm` parameter; unknown values take the platform default.
pub fn algorithm_from(params: &ParamValues) -> FsCompressAlgo {
    match params.text("algorithm", "") {
        "xpress4k" => FsCompressAlgo::Xpress4k,
        "xpress8k" => FsCompressAlgo::Xpress8k,
        "xpress16k" => FsCompressAlgo::Xpress16k,
        "lzx" => FsCompressAlgo::Lzx,
        "btrfs_zstd" => FsCompressAlgo::BtrfsZstd,
        _ => default_algorithm(),
    }
}

/// Size after compression relative to the deflate-level-1 probe (measured
/// orders of magnitude: XPRESS is weaker than deflate, LZX and zstd similar).
pub fn algo_factor(a: FsCompressAlgo) -> f64 {
    match a {
        FsCompressAlgo::Xpress4k => 1.15,
        FsCompressAlgo::Xpress8k => 1.10,
        FsCompressAlgo::Xpress16k => 1.07,
        FsCompressAlgo::Lzx => 0.97,
        FsCompressAlgo::BtrfsZstd => 1.0,
    }
}

/// OPT-FSC-04: the warning for a path inside a known game library.
pub fn directstorage_warning(path: &Path) -> Option<&'static str> {
    let markers = ["steamapps", "epic games", "xboxgames", "gog galaxy", "gog games", "windowsapps"];
    path.components()
        .any(|c| {
            let s = c.as_os_str().to_string_lossy().to_ascii_lowercase();
            markers.contains(&s.as_str())
        })
        .then_some(DIRECTSTORAGE_WARNING)
}

/// Compressed fraction of up to [`PROBE_BLOCKS`] evenly spaced blocks (deflate level 1).
pub fn probe_ratio(path: &Path) -> std::io::Result<f64> {
    let mut f = File::open(path)?;
    let len = f.metadata()?.len();
    if len == 0 {
        return Ok(1.0);
    }
    let blocks = PROBE_BLOCKS.min(len.div_ceil(PROBE_BLOCK as u64));
    let mut raw = 0usize;
    let mut packed = 0usize;
    let mut buf = vec![0u8; PROBE_BLOCK];
    for i in 0..blocks {
        let off = if blocks == 1 { 0 } else { (len - (PROBE_BLOCK as u64).min(len)) * i / (blocks - 1) };
        f.seek(SeekFrom::Start(off))?;
        let n = read_full(&mut f, &mut buf)?;
        if n == 0 {
            break;
        }
        raw += n;
        packed += deflate_raw(&buf[..n], 1).len();
    }
    Ok(if raw == 0 { 1.0 } else { packed as f64 / raw as f64 })
}

fn read_full(f: &mut File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match f.read(&mut buf[n..])? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
}

/// Files smaller than this are never proposed (they fit in a cluster or two).
fn min_size(params: &ParamValues) -> u64 {
    (params.number("min_size_kib", 16.0).max(1.0) as u64) * 1024
}

/// Estimate one file; the second value is the skip reason code, if skipped.
pub fn estimate_file(entry: &FileEntry, params: &ParamValues) -> (Estimate, Option<&'static str>) {
    let before = entry.size;
    let skip = |code| {
        (
            Estimate {
                files: 1,
                bytes_before: before,
                bytes_after: before,
                confidence: Confidence::Estimated,
                skipped: 1,
            },
            Some(code),
        )
    };
    if entry.is_online_only() {
        return skip("online_only");
    }
    let skip_poor = params.bool("skip_incompressible", true);
    if skip_poor && FSCOMPRESSION_EXTENSIONS.contains(&entry.extension().as_str()) {
        return skip("already_compressed_format");
    }
    let ratio = match probe_ratio(&entry.path) {
        Ok(r) => r,
        Err(_) => return skip("unreadable"),
    };
    if skip_poor && ratio > SKIP_RATIO {
        return skip("poorly_compressible");
    }
    let algo = algorithm_from(params);
    let after = ((before as f64 * (ratio * algo_factor(algo)).min(1.0)) as u64).div_ceil(CLUSTER) * CLUSTER;
    (
        Estimate {
            files: 1,
            bytes_before: before,
            bytes_after: after.min(before),
            confidence: Confidence::Estimated,
            skipped: 0,
        },
        None,
    )
}

fn settings() -> Vec<SettingSpec> {
    let opt = |v: &str, l: &str, h: &str| ChoiceOption { value: v.into(), label: l.into(), help: Some(h.into()) };
    let options = if cfg!(windows) {
        vec![
            opt("xpress4k", "XPRESS 4K", "Fastest, smallest saving"),
            opt("xpress8k", "XPRESS 8K", "Balanced (default)"),
            opt("xpress16k", "XPRESS 16K", "Better ratio, a little slower"),
            opt("lzx", "LZX", "Best ratio, slowest; for rarely used files"),
        ]
    } else {
        vec![opt("btrfs_zstd", "Btrfs zstd", "Btrfs compression property, existing data rewritten")]
    };
    let number = |key: &str, label: &str, default: f64, max: f64, unit: &str| SettingSpec {
        key: key.into(),
        label: label.into(),
        help: None,
        kind: SettingKind::Number { default, min: 0.0, max, step: 1.0, unit: Some(unit.into()) },
        advanced: true,
        lossy: false,
        disabled_reason: None,
    };
    vec![
        SettingSpec {
            key: "algorithm".into(),
            label: "Algorithm".into(),
            help: None,
            kind: SettingKind::Choice { default: algo_key(default_algorithm()).into(), options },
            advanced: false,
            lossy: false,
            disabled_reason: None,
        },
        SettingSpec {
            key: "skip_incompressible".into(),
            label: "Skip poorly compressible files".into(),
            help: Some("Already compressed formats, and files whose samples shrink by less than 10%".into()),
            kind: SettingKind::Toggle { default: true },
            advanced: false,
            lossy: false,
            disabled_reason: None,
        },
        number("min_age_days", "Not modified for at least", 90.0, 3650.0, "days"),
        number("min_size_kib", "Minimum file size", 16.0, 1_048_576.0, "KiB"),
        SettingSpec {
            key: "watcher".into(),
            label: "Recompress after updates in the background".into(),
            help: None,
            kind: SettingKind::Toggle { default: false },
            advanced: true,
            lossy: false,
            disabled_reason: Some(WATCHER_REASON.into()),
        },
    ]
}

impl Optimiser for FsCompression {
    fn info(&self) -> OptimiserInfo {
        OptimiserInfo {
            id: OptimiserId::FsCompression,
            title: "Transparent compression".into(),
            description: "Compress cold folders, files stay usable".into(),
            category: Category::Optimise,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::FsCompress,
            available_actions: vec![ActionKind::FsCompress, ActionKind::FsDecompress],
            presets: Vec::new(),
            settings: settings(),
            platform: PlatformSupport::Both,
            tools: vec![],
            feature_ids: vec![
                "OPT-FSC-01".into(),
                "OPT-FSC-02".into(),
                "OPT-FSC-04".into(),
                "OPT-FSC-05".into(),
                "OPT-FSC-06".into(),
            ],
            // Candidates are files: the action works per file.
            unit: "files".into(),
        }
    }

    fn availability(&self, tools: &[ExternalTool]) -> Availability {
        let _ = tools;
        match platform::any_volume_supported() {
            Ok(()) => Availability::Available,
            Err(reason) => Availability::Disabled { reason },
        }
    }

    /// Regular local files of at least 4 KiB (the per-step minimum size and
    /// age are applied in `analyse`, which has the parameters).
    fn applies_to(&self, entry: &FileEntry) -> bool {
        entry.is_file() && !entry.is_online_only() && entry.size >= CLUSTER
    }

    fn estimate(&self, entry: &FileEntry, params: &ParamValues) -> Result<Estimate> {
        Ok(estimate_file(entry, params).0)
    }

    fn optimise(&self, job: &OptimiseJob<'_>) -> Result<OptimiseOutput> {
        let _ = job;
        Err(Error::InvalidInput(
            "Transparent compression acts in place through the FsCompress action; it has no src -> dst transform"
                .into(),
        ))
    }

    fn analyse(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let min_age_ms = (ctx.params.number("min_age_days", 90.0).max(0.0) * 86_400_000.0) as i64;
        let min = min_size(ctx.params);
        let mut out = DetectorOutput::default();
        let mut entries: Vec<&FileEntry> = Vec::new();
        for e in ctx.index.files().filter(|e| self.applies_to(e) && e.size >= min) {
            match e.modified {
                Some(m) if ctx.now.0.saturating_sub(m.0) < min_age_ms => out.diagnostics.skip("recently_modified"),
                _ => entries.push(e),
            }
        }
        let results = mj_core::pool::parallel_map(&entries, ctx.threads, ctx.cancel, |_, e| {
            platform::support(&e.path)?;
            if matches!(platform::state(&e.path), Ok(Some(_))) {
                return Ok((Estimate::default(), Some("already_compressed")));
            }
            Ok(estimate_file(e, ctx.params))
        });
        ctx.cancel.checkpoint()?;
        let mut unsupported: Option<String> = None;
        let mut warned = false;
        for (e, r) in entries.iter().zip(results) {
            match r {
                None | Some(Ok((_, Some("unreadable")))) => out.diagnostics.unreadable(&e.path),
                Some(Err(reason)) => {
                    out.diagnostics.skip("unsupported_volume");
                    unsupported.get_or_insert(reason);
                }
                Some(Ok((_, Some(code)))) => out.diagnostics.skip(code),
                Some(Ok((est, None))) if est.saving() == 0 => out.diagnostics.skip("not_smaller"),
                Some(Ok((est, None))) => {
                    let pct = est.saving() * 100 / est.bytes_before.max(1);
                    let mut c = crate::pdf::file_candidate(
                        e,
                        OptimiserId::FsCompression,
                        est.bytes_after,
                        format!("Compresses to about {}% of its size", 100 - pct),
                    );
                    if let Some(w) = directstorage_warning(&e.path) {
                        c.columns.insert("warning".into(), w.into());
                        c.sub_group = Some("game_folder".into());
                        c.default_selected = false;
                        warned = true;
                    }
                    out.candidates.push(c);
                }
            }
        }
        if let Some(reason) = unsupported {
            out.diagnostics.notes.push(format!("Some files were not considered: {reason}"));
        }
        if warned {
            out.diagnostics.notes.push(DIRECTSTORAGE_WARNING.into());
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::testutil::TempDir;
    use mj_core::{
        CancelToken, EntryKind, FileIndex, NoFingerprintStore, NullProgress, ParamValue, ScanStats, Timestamp,
    };

    fn entry(p: &Path) -> FileEntry {
        let mut e = FileEntry::new(p.to_path_buf(), EntryKind::File, std::fs::metadata(p).unwrap().len());
        e.modified = Some(Timestamp(0));
        e
    }

    fn noise(n: usize) -> Vec<u8> {
        let mut s = 0x9E37_79B9_7F4A_7C15u64;
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                s as u8
            })
            .collect()
    }

    #[test]
    fn probe_and_skip_rules() {
        let t = TempDir::new("fsc-probe").unwrap();
        let text = t.write("log.txt", &b"2026-01-01 INFO service started on port 8080\n".repeat(20_000)).unwrap();
        let random = t.write("blob.bin", &noise(300_000)).unwrap();
        let jpg = t.write("photo.jpg", &b"x".repeat(100_000)).unwrap();
        assert!(probe_ratio(&text).unwrap() < 0.2);
        assert!(probe_ratio(&random).unwrap() > 0.99);

        let p = ParamValues::new();
        let (est, why) = estimate_file(&entry(&text), &p);
        assert_eq!(why, None);
        assert!(est.bytes_after < est.bytes_before / 3 && est.bytes_after % CLUSTER == 0);
        assert_eq!(estimate_file(&entry(&random), &p).1, Some("poorly_compressible"));
        assert_eq!(estimate_file(&entry(&jpg), &p).1, Some("already_compressed_format"));
        // With the toggle off, the extension table is not used.
        let off = ParamValues::new().with("skip_incompressible", ParamValue::Bool(false));
        assert_eq!(estimate_file(&entry(&jpg), &off).1, None);
        // LZX is estimated smaller than XPRESS 4K.
        let lzx = estimate_file(&entry(&text), &ParamValues::new().with("algorithm", ParamValue::Text("lzx".into()))).0;
        let x4 =
            estimate_file(&entry(&text), &ParamValues::new().with("algorithm", ParamValue::Text("xpress4k".into()))).0;
        assert!(lzx.bytes_after <= x4.bytes_after);
    }

    #[test]
    fn directstorage_warning_on_game_folders() {
        assert!(directstorage_warning(Path::new("/d/SteamLibrary/steamapps/common/Game/data.pak")).is_some());
        assert!(directstorage_warning(Path::new("/d/Epic Games/Fortnite/x.pak")).is_some());
        assert!(directstorage_warning(Path::new("/d/XboxGames/Halo/y")).is_some());
        assert!(directstorage_warning(Path::new("/home/me/Documents/steam-notes.txt")).is_none());
    }

    #[test]
    fn analyse_on_the_session_filesystem() {
        let t = TempDir::new("fsc-analyse").unwrap();
        let a = t.write("old/report.txt", &b"quarterly figures, quarterly figures\n".repeat(5_000)).unwrap();
        let game = t.write("steamapps/common/G/level.dat", &b"level geometry level geometry\n".repeat(5_000)).unwrap();
        let mut recent = entry(&t.write("new.txt", &b"fresh fresh fresh\n".repeat(5_000)).unwrap());
        recent.modified = Some(Timestamp(400 * 86_400_000));
        let index = FileIndex::new(
            vec![t.path().to_path_buf()],
            vec![entry(&a), entry(&game), recent],
            ScanStats::default(),
            Timestamp(0),
        );
        let cancel = CancelToken::new();
        let params = ParamValues::new();
        let ctx = DetectContext {
            index: &index,
            params: &params,
            now: Timestamp(400 * 86_400_000),
            fingerprints: &NoFingerprintStore,
            cancel: &cancel,
            progress: &NullProgress,
            threads: 2,
        };
        let out = FsCompression.analyse(&ctx).unwrap();
        assert_eq!(out.diagnostics.skipped.get("recently_modified"), Some(&1));
        match platform::support(&a) {
            Ok(()) => {
                assert_eq!(out.candidates.len(), 2);
                let g = out.candidates.iter().find(|c| c.path == game).unwrap();
                assert!(!g.default_selected && g.columns.contains_key("warning"));
            }
            Err(reason) => {
                // Not Btrfs/NTFS here: nothing proposed, and the reason is reported, never "nothing found".
                assert!(out.candidates.is_empty());
                assert_eq!(out.diagnostics.skipped.get("unsupported_volume"), Some(&2));
                assert!(out.diagnostics.notes.iter().any(|n| n.contains(&reason)));
            }
        }
    }

    #[test]
    fn availability_and_contract() {
        match FsCompression.availability(&[]) {
            Availability::Available => assert!(platform::any_volume_supported().is_ok()),
            Availability::Disabled { reason } => assert!(!reason.is_empty()),
        }
        let info = FsCompression.info();
        let watcher = info.settings.iter().find(|s| s.key == "watcher").unwrap();
        assert_eq!(watcher.disabled_reason.as_deref(), Some(WATCHER_REASON));
        assert_eq!(algorithm_from(&ParamValues::new()), default_algorithm());
        let t = TempDir::new("fsc-job").unwrap();
        let src = t.write("a", b"x").unwrap();
        let cancel = CancelToken::new();
        let params = ParamValues::new();
        let job = OptimiseJob {
            src: &src,
            dst: &t.join("b"),
            params: &params,
            tools: &[],
            temp_dir: t.path(),
            cancel: &cancel,
            progress: &NullProgress,
        };
        assert!(FsCompression.optimise(&job).is_err());
    }
}
