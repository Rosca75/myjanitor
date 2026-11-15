//! Native ZIP archives through the in-house writer and reader.
//!
//! The writer compresses each entry in memory, so files above
//! [`MAX_ZIP_FILE`] make creation fail with a reason (7z or tar.zst have no
//! cap); nothing is ever left out silently. ZIP64 is handled by the writer
//! when an offset, a size or the entry count needs it.
//!
//! Every entry carries its modification time twice: the MS-DOS fields (UTC,
//! 2-second resolution, clamped to 1980-2107) and an extended-timestamp extra
//! field (0x5455, Unix seconds). On Unix the permission bits are stored too.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::Path;

use mj_core::hash::sha256;
use mj_core::{CancelToken, Error, Phase, Progress, ProgressSink, Result, Timestamp};
use mj_formats::zip::write::{Compression, EntryOptions, Prepared, MADE_BY_UNIX, MAX_LEVEL};
use mj_formats::zip::{ZipArchive, ZipWriter};

use super::index;
use super::walk::{Item, Kind};

/// Largest file stored in a native ZIP (held in memory with its compressed copy).
pub const MAX_ZIP_FILE: u64 = 512 * 1024 * 1024;

/// MS-DOS `(time, date)` of a UTC timestamp.
pub fn dos_datetime(t: Timestamp) -> (u16, u16) {
    let (y, mo, d) = t.ymd();
    let (h, mi, s) = t.hms();
    if y < 1980 {
        return (0, (1 << 5) | 1); // 1980-01-01 00:00:00
    }
    if y > 2107 {
        return ((23 << 11) | (59 << 5) | 29, (127 << 9) | (12 << 5) | 31);
    }
    let time = ((h as u16) << 11) | ((mi as u16) << 5) | (s as u16 / 2);
    let date = (((y - 1980) as u16) << 9) | ((mo as u16) << 5) | d as u16;
    (time, date)
}

/// Extended-timestamp extra field (`UT`, 0x5455) with the modification time only.
pub fn ut_extra(t: Timestamp) -> Vec<u8> {
    let secs = t.secs().clamp(i32::MIN as i64, i32::MAX as i64) as i32;
    let mut e = vec![0x55, 0x54, 5, 0, 1];
    e.extend_from_slice(&secs.to_le_bytes());
    e
}

fn options(it: &Item, name: &str) -> EntryOptions {
    let (t, d) = dos_datetime(it.mtime);
    let mut o = EntryOptions::new(name, t, d);
    let ut = ut_extra(it.mtime);
    o.local_extra = ut.clone();
    o.central_extra = ut;
    let dir = it.kind == Kind::Dir;
    let dos_attr = if dir { 0x10 } else { 0 };
    if let Some(mode) = it.mode {
        let ty = if dir { 0o040000 } else { 0o100000 };
        o.version_made_by = MADE_BY_UNIX;
        o.external_attrs = ((ty | mode) << 16) | dos_attr;
    } else {
        o.external_attrs = dos_attr;
    }
    o
}

/// Map an `ArchiveJob` level (0-9) to the writer: 0 stored, 9 the strongest deflate.
pub fn compression(level: u8) -> Compression {
    match level {
        0 => Compression::Stored,
        9.. => Compression::Best(MAX_LEVEL),
        l => Compression::Best(l),
    }
}

/// Write the archive (index first, then every directory and file). Returns its size.
/// `hashes` are the pass-one hashes; a file whose content differs when read
/// again is "changed during archiving" and fails the whole archive.
#[allow(clippy::too_many_arguments)]
pub fn create(
    items: &[Item],
    hashes: &[Option<String>],
    index_bytes: &[u8],
    index_name: &str,
    dst: &Path,
    level: u8,
    cancel: &CancelToken,
    progress: &dyn ProgressSink,
) -> Result<u64> {
    if let Some(big) = items.iter().find(|i| i.size() > MAX_ZIP_FILE) {
        return Err(Error::unsupported(
            "ZIP archive",
            format!("{} is larger than 512 MB; use 7z or tar.zst for folders with very large files", big.abs.display()),
        ));
    }
    let f = std::fs::OpenOptions::new().write(true).create_new(true).open(dst).map_err(|e| Error::io(dst, e))?;
    let mut w = ZipWriter::new(BufWriter::new(f));
    let c = compression(level);
    let now = Timestamp::now();
    let idx_item =
        Item { rel: index_name.into(), abs: dst.into(), kind: Kind::File { size: 0 }, mtime: now, mode: Some(0o644) };
    w.add_prepared(&options(&idx_item, index_name), &Prepared::new(index_bytes, c))?;
    let total: u64 = items.iter().map(Item::size).sum();
    let mut done = 0u64;
    for (it, h) in items.iter().zip(hashes) {
        cancel.checkpoint()?;
        match &it.kind {
            Kind::Link { .. } => continue,
            Kind::Dir => {
                let name = format!("{}/", it.rel);
                w.add_prepared(&options(it, &name), &Prepared::new(&[], Compression::Stored))?;
            }
            Kind::File { size } => {
                let data = std::fs::read(&it.abs).map_err(|e| Error::io(&it.abs, e))?;
                if data.len() as u64 != *size || h.as_deref() != Some(mj_core::hash::to_hex(&sha256(&data)).as_str()) {
                    return Err(Error::Changed(it.abs.clone()));
                }
                w.add_prepared(&options(it, &it.rel), &Prepared::new(&data, c))?;
                done += size;
                let mut p = Progress::new(Phase::Applying, done, Some(total));
                p.bytes = done;
                progress.report(&p);
            }
        }
    }
    let mut bw = w.finish()?;
    bw.flush().map_err(|e| Error::io(dst, e))?;
    let file = bw.into_inner().map_err(|e| Error::io(dst, e.into_error()))?;
    file.sync_all().map_err(|e| Error::io(dst, e))?;
    Ok(file.metadata().map_err(|e| Error::io(dst, e))?.len())
}

fn mismatch(msg: impl Into<String>) -> Error {
    Error::parse("archive integrity test", msg)
}

/// OPT-ARC-02: every directory and file of `items` is in the archive with the
/// same size and content (CRC-32 checked by the reader, SHA-256 compared with
/// the source as read now), the index matches, and nothing else is inside.
pub fn verify(archive: &Path, items: &[Item], fresh_hashes: &[Option<String>], cancel: &CancelToken) -> Result<()> {
    let f = File::open(archive).map_err(|e| Error::io(archive, e))?;
    let mut z = ZipArchive::new(BufReader::new(f))?;
    let mut by_name: HashMap<String, usize> = HashMap::new();
    for (i, e) in z.entries().iter().enumerate() {
        if by_name.insert(e.name.clone(), i).is_some() {
            return Err(mismatch(format!("duplicate entry {}", e.name)));
        }
    }
    let idx_name = index::index_name(items);
    let mut expected = 1usize;
    let idx = *by_name.get(&idx_name).ok_or_else(|| mismatch("index entry missing"))?;
    if idx != 0 {
        return Err(mismatch("index is not the first entry"));
    }
    if z.read(idx, 1 << 30)? != index::build(items, fresh_hashes) {
        return Err(mismatch("index does not match the sources"));
    }
    for (it, h) in items.iter().zip(fresh_hashes) {
        cancel.checkpoint()?;
        match &it.kind {
            Kind::Link { .. } => {
                if by_name.contains_key(&it.rel) {
                    return Err(mismatch(format!("link {} was archived", it.rel)));
                }
            }
            Kind::Dir => {
                expected += 1;
                by_name.get(&format!("{}/", it.rel)).ok_or_else(|| mismatch(format!("folder {} missing", it.rel)))?;
            }
            Kind::File { size } => {
                expected += 1;
                let i = *by_name.get(&it.rel).ok_or_else(|| mismatch(format!("{} missing", it.rel)))?;
                if z.entries()[i].uncompressed_size != *size {
                    return Err(mismatch(format!("{}: size differs", it.rel)));
                }
                let data = z.read(i, MAX_ZIP_FILE)?;
                if Some(mj_core::hash::to_hex(&sha256(&data))) != *h {
                    return Err(mismatch(format!("{}: content differs from the source", it.rel)));
                }
            }
        }
    }
    if z.entries().len() != expected {
        return Err(mismatch(format!("{} entries, expected {expected}", z.entries().len())));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dos_times() {
        let t = Timestamp::from_ymd_hms(2024, 3, 9, 13, 45, 31);
        let (time, date) = dos_datetime(t);
        assert_eq!(time, (13 << 11) | (45 << 5) | 15);
        assert_eq!(date, (44 << 9) | (3 << 5) | 9);
        assert_eq!(dos_datetime(Timestamp(0)), (0, 33));
        assert_eq!(ut_extra(Timestamp::from_secs(16)), vec![0x55, 0x54, 5, 0, 1, 16, 0, 0, 0]);
        assert_eq!(compression(0), Compression::Stored);
        assert_eq!(compression(9), Compression::Best(MAX_LEVEL));
    }
}
