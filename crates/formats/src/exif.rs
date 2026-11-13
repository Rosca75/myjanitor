//! EXIF / TIFF IFD parsing from bytes already in memory. Owner: W2-B.
//!
//! Reads the facts MyJanitor needs from the TIFF structure inside a JPEG APP1
//! segment (see [`crate::jpeg::exif_tiff`]) or a TIFF file header: orientation,
//! pixel dimensions, date taken, camera make/model, lens, description, GPS
//! presence and the location of the embedded JPEG thumbnail (IFD1).
//!
//! The parser is meant for **bounded header windows** of hostile files: every
//! offset and count is bounds- and overflow-checked, each IFD is visited at most
//! once (loop guard), entry counts are capped, only IFD0 -> IFD1 is followed in
//! the chain, and malformed sub-IFDs are ignored rather than failing the whole
//! parse. A thumbnail whose bytes lie outside the buffer is still *located*
//! (so a caller may widen its read) but [`Exif::thumbnail_bytes`] returns `None`.

use mj_core::{Error, Result, Timestamp};

/// Maximum entries read from one IFD (real files have well under 100).
pub const MAX_IFD_ENTRIES: usize = 1024;
/// Maximum length of a text value kept (longer values are truncated).
pub const MAX_TEXT: usize = 256;

const TAG_IMAGE_WIDTH: u16 = 0x0100;
const TAG_IMAGE_LENGTH: u16 = 0x0101;
const TAG_COMPRESSION: u16 = 0x0103;
const TAG_DESCRIPTION: u16 = 0x010E;
const TAG_MAKE: u16 = 0x010F;
const TAG_MODEL: u16 = 0x0110;
const TAG_ORIENTATION: u16 = 0x0112;
const TAG_DATETIME: u16 = 0x0132;
const TAG_THUMB_OFFSET: u16 = 0x0201;
const TAG_THUMB_LENGTH: u16 = 0x0202;
const TAG_EXIF_IFD: u16 = 0x8769;
const TAG_GPS_IFD: u16 = 0x8825;
const TAG_DATETIME_ORIGINAL: u16 = 0x9003;
const TAG_OFFSET_TIME_ORIGINAL: u16 = 0x9011;
const TAG_PIXEL_X: u16 = 0xA002;
const TAG_PIXEL_Y: u16 = 0xA003;
const TAG_LENS_MODEL: u16 = 0xA434;
const TAG_GPS_LATITUDE: u16 = 0x0002;
const TAG_GPS_LONGITUDE: u16 = 0x0004;

/// Location of the embedded JPEG thumbnail, relative to the start of the TIFF structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThumbnailLocation {
    pub offset: usize,
    pub len: usize,
}

/// The parsed facts. Every field is optional: absence is not an error.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Exif {
    /// TIFF orientation 1..=8 (values outside that range are dropped).
    pub orientation: Option<u16>,
    /// Pixel dimensions (Exif PixelX/YDimension, else IFD0 ImageWidth/Length).
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// DateTimeOriginal (else IFD0 DateTime). Interpreted as UTC unless
    /// OffsetTimeOriginal gives the offset, in which case it is converted to UTC.
    pub date_taken: Option<Timestamp>,
    pub make: Option<String>,
    pub model: Option<String>,
    pub lens: Option<String>,
    pub description: Option<String>,
    /// A GPS IFD with a non-zero latitude or longitude.
    pub has_gps: bool,
    /// IFD1 JPEG thumbnail, when declared.
    pub thumbnail: Option<ThumbnailLocation>,
}

impl Exif {
    /// "Make Model", without repeating the make when the model already starts with it.
    pub fn camera(&self) -> Option<String> {
        match (&self.make, &self.model) {
            (Some(m), Some(md)) if md.to_lowercase().starts_with(&m.to_lowercase()) => Some(md.clone()),
            (Some(m), Some(md)) => Some(format!("{m} {md}")),
            (Some(m), None) => Some(m.clone()),
            (None, Some(md)) => Some(md.clone()),
            (None, None) => None,
        }
    }

    /// The thumbnail bytes inside `tiff`, when they lie entirely within it and look like a JPEG.
    pub fn thumbnail_bytes<'a>(&self, tiff: &'a [u8]) -> Option<&'a [u8]> {
        let t = self.thumbnail?;
        let end = t.offset.checked_add(t.len)?;
        let b = tiff.get(t.offset..end)?;
        crate::jpeg::is_jpeg(b).then_some(b)
    }
}

#[derive(Clone, Copy)]
struct Reader<'a> {
    buf: &'a [u8],
    le: bool,
}

impl<'a> Reader<'a> {
    fn u16(&self, at: usize) -> Option<u16> {
        let b = self.buf.get(at..at.checked_add(2)?)?;
        Some(if self.le { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) })
    }

    fn u32(&self, at: usize) -> Option<u32> {
        let b = self.buf.get(at..at.checked_add(4)?)?;
        let a = [b[0], b[1], b[2], b[3]];
        Some(if self.le { u32::from_le_bytes(a) } else { u32::from_be_bytes(a) })
    }
}

/// One IFD entry with its value bytes resolved (inline or at the offset).
struct Entry<'a> {
    tag: u16,
    typ: u16,
    count: u32,
    /// Value bytes when they lie within the buffer.
    data: Option<&'a [u8]>,
    /// The raw 4-byte value/offset field (for LONG offsets to sub-IFDs).
    raw: u32,
}

fn type_size(typ: u16) -> Option<usize> {
    Some(match typ {
        1 | 2 | 6 | 7 => 1,
        3 | 8 => 2,
        4 | 9 | 11 | 13 => 4,
        5 | 10 | 12 => 8,
        _ => return None,
    })
}

impl<'a> Entry<'a> {
    fn uint(&self, r: &Reader<'_>) -> Option<u32> {
        if self.count == 0 {
            return None;
        }
        let d = self.data?;
        let rr = Reader { buf: d, le: r.le };
        match self.typ {
            1 => d.first().map(|&b| b as u32),
            3 => rr.u16(0).map(u32::from),
            4 => rr.u32(0),
            _ => None,
        }
    }

    fn text(&self) -> Option<String> {
        if self.typ != 2 && self.typ != 7 {
            return None;
        }
        let d = self.data?;
        let d = &d[..d.iter().position(|&b| b == 0).unwrap_or(d.len())];
        let s = String::from_utf8_lossy(&d[..d.len().min(MAX_TEXT)]);
        let s = s.trim();
        (!s.is_empty()).then(|| s.to_string())
    }

    /// True when any RATIONAL component has a non-zero numerator.
    fn nonzero_rational(&self, r: &Reader<'_>) -> bool {
        let Some(d) = self.data else { return false };
        if self.typ != 5 && self.typ != 10 {
            return false;
        }
        let rr = Reader { buf: d, le: r.le };
        (0..self.count.min(3) as usize).any(|i| rr.u32(i * 8).is_some_and(|n| n != 0))
    }
}

/// Parse the IFD at `offset`. Returns its entries and the next-IFD offset.
fn read_ifd<'a>(r: &Reader<'a>, offset: usize, visited: &mut Vec<usize>) -> Option<(Vec<Entry<'a>>, u32)> {
    if offset < 8 || visited.contains(&offset) || visited.len() > 16 {
        return None;
    }
    visited.push(offset);
    let n = r.u16(offset)? as usize;
    let n = n.min(MAX_IFD_ENTRIES);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let at = offset + 2 + i * 12;
        let (Some(tag), Some(typ), Some(count), Some(raw)) = (r.u16(at), r.u16(at + 2), r.u32(at + 4), r.u32(at + 8))
        else {
            // The IFD runs past the buffer: keep what was read.
            return Some((out, 0));
        };
        let data = type_size(typ).and_then(|sz| {
            let total = sz.checked_mul(count as usize)?;
            if total <= 4 {
                r.buf.get(at + 8..at + 8 + total)
            } else {
                let start = raw as usize;
                r.buf.get(start..start.checked_add(total)?)
            }
        });
        out.push(Entry { tag, typ, count, data, raw });
    }
    let next = r.u32(offset + 2 + n * 12).unwrap_or(0);
    Some((out, next))
}

/// Parse an EXIF date `"YYYY:MM:DD HH:MM:SS"` (also accepts `-` as the date separator).
pub fn parse_datetime(s: &str) -> Option<(i32, u32, u32, u32, u32, u32)> {
    let b = s.trim().as_bytes();
    if b.len() < 19 {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<u32> {
        let t = std::str::from_utf8(&b[r]).ok()?;
        if !t.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        t.parse().ok()
    };
    let sep_ok = matches!(b[4], b':' | b'-') && matches!(b[7], b':' | b'-') && matches!(b[10], b' ' | b'T');
    if !sep_ok || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let (y, mo, d, h, mi, se) = (num(0..4)?, num(5..7)?, num(8..10)?, num(11..13)?, num(14..16)?, num(17..19)?);
    if y == 0 || !(1..=12).contains(&mo) || d == 0 || d > mj_core::time::days_in_month(y as i32, mo) {
        return None;
    }
    if h > 23 || mi > 59 || se > 60 {
        return None;
    }
    Some((y as i32, mo, d, h, mi, se.min(59)))
}

/// Parse an EXIF offset `"+HH:MM"` / `"-HH:MM"` into seconds east of UTC.
fn parse_offset(s: &str) -> Option<i64> {
    let b = s.trim().as_bytes();
    if b.len() != 6 || b[3] != b':' {
        return None;
    }
    let sign = match b[0] {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let h: i64 = std::str::from_utf8(&b[1..3]).ok()?.parse().ok()?;
    let m: i64 = std::str::from_utf8(&b[4..6]).ok()?.parse().ok()?;
    (h <= 14 && m < 60).then_some(sign * (h * 3600 + m * 60))
}

fn to_timestamp(s: &str, offset: Option<i64>) -> Option<Timestamp> {
    let (y, mo, d, h, mi, se) = parse_datetime(s)?;
    let t = Timestamp::from_ymd_hms(y, mo, d, h, mi, se);
    Some(match offset {
        Some(o) => Timestamp(t.0 - o * mj_core::time::NANOS_PER_SEC),
        None => t,
    })
}

/// Parse a TIFF structure (`II*\0` or `MM\0*` header) held in `tiff`.
pub fn parse(tiff: &[u8]) -> Result<Exif> {
    let le = match tiff.get(0..4) {
        Some(b"II*\0") => true,
        Some(b"MM\0*") => false,
        _ => return Err(Error::parse("EXIF", "missing TIFF header")),
    };
    let r = Reader { buf: tiff, le };
    let ifd0_off = r.u32(4).ok_or_else(|| Error::parse("EXIF", "truncated TIFF header"))? as usize;
    let mut visited = Vec::new();
    let (ifd0, next) =
        read_ifd(&r, ifd0_off, &mut visited).ok_or_else(|| Error::parse("EXIF", "IFD0 offset out of range"))?;

    let mut x = Exif::default();
    let mut date_ifd0 = None;
    let mut exif_ifd = None;
    let mut gps_ifd = None;
    for e in &ifd0 {
        match e.tag {
            TAG_ORIENTATION => x.orientation = e.uint(&r).map(|v| v as u16).filter(|v| (1..=8).contains(v)),
            TAG_MAKE => x.make = e.text(),
            TAG_MODEL => x.model = e.text(),
            TAG_DESCRIPTION => x.description = e.text(),
            TAG_DATETIME => date_ifd0 = e.text(),
            TAG_IMAGE_WIDTH => x.width = x.width.or(e.uint(&r)),
            TAG_IMAGE_LENGTH => x.height = x.height.or(e.uint(&r)),
            TAG_EXIF_IFD if e.count >= 1 => exif_ifd = Some(e.raw as usize),
            TAG_GPS_IFD if e.count >= 1 => gps_ifd = Some(e.raw as usize),
            _ => {}
        }
    }

    let mut date_orig = None;
    let mut offset_orig = None;
    if let Some((entries, _)) = exif_ifd.and_then(|o| read_ifd(&r, o, &mut visited)) {
        let (mut px, mut py) = (None, None);
        for e in &entries {
            match e.tag {
                TAG_DATETIME_ORIGINAL => date_orig = e.text(),
                TAG_OFFSET_TIME_ORIGINAL => offset_orig = e.text().and_then(|s| parse_offset(&s)),
                TAG_PIXEL_X => px = e.uint(&r),
                TAG_PIXEL_Y => py = e.uint(&r),
                TAG_LENS_MODEL => x.lens = e.text(),
                _ => {}
            }
        }
        if let (Some(w), Some(h)) = (px, py) {
            if w > 0 && h > 0 {
                x.width = Some(w);
                x.height = Some(h);
            }
        }
    }
    x.date_taken = date_orig
        .as_deref()
        .and_then(|s| to_timestamp(s, offset_orig))
        .or_else(|| date_ifd0.as_deref().and_then(|s| to_timestamp(s, None)));

    if let Some((entries, _)) = gps_ifd.and_then(|o| read_ifd(&r, o, &mut visited)) {
        x.has_gps =
            entries.iter().any(|e| matches!(e.tag, TAG_GPS_LATITUDE | TAG_GPS_LONGITUDE) && e.nonzero_rational(&r));
    }

    if next != 0 {
        if let Some((ifd1, _)) = read_ifd(&r, next as usize, &mut visited) {
            let mut off = None;
            let mut len = None;
            let mut compression = None;
            for e in &ifd1 {
                match e.tag {
                    TAG_THUMB_OFFSET => off = e.uint(&r),
                    TAG_THUMB_LENGTH => len = e.uint(&r),
                    TAG_COMPRESSION => compression = e.uint(&r),
                    _ => {}
                }
            }
            if let (Some(o), Some(l)) = (off, len) {
                // Compression 6 = old-style JPEG; absent is tolerated (many writers omit it).
                if l > 0 && o > 0 && compression.is_none_or(|c| c == 6) {
                    x.thumbnail = Some(ThumbnailLocation { offset: o as usize, len: l as usize });
                }
            }
        }
    }
    Ok(x)
}

/// Test-support builder for hand-made TIFF/EXIF buffers (also used by `mj-dupes` tests).
#[doc(hidden)]
pub mod build {
    /// A value of an IFD entry.
    #[derive(Debug, Clone)]
    pub enum Value {
        Short(u16),
        Long(u32),
        Ascii(String),
        /// Unsigned rationals (numerator, denominator).
        Rational(Vec<(u32, u32)>),
        /// A sub-IFD pointer (`Long`) whose target is the given IFD.
        SubIfd(Vec<(u16, Value)>),
        /// IFD1 thumbnail: emits JPEGInterchangeFormat + Length tags pointing at the bytes.
        Thumbnail(Vec<u8>),
    }

    /// Build a little-endian TIFF with IFD0 = `ifd0` and optional IFD1 = `ifd1`.
    pub fn tiff(ifd0: &[(u16, Value)], ifd1: Option<&[(u16, Value)]>) -> Vec<u8> {
        let mut out = b"II*\0".to_vec();
        out.extend_from_slice(&8u32.to_le_bytes());
        let next_patch = write_ifd(&mut out, ifd0);
        if let Some(ifd1) = ifd1 {
            let at = out.len() as u32;
            out[next_patch..next_patch + 4].copy_from_slice(&at.to_le_bytes());
            write_ifd(&mut out, ifd1);
        }
        out
    }

    /// Writes an IFD at the end of `out` (data after it); returns the position of its next-IFD field.
    fn write_ifd(out: &mut Vec<u8>, entries: &[(u16, Value)]) -> usize {
        let mut flat: Vec<(u16, Value)> = Vec::new();
        for (tag, v) in entries {
            if let Value::Thumbnail(b) = v {
                flat.push((0x0103, Value::Short(6)));
                flat.push((0x0201, Value::Thumbnail(b.clone())));
                flat.push((0x0202, Value::Long(b.len() as u32)));
            } else {
                flat.push((*tag, v.clone()));
            }
        }
        flat.sort_by_key(|(t, _)| *t);
        out.extend_from_slice(&(flat.len() as u16).to_le_bytes());
        let entries_at = out.len();
        out.resize(entries_at + flat.len() * 12 + 4, 0);
        let next_field = entries_at + flat.len() * 12;
        for (i, (tag, v)) in flat.iter().enumerate() {
            let at = entries_at + i * 12;
            let (typ, count, inline, data): (u16, u32, Option<[u8; 4]>, Vec<u8>) = match v {
                Value::Short(s) => (3, 1, Some([s.to_le_bytes()[0], s.to_le_bytes()[1], 0, 0]), vec![]),
                Value::Long(l) => (4, 1, Some(l.to_le_bytes()), vec![]),
                Value::Ascii(s) => {
                    let mut b = s.as_bytes().to_vec();
                    b.push(0);
                    if b.len() <= 4 {
                        let mut a = [0u8; 4];
                        a[..b.len()].copy_from_slice(&b);
                        (2, b.len() as u32, Some(a), vec![])
                    } else {
                        (2, b.len() as u32, None, b)
                    }
                }
                Value::Rational(r) => {
                    let mut b = Vec::new();
                    for (n, d) in r {
                        b.extend_from_slice(&n.to_le_bytes());
                        b.extend_from_slice(&d.to_le_bytes());
                    }
                    (5, r.len() as u32, None, b)
                }
                Value::Thumbnail(b) => (4, 1, None, b.clone()),
                Value::SubIfd(_) => (4, 1, None, vec![]),
            };
            out[at..at + 2].copy_from_slice(&tag.to_le_bytes());
            out[at + 2..at + 4].copy_from_slice(&typ.to_le_bytes());
            out[at + 4..at + 8].copy_from_slice(&count.to_le_bytes());
            let value_field = match (inline, v) {
                (Some(a), _) => a,
                (None, Value::SubIfd(sub)) => {
                    let pos = out.len() as u32;
                    write_ifd(out, sub);
                    pos.to_le_bytes()
                }
                (None, _) => {
                    let pos = out.len() as u32;
                    out.extend_from_slice(&data);
                    pos.to_le_bytes()
                }
            };
            out[at + 8..at + 12].copy_from_slice(&value_field);
        }
        next_field
    }

    /// Wrap a TIFF as a JPEG APP1 segment (`FF E1 len "Exif\0\0" tiff`).
    pub fn app1(tiff: &[u8]) -> Vec<u8> {
        let mut v = vec![0xFF, 0xE1];
        v.extend_from_slice(&((tiff.len() + 8) as u16).to_be_bytes());
        v.extend_from_slice(b"Exif\0\0");
        v.extend_from_slice(tiff);
        v
    }

    /// Insert an APP1 segment right after the SOI marker of `jpeg`.
    pub fn insert_app1(jpeg: &[u8], app1: &[u8]) -> Vec<u8> {
        let mut v = jpeg[..2.min(jpeg.len())].to_vec();
        v.extend_from_slice(app1);
        v.extend_from_slice(jpeg.get(2..).unwrap_or(&[]));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::build::{tiff, Value};
    use super::*;

    fn fake_thumb() -> Vec<u8> {
        let mut t = vec![0xFF, 0xD8, 0xFF, 0xDB, 0, 4, 1, 2];
        t.extend_from_slice(&[0xFF, 0xD9]);
        t
    }

    fn full() -> Vec<u8> {
        tiff(
            &[
                (TAG_MAKE, Value::Ascii("Canon".into())),
                (TAG_MODEL, Value::Ascii("Canon EOS 5D".into())),
                (TAG_ORIENTATION, Value::Short(6)),
                (TAG_DESCRIPTION, Value::Ascii("Team offsite".into())),
                (TAG_DATETIME, Value::Ascii("2020:01:01 00:00:00".into())),
                (
                    TAG_EXIF_IFD,
                    Value::SubIfd(vec![
                        (TAG_DATETIME_ORIGINAL, Value::Ascii("2024:05:01 10:20:30".into())),
                        (TAG_OFFSET_TIME_ORIGINAL, Value::Ascii("+02:00".into())),
                        (TAG_PIXEL_X, Value::Long(4000)),
                        (TAG_PIXEL_Y, Value::Short(3000)),
                        (TAG_LENS_MODEL, Value::Ascii("EF 24-70mm".into())),
                    ]),
                ),
                (
                    TAG_GPS_IFD,
                    Value::SubIfd(vec![(TAG_GPS_LATITUDE, Value::Rational(vec![(48, 1), (51, 1), (0, 1)]))]),
                ),
            ],
            Some(&[(0, Value::Thumbnail(fake_thumb()))]),
        )
    }

    #[test]
    fn parses_every_field() {
        let t = full();
        let x = parse(&t).unwrap();
        assert_eq!(x.orientation, Some(6));
        assert_eq!((x.width, x.height), (Some(4000), Some(3000)));
        assert_eq!(x.camera().as_deref(), Some("Canon EOS 5D"));
        assert_eq!(x.lens.as_deref(), Some("EF 24-70mm"));
        assert_eq!(x.description.as_deref(), Some("Team offsite"));
        assert!(x.has_gps);
        // 10:20:30 at +02:00 is 08:20:30 UTC.
        assert_eq!(x.date_taken, Some(Timestamp::from_ymd_hms(2024, 5, 1, 8, 20, 30)));
        assert_eq!(x.thumbnail_bytes(&t).unwrap(), &fake_thumb()[..]);
    }

    #[test]
    fn big_endian_and_fallbacks() {
        // Hand-written big-endian TIFF: IFD0 with Make (inline "Sony") and DateTime.
        let mut t = b"MM\0*\0\0\0\x08".to_vec();
        t.extend_from_slice(&2u16.to_be_bytes());
        // Make: ASCII, count 4 ("Abc\0") inline.
        t.extend_from_slice(&[0x01, 0x0F, 0, 2, 0, 0, 0, 4, b'A', b'b', b'c', 0]);
        // DateTime: ASCII count 20 at offset 38.
        t.extend_from_slice(&[0x01, 0x32, 0, 2, 0, 0, 0, 20, 0, 0, 0, 38]);
        t.extend_from_slice(&[0, 0, 0, 0]);
        t.extend_from_slice(b"2021:12:31 23:59:59\0");
        let x = parse(&t).unwrap();
        assert_eq!(x.make.as_deref(), Some("Abc"));
        assert_eq!(x.camera().as_deref(), Some("Abc"));
        assert_eq!(x.date_taken, Some(Timestamp::from_ymd_hms(2021, 12, 31, 23, 59, 59)));
        assert!(x.thumbnail.is_none() && !x.has_gps);
    }

    #[test]
    fn malformed_buffers_never_panic() {
        assert!(parse(b"").is_err());
        assert!(parse(b"II*\0").is_err());
        assert!(parse(b"II*\0\xff\xff\xff\xff").is_err());
        assert!(parse(b"XX*\0\x08\0\0\0").is_err());
        let t = full();
        for cut in 0..t.len() {
            let _ = parse(&t[..cut]);
        }
        // Flip every byte in turn.
        for i in 0..t.len() {
            let mut m = t.clone();
            m[i] ^= 0xFF;
            let _ = parse(&m);
        }
        // Self-referencing IFD chain and sub-IFD loops terminate.
        let mut lp = b"II*\0\x08\0\0\0".to_vec();
        lp.extend_from_slice(&1u16.to_le_bytes());
        lp.extend_from_slice(&[0x69, 0x87, 4, 0, 1, 0, 0, 0, 8, 0, 0, 0]); // Exif IFD -> IFD0 itself
        lp.extend_from_slice(&8u32.to_le_bytes()); // next IFD -> itself
        let x = parse(&lp).unwrap();
        assert!(x.thumbnail.is_none());
        // Huge counts / offsets.
        let mut big = b"II*\0\x08\0\0\0".to_vec();
        big.extend_from_slice(&2u16.to_le_bytes());
        big.extend_from_slice(&[0x0F, 0x01, 2, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0x10]);
        big.extend_from_slice(&[0x01, 0x02, 5, 0, 0xFF, 0xFF, 0xFF, 0x7F, 0xFF, 0xFF, 0xFF, 0xFF]);
        big.extend_from_slice(&0u32.to_le_bytes());
        let x = parse(&big).unwrap();
        assert!(x.make.is_none());
        // Pseudo-random garbage behind a valid header.
        let mut s: u32 = 7;
        for _ in 0..300 {
            let mut g = b"II*\0\x08\0\0\0".to_vec();
            for _ in 0..200 {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                g.push((s >> 24) as u8);
            }
            let _ = parse(&g);
        }
    }

    #[test]
    fn thumbnail_outside_window_is_located_not_returned() {
        let t = full();
        let x = parse(&t).unwrap();
        let loc = x.thumbnail.unwrap();
        let cut = &t[..loc.offset + 2];
        let y = parse(cut).unwrap();
        assert_eq!(y.thumbnail, Some(loc));
        assert!(y.thumbnail_bytes(cut).is_none());
    }

    #[test]
    fn dates_are_validated() {
        assert!(parse_datetime("0000:00:00 00:00:00").is_none());
        assert!(parse_datetime("2023:02:29 10:00:00").is_none());
        assert!(parse_datetime("2024:02:29 10:00:00").is_some());
        assert!(parse_datetime("2024:13:01 10:00:00").is_none());
        assert!(parse_datetime("    :  :     :  :  ").is_none());
        assert!(parse_datetime("2024-05-01T10:00:00").is_some());
        assert_eq!(parse_offset("-05:30"), Some(-(5 * 3600 + 30 * 60)));
        assert_eq!(parse_offset("+99:00"), None);
    }

    #[test]
    fn app1_round_trip_through_jpeg_walker() {
        let t = full();
        let jpeg = [0xFF, 0xD8, 0xFF, 0xDA, 0, 2, 0xFF, 0xD9];
        let f = build::insert_app1(&jpeg, &build::app1(&t));
        assert_eq!(crate::jpeg::exif_tiff(&f).unwrap(), &t[..]);
    }
}
