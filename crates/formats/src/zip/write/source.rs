//! What a lossless repack needs from the source archive beyond
//! [`crate::zip::ZipArchive`]: local headers (raw name bytes, local extra
//! fields), the raw compressed bytes of an entry, the archive comment and the
//! data layout (no prefix, no hidden data between or after entries, no
//! overlapping entries). Everything is bounds-checked against the real stream
//! length; nothing panics on malformed input. Owner: W3-A.

use std::io::{Read, Seek, SeekFrom};

use mj_core::{Error, Result};

use crate::zip::ZipEntry;

const SIG_LOCAL: u32 = 0x0403_4b50;
const SIG_EOCD: u32 = 0x0605_4b50;
const SIG_ZIP64_EOCD: u32 = 0x0606_4b50;
const SIG_ZIP64_LOCATOR: u32 = 0x0706_4b50;
/// Magic at the end of an APK Signing Block (v2+ signatures), just before the central directory.
pub const APK_SIG_MAGIC: &[u8; 16] = b"APK Sig Block 42";

fn perr(msg: impl Into<String>) -> Error {
    Error::parse("ZIP archive", msg)
}

fn read_at<R: Read + Seek>(r: &mut R, offset: u64, n: usize) -> Result<Vec<u8>> {
    r.seek(SeekFrom::Start(offset)).map_err(|e| perr(format!("seek: {e}")))?;
    let mut buf = Vec::new();
    r.by_ref().take(n as u64).read_to_end(&mut buf).map_err(|e| perr(format!("read: {e}")))?;
    if buf.len() != n {
        return Err(perr("unexpected end of archive"));
    }
    Ok(buf)
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).unwrap_or(0)
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).unwrap_or(0)
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    b.get(o..o + 8)
        .map(|s| {
            let mut a = [0u8; 8];
            a.copy_from_slice(s);
            u64::from_le_bytes(a)
        })
        .unwrap_or(0)
}

/// The local header of one entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalInfo {
    pub name_raw: Vec<u8>,
    pub extra: Vec<u8>,
    /// Absolute offset of the entry data.
    pub data_offset: u64,
}

/// Where the central directory is, plus the archive comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveInfo {
    pub len: u64,
    pub cd_offset: u64,
    pub comment: Vec<u8>,
    /// An APK Signing Block sits right before the central directory.
    pub apk_signing_block: bool,
}

/// Locate the end records (the EOCD whose comment ends exactly at end of file).
pub fn archive_info<R: Read + Seek>(r: &mut R) -> Result<ArchiveInfo> {
    let len = r.seek(SeekFrom::End(0)).map_err(|e| perr(format!("seek: {e}")))?;
    if len < 22 {
        return Err(perr("too short to be a ZIP archive"));
    }
    let tail_len = len.min(22 + 0xFFFF);
    let tail_start = len - tail_len;
    let tail = read_at(r, tail_start, tail_len as usize)?;
    let mut pos = tail.len() - 22;
    loop {
        if u32_at(&tail, pos) == SIG_EOCD && pos + 22 + usize::from(u16_at(&tail, pos + 20)) == tail.len() {
            let abs = tail_start + pos as u64;
            let comment = tail[pos + 22..].to_vec();
            let mut cd_offset = u64::from(u32_at(&tail, pos + 16));
            if abs >= 20 {
                let loc = read_at(r, abs - 20, 20)?;
                if u32_at(&loc, 0) == SIG_ZIP64_LOCATOR {
                    let z = u64_at(&loc, 8);
                    if z.checked_add(56).is_some_and(|e| e <= abs - 20) {
                        let h = read_at(r, z, 56)?;
                        if u32_at(&h, 0) == SIG_ZIP64_EOCD {
                            cd_offset = u64_at(&h, 48);
                        }
                    }
                }
            }
            if cd_offset > abs {
                return Err(perr("central directory offset out of range"));
            }
            let apk_signing_block = cd_offset >= 16 && read_at(r, cd_offset - 16, 16)? == APK_SIG_MAGIC;
            return Ok(ArchiveInfo { len, cd_offset, comment, apk_signing_block });
        }
        if pos == 0 {
            return Err(perr("no end-of-central-directory record"));
        }
        pos -= 1;
    }
}

/// Read the local header of `e`; it must lie before `cd_offset`.
pub fn local_info<R: Read + Seek>(r: &mut R, e: &ZipEntry, cd_offset: u64) -> Result<LocalInfo> {
    if e.local_header_offset.checked_add(30).is_none_or(|end| end > cd_offset) {
        return Err(perr(format!("entry {}: local header outside the data area", e.name)));
    }
    let h = read_at(r, e.local_header_offset, 30)?;
    if u32_at(&h, 0) != SIG_LOCAL {
        return Err(perr(format!("entry {}: bad local header signature", e.name)));
    }
    let n = usize::from(u16_at(&h, 26));
    let x = usize::from(u16_at(&h, 28));
    let data_offset = e.local_header_offset + 30 + n as u64 + x as u64;
    if data_offset > cd_offset {
        return Err(perr(format!("entry {}: local header runs into the central directory", e.name)));
    }
    let v = read_at(r, e.local_header_offset + 30, n + x)?;
    Ok(LocalInfo { name_raw: v[..n].to_vec(), extra: v[n..].to_vec(), data_offset })
}

/// Raw (still compressed) bytes of `e`.
pub fn read_raw<R: Read + Seek>(r: &mut R, e: &ZipEntry, l: &LocalInfo, cd_offset: u64) -> Result<Vec<u8>> {
    let end = l.data_offset.checked_add(e.compressed_size).ok_or_else(|| perr("entry size overflows"))?;
    if end > cd_offset {
        return Err(perr(format!("entry {} runs past the data area", e.name)));
    }
    let n = usize::try_from(e.compressed_size).map_err(|_| perr("entry too large"))?;
    read_at(r, l.data_offset, n)
}

/// Bytes a data descriptor may take after the data (signature + CRC + 2 x 8-byte sizes).
const MAX_DESCRIPTOR: u64 = 24;

/// Check that the entries tile the data area: the first entry starts at 0, entries do not
/// overlap, and nothing but data descriptors sits between entries or before the central
/// directory. Hidden or prefixed data (self-extractor stubs, APK signing blocks, smuggled
/// payloads, overlapping "zip bombs") would be lost or mangled by a repack, so the caller
/// refuses such archives.
pub fn check_layout(entries: &[ZipEntry], locals: &[LocalInfo], cd_offset: u64) -> Result<()> {
    let mut spans: Vec<(u64, u64, u64)> = Vec::with_capacity(entries.len());
    for (e, l) in entries.iter().zip(locals) {
        let end = l.data_offset.checked_add(e.compressed_size).ok_or_else(|| perr("entry size overflows"))?;
        let slack = if e.flags & 0x0008 != 0 { MAX_DESCRIPTOR } else { 0 };
        spans.push((e.local_header_offset, end, slack));
    }
    spans.sort_unstable();
    let mut at = 0u64;
    let mut slack = 0u64;
    for (start, end, s) in spans {
        if start < at {
            return Err(perr("entries overlap"));
        }
        if start - at > slack {
            return Err(perr(if at == 0 { "data before the first entry" } else { "data between entries" }));
        }
        at = end;
        slack = s;
    }
    if cd_offset < at {
        return Err(perr("entry data runs into the central directory"));
    }
    if cd_offset - at > slack {
        return Err(perr("data between the last entry and the central directory"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zip::{ZipArchive, ZipWriter};
    use std::io::Cursor;

    fn zip() -> Vec<u8> {
        let mut w = ZipWriter::new(Vec::new());
        w.add("a.txt", &b"aaaa".repeat(100), 10, 0, 0).unwrap();
        w.add("b.txt", b"b", 0, 0, 0).unwrap();
        w.set_comment(b"hi").unwrap();
        w.finish().unwrap()
    }

    #[test]
    fn reads_local_headers_raw_data_and_comment() {
        let b = zip();
        let z = ZipArchive::new(Cursor::new(b.clone())).unwrap();
        let mut c = Cursor::new(b);
        let info = archive_info(&mut c).unwrap();
        assert_eq!(info.comment, b"hi");
        assert!(!info.apk_signing_block);
        let locals: Vec<_> = z.entries().iter().map(|e| local_info(&mut c, e, info.cd_offset).unwrap()).collect();
        assert_eq!(locals[0].name_raw, b"a.txt");
        let raw = read_raw(&mut c, &z.entries()[0], &locals[0], info.cd_offset).unwrap();
        assert_eq!(miniz_oxide::inflate::decompress_to_vec(&raw).unwrap(), b"aaaa".repeat(100));
        check_layout(z.entries(), &locals, info.cd_offset).unwrap();
    }

    #[test]
    fn hidden_data_and_overlaps_are_detected() {
        let b = zip();
        let z = ZipArchive::new(Cursor::new(b.clone())).unwrap();
        let mut c = Cursor::new(b);
        let info = archive_info(&mut c).unwrap();
        let locals: Vec<_> = z.entries().iter().map(|e| local_info(&mut c, e, info.cd_offset).unwrap()).collect();
        assert!(check_layout(z.entries(), &locals, info.cd_offset + 5).is_err(), "gap before the CD");
        let mut ents = z.entries().to_vec();
        ents[1].local_header_offset = ents[0].local_header_offset; // overlap
        assert!(check_layout(&ents, &locals, info.cd_offset).is_err());
        let mut ents = z.entries().to_vec();
        ents[0].local_header_offset = 3;
        assert!(check_layout(&ents, &locals, info.cd_offset).is_err());
    }

    #[test]
    fn garbage_never_panics() {
        for n in 0..64 {
            let mut c = Cursor::new(vec![0x50u8; n]);
            let _ = archive_info(&mut c);
        }
        let b = zip();
        for cut in 0..b.len() {
            let mut c = Cursor::new(b[..cut].to_vec());
            let _ = archive_info(&mut c);
        }
    }
}
