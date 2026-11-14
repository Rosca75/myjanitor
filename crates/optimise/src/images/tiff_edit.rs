//! Minimal in-place TIFF/EXIF editing for the JPEG metadata policies: remove the GPS IFD (pointer
//! entry, table and out-of-line values), remove the embedded IFD1 thumbnail, and build a tiny
//! orientation-only EXIF. Offsets inside a TIFF are absolute from its header, so nothing is moved:
//! removed regions are zeroed and, when they sit at the very end, cut off.
//!
//! Hardening: every offset and length is bounds-checked, a region that overlaps the header or IFD0
//! itself makes the edit fail (the caller then falls back to a safer, larger removal), and nothing
//! here panics on malformed input.

use std::ops::Range;

use mj_core::{Error, Result};

const TAG_GPS_POINTER: u16 = 0x8825;
const TAG_THUMB_OFFSET: u16 = 0x0201;
const TAG_THUMB_LENGTH: u16 = 0x0202;

struct Order(bool);

impl Order {
    fn u16(&self, b: &[u8], at: usize) -> Option<u16> {
        let s: [u8; 2] = b.get(at..at.checked_add(2)?)?.try_into().ok()?;
        Some(if self.0 { u16::from_le_bytes(s) } else { u16::from_be_bytes(s) })
    }
    fn u32(&self, b: &[u8], at: usize) -> Option<u32> {
        let s: [u8; 4] = b.get(at..at.checked_add(4)?)?.try_into().ok()?;
        Some(if self.0 { u32::from_le_bytes(s) } else { u32::from_be_bytes(s) })
    }
    fn put_u16(&self, v: u16) -> [u8; 2] {
        if self.0 {
            v.to_le_bytes()
        } else {
            v.to_be_bytes()
        }
    }
    fn put_u32(&self, v: u32) -> [u8; 4] {
        if self.0 {
            v.to_le_bytes()
        } else {
            v.to_be_bytes()
        }
    }
}

/// An IFD located in the buffer.
struct Ifd {
    /// Offset of the entry count.
    at: usize,
    count: usize,
}

impl Ifd {
    fn entries(&self) -> usize {
        self.at + 2
    }
    /// Offset of the next-IFD pointer.
    fn next_at(&self) -> usize {
        self.entries() + 12 * self.count
    }
    /// The whole table: count, entries, next pointer.
    fn span(&self) -> Range<usize> {
        self.at..self.next_at() + 4
    }
}

fn header(t: &[u8]) -> Result<(Order, usize)> {
    let o = match t.get(..4) {
        Some(b"II*\0") => Order(true),
        Some(b"MM\0*") => Order(false),
        _ => return Err(Error::parse("EXIF", "not a TIFF header")),
    };
    let off = o.u32(t, 4).ok_or_else(|| Error::parse("EXIF", "truncated TIFF header"))? as usize;
    Ok((o, off))
}

fn ifd_at(o: &Order, t: &[u8], at: usize) -> Result<Ifd> {
    let count = o.u16(t, at).ok_or_else(|| Error::parse("EXIF", "IFD offset out of range"))? as usize;
    let ifd = Ifd { at, count };
    if at < 8 || ifd.span().end > t.len() {
        return Err(Error::parse("EXIF", "IFD does not fit in the segment"));
    }
    Ok(ifd)
}

fn type_size(t: u16) -> usize {
    match t {
        1 | 2 | 6 | 7 => 1,
        3 | 8 => 2,
        4 | 9 | 11 => 4,
        5 | 10 | 12 => 8,
        _ => 1,
    }
}

/// Out-of-line value range of entry `e` (offset of the entry itself), if it has one inside the buffer.
fn value_range(o: &Order, t: &[u8], e: usize) -> Option<Range<usize>> {
    let (typ, count) = (o.u16(t, e + 2)?, o.u32(t, e + 4)? as usize);
    let size = type_size(typ).checked_mul(count)?;
    if size <= 4 {
        return None;
    }
    let off = o.u32(t, e + 8)? as usize;
    let end = off.checked_add(size)?;
    (end <= t.len() && off >= 8).then_some(off..end)
}

fn overlaps(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

/// Zero the regions and cut any that end the buffer.
fn erase(t: &mut Vec<u8>, mut regions: Vec<Range<usize>>) {
    for r in &regions {
        t[r.clone()].fill(0);
    }
    while let Some(i) = regions.iter().position(|r| r.end == t.len()) {
        let r = regions.swap_remove(i);
        t.truncate(r.start);
    }
}

fn check_clear(regions: &[Range<usize>], protected: &[Range<usize>]) -> Result<()> {
    for r in regions {
        if r.start < 8 || protected.iter().any(|p| overlaps(r, p)) {
            return Err(Error::parse("EXIF", "removed data overlaps the TIFF structure"));
        }
    }
    Ok(())
}

/// Remove the GPS IFD. Returns whether anything changed.
pub fn strip_gps(t: &mut Vec<u8>) -> Result<bool> {
    let (o, off0) = header(t)?;
    let ifd0 = ifd_at(&o, t, off0)?;
    let Some(k) = (0..ifd0.count).find(|&i| o.u16(t, ifd0.entries() + 12 * i) == Some(TAG_GPS_POINTER)) else {
        return Ok(false);
    };
    let entry = ifd0.entries() + 12 * k;
    let gps_off = o.u32(t, entry + 8).ok_or_else(|| Error::parse("EXIF", "bad GPS pointer"))? as usize;
    let mut regions = Vec::new();
    if gps_off != 0 {
        let gps = ifd_at(&o, t, gps_off)?;
        regions.push(gps.span());
        for i in 0..gps.count {
            if let Some(r) = value_range(&o, t, gps.entries() + 12 * i) {
                regions.push(r);
            }
        }
        check_clear(&regions, &[0..8, ifd0.span()])?;
    }
    // Unlink: shift the later entries and the next pointer up by one entry.
    let tail_start = entry + 12;
    let tail_end = ifd0.next_at() + 4;
    t.copy_within(tail_start..tail_end, entry);
    t[tail_end - 12..tail_end].fill(0);
    let c = o.put_u16((ifd0.count - 1) as u16);
    t[ifd0.at..ifd0.at + 2].copy_from_slice(&c);
    erase(t, regions);
    Ok(true)
}

/// Remove the embedded thumbnail (IFD1). Returns whether anything changed.
pub fn strip_ifd1(t: &mut Vec<u8>) -> Result<bool> {
    let (o, off0) = header(t)?;
    let ifd0 = ifd_at(&o, t, off0)?;
    let next = o.u32(t, ifd0.next_at()).ok_or_else(|| Error::parse("EXIF", "bad IFD1 pointer"))? as usize;
    if next == 0 {
        return Ok(false);
    }
    let ifd1 = ifd_at(&o, t, next)?;
    let mut regions = vec![ifd1.span()];
    let (mut off, mut len) = (None, None);
    for i in 0..ifd1.count {
        let e = ifd1.entries() + 12 * i;
        match o.u16(t, e) {
            Some(TAG_THUMB_OFFSET) => off = o.u32(t, e + 8).map(|v| v as usize),
            Some(TAG_THUMB_LENGTH) => len = o.u32(t, e + 8).map(|v| v as usize),
            _ => {}
        }
        if let Some(r) = value_range(&o, t, e) {
            regions.push(r);
        }
    }
    if let (Some(off), Some(len)) = (off, len) {
        if let Some(end) = off.checked_add(len).filter(|&e| e <= t.len() && len > 0) {
            regions.push(off..end);
        }
    }
    check_clear(&regions, &[0..8, ifd0.span()])?;
    let at = ifd0.next_at();
    t[at..at + 4].copy_from_slice(&o.put_u32(0));
    erase(t, regions);
    Ok(true)
}

/// A 26-byte little-endian TIFF whose only entry is the orientation.
pub fn orientation_only(orientation: u16) -> Vec<u8> {
    let mut t = b"II*\0".to_vec();
    t.extend_from_slice(&8u32.to_le_bytes());
    t.extend_from_slice(&1u16.to_le_bytes());
    t.extend_from_slice(&0x0112u16.to_le_bytes());
    t.extend_from_slice(&3u16.to_le_bytes());
    t.extend_from_slice(&1u32.to_le_bytes());
    t.extend_from_slice(&orientation.to_le_bytes());
    t.extend_from_slice(&[0, 0]);
    t.extend_from_slice(&0u32.to_le_bytes());
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_formats::exif::build::{tiff, Value};

    fn sample() -> Vec<u8> {
        let gps = vec![
            (0x0001, Value::Ascii("N".into())),
            (0x0002, Value::Rational(vec![(52, 1), (30, 1), (0, 1)])),
            (0x0003, Value::Ascii("E".into())),
            (0x0004, Value::Rational(vec![(13, 1), (24, 1), (0, 1)])),
        ];
        let ifd0 = vec![
            (0x010F, Value::Ascii("ACME Camera Co".into())),
            (0x0112, Value::Short(6)),
            (0x8825, Value::SubIfd(gps)),
        ];
        let mut thumb = vec![0xFF, 0xD8, 0xFF, 0xDB, 0, 4, 1, 2];
        thumb.extend([7u8; 300]);
        thumb.extend([0xFF, 0xD9]);
        tiff(&ifd0, Some(&[(0, Value::Thumbnail(thumb))]))
    }

    #[test]
    fn gps_removed_everything_else_intact() {
        let mut t = sample();
        let before = mj_formats::exif::parse(&t).unwrap();
        assert!(before.has_gps && before.thumbnail.is_some());
        assert!(strip_gps(&mut t).unwrap());
        let after = mj_formats::exif::parse(&t).unwrap();
        assert!(!after.has_gps);
        assert_eq!((after.orientation, after.make.as_deref()), (Some(6), Some("ACME Camera Co")));
        assert!(after.thumbnail.is_some() && after.thumbnail_bytes(&t).is_some());
        assert!(!strip_gps(&mut t).unwrap(), "second run is a no-op");
    }

    #[test]
    fn thumbnail_removed_and_buffer_shrinks() {
        let mut t = sample();
        let n = t.len();
        assert!(strip_ifd1(&mut t).unwrap());
        assert!(t.len() < n - 300, "{} -> {}", n, t.len());
        let after = mj_formats::exif::parse(&t).unwrap();
        assert!(after.thumbnail.is_none() && after.has_gps);
        assert_eq!(after.orientation, Some(6));
        assert!(!strip_ifd1(&mut t).unwrap());
    }

    #[test]
    fn both_edits_compose() {
        let mut t = sample();
        strip_gps(&mut t).unwrap();
        strip_ifd1(&mut t).unwrap();
        let a = mj_formats::exif::parse(&t).unwrap();
        assert!(!a.has_gps && a.thumbnail.is_none() && a.orientation == Some(6));
    }

    #[test]
    fn orientation_only_round_trips() {
        let t = orientation_only(8);
        assert_eq!(t.len(), 26);
        assert_eq!(mj_formats::exif::parse(&t).unwrap().orientation, Some(8));
    }

    #[test]
    fn malformed_input_errors_without_panicking() {
        let good = sample();
        for cut in 0..good.len() {
            let mut t = good[..cut].to_vec();
            let _ = strip_gps(&mut t);
            let mut t = good[..cut].to_vec();
            let _ = strip_ifd1(&mut t);
        }
        // GPS pointer aimed at the header, and at IFD0 itself, must be refused.
        let mut t = sample();
        let mut x: u32 = 3;
        for _ in 0..500 {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let i = (x >> 8) as usize % t.len();
            t[i] ^= (x >> 3) as u8;
            let mut c = t.clone();
            let _ = strip_gps(&mut c);
            let mut c = t.clone();
            let _ = strip_ifd1(&mut c);
        }
        assert!(strip_gps(&mut b"II*\0\x08\0\0\0".to_vec()).is_err());
        assert!(strip_gps(&mut b"not tiff".to_vec()).is_err());
    }
}
