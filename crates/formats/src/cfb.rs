//! OLE compound files (MS-CFB): detect encrypted OOXML (`EncryptionInfo` /
//! `EncryptedPackage` streams) and legacy-Office password protection (SAF-07).
//! Owner: W2-E.
//!
//! A minimal, hardened reader: header, DIFAT, FAT, mini FAT, directory and
//! stream reading over any `Read + Seek`. Every sector chain is followed with
//! cycle detection (a chain may not be longer than the number of sectors in
//! the file), directory trees are walked iteratively with a visited set, and
//! all sizes are checked against the real file length before allocating.
//! Malformed input yields `Error::Parse`, never a panic.

use std::io::{Cursor, Read, Seek, SeekFrom};

use mj_core::{Error, Result};

/// Signature of a compound file.
pub const CFB_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

const FREESECT: u32 = 0xFFFF_FFFF;
const ENDOFCHAIN: u32 = 0xFFFF_FFFE;
const NOSTREAM: u32 = 0xFFFF_FFFF;
/// Largest FAT we accept (in entries): 16 M sectors (8 GiB with 512-byte sectors).
const MAX_FAT_ENTRIES: usize = 16 * 1024 * 1024;
/// Largest directory we accept (entries).
const MAX_DIR_ENTRIES: usize = 1 << 20;

fn perr(msg: impl Into<String>) -> Error {
    Error::parse("compound file", msg)
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).unwrap_or(0)
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).unwrap_or(0)
}

fn u64_at(b: &[u8], o: usize) -> u64 {
    let lo = u64::from(u32_at(b, o));
    let hi = u64::from(u32_at(b, o + 4));
    lo | (hi << 32)
}

/// Kind of a directory entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryType {
    Storage,
    Stream,
    Root,
}

/// One directory entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub kind: EntryType,
    left: u32,
    right: u32,
    child: u32,
    start: u32,
    pub size: u64,
}

/// What kind of protection a compound file carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protection {
    /// Encrypted OOXML package (password or RMS/IRM): `EncryptionInfo` + `EncryptedPackage`.
    EncryptedPackage,
    /// Rights-management data spaces in a legacy file (`\x06DataSpaces`).
    RightsManaged,
    /// Word 97-2003 with `fEncrypted` in the FIB.
    LegacyWord,
    /// Excel 97-2003 with a `FILEPASS` record.
    LegacyExcel,
    /// PowerPoint 97-2003 with an `EncryptedSummary` stream.
    LegacyPowerPoint,
}

/// An open compound file.
pub struct CompoundFile<R: Read + Seek> {
    reader: R,
    len: u64,
    sector_shift: u32,
    mini_shift: u32,
    mini_cutoff: u64,
    fat: Vec<u32>,
    minifat: Vec<u32>,
    dir: Vec<DirEntry>,
}

impl<R: Read + Seek> CompoundFile<R> {
    /// Parse the header, FAT, mini FAT and directory.
    pub fn open(mut reader: R) -> Result<Self> {
        let len = reader.seek(SeekFrom::End(0)).map_err(|e| perr(format!("seek: {e}")))?;
        let h = read_at(&mut reader, 0, 512, len)?;
        if h[..8] != CFB_MAGIC {
            return Err(perr("not a compound file"));
        }
        if u16_at(&h, 28) != 0xFFFE {
            return Err(perr("bad byte order mark"));
        }
        let sector_shift = u32::from(u16_at(&h, 30));
        let mini_shift = u32::from(u16_at(&h, 32));
        if !(sector_shift == 9 || sector_shift == 12) || mini_shift != 6 {
            return Err(perr("unsupported sector size"));
        }
        let num_fat = u32_at(&h, 44) as usize;
        let first_dir = u32_at(&h, 48);
        let mini_cutoff = u64::from(u32_at(&h, 56));
        let first_minifat = u32_at(&h, 60);
        let num_minifat = u32_at(&h, 64) as usize;
        let mut difat_sector = u32_at(&h, 68);
        let num_difat = u32_at(&h, 72) as usize;
        let ssz = 1usize << sector_shift;
        let sectors_in_file = (len.saturating_sub(ssz as u64) >> sector_shift) as usize + 1;
        if num_fat > sectors_in_file || num_minifat > sectors_in_file || num_difat > sectors_in_file {
            return Err(perr("sector counts exceed the file size"));
        }
        // DIFAT: 109 entries in the header, then a chain of DIFAT sectors.
        let mut fat_sectors: Vec<u32> = (0..109).map(|i| u32_at(&h, 76 + i * 4)).filter(|&s| s != FREESECT).collect();
        let per = ssz / 4 - 1;
        let mut seen = 0usize;
        while difat_sector != ENDOFCHAIN && difat_sector != FREESECT && fat_sectors.len() < num_fat {
            seen += 1;
            if seen > num_difat.max(1) || seen > sectors_in_file {
                return Err(perr("DIFAT chain too long"));
            }
            let s = read_sector(&mut reader, difat_sector, sector_shift, len)?;
            fat_sectors.extend((0..per).map(|i| u32_at(&s, i * 4)).filter(|&v| v != FREESECT));
            difat_sector = u32_at(&s, per * 4);
        }
        fat_sectors.truncate(num_fat);
        if fat_sectors.len() * (ssz / 4) > MAX_FAT_ENTRIES {
            return Err(perr("FAT too large"));
        }
        let mut fat = Vec::with_capacity(fat_sectors.len() * (ssz / 4));
        for &s in &fat_sectors {
            let b = read_sector(&mut reader, s, sector_shift, len)?;
            fat.extend((0..ssz / 4).map(|i| u32_at(&b, i * 4)));
        }
        let mut cf = CompoundFile {
            reader,
            len,
            sector_shift,
            mini_shift,
            mini_cutoff,
            fat,
            minifat: Vec::new(),
            dir: Vec::new(),
        };
        let dir_bytes = cf.read_chain(first_dir, None, MAX_DIR_ENTRIES * 128)?;
        cf.dir = dir_bytes.chunks_exact(128).map(parse_dir_entry).collect();
        if cf.dir.first().map(|e| e.kind) != Some(EntryType::Root) {
            return Err(perr("missing root entry"));
        }
        if num_minifat > 0 && first_minifat != ENDOFCHAIN {
            let mf = cf.read_chain(first_minifat, None, MAX_FAT_ENTRIES * 4)?;
            cf.minifat = mf.chunks_exact(4).map(|c| u32_at(c, 0)).collect();
        }
        Ok(cf)
    }

    /// Every directory entry (index 0 is the root).
    pub fn entries(&self) -> &[DirEntry] {
        &self.dir
    }

    /// Children of the storage at `index`, in tree order, iteratively with cycle detection.
    pub fn children(&self, index: usize) -> Result<Vec<&DirEntry>> {
        let parent = self.dir.get(index).ok_or_else(|| perr("bad entry index"))?;
        let mut out = Vec::new();
        let mut seen = vec![false; self.dir.len()];
        let mut stack = vec![parent.child];
        while let Some(id) = stack.pop() {
            if id == NOSTREAM {
                continue;
            }
            let i = id as usize;
            let e = self.dir.get(i).ok_or_else(|| perr("directory link out of range"))?;
            if std::mem::replace(&mut seen[i], true) {
                return Err(perr("directory tree has a cycle"));
            }
            out.push(e);
            stack.push(e.left);
            stack.push(e.right);
        }
        Ok(out)
    }

    /// Entries directly below the root storage.
    pub fn root_entries(&self) -> Result<Vec<&DirEntry>> {
        self.children(0)
    }

    /// Content of a root-level stream (case-insensitive name), refusing more than `max_bytes`.
    pub fn read_stream(&mut self, name: &str, max_bytes: u64) -> Result<Option<Vec<u8>>> {
        let found = self
            .root_entries()?
            .into_iter()
            .find(|e| e.kind == EntryType::Stream && e.name.eq_ignore_ascii_case(name))
            .cloned();
        let Some(e) = found else { return Ok(None) };
        if e.size > max_bytes {
            return Err(perr(format!("stream {name} exceeds the {max_bytes}-byte limit")));
        }
        let n = e.size as usize;
        if e.size < self.mini_cutoff {
            let root = self.dir[0].clone();
            let mini =
                self.read_chain(root.start, Some(root.size), usize::try_from(root.size).unwrap_or(usize::MAX))?;
            let mut out = Vec::with_capacity(n);
            let msz = 1usize << self.mini_shift;
            let mut s = e.start;
            let mut steps = 0usize;
            while out.len() < n {
                if s == ENDOFCHAIN || s == FREESECT || steps > self.minifat.len() {
                    return Err(perr("mini stream chain broken"));
                }
                steps += 1;
                let off = (s as usize).checked_mul(msz).ok_or_else(|| perr("mini sector overflow"))?;
                let chunk = mini.get(off..off + msz).ok_or_else(|| perr("mini sector out of range"))?;
                out.extend_from_slice(&chunk[..msz.min(n - out.len())]);
                s = *self.minifat.get(s as usize).ok_or_else(|| perr("mini FAT index out of range"))?;
            }
            Ok(Some(out))
        } else {
            Ok(Some(self.read_chain(e.start, Some(e.size), n)?))
        }
    }

    /// Follow a FAT chain from `start`, reading at most `max` bytes (or `size` exactly).
    fn read_chain(&mut self, start: u32, size: Option<u64>, max: usize) -> Result<Vec<u8>> {
        let ssz = 1usize << self.sector_shift;
        let want = size.map(|s| s as usize);
        if want.is_some_and(|w| w as u64 > self.len) {
            return Err(perr("stream larger than the file"));
        }
        let mut out = Vec::new();
        let mut s = start;
        let mut steps = 0usize;
        while s != ENDOFCHAIN {
            if out.len() >= want.unwrap_or(max) {
                break;
            }
            if s == FREESECT || steps > self.fat.len() {
                return Err(perr("sector chain broken or cyclic"));
            }
            steps += 1;
            let b = read_sector(&mut self.reader, s, self.sector_shift, self.len)?;
            out.extend_from_slice(&b);
            if out.len() > max.saturating_add(ssz) {
                return Err(perr("sector chain exceeds the limit"));
            }
            s = *self.fat.get(s as usize).ok_or_else(|| perr("FAT index out of range"))?;
        }
        if let Some(w) = want {
            if out.len() < w {
                return Err(perr("stream shorter than declared"));
            }
            out.truncate(w);
        } else {
            out.truncate(max.min(out.len()));
        }
        Ok(out)
    }

    /// Detect encryption / rights management (SAF-07).
    pub fn protection(&mut self) -> Result<Option<Protection>> {
        let names: Vec<(String, EntryType)> =
            self.root_entries()?.into_iter().map(|e| (e.name.clone(), e.kind)).collect();
        let has = |n: &str| names.iter().any(|(m, _)| m.eq_ignore_ascii_case(n));
        if has("EncryptedPackage") || has("EncryptionInfo") {
            return Ok(Some(Protection::EncryptedPackage));
        }
        if has("\u{6}DataSpaces") {
            return Ok(Some(Protection::RightsManaged));
        }
        if has("EncryptedSummary") {
            return Ok(Some(Protection::LegacyPowerPoint));
        }
        if has("WordDocument") {
            if let Some(fib) = self.read_stream_prefix("WordDocument", 32)? {
                // FibBase: wIdent 0xA5EC at 0, flags at 0x0A; fEncrypted = bit 8.
                if u16_at(&fib, 0) == 0xA5EC && u16_at(&fib, 0x0A) & 0x0100 != 0 {
                    return Ok(Some(Protection::LegacyWord));
                }
            }
        }
        for book in ["Workbook", "Book"] {
            if has(book) {
                if let Some(data) = self.read_stream_prefix(book, 64 * 1024)? {
                    if excel_has_filepass(&data) {
                        return Ok(Some(Protection::LegacyExcel));
                    }
                }
            }
        }
        Ok(None)
    }

    /// Up to `n` leading bytes of a root stream (streams of any size).
    fn read_stream_prefix(&mut self, name: &str, n: u64) -> Result<Option<Vec<u8>>> {
        let found = self
            .root_entries()?
            .into_iter()
            .find(|e| e.kind == EntryType::Stream && e.name.eq_ignore_ascii_case(name))
            .cloned();
        let Some(e) = found else { return Ok(None) };
        if e.size <= n || e.size < self.mini_cutoff {
            return self.read_stream(name, e.size.min(self.mini_cutoff.max(n)));
        }
        let take = n as usize;
        Ok(Some(self.read_chain(e.start, None, take)?))
    }
}

/// Walk BIFF records from the BOF: a `FILEPASS` (0x002F) before the first `EOF` (0x000A)
/// means the workbook is password protected.
fn excel_has_filepass(data: &[u8]) -> bool {
    let mut p = 0usize;
    for _ in 0..4096 {
        if p + 4 > data.len() {
            return false;
        }
        let ty = u16_at(data, p);
        let len = usize::from(u16_at(data, p + 2));
        match ty {
            0x002F => return true,
            0x000A => return false,
            _ => {}
        }
        p += 4 + len;
    }
    false
}

fn parse_dir_entry(b: &[u8]) -> DirEntry {
    let name_len = usize::from(u16_at(b, 64)).min(64);
    let units: Vec<u16> = (0..name_len / 2).map(|i| u16_at(b, i * 2)).take_while(|&u| u != 0).collect();
    let kind = match b.get(66).copied().unwrap_or(0) {
        1 => EntryType::Storage,
        2 => EntryType::Stream,
        5 => EntryType::Root,
        // Unknown or unallocated: treated as an empty storage (never read as a stream).
        _ => EntryType::Storage,
    };
    DirEntry {
        name: String::from_utf16_lossy(&units),
        kind,
        left: u32_at(b, 68),
        right: u32_at(b, 72),
        child: u32_at(b, 76),
        start: u32_at(b, 116),
        size: u64_at(b, 120),
    }
}

fn read_sector<R: Read + Seek>(r: &mut R, sector: u32, shift: u32, len: u64) -> Result<Vec<u8>> {
    if sector >= 0xFFFF_FFFA {
        return Err(perr("special sector number in a chain"));
    }
    let size = 1u64 << shift;
    let off = (u64::from(sector) + 1).checked_mul(size).ok_or_else(|| perr("sector offset overflow"))?;
    read_at(r, off, size as usize, len)
}

fn read_at<R: Read + Seek>(r: &mut R, off: u64, n: usize, len: u64) -> Result<Vec<u8>> {
    if off.checked_add(n as u64).is_none_or(|e| e > len) {
        // A short final sector is allowed: pad with zeros when the start is inside the file.
        if off >= len {
            return Err(perr("sector beyond the end of the file"));
        }
    }
    r.seek(SeekFrom::Start(off)).map_err(|e| perr(format!("seek: {e}")))?;
    let mut buf = Vec::with_capacity(n);
    r.by_ref().take(n as u64).read_to_end(&mut buf).map_err(|e| perr(format!("read: {e}")))?;
    buf.resize(n, 0);
    Ok(buf)
}

/// Names of the root-storage streams of a compound file.
pub fn stream_names(data: &[u8]) -> Result<Vec<String>> {
    let cf = CompoundFile::open(Cursor::new(data))?;
    let mut v: Vec<String> =
        cf.root_entries()?.into_iter().filter(|e| e.kind == EntryType::Stream).map(|e| e.name.clone()).collect();
    v.sort();
    Ok(v)
}

/// True when `data` starts with the compound-file signature.
pub fn is_compound_file(data: &[u8]) -> bool {
    data.len() >= 8 && data[..8] == CFB_MAGIC
}

/// Build a small version-3 compound file (tests in this crate and in `mj-actions`).
///
/// `streams` are root-level streams (each stored in regular sectors, padded to 4096 bytes
/// so the mini stream is never needed), `storages` root-level empty storages.
#[doc(hidden)]
pub fn build_test_file(streams: &[(&str, &[u8])], storages: &[&str]) -> Vec<u8> {
    let ssz = 512usize;
    // Layout: sector 0 = FAT, sector 1.. = directory, then stream data.
    let n_entries = 1 + streams.len() + storages.len();
    let dir_sectors = n_entries.div_ceil(4);
    let mut data_sectors = Vec::new();
    let mut next = 1 + dir_sectors as u32;
    let mut starts = Vec::new();
    for (_, content) in streams {
        let padded = content.len().max(4096);
        let n = padded.div_ceil(ssz);
        starts.push((next, n, padded));
        let mut c = content.to_vec();
        c.resize(n * ssz, 0);
        data_sectors.push(c);
        next += n as u32;
    }
    let total_sectors = next as usize;
    assert!(total_sectors <= 128, "test builder: one FAT sector only");
    let mut fat = vec![FREESECT; 128];
    fat[0] = 0xFFFF_FFFD; // FATSECT
    for i in 0..dir_sectors {
        fat[1 + i] = if i + 1 == dir_sectors { ENDOFCHAIN } else { (2 + i) as u32 };
    }
    for &(s, n, _) in &starts {
        for k in 0..n {
            fat[s as usize + k] = if k + 1 == n { ENDOFCHAIN } else { s + k as u32 + 1 };
        }
    }
    let mut out = vec![0u8; 512];
    out[..8].copy_from_slice(&CFB_MAGIC);
    out[24..26].copy_from_slice(&0x3Eu16.to_le_bytes());
    out[26..28].copy_from_slice(&3u16.to_le_bytes());
    out[28..30].copy_from_slice(&0xFFFEu16.to_le_bytes());
    out[30..32].copy_from_slice(&9u16.to_le_bytes());
    out[32..34].copy_from_slice(&6u16.to_le_bytes());
    out[44..48].copy_from_slice(&1u32.to_le_bytes());
    out[48..52].copy_from_slice(&1u32.to_le_bytes());
    out[56..60].copy_from_slice(&4096u32.to_le_bytes());
    out[60..64].copy_from_slice(&ENDOFCHAIN.to_le_bytes());
    out[68..72].copy_from_slice(&ENDOFCHAIN.to_le_bytes());
    for i in 0..109 {
        let v = if i == 0 { 0u32 } else { FREESECT };
        out[76 + i * 4..80 + i * 4].copy_from_slice(&v.to_le_bytes());
    }
    for v in &fat {
        out.extend_from_slice(&v.to_le_bytes());
    }
    // Directory: root (0) with children 1..n chained as a right-leaning list.
    let mut dir = Vec::new();
    let entry = |name: &str, ty: u8, child: u32, right: u32, start: u32, size: u64| {
        let mut e = vec![0u8; 128];
        let units: Vec<u16> = name.encode_utf16().collect();
        for (i, u) in units.iter().take(31).enumerate() {
            e[i * 2..i * 2 + 2].copy_from_slice(&u.to_le_bytes());
        }
        e[64..66].copy_from_slice(&(((units.len().min(31) + 1) * 2) as u16).to_le_bytes());
        e[66] = ty;
        e[68..72].copy_from_slice(&NOSTREAM.to_le_bytes());
        e[72..76].copy_from_slice(&right.to_le_bytes());
        e[76..80].copy_from_slice(&child.to_le_bytes());
        e[116..120].copy_from_slice(&start.to_le_bytes());
        e[120..128].copy_from_slice(&size.to_le_bytes());
        e
    };
    let child_count = (streams.len() + storages.len()) as u32;
    dir.extend(entry("Root Entry", 5, if child_count > 0 { 1 } else { NOSTREAM }, NOSTREAM, ENDOFCHAIN, 0));
    let mut idx = 1u32;
    for (i, (name, content)) in streams.iter().enumerate() {
        let right = if idx < child_count { idx + 1 } else { NOSTREAM };
        dir.extend(entry(name, 2, NOSTREAM, right, starts[i].0, content.len().max(4096) as u64));
        idx += 1;
    }
    for name in storages {
        let right = if idx < child_count { idx + 1 } else { NOSTREAM };
        dir.extend(entry(name, 1, NOSTREAM, right, 0, 0));
        idx += 1;
    }
    dir.resize(dir_sectors * ssz, 0);
    // Unused directory slots must read as unallocated entries with NOSTREAM links.
    for slot in n_entries..dir_sectors * 4 {
        let o = slot * 128;
        for f in [68usize, 72, 76] {
            dir[o + f..o + f + 4].copy_from_slice(&NOSTREAM.to_le_bytes());
        }
    }
    out.extend_from_slice(&dir);
    for s in data_sectors {
        out.extend_from_slice(&s);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_root_streams_and_detects_encrypted_ooxml() {
        let f = build_test_file(
            &[("EncryptionInfo", b"\x04\x00\x04\x00"), ("EncryptedPackage", b"zz")],
            &["\u{6}DataSpaces"],
        );
        assert_eq!(stream_names(&f).unwrap(), ["EncryptedPackage", "EncryptionInfo"]);
        let mut cf = CompoundFile::open(Cursor::new(&f)).unwrap();
        assert_eq!(cf.protection().unwrap(), Some(Protection::EncryptedPackage));
        let s = cf.read_stream("EncryptionInfo", 1 << 20).unwrap().unwrap();
        assert_eq!(&s[..4], b"\x04\x00\x04\x00");
        assert!(cf.read_stream("missing", 10).unwrap().is_none());
        assert!(cf.read_stream("EncryptionInfo", 10).is_err(), "size cap");
    }

    #[test]
    fn legacy_word_and_excel_flags() {
        let mut fib = vec![0u8; 64];
        fib[0..2].copy_from_slice(&0xA5ECu16.to_le_bytes());
        fib[0x0A..0x0C].copy_from_slice(&0x0100u16.to_le_bytes());
        let f = build_test_file(&[("WordDocument", &fib)], &[]);
        assert_eq!(CompoundFile::open(Cursor::new(&f)).unwrap().protection().unwrap(), Some(Protection::LegacyWord));
        fib[0x0A] = 0;
        fib[0x0B] = 0;
        let f = build_test_file(&[("WordDocument", &fib)], &[]);
        assert_eq!(CompoundFile::open(Cursor::new(&f)).unwrap().protection().unwrap(), None);

        // BOF, FILEPASS, EOF.
        let mut wb = Vec::new();
        wb.extend_from_slice(&[0x09, 0x08, 4, 0, 0, 6, 5, 0]);
        wb.extend_from_slice(&[0x2F, 0x00, 2, 0, 1, 0]);
        wb.extend_from_slice(&[0x0A, 0x00, 0, 0]);
        let f = build_test_file(&[("Workbook", &wb)], &[]);
        assert_eq!(CompoundFile::open(Cursor::new(&f)).unwrap().protection().unwrap(), Some(Protection::LegacyExcel));
        let plain = build_test_file(&[("Workbook", &[0x09, 0x08, 0, 0, 0x0A, 0, 0, 0])], &[]);
        assert_eq!(CompoundFile::open(Cursor::new(&plain)).unwrap().protection().unwrap(), None);
        let ppt = build_test_file(&[("PowerPoint Document", b"x"), ("EncryptedSummary", b"y")], &[]);
        assert_eq!(
            CompoundFile::open(Cursor::new(&ppt)).unwrap().protection().unwrap(),
            Some(Protection::LegacyPowerPoint)
        );
    }

    #[test]
    fn malformed_files_fail_cleanly() {
        let good = build_test_file(&[("A", b"1"), ("B", b"2")], &["S"]);
        assert!(stream_names(&[]).is_err());
        assert!(stream_names(&good[..100]).is_err());
        for cut in (0..good.len()).step_by(37) {
            let _ = stream_names(&good[..cut]);
        }
        // Corrupt every header and FAT/directory byte (the data sectors are not structural).
        for i in 0..2048.min(good.len()) {
            for v in [0u8, 0xFF, 0x01] {
                let mut bad = good.clone();
                bad[i] = v;
                if let Ok(mut cf) = CompoundFile::open(Cursor::new(&bad)) {
                    let _ = cf.protection();
                    let _ = cf.read_stream("A", 1 << 16);
                }
            }
        }
        // A FAT that loops on itself is detected.
        let mut cyc = good.clone();
        let fat_off = 512;
        cyc[fat_off + 4..fat_off + 8].copy_from_slice(&1u32.to_le_bytes()); // dir sector 1 -> 1
        assert!(CompoundFile::open(Cursor::new(&cyc)).is_err());
        // A directory tree that loops on itself is detected.
        let mut loopy = good.clone();
        let dir_off = 1024;
        loopy[dir_off + 128 + 72..dir_off + 128 + 76].copy_from_slice(&1u32.to_le_bytes());
        let cf = CompoundFile::open(Cursor::new(&loopy)).unwrap();
        assert!(cf.root_entries().is_err());
    }

    #[test]
    fn magic() {
        assert!(is_compound_file(&build_test_file(&[], &[])));
        assert!(!is_compound_file(b"PK\x03\x04"));
    }
}
