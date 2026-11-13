//! ZIP reading: end of central directory (incl. ZIP64), central directory,
//! entry extraction with CRC check and a decompression-size cap. Owner: W2-E.
//!
//! Everything read from the archive is hostile:
//!
//! - the EOCD record is searched backwards in at most the last 65 557 bytes and
//!   must be self-consistent (its comment must end exactly at end of file, or
//!   the archive is rejected only after every candidate failed);
//! - sizes, counts and offsets are checked against the real stream length
//!   before anything is allocated (a declared size is never trusted);
//! - the number of entries is capped ([`MAX_ENTRIES`]) and the central
//!   directory size is capped ([`MAX_CENTRAL_DIRECTORY`]);
//! - entry names are kept as data: [`safe_relative_path`] is the only way to
//!   turn one into a path, and it refuses absolute names, drive letters,
//!   `..`, backslashes, NUL and Windows reserved names (zip-slip);
//! - decompression stops at the caller's `max_bytes`, and the CRC-32 and the
//!   declared size must match.
//!
//! Nothing here panics on malformed input; every failure is `Error::Parse`.

use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

use mj_core::{Error, Result};

use super::{ZipEntry, METHOD_DEFLATE, METHOD_STORED};

/// Upper bound on central-directory entries we accept (a real archive with more is refused).
pub const MAX_ENTRIES: u64 = 1_000_000;
/// Upper bound on the central-directory size we read into memory.
pub const MAX_CENTRAL_DIRECTORY: u64 = 256 * 1024 * 1024;

const SIG_LOCAL: u32 = 0x0403_4b50;
const SIG_CENTRAL: u32 = 0x0201_4b50;
const SIG_EOCD: u32 = 0x0605_4b50;
const SIG_ZIP64_EOCD: u32 = 0x0606_4b50;
const SIG_ZIP64_LOCATOR: u32 = 0x0706_4b50;
const EOCD_LEN: u64 = 22;
const MAX_COMMENT: u64 = 0xFFFF;

fn perr(msg: impl Into<String>) -> Error {
    Error::parse("ZIP archive", msg)
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]]))
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    b.get(o..o + 8).map(|s| {
        let mut a = [0u8; 8];
        a.copy_from_slice(s);
        u64::from_le_bytes(a)
    })
}

/// Where the central directory is, as read from the (ZIP64) EOCD.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Directory {
    entries: u64,
    size: u64,
    offset: u64,
}

/// An open ZIP archive.
pub struct ZipArchive<R: Read + Seek> {
    reader: R,
    entries: Vec<ZipEntry>,
    len: u64,
    /// Offset of the central directory (local data must end before it).
    cd_offset: u64,
}

impl<R: Read + Seek> ZipArchive<R> {
    /// Parse the central directory.
    pub fn new(mut reader: R) -> Result<Self> {
        let len = reader.seek(SeekFrom::End(0)).map_err(|e| perr(format!("seek: {e}")))?;
        if len < EOCD_LEN {
            return Err(perr("too short to be a ZIP archive"));
        }
        let tail_len = len.min(EOCD_LEN + MAX_COMMENT);
        let tail_start = len - tail_len;
        let tail = read_exact_at(&mut reader, tail_start, tail_len as usize)?;
        // Candidates from the end backwards; the first self-consistent one wins.
        let mut last_err = perr("no end-of-central-directory record");
        let mut pos = tail.len().saturating_sub(EOCD_LEN as usize);
        loop {
            if u32_at(&tail, pos) == Some(SIG_EOCD) {
                let abs = tail_start + pos as u64;
                match parse_eocd(&mut reader, &tail[pos..], abs, len) {
                    Ok(dir) => match read_central(&mut reader, dir, len) {
                        Ok(entries) => {
                            return Ok(ZipArchive { reader, entries, len, cd_offset: dir.offset });
                        }
                        Err(e) => last_err = e,
                    },
                    Err(e) => last_err = e,
                }
            }
            if pos == 0 {
                break;
            }
            pos -= 1;
        }
        Err(last_err)
    }

    /// Central-directory entries in stored order.
    pub fn entries(&self) -> &[ZipEntry] {
        &self.entries
    }

    /// Index of the entry named `name`.
    pub fn find(&self, name: &str) -> Option<usize> {
        self.entries.iter().position(|e| e.name == name)
    }

    /// True when any entry is encrypted (general-purpose flag bit 0).
    pub fn any_encrypted(&self) -> bool {
        self.entries.iter().any(ZipEntry::is_encrypted)
    }

    /// Decompressed content of entry `index`, refusing more than `max_bytes`.
    pub fn read(&mut self, index: usize, max_bytes: u64) -> Result<Vec<u8>> {
        let e = self.entries.get(index).cloned().ok_or_else(|| perr(format!("no entry {index}")))?;
        if e.is_encrypted() {
            return Err(perr(format!("entry {} is encrypted", e.name)));
        }
        if e.uncompressed_size > max_bytes {
            return Err(perr(format!("entry {} is larger than the {max_bytes}-byte limit", e.name)));
        }
        let data_offset = self.data_offset(&e)?;
        let end = data_offset.checked_add(e.compressed_size).ok_or_else(|| perr("entry size overflows"))?;
        if end > self.cd_offset || end > self.len {
            return Err(perr(format!("entry {} runs past the end of the data", e.name)));
        }
        // Deflate output can be at most ~1032x its input but never smaller than input/1.0x
        // minus a few bytes of framing; a compressed size far beyond the limit is refused.
        let compressed_cap = max_bytes.saturating_add(max_bytes / 8).saturating_add(1024);
        if e.compressed_size > compressed_cap {
            return Err(perr(format!("entry {} is larger than the {max_bytes}-byte limit", e.name)));
        }
        let raw = read_exact_at(&mut self.reader, data_offset, e.compressed_size as usize)?;
        let data = match e.method {
            METHOD_STORED => {
                if e.compressed_size != e.uncompressed_size {
                    return Err(perr(format!("stored entry {} has inconsistent sizes", e.name)));
                }
                raw
            }
            METHOD_DEFLATE => {
                let limit = usize::try_from(max_bytes).unwrap_or(usize::MAX);
                miniz_oxide::inflate::decompress_to_vec_with_limit(&raw, limit)
                    .map_err(|err| perr(format!("entry {}: deflate stream invalid or too large ({err:?})", e.name)))?
            }
            m => return Err(Error::unsupported("ZIP compression", format!("method {m} in entry {}", e.name))),
        };
        if data.len() as u64 != e.uncompressed_size {
            return Err(perr(format!("entry {}: size differs from the directory", e.name)));
        }
        if crate::crc32::crc32(&data) != e.crc32 {
            return Err(perr(format!("entry {}: CRC-32 mismatch", e.name)));
        }
        Ok(data)
    }

    /// Absolute offset of an entry's data (after its local header).
    fn data_offset(&mut self, e: &ZipEntry) -> Result<u64> {
        if e.local_header_offset.checked_add(30).is_none_or(|end| end > self.cd_offset) {
            return Err(perr(format!("entry {}: local header outside the data area", e.name)));
        }
        let h = read_exact_at(&mut self.reader, e.local_header_offset, 30)?;
        if u32_at(&h, 0) != Some(SIG_LOCAL) {
            return Err(perr(format!("entry {}: bad local header signature", e.name)));
        }
        let n = u64::from(u16_at(&h, 26).unwrap_or(0));
        let x = u64::from(u16_at(&h, 28).unwrap_or(0));
        Ok(e.local_header_offset + 30 + n + x)
    }

    /// The inner reader.
    pub fn into_inner(self) -> R {
        self.reader
    }
}

/// Seek and read exactly `n` bytes, without trusting `n` beyond what the stream holds.
fn read_exact_at<R: Read + Seek>(r: &mut R, offset: u64, n: usize) -> Result<Vec<u8>> {
    r.seek(SeekFrom::Start(offset)).map_err(|e| perr(format!("seek: {e}")))?;
    let mut buf = Vec::new();
    // `take` bounds the allocation by what is actually there.
    r.by_ref().take(n as u64).read_to_end(&mut buf).map_err(|e| perr(format!("read: {e}")))?;
    if buf.len() != n {
        return Err(perr("unexpected end of archive"));
    }
    Ok(buf)
}

/// Parse an EOCD at absolute offset `abs` (and the ZIP64 records it points to).
fn parse_eocd<R: Read + Seek>(r: &mut R, rec: &[u8], abs: u64, len: u64) -> Result<Directory> {
    let disk = u16_at(rec, 4).ok_or_else(|| perr("truncated EOCD"))?;
    let cd_disk = u16_at(rec, 6).ok_or_else(|| perr("truncated EOCD"))?;
    let on_disk = u16_at(rec, 8).ok_or_else(|| perr("truncated EOCD"))?;
    let total = u16_at(rec, 10).ok_or_else(|| perr("truncated EOCD"))?;
    let size = u32_at(rec, 12).ok_or_else(|| perr("truncated EOCD"))?;
    let offset = u32_at(rec, 16).ok_or_else(|| perr("truncated EOCD"))?;
    let comment = u64::from(u16_at(rec, 20).ok_or_else(|| perr("truncated EOCD"))?);
    if abs + EOCD_LEN + comment > len {
        return Err(perr("EOCD comment runs past the end of the file"));
    }
    let needs64 = total == 0xFFFF || on_disk == 0xFFFF || size == 0xFFFF_FFFF || offset == 0xFFFF_FFFF;
    if needs64 || (abs >= 20 && locator_at(r, abs - 20).is_some()) {
        if abs < 20 {
            return Err(perr("ZIP64 locator missing"));
        }
        let z64 = locator_at(r, abs - 20).ok_or_else(|| perr("ZIP64 locator missing"))?;
        if z64.checked_add(56).is_none_or(|e| e > abs - 20) {
            return Err(perr("ZIP64 EOCD offset out of range"));
        }
        let h = read_exact_at(r, z64, 56)?;
        if u32_at(&h, 0) != Some(SIG_ZIP64_EOCD) {
            return Err(perr("bad ZIP64 EOCD signature"));
        }
        let d64 = u32_at(&h, 16).unwrap_or(1);
        let cd64 = u32_at(&h, 20).unwrap_or(1);
        if d64 != 0 || cd64 != 0 {
            return Err(Error::unsupported("ZIP archive", "multi-disk (split) archives"));
        }
        let on = u64_at(&h, 24).unwrap_or(0);
        let entries = u64_at(&h, 32).unwrap_or(0);
        if on != entries {
            return Err(Error::unsupported("ZIP archive", "multi-disk (split) archives"));
        }
        let dir = Directory { entries, size: u64_at(&h, 40).unwrap_or(0), offset: u64_at(&h, 48).unwrap_or(0) };
        return check_dir(dir, z64);
    }
    if disk != 0 || cd_disk != 0 || on_disk != total {
        return Err(Error::unsupported("ZIP archive", "multi-disk (split) archives"));
    }
    check_dir(Directory { entries: u64::from(total), size: u64::from(size), offset: u64::from(offset) }, abs)
}

/// The ZIP64 EOCD offset from a locator at `at`, if one is there.
fn locator_at<R: Read + Seek>(r: &mut R, at: u64) -> Option<u64> {
    let l = read_exact_at(r, at, 20).ok()?;
    if u32_at(&l, 0) != Some(SIG_ZIP64_LOCATOR) {
        return None;
    }
    u64_at(&l, 8)
}

/// Bounds of the central directory: it must end at or before `end` (the EOCD).
fn check_dir(d: Directory, end: u64) -> Result<Directory> {
    if d.entries > MAX_ENTRIES {
        return Err(perr(format!("{} entries exceed the limit", d.entries)));
    }
    if d.size > MAX_CENTRAL_DIRECTORY {
        return Err(perr("central directory too large"));
    }
    if d.offset.checked_add(d.size).is_none_or(|e| e > end) {
        return Err(perr("central directory out of range"));
    }
    // Each entry takes at least 46 bytes.
    if d.entries.saturating_mul(46) > d.size {
        return Err(perr("entry count does not fit the central directory"));
    }
    Ok(d)
}

fn read_central<R: Read + Seek>(r: &mut R, d: Directory, len: u64) -> Result<Vec<ZipEntry>> {
    let _ = len;
    let cd = read_exact_at(r, d.offset, d.size as usize)?;
    let mut out = Vec::with_capacity(d.entries.min(65_536) as usize);
    let mut p = 0usize;
    for i in 0..d.entries {
        let e = parse_central_entry(&cd, &mut p).map_err(|e| match e {
            Error::Parse { message, .. } => perr(format!("entry {i}: {message}")),
            other => other,
        })?;
        if e.local_header_offset >= d.offset {
            return Err(perr(format!("entry {}: local header after the central directory", e.name)));
        }
        out.push(e);
    }
    Ok(out)
}

fn parse_central_entry(cd: &[u8], p: &mut usize) -> Result<ZipEntry> {
    let b = cd.get(*p..).ok_or_else(|| perr("truncated central directory"))?;
    if b.len() < 46 || u32_at(b, 0) != Some(SIG_CENTRAL) {
        return Err(perr("bad central directory signature"));
    }
    let flags = u16_at(b, 8).unwrap_or(0);
    let method = u16_at(b, 10).unwrap_or(0);
    let dos_time = u16_at(b, 12).unwrap_or(0);
    let dos_date = u16_at(b, 14).unwrap_or(0);
    let crc32 = u32_at(b, 16).unwrap_or(0);
    let mut csize = u64::from(u32_at(b, 20).unwrap_or(0));
    let mut usize_ = u64::from(u32_at(b, 24).unwrap_or(0));
    let n = usize::from(u16_at(b, 28).unwrap_or(0));
    let x = usize::from(u16_at(b, 30).unwrap_or(0));
    let c = usize::from(u16_at(b, 32).unwrap_or(0));
    let external_attrs = u32_at(b, 38).unwrap_or(0);
    let mut offset = u64::from(u32_at(b, 42).unwrap_or(0));
    let total = 46 + n + x + c;
    if b.len() < total {
        return Err(perr("truncated central directory entry"));
    }
    let name_bytes = &b[46..46 + n];
    let extra = b[46 + n..46 + n + x].to_vec();
    let comment = b[46 + n + x..total].to_vec();
    // ZIP64 extended information (0x0001): only the fields saturated in the record are present.
    let mut q = 0usize;
    while q + 4 <= extra.len() {
        let id = u16_at(&extra, q).unwrap_or(0);
        let sz = usize::from(u16_at(&extra, q + 2).unwrap_or(0));
        let body = extra.get(q + 4..q + 4 + sz).ok_or_else(|| perr("truncated extra field"))?;
        if id == 0x0001 {
            let mut k = 0usize;
            for field in [&mut usize_, &mut csize, &mut offset] {
                if *field == 0xFFFF_FFFF {
                    *field = u64_at(body, k).ok_or_else(|| perr("truncated ZIP64 extra field"))?;
                    k += 8;
                }
            }
        }
        q += 4 + sz;
    }
    *p += total;
    Ok(ZipEntry {
        name: decode_name(name_bytes, flags),
        method,
        flags,
        crc32,
        compressed_size: csize,
        uncompressed_size: usize_,
        local_header_offset: offset,
        dos_time,
        dos_date,
        external_attrs,
        extra,
        comment,
    })
}

/// Entry names: UTF-8 when flag bit 11 is set or the bytes are valid UTF-8, else CP437.
fn decode_name(b: &[u8], flags: u16) -> String {
    if flags & 0x0800 != 0 {
        return String::from_utf8_lossy(b).into_owned();
    }
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) => b.iter().map(|&c| cp437(c)).collect(),
    }
}

/// CP437 to Unicode (upper half; the lower half is ASCII).
fn cp437(c: u8) -> char {
    const HIGH: &str = "ÇüéâäàåçêëèïîìÄÅÉæÆôöòûùÿÖÜ¢£¥₧ƒáíóúñÑªº¿⌐¬½¼¡«»░▒▓│┤╡╢╖╕╣║╗╝╜╛┐└┴┬├─┼╞╟╚╔╩╦╠═╬╧╨╤╥╙╘╒╓╫╪┘┌█▄▌▐▀αßΓπΣσµτΦΘΩδ∞φε∩≡±≥≤⌠⌡÷≈°∙·√ⁿ²■\u{a0}";
    if c < 0x80 {
        c as char
    } else {
        HIGH.chars().nth(usize::from(c - 0x80)).unwrap_or('\u{fffd}')
    }
}

/// Turn an entry name into a safe relative path, or `None` when it could escape the
/// extraction folder or is not portable (zip-slip defence): absolute names, drive
/// letters, `..`, `.`-only or empty components, backslashes, NUL/control characters,
/// and Windows reserved or invalid names are refused.
pub fn safe_relative_path(name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.starts_with('/') || name.contains('\\') || name.contains('\0') {
        return None;
    }
    let trimmed = name.strip_suffix('/').unwrap_or(name);
    let mut out = PathBuf::new();
    for comp in trimmed.split('/') {
        if comp.is_empty() || comp == "." || comp == ".." {
            return None;
        }
        if mj_core::pathutil::has_invalid_windows_name(comp) {
            return None;
        }
        out.push(comp);
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Minimal hand-built archive writer for tests (the real writer is W3-A's).
    pub(crate) struct TestZip {
        pub data: Vec<u8>,
        central: Vec<u8>,
        count: u64,
    }

    impl TestZip {
        pub fn new() -> Self {
            TestZip { data: Vec::new(), central: Vec::new(), count: 0 }
        }

        pub fn add(&mut self, name: &str, content: &[u8], deflate: bool, flags: u16) {
            let comp = if deflate { miniz_oxide::deflate::compress_to_vec(content, 6) } else { content.to_vec() };
            let method: u16 = if deflate { 8 } else { 0 };
            let crc = crate::crc32::crc32(content);
            let off = self.data.len() as u32;
            let mut h = Vec::new();
            h.extend_from_slice(&SIG_LOCAL.to_le_bytes());
            h.extend_from_slice(&20u16.to_le_bytes());
            h.extend_from_slice(&flags.to_le_bytes());
            h.extend_from_slice(&method.to_le_bytes());
            h.extend_from_slice(&[0, 0, 0x21, 0]);
            h.extend_from_slice(&crc.to_le_bytes());
            h.extend_from_slice(&(comp.len() as u32).to_le_bytes());
            h.extend_from_slice(&(content.len() as u32).to_le_bytes());
            h.extend_from_slice(&(name.len() as u16).to_le_bytes());
            h.extend_from_slice(&0u16.to_le_bytes());
            h.extend_from_slice(name.as_bytes());
            self.data.extend_from_slice(&h);
            self.data.extend_from_slice(&comp);
            let c = &mut self.central;
            c.extend_from_slice(&SIG_CENTRAL.to_le_bytes());
            c.extend_from_slice(&20u16.to_le_bytes());
            c.extend_from_slice(&20u16.to_le_bytes());
            c.extend_from_slice(&flags.to_le_bytes());
            c.extend_from_slice(&method.to_le_bytes());
            c.extend_from_slice(&[0, 0, 0x21, 0]);
            c.extend_from_slice(&crc.to_le_bytes());
            c.extend_from_slice(&(comp.len() as u32).to_le_bytes());
            c.extend_from_slice(&(content.len() as u32).to_le_bytes());
            c.extend_from_slice(&(name.len() as u16).to_le_bytes());
            c.extend_from_slice(&[0; 2]); // extra
            c.extend_from_slice(&[0; 2]); // comment
            c.extend_from_slice(&[0; 2]); // disk
            c.extend_from_slice(&[0; 2]); // internal attrs
            c.extend_from_slice(&[0; 4]); // external attrs
            c.extend_from_slice(&off.to_le_bytes());
            c.extend_from_slice(name.as_bytes());
            self.count += 1;
        }

        pub fn finish(mut self, comment: &[u8]) -> Vec<u8> {
            let cd_off = self.data.len() as u32;
            let cd_len = self.central.len() as u32;
            self.data.extend_from_slice(&self.central);
            self.data.extend_from_slice(&SIG_EOCD.to_le_bytes());
            self.data.extend_from_slice(&[0; 4]);
            self.data.extend_from_slice(&(self.count as u16).to_le_bytes());
            self.data.extend_from_slice(&(self.count as u16).to_le_bytes());
            self.data.extend_from_slice(&cd_len.to_le_bytes());
            self.data.extend_from_slice(&cd_off.to_le_bytes());
            self.data.extend_from_slice(&(comment.len() as u16).to_le_bytes());
            self.data.extend_from_slice(comment);
            self.data
        }

        /// Same, but with ZIP64 records (the classic EOCD saturated).
        pub fn finish_zip64(mut self) -> Vec<u8> {
            let cd_off = self.data.len() as u64;
            let cd_len = self.central.len() as u64;
            self.data.extend_from_slice(&self.central);
            let z64 = self.data.len() as u64;
            let d = &mut self.data;
            d.extend_from_slice(&SIG_ZIP64_EOCD.to_le_bytes());
            d.extend_from_slice(&44u64.to_le_bytes());
            d.extend_from_slice(&45u16.to_le_bytes());
            d.extend_from_slice(&45u16.to_le_bytes());
            d.extend_from_slice(&0u32.to_le_bytes());
            d.extend_from_slice(&0u32.to_le_bytes());
            d.extend_from_slice(&self.count.to_le_bytes());
            d.extend_from_slice(&self.count.to_le_bytes());
            d.extend_from_slice(&cd_len.to_le_bytes());
            d.extend_from_slice(&cd_off.to_le_bytes());
            d.extend_from_slice(&SIG_ZIP64_LOCATOR.to_le_bytes());
            d.extend_from_slice(&0u32.to_le_bytes());
            d.extend_from_slice(&z64.to_le_bytes());
            d.extend_from_slice(&1u32.to_le_bytes());
            d.extend_from_slice(&SIG_EOCD.to_le_bytes());
            d.extend_from_slice(&[0; 4]);
            d.extend_from_slice(&0xFFFFu16.to_le_bytes());
            d.extend_from_slice(&0xFFFFu16.to_le_bytes());
            d.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
            d.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
            d.extend_from_slice(&0u16.to_le_bytes());
            self.data
        }
    }

    fn sample() -> Vec<u8> {
        let mut z = TestZip::new();
        z.add("a.txt", b"hello hello hello hello", true, 0);
        z.add("dir/", b"", false, 0);
        z.add("dir/b.bin", &[1, 2, 3, 4], false, 0);
        z.add("secret.txt", b"xxxx", false, 1);
        z.finish(b"a comment with PK\x05\x06 inside")
    }

    #[test]
    fn reads_entries_and_data() {
        let mut a = ZipArchive::new(Cursor::new(sample())).unwrap();
        let names: Vec<&str> = a.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["a.txt", "dir/", "dir/b.bin", "secret.txt"]);
        assert!(a.entries()[1].is_dir());
        assert!(a.any_encrypted());
        assert!(a.entries()[3].is_encrypted());
        assert_eq!(a.read(0, 1024).unwrap(), b"hello hello hello hello");
        assert_eq!(a.read(a.find("dir/b.bin").unwrap(), 4).unwrap(), [1, 2, 3, 4]);
        assert_eq!(a.read(0, 5).unwrap_err().code(), "parse", "size cap");
        assert_eq!(a.read(3, 1024).unwrap_err().code(), "parse", "encrypted entries are not read");
        assert!(a.read(99, 10).is_err());
    }

    #[test]
    fn reads_zip64() {
        let mut z = TestZip::new();
        z.add("big.txt", b"zip64 body", false, 0);
        let mut a = ZipArchive::new(Cursor::new(z.finish_zip64())).unwrap();
        assert_eq!(a.entries().len(), 1);
        assert_eq!(a.read(0, 100).unwrap(), b"zip64 body");
    }

    #[test]
    fn zip64_extra_field_sizes() {
        // Patch an entry so the central record saturates the sizes and carries a 0x0001 extra.
        let mut z = TestZip::new();
        z.add("x", b"abc", false, 0);
        let raw = z.finish(b"");
        let mut a = ZipArchive::new(Cursor::new(raw.clone())).unwrap();
        let e = a.entries()[0].clone();
        assert_eq!(e.uncompressed_size, 3);
        // Rebuild central directory by hand with ZIP64 extra.
        let local_len = 30 + 1 + 3;
        let mut out = raw[..local_len].to_vec();
        let cd_off = out.len() as u32;
        let mut c = Vec::new();
        c.extend_from_slice(&SIG_CENTRAL.to_le_bytes());
        c.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0x21, 0]);
        c.extend_from_slice(&e.crc32.to_le_bytes());
        c.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        c.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        c.extend_from_slice(&1u16.to_le_bytes());
        c.extend_from_slice(&20u16.to_le_bytes());
        c.extend_from_slice(&[0; 10]);
        c.extend_from_slice(&0u32.to_le_bytes());
        c.push(b'x');
        c.extend_from_slice(&1u16.to_le_bytes());
        c.extend_from_slice(&16u16.to_le_bytes());
        c.extend_from_slice(&3u64.to_le_bytes());
        c.extend_from_slice(&3u64.to_le_bytes());
        out.extend_from_slice(&c);
        out.extend_from_slice(&SIG_EOCD.to_le_bytes());
        out.extend_from_slice(&[0, 0, 0, 0, 1, 0, 1, 0]);
        out.extend_from_slice(&(c.len() as u32).to_le_bytes());
        out.extend_from_slice(&cd_off.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        a = ZipArchive::new(Cursor::new(out)).unwrap();
        assert_eq!(a.entries()[0].compressed_size, 3);
        assert_eq!(a.read(0, 10).unwrap(), b"abc");
    }

    #[test]
    fn malformed_archives_are_errors_not_panics() {
        let good = sample();
        assert!(ZipArchive::new(Cursor::new(Vec::new())).is_err());
        assert!(ZipArchive::new(Cursor::new(b"PK\x05\x06".to_vec())).is_err());
        // Every truncation and every single-byte corruption must fail cleanly or parse.
        for cut in 0..good.len() {
            let _ = ZipArchive::new(Cursor::new(good[..cut].to_vec())).map(|mut a| {
                for i in 0..a.entries().len() {
                    let _ = a.read(i, 1 << 20);
                }
            });
        }
        for i in 0..good.len() {
            for v in [0x00u8, 0xFF, 0x7F] {
                let mut bad = good.clone();
                bad[i] = v;
                if let Ok(mut a) = ZipArchive::new(Cursor::new(bad)) {
                    for k in 0..a.entries().len() {
                        let _ = a.read(k, 1 << 20);
                    }
                }
            }
        }
        // Huge declared counts and sizes are refused before allocation.
        let mut eocd = Vec::new();
        eocd.extend_from_slice(&SIG_EOCD.to_le_bytes());
        eocd.extend_from_slice(&[0, 0, 0, 0, 0xFF, 0xFE, 0xFF, 0xFE]);
        eocd.extend_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
        eocd.extend_from_slice(&0u32.to_le_bytes());
        eocd.extend_from_slice(&[0, 0]);
        assert!(ZipArchive::new(Cursor::new(eocd)).is_err());
    }

    #[test]
    fn crc_mismatch_is_detected() {
        let mut bad = sample();
        // The stored bytes of dir/b.bin are `1 2 3 4`; flip one.
        let pos = bad.windows(4).position(|w| w == [1, 2, 3, 4]).unwrap();
        bad[pos] = 9;
        let mut a = ZipArchive::new(Cursor::new(bad)).unwrap();
        let i = a.find("dir/b.bin").unwrap();
        assert_eq!(a.read(i, 10).unwrap_err().code(), "parse");
    }

    #[test]
    fn zip_slip_names_are_refused() {
        for bad in [
            "../x",
            "a/../../x",
            "/etc/passwd",
            "C:/x",
            "a\\..\\x",
            "",
            "a//b",
            "./a",
            "con.txt",
            "a/nul",
            "x\0y",
            "trailing. ",
        ] {
            assert_eq!(safe_relative_path(bad), None, "{bad:?}");
        }
        assert_eq!(safe_relative_path("a/b.txt"), Some(PathBuf::from("a").join("b.txt")));
        assert_eq!(safe_relative_path("dir/"), Some(PathBuf::from("dir")));
    }

    #[test]
    fn cp437_names() {
        assert_eq!(decode_name(&[0x82, b'a'], 0), "éa");
        assert_eq!(decode_name("ü".as_bytes(), 0x0800), "ü");
    }
}
