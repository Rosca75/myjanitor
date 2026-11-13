//! On-disk cache format (W2-A), little-endian, versioned, checksummed:
//!
//! ```text
//! header:  magic "MJCACHE\0" (8) | u32 format_version | u32 algo_version
//!          (= mj_core::fingerprint::FINGERPRINT_ALGO_VERSION) | u64 entry_count
//! record:  u32 key_len | key bytes (path_key, UTF-8)
//!          u64 size | i64 mtime_ns | u16 presence bits
//!          then the present fields, in bit order:
//!          bit 0 partial u64       bit 5 width u32
//!          bit 1 full u64          bit 6 height u32
//!          bit 2 sha256 [u8; 32]   bit 7 exif: u8 parts, then i64 date (parts bit 0),
//!          bit 3 dhash u64                     u16 len + camera (bit 1), u16 len + lens (bit 2);
//!          bit 4 phash u64                     parts bit 3 = has_gps, bit 4 = has_description
//!                                  bit 8 flags u32 (FingerprintFlags)
//!                                  bit 9 file id: u64 volume, u64 index
//! trailer: u64 xxh64 (seed 0) of every byte before it
//! ```
//!
//! Records are written sorted by key (deterministic files). Bad magic, other
//! versions, short reads, unknown presence bits, invalid UTF-8, trailing bytes
//! or a trailer mismatch -> the decoder returns an error and the cache starts
//! empty. Nothing is ever partially loaded.

use std::io::{self, Write};

use mj_core::fingerprint::{ExifSummary, FingerprintFlags, Fingerprints, FINGERPRINT_ALGO_VERSION};
use mj_core::hash::xxh64::{xxh64, Xxh64};
use mj_core::{FileId, Timestamp};

/// File magic.
pub const MAGIC: &[u8; 8] = b"MJCACHE\0";
/// Container format version (bump on layout change).
pub const FORMAT_VERSION: u32 = 1;

const HEADER_LEN: usize = 8 + 4 + 4 + 8;
const TRAILER_LEN: usize = 8;
/// Smallest possible record: key_len + size + mtime + presence.
const MIN_RECORD: usize = 4 + 8 + 8 + 2;
const KNOWN_BITS: u16 = (1 << 10) - 1;

/// One cache entry: validity stamp, optional identity, optional fingerprints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheRecord {
    pub size: u64,
    pub mtime_ns: i64,
    pub file_id: Option<FileId>,
    /// `None` = stamp only (seen by a scan, no content stage computed yet).
    pub fp: Option<Box<Fingerprints>>,
    /// In memory only: the scan epoch that last saw this file (pruning). Not persisted.
    pub seen: u32,
}

impl CacheRecord {
    /// A stamp-only record.
    pub fn stamp(size: u64, mtime: Timestamp, file_id: Option<FileId>) -> Self {
        CacheRecord { size, mtime_ns: mtime.0, file_id, fp: None, seen: 0 }
    }
}

/// True when `fp` holds nothing worth storing.
pub fn is_empty_fp(fp: &Fingerprints) -> bool {
    *fp == Fingerprints::default()
}

/// Streams bytes to a writer while hashing them.
struct HashingWriter<W: Write> {
    inner: W,
    hash: Xxh64,
}

impl<W: Write> Write for HashingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hash.update(&buf[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn put_str(w: &mut impl Write, s: &Option<String>) -> io::Result<()> {
    let s = s.as_deref().unwrap_or("");
    // Bounded to u16; cut at a char boundary.
    let mut end = s.len().min(u16::MAX as usize);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    w.write_all(&(end as u16).to_le_bytes())?;
    w.write_all(&s.as_bytes()[..end])
}

fn write_record(w: &mut impl Write, key: &str, r: &CacheRecord) -> io::Result<()> {
    w.write_all(&(key.len() as u32).to_le_bytes())?;
    w.write_all(key.as_bytes())?;
    w.write_all(&r.size.to_le_bytes())?;
    w.write_all(&r.mtime_ns.to_le_bytes())?;
    let empty = Fingerprints::default();
    let fp = r.fp.as_deref().unwrap_or(&empty);
    let mut bits = 0u16;
    let present = [
        fp.partial.is_some(),
        fp.full.is_some(),
        fp.sha256.is_some(),
        fp.dhash.is_some(),
        fp.phash.is_some(),
        fp.width.is_some(),
        fp.height.is_some(),
        fp.exif.is_some(),
        !fp.flags.is_empty(),
        r.file_id.is_some(),
    ];
    for (i, p) in present.iter().enumerate() {
        if *p {
            bits |= 1 << i;
        }
    }
    w.write_all(&bits.to_le_bytes())?;
    for v in [fp.partial, fp.full].into_iter().flatten() {
        w.write_all(&v.to_le_bytes())?;
    }
    if let Some(s) = &fp.sha256 {
        w.write_all(s)?;
    }
    for v in [fp.dhash, fp.phash].into_iter().flatten() {
        w.write_all(&v.to_le_bytes())?;
    }
    for v in [fp.width, fp.height].into_iter().flatten() {
        w.write_all(&v.to_le_bytes())?;
    }
    if let Some(x) = &fp.exif {
        let parts = u8::from(x.date_taken.is_some())
            | u8::from(x.camera.is_some()) << 1
            | u8::from(x.lens.is_some()) << 2
            | u8::from(x.has_gps) << 3
            | u8::from(x.has_description) << 4;
        w.write_all(&[parts])?;
        if let Some(d) = x.date_taken {
            w.write_all(&d.0.to_le_bytes())?;
        }
        if x.camera.is_some() {
            put_str(w, &x.camera)?;
        }
        if x.lens.is_some() {
            put_str(w, &x.lens)?;
        }
    }
    if !fp.flags.is_empty() {
        w.write_all(&fp.flags.bits().to_le_bytes())?;
    }
    if let Some(id) = r.file_id {
        w.write_all(&id.volume.to_le_bytes())?;
        w.write_all(&id.index.to_le_bytes())?;
    }
    Ok(())
}

/// Write a whole cache file: header, `records` (must be sorted by key for
/// deterministic output), trailer.
pub fn encode<'a>(
    w: &mut impl Write,
    count: u64,
    records: impl Iterator<Item = (&'a str, &'a CacheRecord)>,
) -> io::Result<()> {
    let mut hw = HashingWriter { inner: w, hash: Xxh64::new(0) };
    hw.write_all(MAGIC)?;
    hw.write_all(&FORMAT_VERSION.to_le_bytes())?;
    hw.write_all(&FINGERPRINT_ALGO_VERSION.to_le_bytes())?;
    hw.write_all(&count.to_le_bytes())?;
    let mut written = 0u64;
    for (k, r) in records {
        write_record(&mut hw, k, r)?;
        written += 1;
    }
    if written != count {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "record count mismatch"));
    }
    let sum = hw.hash.finish();
    hw.inner.write_all(&sum.to_le_bytes())
}

/// Bounds-checked little-endian reader.
struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.b.len()).ok_or("truncated record")?;
        let s = &self.b[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().map_err(|_| "u16")?))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().map_err(|_| "u32")?))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().map_err(|_| "u64")?))
    }
    fn string(&mut self, n: usize) -> Result<String, String> {
        std::str::from_utf8(self.take(n)?).map(str::to_string).map_err(|_| "invalid UTF-8".to_string())
    }
}

/// Decode a whole cache file. Any problem is an error (the caller then starts empty).
pub fn decode(bytes: &[u8]) -> Result<Vec<(Box<str>, CacheRecord)>, String> {
    if bytes.len() < HEADER_LEN + TRAILER_LEN {
        return Err("file too short".into());
    }
    let (body, trailer) = bytes.split_at(bytes.len() - TRAILER_LEN);
    let want = u64::from_le_bytes(trailer.try_into().map_err(|_| "trailer")?);
    if xxh64(body, 0) != want {
        return Err("checksum mismatch (truncated or corrupted file)".into());
    }
    let mut r = Reader { b: body, pos: 0 };
    if r.take(8)? != MAGIC {
        return Err("not a MyJanitor cache file".into());
    }
    let (fv, av) = (r.u32()?, r.u32()?);
    if fv != FORMAT_VERSION {
        return Err(format!("format version {fv}, expected {FORMAT_VERSION}"));
    }
    if av != FINGERPRINT_ALGO_VERSION {
        return Err(format!("fingerprint version {av}, expected {FINGERPRINT_ALGO_VERSION}"));
    }
    let count = r.u64()?;
    let max = (body.len() - HEADER_LEN) / MIN_RECORD;
    if count > max as u64 {
        return Err(format!("entry count {count} exceeds the file size"));
    }
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let klen = r.u32()? as usize;
        let key = r.string(klen)?;
        let size = r.u64()?;
        let mtime_ns = r.u64()? as i64;
        let bits = r.u16()?;
        if bits & !KNOWN_BITS != 0 {
            return Err(format!("unknown presence bits 0x{bits:x}"));
        }
        let has = |i: u16| bits & (1 << i) != 0;
        let mut fp = Fingerprints::default();
        if has(0) {
            fp.partial = Some(r.u64()?);
        }
        if has(1) {
            fp.full = Some(r.u64()?);
        }
        if has(2) {
            fp.sha256 = Some(r.take(32)?.try_into().map_err(|_| "sha256")?);
        }
        if has(3) {
            fp.dhash = Some(r.u64()?);
        }
        if has(4) {
            fp.phash = Some(r.u64()?);
        }
        if has(5) {
            fp.width = Some(r.u32()?);
        }
        if has(6) {
            fp.height = Some(r.u32()?);
        }
        if has(7) {
            let parts = r.u8()?;
            if parts & !0x1f != 0 {
                return Err("unknown EXIF parts".into());
            }
            let mut x = ExifSummary { has_gps: parts & 8 != 0, has_description: parts & 16 != 0, ..Default::default() };
            if parts & 1 != 0 {
                x.date_taken = Some(Timestamp(r.u64()? as i64));
            }
            if parts & 2 != 0 {
                let n = r.u16()? as usize;
                x.camera = Some(r.string(n)?);
            }
            if parts & 4 != 0 {
                let n = r.u16()? as usize;
                x.lens = Some(r.string(n)?);
            }
            fp.exif = Some(x);
        }
        if has(8) {
            let raw = r.u32()?;
            fp.flags = FingerprintFlags::from_bits_truncate(raw);
            if fp.flags.bits() != raw {
                return Err("unknown fingerprint flags".into());
            }
        }
        let file_id = if has(9) { Some(FileId { volume: r.u64()?, index: r.u64()? }) } else { None };
        let fp = (!is_empty_fp(&fp)).then(|| Box::new(fp));
        out.push((key.into_boxed_str(), CacheRecord { size, mtime_ns, file_id, fp, seen: 0 }));
    }
    if r.pos != body.len() {
        return Err("trailing bytes after the last record".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_record() -> CacheRecord {
        CacheRecord {
            size: 123_456,
            mtime_ns: -5,
            file_id: Some(FileId { volume: 0x801, index: 99 }),
            fp: Some(Box::new(Fingerprints {
                partial: Some(1),
                full: Some(u64::MAX),
                sha256: Some([7u8; 32]),
                dhash: Some(0x1234),
                phash: Some(0x5678),
                width: Some(4032),
                height: Some(3024),
                exif: Some(ExifSummary {
                    date_taken: Some(Timestamp(1_600_000_000_000_000_000)),
                    camera: Some("Apple iPhone 15 Pro".into()),
                    lens: None,
                    has_gps: true,
                    has_description: false,
                }),
                flags: FingerprintFlags::FROM_THUMBNAIL,
            })),
            seen: 0,
        }
    }

    fn encode_vec(recs: &[(&str, CacheRecord)]) -> Vec<u8> {
        let mut out = Vec::new();
        encode(&mut out, recs.len() as u64, recs.iter().map(|(k, r)| (*k, r))).unwrap();
        out
    }

    #[test]
    fn round_trips_every_field_and_stamp_only_records() {
        let recs = vec![
            ("/a/b.jpg", full_record()),
            ("/a/c", CacheRecord::stamp(0, Timestamp(0), None)),
            (
                "/a/é ü",
                CacheRecord {
                    fp: Some(Box::new(Fingerprints { partial: Some(0), ..Default::default() })),
                    ..CacheRecord::stamp(5, Timestamp(9), None)
                },
            ),
        ];
        let bytes = encode_vec(&recs);
        let back = decode(&bytes).unwrap();
        assert_eq!(back.len(), 3);
        for ((k, r), (k2, r2)) in recs.iter().zip(&back) {
            assert_eq!(*k, &**k2);
            assert_eq!(r, r2);
        }
        // A zero hash is a real value, not "absent" (no sentinels).
        assert_eq!(back[2].1.fp.as_ref().unwrap().partial, Some(0));
        assert_eq!(decode(&encode_vec(&[])).unwrap().len(), 0);
    }

    #[test]
    fn long_strings_are_truncated_at_char_boundaries() {
        let mut r = full_record();
        r.fp.as_mut().unwrap().exif.as_mut().unwrap().lens = Some("é".repeat(40_000));
        let back = decode(&encode_vec(&[("/k", r)])).unwrap();
        let lens = back[0].1.fp.as_ref().unwrap().exif.as_ref().unwrap().lens.clone().unwrap();
        assert!(lens.len() <= u16::MAX as usize && lens.chars().all(|c| c == 'é'));
    }

    #[test]
    fn every_corruption_is_rejected() {
        let good = encode_vec(&[("/a", full_record()), ("/b", CacheRecord::stamp(1, Timestamp(2), None))]);
        assert!(decode(&good).is_ok());
        // Every truncation.
        for n in 0..good.len() {
            assert!(decode(&good[..n]).is_err(), "truncated to {n}");
        }
        // Every single-byte flip.
        for i in 0..good.len() {
            let mut b = good.clone();
            b[i] ^= 0x40;
            assert!(decode(&b).is_err(), "flip at {i}");
        }
        // Valid checksum but wrong version / magic / bits: rebuilt with a fresh trailer.
        let reseal = |mut body: Vec<u8>| {
            body.truncate(body.len() - 8);
            let s = xxh64(&body, 0);
            body.extend_from_slice(&s.to_le_bytes());
            body
        };
        let mut v = good.clone();
        v[8..12].copy_from_slice(&2u32.to_le_bytes());
        assert!(decode(&reseal(v)).unwrap_err().contains("format version"));
        let mut a = good.clone();
        a[12..16].copy_from_slice(&(FINGERPRINT_ALGO_VERSION + 1).to_le_bytes());
        assert!(decode(&reseal(a)).unwrap_err().contains("fingerprint version"));
        let mut m = good.clone();
        m[0] = b'X';
        assert!(decode(&reseal(m)).is_err());
        let mut c = good.clone();
        c[16..24].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(decode(&reseal(c)).is_err());
        let mut extra = good.clone();
        extra.splice(good.len() - 8..good.len() - 8, [0u8; 3]);
        assert!(decode(&reseal(extra)).unwrap_err().contains("trailing"));
        assert!(decode(b"").is_err());
    }
}
