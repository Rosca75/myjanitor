//! Lossless JPEG rewrite at segment level (OPT-IMG-01, OPT-IMG-05): the entropy-coded data is never
//! decoded, re-encoded or touched; only the marker segments before the first scan are filtered by the
//! metadata policy. Output = SOI + kept (or edited) segments + the original bytes from the first SOS.
//!
//! Safety rules: multi-picture files (APP2 `MPF`, whose offsets point past the image) are left alone;
//! `Adobe` APP14 (colour transform) and `JFIF` APP0 are always kept; the EXIF orientation survives
//! every policy; data after the last EOI is kept unless it is only zero padding.

use mj_core::{Error, Result};
use mj_formats::jpeg;

use super::tiff_edit;

/// Which metadata to remove (OPT-IMG-05).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JpegMetadata {
    KeepAll,
    /// Remove the GPS IFD from EXIF (and XMP packets that mention GPS).
    StripGps,
    /// Remove the embedded EXIF thumbnail and JFXX thumbnails.
    StripThumbnail,
    /// Remove EXIF, XMP, IPTC, comments and vendor segments; keep orientation and (unless `icc`) the colour profile.
    StripAll {
        icc: bool,
    },
}

impl JpegMetadata {
    /// Parse the `metadata` parameter (`keep_all`, `strip_gps`, `strip_thumbnail`, `strip_all`).
    pub fn parse(s: &str, strip_icc: bool) -> JpegMetadata {
        match s {
            "keep_all" => JpegMetadata::KeepAll,
            "strip_gps" => JpegMetadata::StripGps,
            "strip_all" => JpegMetadata::StripAll { icc: strip_icc },
            _ => JpegMetadata::StripThumbnail,
        }
    }
}

/// Result of [`rewrite`].
#[derive(Debug, Clone)]
pub struct Rewritten {
    pub bytes: Vec<u8>,
    /// Human notes about what was removed or changed.
    pub notes: Vec<String>,
    /// The colour profile was removed (appearance may change).
    pub icc_removed: bool,
}

enum Act {
    Keep,
    Drop(&'static str),
    Replace(Vec<u8>, &'static str),
}

struct Seg {
    marker: u8,
    start: usize,
    end: usize,
}

const XMP: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
const XMP_EXT: &[u8] = b"http://ns.adobe.com/xmp/extension/\0";

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// Orientation EXIF to keep (or `None`) from an EXIF body.
fn kept_orientation(tiff: &[u8]) -> Option<Vec<u8>> {
    let o = mj_formats::exif::parse(tiff).ok()?.orientation.filter(|&o| o > 1)?;
    Some(exif_body(&tiff_edit::orientation_only(o)))
}

fn exif_body(tiff: &[u8]) -> Vec<u8> {
    let mut v = b"Exif\0\0".to_vec();
    v.extend_from_slice(tiff);
    v
}

fn decide(marker: u8, body: &[u8], policy: JpegMetadata, xmp_dropped: &mut bool) -> Result<Act> {
    use JpegMetadata::*;
    let all = matches!(policy, StripAll { .. });
    Ok(match marker {
        0xFE if policy != KeepAll => Act::Drop("comments"),
        0xE0 if body.starts_with(b"JFXX\0") && matches!(policy, StripThumbnail | StripAll { .. }) => {
            Act::Drop("JFXX thumbnail")
        }
        0xE1 if body.starts_with(b"Exif\0\0") => {
            let tiff = &body[6..];
            match policy {
                KeepAll => Act::Keep,
                StripThumbnail => {
                    let mut t = tiff.to_vec();
                    match tiff_edit::strip_ifd1(&mut t) {
                        Ok(true) => Act::Replace(exif_body(&t), "EXIF thumbnail"),
                        _ => Act::Keep,
                    }
                }
                StripGps => {
                    let mut t = tiff.to_vec();
                    match tiff_edit::strip_gps(&mut t) {
                        Ok(true) => Act::Replace(exif_body(&t), "GPS position"),
                        Ok(false) => Act::Keep,
                        // Unreadable GPS data: remove all EXIF rather than risk leaking a location.
                        Err(_) => kept_orientation(tiff)
                            .map_or(Act::Drop("EXIF (GPS unreadable)"), |b| Act::Replace(b, "EXIF (GPS unreadable)")),
                    }
                }
                StripAll { .. } => {
                    kept_orientation(tiff).map_or(Act::Drop("EXIF"), |b| Act::Replace(b, "EXIF (orientation kept)"))
                }
            }
        }
        0xE1 if body.starts_with(XMP) => match policy {
            StripAll { .. } => {
                *xmp_dropped = true;
                Act::Drop("XMP")
            }
            StripGps if contains(body, b"GPS") => {
                *xmp_dropped = true;
                Act::Drop("XMP with GPS")
            }
            _ => Act::Keep,
        },
        0xE1 if body.starts_with(XMP_EXT) && (all || (policy == StripGps && *xmp_dropped)) => Act::Drop("extended XMP"),
        0xE1 if all => Act::Drop("APP1 data"),
        0xE2 if body.starts_with(b"ICC_PROFILE\0") => match policy {
            StripAll { icc: true } => Act::Drop("colour profile"),
            _ => Act::Keep,
        },
        0xE2 if body.starts_with(b"MPF\0") => {
            return Err(Error::unsupported("JPEG", "multi-picture JPEG (MPF) is left untouched"))
        }
        0xED if all => Act::Drop("IPTC/Photoshop data"),
        0xE2..=0xEC | 0xEF if all => Act::Drop("vendor data"),
        _ => Act::Keep,
    })
}

/// Rewrite `buf` under `policy`. `Err(Unsupported)` for files that must be left alone, `Err(Parse)` for
/// files that are not a complete JPEG header.
pub fn rewrite(buf: &[u8], policy: JpegMetadata) -> Result<Rewritten> {
    if !jpeg::is_jpeg(buf) {
        return Err(Error::parse("JPEG", "missing SOI marker"));
    }
    let bad = |m: &str| Error::parse("JPEG", m.to_string());
    let (mut pos, mut segs) = (2usize, Vec::new());
    let scan_at = loop {
        if segs.len() >= jpeg::MAX_SEGMENTS {
            return Err(bad("too many segments"));
        }
        while buf.get(pos) == Some(&0xFF) && buf.get(pos + 1) == Some(&0xFF) {
            pos += 1; // fill bytes
        }
        if buf.get(pos) != Some(&0xFF) {
            return Err(bad("expected a marker"));
        }
        let marker = *buf.get(pos + 1).ok_or_else(|| bad("truncated"))?;
        match marker {
            0xDA => break pos,
            0xD9 | 0x00 | 0x01 | 0xD0..=0xD8 => return Err(bad("unexpected marker before the first scan")),
            _ => {}
        }
        let len = buf
            .get(pos + 2..pos + 4)
            .map(|b| usize::from(u16::from_be_bytes([b[0], b[1]])))
            .ok_or_else(|| bad("truncated segment"))?;
        let end = pos + 2 + len;
        if len < 2 || end > buf.len() {
            return Err(bad("segment length out of range"));
        }
        segs.push(Seg { marker, start: pos, end });
        pos = end;
    };
    let mut out = Vec::with_capacity(buf.len());
    out.extend_from_slice(&buf[..2]);
    let (mut notes, mut xmp_dropped, mut icc_removed) = (Vec::new(), false, false);
    for s in &segs {
        let body = &buf[s.start + 4..s.end];
        match decide(s.marker, body, policy, &mut xmp_dropped)? {
            Act::Keep => out.extend_from_slice(&buf[s.start..s.end]),
            Act::Drop(what) => {
                icc_removed |= what == "colour profile";
                notes.push(format!("removed {what}"));
            }
            Act::Replace(new, what) => {
                out.extend_from_slice(&[0xFF, s.marker]);
                out.extend_from_slice(&((new.len() + 2) as u16).to_be_bytes());
                out.extend_from_slice(&new);
                notes.push(format!("removed {what}"));
            }
        }
    }
    // Entropy-coded data and everything after it: untouched, except trailing zero padding after the EOI.
    let tail = &buf[scan_at..];
    let trimmed = tail.len() - tail.iter().rev().take_while(|&&b| b == 0).count();
    let tail = if tail[..trimmed].ends_with(&[0xFF, 0xD9]) && trimmed < tail.len() {
        notes.push("removed trailing padding".into());
        &tail[..trimmed]
    } else {
        tail
    };
    let new_scan_at = out.len();
    out.extend_from_slice(tail);
    verify(buf, &out, scan_at, new_scan_at, tail.len())?;
    notes.dedup();
    Ok(Rewritten { bytes: out, notes, icc_removed })
}

/// The output must parse, describe the same frame and carry the scan data byte for byte.
fn verify(src: &[u8], out: &[u8], src_scan: usize, out_scan: usize, tail_len: usize) -> Result<()> {
    let fail = |m: &str| Error::Internal(format!("JPEG verification failed: {m}"));
    let (a, b) = (jpeg::walk(src).ok_or_else(|| fail("source"))?, jpeg::walk(out).ok_or_else(|| fail("output"))?);
    if a.frame != b.frame || !b.reached_scan {
        return Err(fail("frame header changed"));
    }
    if out[out_scan..out_scan + tail_len] != src[src_scan..src_scan + tail_len] || out.len() != out_scan + tail_len {
        return Err(fail("scan data changed"));
    }
    Ok(())
}

/// Frame header and every APP/COM segment before the first scan (used to check what an external
/// tool did to the headers).
pub fn header_signature(buf: &[u8]) -> Option<(jpeg::FrameInfo, Vec<(u8, Vec<u8>)>)> {
    let h = jpeg::walk(buf)?;
    let mut pos = 2usize;
    let mut v = Vec::new();
    while buf.get(pos) == Some(&0xFF) {
        let m = *buf.get(pos + 1)?;
        if m == 0xDA || m == 0xFF {
            break;
        }
        let len = usize::from(u16::from_be_bytes([*buf.get(pos + 2)?, *buf.get(pos + 3)?]));
        let end = pos + 2 + len;
        if matches!(m, 0xE0..=0xEF | 0xFE) {
            v.push((m, buf.get(pos + 4..end)?.to_vec()));
        }
        pos = end;
    }
    Some((h.frame?, v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_formats::exif::build::{app1, insert_app1, tiff, Value};

    fn seg(marker: u8, body: &[u8]) -> Vec<u8> {
        let mut v = vec![0xFF, marker];
        v.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
        v.extend_from_slice(body);
        v
    }

    fn exif(orientation: u16, gps: bool, thumb: bool) -> Vec<u8> {
        let mut ifd0 = vec![(0x010F, Value::Ascii("ACME".into())), (0x0112, Value::Short(orientation))];
        if gps {
            ifd0.push((
                0x8825,
                Value::SubIfd(vec![
                    (1, Value::Ascii("N".into())),
                    (2, Value::Rational(vec![(52, 1), (30, 1), (0, 1)])),
                    (3, Value::Ascii("E".into())),
                    (4, Value::Rational(vec![(13, 1), (24, 1), (0, 1)])),
                ]),
            ));
        }
        let t1 = vec![(
            0,
            Value::Thumbnail([vec![0xFF, 0xD8, 0xFF, 0xDB, 0, 4, 1, 2], vec![9; 400], vec![0xFF, 0xD9]].concat()),
        )];
        tiff(&ifd0, thumb.then_some(&t1[..]))
    }

    /// SOI, JFIF, EXIF, XMP, ICC, IPTC, Adobe, COM, DQT, SOF0, DHT, SOS + scan bytes + EOI (+ padding).
    fn sample(orientation: u16, padding: usize) -> Vec<u8> {
        let mut f = vec![0xFF, 0xD8];
        f.extend(seg(0xE0, b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0"));
        f.extend(app1(&exif(orientation, true, true)));
        f.extend(seg(0xE1, &[XMP, b"<x:xmpmeta exif:GPSLatitude=\"52\"/>"].concat()));
        f.extend(seg(0xE2, &[b"ICC_PROFILE\0\x01\x01".as_slice(), &[5; 50]].concat()));
        f.extend(seg(0xED, b"Photoshop 3.0\0stuff"));
        f.extend(seg(0xEE, b"Adobe\0\x64\0\0\0\0\x01"));
        f.extend(seg(0xFE, b"made with a tool"));
        f.extend(seg(0xDB, &[0; 65]));
        f.extend(seg(0xC0, &[8, 0, 8, 0, 8, 1, 1, 0x11, 0]));
        f.extend(seg(0xC4, &[0; 20]));
        f.extend(seg(0xDA, &[1, 1, 0, 0, 63, 0]));
        f.extend([0x12, 0x34, 0xFF, 0x00, 0x56, 0xFF, 0xD0, 0x78, 0xFF, 0xD9]);
        f.extend(vec![0u8; padding]);
        f
    }

    fn scan(buf: &[u8]) -> &[u8] {
        let at = buf.windows(8).position(|w| w[..2] == [0xFF, 0xDA]).unwrap();
        &buf[at..]
    }

    fn markers(buf: &[u8]) -> Vec<u8> {
        let mut pos = 2;
        let mut v = Vec::new();
        while buf[pos] == 0xFF && buf[pos + 1] != 0xDA {
            v.push(buf[pos + 1]);
            pos += 2 + usize::from(u16::from_be_bytes([buf[pos + 2], buf[pos + 3]]));
        }
        v
    }

    #[test]
    fn keep_all_is_a_byte_identical_copy() {
        let f = sample(6, 0);
        let r = rewrite(&f, JpegMetadata::KeepAll).unwrap();
        assert_eq!(r.bytes, f);
        assert!(r.notes.is_empty());
    }

    #[test]
    fn scan_bytes_are_identical_under_every_policy() {
        let f = sample(6, 0);
        for p in [
            JpegMetadata::StripGps,
            JpegMetadata::StripThumbnail,
            JpegMetadata::StripAll { icc: false },
            JpegMetadata::StripAll { icc: true },
        ] {
            let r = rewrite(&f, p).unwrap();
            assert_eq!(scan(&r.bytes), scan(&f), "{p:?}");
            assert!(r.bytes.len() <= f.len(), "{p:?}");
            assert!(jpeg::walk(&r.bytes).unwrap().reached_scan);
        }
    }

    #[test]
    fn strip_gps_only_removes_gps() {
        let f = sample(6, 0);
        let r = rewrite(&f, JpegMetadata::StripGps).unwrap();
        let tiff = jpeg::exif_tiff(&r.bytes).unwrap();
        let e = mj_formats::exif::parse(tiff).unwrap();
        assert!(!e.has_gps && e.orientation == Some(6) && e.make.as_deref() == Some("ACME") && e.thumbnail.is_some());
        let m = markers(&r.bytes);
        assert!(!contains(&r.bytes, b"GPSLatitude"), "XMP with GPS must go");
        assert!(m.contains(&0xE2) && m.contains(&0xED) && !m.contains(&0xFE), "{m:x?}");
        assert!(r.notes.iter().any(|n| n.contains("GPS position")));
    }

    #[test]
    fn strip_thumbnail_keeps_gps() {
        let r = rewrite(&sample(1, 0), JpegMetadata::StripThumbnail).unwrap();
        let e = mj_formats::exif::parse(jpeg::exif_tiff(&r.bytes).unwrap()).unwrap();
        assert!(e.thumbnail.is_none() && e.has_gps);
        assert!(r.bytes.len() < sample(1, 0).len() - 400);
        assert!(!markers(&r.bytes).contains(&0xFE), "comments are dropped by every policy but keep_all");
    }

    #[test]
    fn strip_all_keeps_orientation_icc_adobe_and_jfif() {
        let r = rewrite(&sample(8, 0), JpegMetadata::StripAll { icc: false }).unwrap();
        let m = markers(&r.bytes);
        assert_eq!(m, [0xE0, 0xE1, 0xE2, 0xEE, 0xDB, 0xC0, 0xC4], "{m:x?}");
        assert_eq!(mj_formats::exif::parse(jpeg::exif_tiff(&r.bytes).unwrap()).unwrap().orientation, Some(8));
        assert!(!r.icc_removed);
        let r2 = rewrite(&sample(1, 0), JpegMetadata::StripAll { icc: true }).unwrap();
        assert!(r2.icc_removed && !markers(&r2.bytes).contains(&0xE2) && !markers(&r2.bytes).contains(&0xE1));
        assert!(markers(&r2.bytes).contains(&0xEE));
    }

    #[test]
    fn trailing_zero_padding_is_trimmed_but_other_trailers_are_kept() {
        let r = rewrite(&sample(1, 4096), JpegMetadata::KeepAll).unwrap();
        assert_eq!(r.bytes.len(), sample(1, 0).len());
        let mut f = sample(1, 0);
        f.extend_from_slice(b"MOTIONPHOTO-VIDEO");
        assert_eq!(
            rewrite(&f, JpegMetadata::StripAll { icc: false }).unwrap().bytes.ends_with(b"MOTIONPHOTO-VIDEO"),
            true
        );
    }

    #[test]
    fn multi_picture_files_are_left_alone() {
        let mut f = vec![0xFF, 0xD8];
        f.extend(seg(0xE2, b"MPF\0xxxxxxxx"));
        f.extend(seg(0xDA, &[1, 1, 0, 0, 63, 0]));
        f.extend([1, 2, 0xFF, 0xD9]);
        assert!(matches!(rewrite(&f, JpegMetadata::StripAll { icc: true }), Err(Error::Unsupported { .. })));
    }

    #[test]
    fn broken_files_are_errors_and_never_panic() {
        assert!(rewrite(b"nope", JpegMetadata::KeepAll).is_err());
        let f = sample(6, 0);
        for cut in 0..f.len() {
            let _ = rewrite(&f[..cut], JpegMetadata::StripAll { icc: true });
        }
        let mut x: u32 = 9;
        for _ in 0..300 {
            let mut g = f.clone();
            for _ in 0..4 {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                let i = (x >> 8) as usize % g.len();
                g[i] = (x >> 20) as u8;
            }
            let _ = rewrite(&g, JpegMetadata::StripGps);
            let _ = rewrite(&g, JpegMetadata::StripThumbnail);
        }
    }

    #[test]
    fn header_signature_lists_app_segments() {
        let f = insert_app1(&[0xFF, 0xD8], &[]);
        assert!(header_signature(&f).is_none());
        let (frame, segs) = header_signature(&sample(1, 0)).unwrap();
        assert_eq!((frame.width, frame.height), (8, 8));
        assert!(segs.iter().any(|(m, _)| *m == 0xFE));
    }
}
