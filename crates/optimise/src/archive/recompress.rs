//! OPT-ARC-03: recompress existing ZIP archives with the strongest deflate,
//! through `zip_repack::repack_file` (not duplicated here), then an extra
//! check of our own: the same entry names in the same order and the same
//! SHA-256 of every entry's content.
//!
//! ZIP -> 7z is not offered: an `Optimise` result replaces the original under
//! its own name, so a 7z file would end up named `.zip`.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use mj_core::hash::sha256;
use mj_core::{CancelToken, Confidence, Error, Estimate, FileEntry, OptimiseOutput, Result};
use mj_formats::sniff::FileKind;
use mj_formats::zip::write::repack::Caps;
use mj_formats::zip::{ZipArchive, METHOD_DEFLATE, METHOD_STORED};

use crate::zip_repack::{repack_file, RepackPlan};

fn skipped(before: u64, reason: impl Into<String>) -> OptimiseOutput {
    OptimiseOutput {
        bytes_before: before,
        bytes_after: before,
        lossy: false,
        skipped: Some(reason.into()),
        notes: Vec::new(),
        ..Default::default()
    }
}

fn open(path: &Path) -> Result<ZipArchive<BufReader<File>>> {
    ZipArchive::new(BufReader::new(File::open(path).map_err(|e| Error::io(path, e))?))
}

/// Names and content hashes of every entry, in stored order.
pub fn content_digest(path: &Path, caps: Caps, cancel: &CancelToken) -> Result<Vec<(String, [u8; 32])>> {
    let mut z = open(path)?;
    if z.entries().len() as u64 > caps.max_entries {
        return Err(Error::unsupported("ZIP recompression", "too many entries"));
    }
    let names: Vec<String> = z.entries().iter().map(|e| e.name.clone()).collect();
    let mut out = Vec::with_capacity(names.len());
    for (i, n) in names.into_iter().enumerate() {
        cancel.checkpoint()?;
        out.push((n, sha256(&z.read(i, caps.max_entry)?)));
    }
    Ok(out)
}

/// Recompress `src` into `dst`. Refusals and failures are skips (nothing at `dst`).
pub fn recompress(src: &Path, dst: &Path, cancel: &CancelToken) -> Result<OptimiseOutput> {
    let before = std::fs::metadata(src).map_err(|e| Error::io(src, e))?.len();
    match crate::framework::content_kind(src)? {
        Some(FileKind::Zip) => {}
        _ => return Ok(skipped(before, "Not a plain ZIP archive (documents and packages have their own optimisers)")),
    }
    let z = open(src)?;
    if z.any_encrypted() {
        return Ok(skipped(before, "Encrypted archive: left unchanged"));
    }
    let names: Vec<String> = z.entries().iter().map(|e| e.name.to_ascii_lowercase()).collect();
    if names.iter().any(|n| n.starts_with("meta-inf/") && (n.ends_with(".sf") || n.ends_with(".rsa"))) {
        return Ok(skipped(before, "Signed archive: left unchanged"));
    }
    drop(z);
    let caps = Caps::default();
    let repacked = match repack_file(src, dst, &RepackPlan::default(), caps, cancel) {
        Ok(r) => r,
        Err(Error::Cancelled) => return Err(Error::Cancelled),
        Err(e) => return Ok(skipped(before, format!("Not recompressed: {}", e.info().message))),
    };
    let same = content_digest(src, caps, cancel).and_then(|a| content_digest(dst, caps, cancel).map(|b| a == b));
    match same {
        Ok(true) => {}
        Ok(false) => {
            let _ = std::fs::remove_file(dst);
            return Ok(skipped(before, "Not recompressed: verification found different content"));
        }
        Err(e) => {
            let _ = std::fs::remove_file(dst);
            if e == Error::Cancelled {
                return Err(e);
            }
            return Ok(skipped(before, format!("Not recompressed: verification failed ({})", e.info().message)));
        }
    }
    if repacked.bytes_after >= before {
        let _ = std::fs::remove_file(dst);
        return Ok(skipped(before, "Already compact: recompression would not be smaller"));
    }
    Ok(OptimiseOutput {
        bytes_before: before,
        bytes_after: repacked.bytes_after,
        lossy: false,
        skipped: None,
        notes: vec![format!("{} entries recompressed and verified", repacked.entries)],
        ..Default::default()
    })
}

/// Model estimate from the central directory (no decompression).
pub fn estimate(entry: &FileEntry) -> Estimate {
    let before = entry.size;
    let mut est =
        Estimate { files: 1, bytes_before: before, bytes_after: before, confidence: Confidence::Estimated, skipped: 0 };
    let Ok(z) = open(&entry.path) else {
        est.skipped = 1;
        return est;
    };
    if z.any_encrypted() {
        est.skipped = 1;
        return est;
    }
    let mut gain = 0u64;
    for e in z.entries() {
        let ext = e.name.rsplit_once('.').map(|(_, x)| x.to_ascii_lowercase()).unwrap_or_default();
        if e.is_dir() || crate::fs_compression::FSCOMPRESSION_EXTENSIONS.contains(&ext.as_str()) {
            continue;
        }
        gain += match e.method {
            METHOD_STORED if e.uncompressed_size > 256 => e.uncompressed_size - e.uncompressed_size / 3,
            METHOD_DEFLATE => e.compressed_size / 25,
            _ => 0,
        };
    }
    est.bytes_after = before.saturating_sub(gain);
    est
}
