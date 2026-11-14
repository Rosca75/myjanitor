//! Cold-folder candidates (OPT-ARC-01) and the sample-based savings estimate.
//!
//! A folder is cold when nothing in it was modified for `min_age_months`
//! (access times are not used: most systems mount with `relatime`/`noatime`
//! and Windows disables last-access updates by default, so they are not
//! evidence of use). Only the topmost cold folder of a subtree is proposed.
//!
//! The estimate never archives: files of already-compressed types count at
//! full size; the others are sampled (up to [`SAMPLES`] files, a few 64 KiB
//! blocks each, deflate level 1, see `fs_compression::probe_ratio`) and the
//! measured ratio is scaled to the archive's stronger level.

use std::path::Path;

use mj_core::{
    Candidate, Confidence, DetectContext, DetectorOutput, Estimate, FileEntry, FileIndex, OptimiserId, ParamValues,
    Result, TargetKind, Timestamp,
};

use crate::fs_compression::{probe_ratio, FSCOMPRESSION_EXTENSIONS};

/// Files sampled per folder.
pub const SAMPLES: usize = 12;
/// Best-level deflate relative to the level-1 probe.
const LEVEL_FACTOR: f64 = 0.9;
/// Header and directory overhead per archived entry (ZIP, roughly).
const PER_ENTRY: u64 = 120;
/// Ratio assumed when nothing could be sampled.
const FALLBACK_RATIO: f64 = 0.6;

fn is_compressed_type(path: &Path) -> bool {
    FSCOMPRESSION_EXTENSIONS.contains(&mj_core::pathutil::extension_lower(path).as_str())
}

/// A file to estimate: its path and size.
pub type FileRef<'a> = (&'a Path, u64);

/// Estimated archive size of a set of files `(path, size)`.
pub fn estimate_files(files: &[(&Path, u64)]) -> Estimate {
    let before: u64 = files.iter().map(|f| f.1).sum();
    let (packed, plain): (Vec<&FileRef>, Vec<&FileRef>) = files.iter().partition(|f| is_compressed_type(f.0));
    let packed_bytes: u64 = packed.iter().map(|f| f.1).sum();
    let plain_bytes: u64 = plain.iter().map(|f| f.1).sum();
    let step = (plain.len() / SAMPLES).max(1);
    let (mut raw, mut est) = (0f64, 0f64);
    for f in plain.iter().step_by(step).take(SAMPLES) {
        if let Ok(r) = probe_ratio(f.0) {
            // Data that barely compresses is stored as is by the writer.
            let r = if r >= 0.95 { 1.0 } else { r * LEVEL_FACTOR };
            raw += f.1 as f64;
            est += f.1 as f64 * r;
        }
    }
    let ratio = if raw > 0.0 { est / raw } else { FALLBACK_RATIO };
    let after = packed_bytes + (plain_bytes as f64 * ratio.min(1.0)) as u64 + files.len() as u64 * PER_ENTRY;
    Estimate {
        files: 1,
        bytes_before: before,
        bytes_after: after.min(before),
        confidence: Confidence::Estimated,
        skipped: 0,
    }
}

/// Estimate for a folder on disk (walk bounded to 20,000 entries, links not followed).
pub fn estimate_folder(path: &Path) -> Estimate {
    let mut files: Vec<(std::path::PathBuf, u64)> = Vec::new();
    let mut stack = vec![path.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(md) = std::fs::symlink_metadata(e.path()) else { continue };
            if md.is_dir() {
                stack.push(e.path());
            } else if md.is_file() {
                files.push((e.path(), md.len()));
            }
            if files.len() >= 20_000 {
                break;
            }
        }
    }
    files.sort();
    let refs: Vec<(&Path, u64)> = files.iter().map(|(p, s)| (p.as_path(), *s)).collect();
    estimate_files(&refs)
}

/// Topmost folders whose newest modification is older than the threshold.
pub fn cold_folders(index: &FileIndex, now: Timestamp, months: i64, min_bytes: u64) -> Vec<usize> {
    let cutoff = now.add_months(-months);
    let mut out = Vec::new();
    let mut i = 0;
    while i < index.len() {
        let e = index.entry(i);
        if e.is_dir() && e.depth > 0 {
            let t = index.totals(i);
            let old = t.newest_modified.is_some_and(|m| m < cutoff) && e.modified.is_none_or(|m| m < cutoff);
            if old && t.files > 0 && t.bytes >= min_bytes {
                out.push(i);
                i = index.subtree(i).end.max(i + 1);
                continue;
            }
        }
        i += 1;
    }
    out
}

/// The analysis of `ArchiveFolders`.
pub fn analyse(ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
    let months = ctx.params.number("min_age_months", 12.0).clamp(1.0, 600.0) as i64;
    let min_bytes = (ctx.params.number("min_size_mib", 10.0).max(0.0) * 1048576.0) as u64;
    let folders = cold_folders(ctx.index, ctx.now, months, min_bytes);
    let est = mj_core::pool::parallel_map(&folders, ctx.threads, ctx.cancel, |_, &i| {
        let files: Vec<&FileEntry> =
            ctx.index.entries()[index_range(ctx.index, i)].iter().filter(|e| e.is_file()).collect();
        let refs: Vec<(&Path, u64)> =
            files.iter().filter(|e| !e.is_online_only()).map(|e| (e.path.as_path(), e.size)).collect();
        (estimate_files(&refs), files.iter().any(|e| e.is_online_only()))
    });
    ctx.cancel.checkpoint()?;
    let mut out = DetectorOutput::default();
    for (&i, r) in folders.iter().zip(est) {
        let e = ctx.index.entry(i);
        let Some((est, online)) = r else {
            out.diagnostics.unreadable(&e.path);
            continue;
        };
        if online {
            // Archiving would download every placeholder.
            out.diagnostics.skip("online_only_files");
            continue;
        }
        if est.saving() == 0 {
            out.diagnostics.skip("not_smaller");
            continue;
        }
        let t = ctx.index.totals(i);
        let mut c: Candidate = crate::pdf::file_candidate(
            e,
            OptimiserId::ArchiveFolders,
            est.bytes_after,
            format!("Nothing modified for {months}+ months"),
        );
        c.target = TargetKind::Folder;
        c.size = est.bytes_before;
        c.file_count = Some(t.files);
        c.modified = t.newest_modified;
        out.candidates.push(c);
    }
    Ok(out)
}

fn index_range(index: &FileIndex, i: usize) -> std::ops::Range<usize> {
    index.subtree(i)
}

/// Level (0-9) of the `level` setting.
pub fn level_from(params: &ParamValues) -> u8 {
    match params.text("level", "best") {
        "stored" => 0,
        "fast" => 1,
        _ => 9,
    }
}
