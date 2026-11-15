//! JPEG segment walking without decoding: frame dimensions (SOF) and the
//! location of the EXIF APP1 payload. Owner: W2-B.
//!
//! Pixel decoding for perceptual hashing lives in `mj-dupes` (it uses the
//! `zune-jpeg` crate, see `docs/dependencies/W2-B.md`); this module only reads
//! marker segments, so it works on a bounded header window (the first 64-128 KiB
//! of a file) and never needs the whole file.
//!
//! Hardening: every length is bounds-checked against the buffer, the walk stops
//! at the first start-of-scan, after [`MAX_SEGMENTS`] segments or at the end of
//! the buffer (reported as `truncated`), and nothing here can panic.

use std::ops::Range;

/// Upper bound on segments walked before giving up (a real header has a few dozen).
pub const MAX_SEGMENTS: usize = 4096;

/// Frame header facts (SOF0..SOF15 except DHT/JPG/DAC).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameInfo {
    pub width: u16,
    pub height: u16,
    /// Number of colour components (1 = greyscale, 3 = YCbCr, 4 = CMYK/YCCK).
    pub components: u8,
    /// Sample precision in bits (8 for nearly every file).
    pub precision: u8,
    /// SOF2/SOF6/SOF10/SOF14: progressive.
    pub progressive: bool,
    /// The SOF marker byte (`0xC0`..=`0xCF`).
    pub marker: u8,
}

/// What a header walk found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct JpegHeader {
    /// First frame header, when it lies within the buffer.
    pub frame: Option<FrameInfo>,
    /// Byte range of the TIFF structure inside the first `Exif\0\0` APP1 segment
    /// (offsets into the walked buffer). The range may be shorter than the declared
    /// segment when the buffer is a truncated window (`exif_truncated`).
    pub exif: Option<Range<usize>>,
    /// The EXIF segment extends past the end of the buffer.
    pub exif_truncated: bool,
    /// Reached a start-of-scan marker: every header segment before the image data was seen.
    pub reached_scan: bool,
    /// The buffer ended (or was malformed) before a start-of-scan marker.
    pub truncated: bool,
}

/// True when `buf` starts with the JPEG SOI marker.
pub fn is_jpeg(buf: &[u8]) -> bool {
    buf.len() >= 3 && buf[0] == 0xFF && buf[1] == 0xD8 && buf[2] == 0xFF
}

/// Walk the marker segments of a JPEG in `buf` (a whole file or a prefix of one).
/// Returns `None` when `buf` does not start with SOI.
pub fn walk(buf: &[u8]) -> Option<JpegHeader> {
    if !is_jpeg(buf) {
        return None;
    }
    let mut h = JpegHeader::default();
    let mut pos = 2usize;
    let mut segments = 0usize;
    loop {
        segments += 1;
        if segments > MAX_SEGMENTS {
            h.truncated = true;
            break;
        }
        // Markers start with 0xFF, possibly preceded by fill bytes (0xFF repeated).
        if pos >= buf.len() || buf[pos] != 0xFF {
            h.truncated = true;
            break;
        }
        while pos < buf.len() && buf[pos] == 0xFF {
            pos += 1;
        }
        let Some(&marker) = buf.get(pos) else {
            h.truncated = true;
            break;
        };
        pos += 1;
        match marker {
            // Standalone markers without a length: TEM, RSTn, SOI (tolerated), stray 0x00.
            0x01 | 0xD0..=0xD8 | 0x00 => continue,
            0xD9 => {
                // EOI before any scan: nothing more to read.
                h.truncated = true;
                break;
            }
            _ => {}
        }
        let Some(len) = be16(buf, pos) else {
            h.truncated = true;
            break;
        };
        let len = len as usize;
        if len < 2 {
            h.truncated = true;
            break;
        }
        let body_start = pos + 2;
        let body_end = pos + len; // exclusive, may exceed buf.len() for a truncated window
        let available = body_end.min(buf.len());
        let body = buf.get(body_start..available).unwrap_or(&[]);
        match marker {
            0xC0..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) => {
                if h.frame.is_none() && body.len() >= 6 {
                    let height = u16::from_be_bytes([body[1], body[2]]);
                    let width = u16::from_be_bytes([body[3], body[4]]);
                    h.frame = Some(FrameInfo {
                        width,
                        height,
                        components: body[5],
                        precision: body[0],
                        progressive: matches!(marker, 0xC2 | 0xC6 | 0xCA | 0xCE),
                        marker,
                    });
                }
            }
            0xE1 if h.exif.is_none() && body.len() >= 6 && &body[..6] == b"Exif\0\0" => {
                h.exif = Some(body_start + 6..available);
                h.exif_truncated = body_end > buf.len();
            }
            0xDA => {
                h.reached_scan = true;
                break;
            }
            _ => {}
        }
        if body_end > buf.len() {
            h.truncated = true;
            break;
        }
        pos = body_end;
    }
    Some(h)
}

/// The TIFF bytes of the EXIF APP1 segment, when present in `buf`.
pub fn exif_tiff(buf: &[u8]) -> Option<&[u8]> {
    let h = walk(buf)?;
    buf.get(h.exif?)
}

fn be16(buf: &[u8], pos: usize) -> Option<u16> {
    let b = buf.get(pos..pos.checked_add(2)?)?;
    Some(u16::from_be_bytes([b[0], b[1]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(marker: u8, body: &[u8]) -> Vec<u8> {
        let mut v = vec![0xFF, marker];
        v.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
        v.extend_from_slice(body);
        v
    }

    fn sample() -> Vec<u8> {
        let mut f = vec![0xFF, 0xD8];
        f.extend(seg(0xE0, b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0"));
        let mut exif = b"Exif\0\0".to_vec();
        exif.extend_from_slice(b"II*\0\x08\0\0\0\0\0");
        f.extend(seg(0xE1, &exif));
        f.extend(seg(0xDB, &[0; 65]));
        f.extend(seg(0xC2, &[8, 0x01, 0xE0, 0x02, 0x80, 3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]));
        f.extend([0xFF, 0xFF]); // fill bytes before the next marker
        f.extend(seg(0xDA, &[1, 1, 0, 0, 63, 0]));
        f.extend([0x12, 0x34, 0xFF, 0xD9]);
        f
    }

    #[test]
    fn walks_frame_and_exif() {
        let f = sample();
        let h = walk(&f).unwrap();
        let fr = h.frame.unwrap();
        assert_eq!((fr.width, fr.height, fr.components, fr.progressive), (640, 480, 3, true));
        assert!(h.reached_scan && !h.truncated && !h.exif_truncated);
        assert_eq!(&f[h.exif.clone().unwrap()], b"II*\0\x08\0\0\0\0\0");
        assert_eq!(exif_tiff(&f).unwrap().len(), 10);
    }

    #[test]
    fn truncated_windows_and_garbage_never_panic() {
        let f = sample();
        for cut in 0..f.len() {
            let _ = walk(&f[..cut]);
        }
        let h = walk(&f[..30]).unwrap();
        assert!(h.truncated && h.exif_truncated && h.frame.is_none());
        assert!(walk(b"not a jpeg").is_none());
        // Zero-length segment and endless fill bytes.
        assert!(walk(&[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x00]).unwrap().truncated);
        assert!(walk(&[0xFF; 100]).is_none());
        let mut v = vec![0xFF, 0xD8];
        v.extend(std::iter::repeat_n(0xFF, 10_000));
        assert!(walk(&v).unwrap().truncated);
        // Pseudo-random bytes after SOI.
        let mut x: u32 = 1;
        for _ in 0..200 {
            let mut v = vec![0xFF, 0xD8, 0xFF];
            for _ in 0..300 {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                v.push((x >> 16) as u8);
            }
            let _ = walk(&v);
        }
    }

    #[test]
    fn many_tiny_segments_are_bounded() {
        let mut v = vec![0xFF, 0xD8];
        for _ in 0..(MAX_SEGMENTS + 10) {
            v.extend(seg(0xFE, &[]));
        }
        assert!(walk(&v).unwrap().truncated);
    }
}
