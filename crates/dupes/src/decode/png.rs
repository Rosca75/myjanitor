//! In-house PNG decoder to luma, for perceptual hashing only.
//!
//! Supports every colour type (grey, RGB, palette, grey+alpha, RGBA) and bit
//! depth (1, 2, 4, 8, 16), non-interlaced. Interlaced (Adam7) files are
//! reported as unsupported (skipped for perceptual matching, still
//! exact-matched). Alpha is ignored and CRCs are not verified (a corrupt
//! stream fails inflate or the size checks). Rows are unfiltered one at a time
//! and fed straight into the grid accumulator, so no full-size grey buffer is
//! kept. Hardened: chunk lengths are bounds-checked, the inflated size is
//! limited to exactly what the header implies, and dimensions are checked
//! against [`super::within_limits`] before inflating.

use super::{luma16, within_limits, DecodeError, Decoded};
use crate::perceptual::{dhash, phash, GridAccumulator};

/// The 8-byte PNG signature.
pub const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// IHDR facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub color_type: u8,
    pub interlaced: bool,
}

impl Header {
    fn channels(&self) -> Option<usize> {
        let ok_depth = |d: &[u8]| d.contains(&self.bit_depth);
        Some(match self.color_type {
            0 if ok_depth(&[1, 2, 4, 8, 16]) => 1,
            2 if ok_depth(&[8, 16]) => 3,
            3 if ok_depth(&[1, 2, 4, 8]) => 1,
            4 if ok_depth(&[8, 16]) => 2,
            6 if ok_depth(&[8, 16]) => 4,
            _ => return None,
        })
    }
}

/// True when `buf` starts with the PNG signature.
pub fn is_png(buf: &[u8]) -> bool {
    buf.starts_with(&SIGNATURE)
}

/// Read the IHDR from the start of a file (a header window is enough).
pub fn header(buf: &[u8]) -> Option<Header> {
    if !is_png(buf) || buf.get(12..16)? != b"IHDR" {
        return None;
    }
    let d = buf.get(16..29)?;
    Some(Header {
        width: u32::from_be_bytes([d[0], d[1], d[2], d[3]]),
        height: u32::from_be_bytes([d[4], d[5], d[6], d[7]]),
        bit_depth: d[8],
        color_type: d[9],
        interlaced: d[12] != 0,
    })
}

/// Decode a whole PNG file and fingerprint it.
pub fn decode(buf: &[u8]) -> Result<Decoded, DecodeError> {
    let hdr = header(buf).ok_or_else(|| DecodeError::Corrupt("missing IHDR".into()))?;
    if !within_limits(hdr.width, hdr.height) {
        return Err(DecodeError::TooLarge);
    }
    if hdr.interlaced {
        return Err(DecodeError::Unsupported("interlaced PNG"));
    }
    let channels = hdr.channels().ok_or_else(|| DecodeError::Corrupt("invalid colour type / depth".into()))?;
    let (w, h) = (hdr.width as usize, hdr.height as usize);
    let bits_per_pixel = channels * hdr.bit_depth as usize;
    let row_bytes = (w * bits_per_pixel).div_ceil(8);
    let expected = (row_bytes + 1).checked_mul(h).ok_or(DecodeError::TooLarge)?;

    // Collect IDAT and PLTE.
    let mut pos = 8usize;
    let mut idat = Vec::new();
    let mut palette: Vec<[u8; 3]> = Vec::new();
    loop {
        let len = buf.get(pos..pos + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize);
        let Some(len) = len else { break };
        let typ = buf.get(pos + 4..pos + 8).ok_or_else(|| DecodeError::Corrupt("truncated chunk".into()))?;
        let data_end = pos.checked_add(8 + len).ok_or_else(|| DecodeError::Corrupt("chunk length".into()))?;
        let data = buf.get(pos + 8..data_end).ok_or_else(|| DecodeError::Corrupt("truncated chunk".into()))?;
        match typ {
            b"IDAT" => {
                if idat.len() + data.len() > expected.saturating_mul(2) + 1024 * 1024 {
                    return Err(DecodeError::Corrupt("IDAT larger than the image".into()));
                }
                idat.extend_from_slice(data);
            }
            b"PLTE" => palette = data.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect(),
            b"IEND" => break,
            _ => {}
        }
        pos = data_end + 4; // skip CRC
    }
    if hdr.color_type == 3 && palette.is_empty() {
        return Err(DecodeError::Corrupt("palette image without PLTE".into()));
    }
    let raw = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&idat, expected)
        .map_err(|e| DecodeError::Corrupt(format!("inflate: {:?}", e.status)))?;
    if raw.len() < expected {
        return Err(DecodeError::Corrupt("image data truncated".into()));
    }

    let bpp = bits_per_pixel.div_ceil(8).max(1);
    let mut prev = vec![0u8; row_bytes];
    let mut cur = vec![0u8; row_bytes];
    let mut gray = vec![0u8; w];
    let mut acc = GridAccumulator::new(w, h);
    for y in 0..h {
        let line = &raw[y * (row_bytes + 1)..(y + 1) * (row_bytes + 1)];
        cur.copy_from_slice(&line[1..]);
        unfilter(line[0], &mut cur, &prev, bpp)?;
        row_to_gray(&hdr, channels, &cur, &palette, &mut gray);
        acc.push_row(&gray);
        std::mem::swap(&mut prev, &mut cur);
    }
    let g = acc.finish();
    Ok(Decoded { width: hdr.width, height: hdr.height, dhash: dhash(&g.g9), phash: phash(&g.g32) })
}

fn unfilter(filter: u8, cur: &mut [u8], prev: &[u8], bpp: usize) -> Result<(), DecodeError> {
    match filter {
        0 => {}
        1 => {
            for i in bpp..cur.len() {
                cur[i] = cur[i].wrapping_add(cur[i - bpp]);
            }
        }
        2 => {
            for i in 0..cur.len() {
                cur[i] = cur[i].wrapping_add(prev[i]);
            }
        }
        3 => {
            for i in 0..cur.len() {
                let left = if i >= bpp { cur[i - bpp] as u16 } else { 0 };
                cur[i] = cur[i].wrapping_add(((left + prev[i] as u16) / 2) as u8);
            }
        }
        4 => {
            for i in 0..cur.len() {
                let a = if i >= bpp { cur[i - bpp] as i16 } else { 0 };
                let b = prev[i] as i16;
                let c = if i >= bpp { prev[i - bpp] as i16 } else { 0 };
                let p = a + b - c;
                let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
                let pred = if pa <= pb && pa <= pc {
                    a
                } else if pb <= pc {
                    b
                } else {
                    c
                };
                cur[i] = cur[i].wrapping_add(pred as u8);
            }
        }
        _ => return Err(DecodeError::Corrupt(format!("filter type {filter}"))),
    }
    Ok(())
}

/// Sample `i` of a row as a 16-bit value (low bit depths are scaled up).
fn sample(row: &[u8], depth: u8, i: usize) -> u32 {
    match depth {
        16 => u16::from_be_bytes([row[2 * i], row[2 * i + 1]]) as u32,
        8 => row[i] as u32 * 257,
        d => {
            let per = 8 / d as usize;
            let byte = row[i / per];
            let shift = 8 - d as usize * (i % per + 1);
            let v = (byte >> shift) as u32 & ((1u32 << d) - 1);
            v * 65535 / ((1u32 << d) - 1)
        }
    }
}

fn index(row: &[u8], depth: u8, i: usize) -> usize {
    match depth {
        8 => row[i] as usize,
        d => {
            let per = 8 / d as usize;
            let shift = 8 - d as usize * (i % per + 1);
            ((row[i / per] >> shift) & ((1u8 << d) - 1)) as usize
        }
    }
}

fn row_to_gray(hdr: &Header, channels: usize, row: &[u8], palette: &[[u8; 3]], out: &mut [u8]) {
    let d = hdr.bit_depth;
    for (x, o) in out.iter_mut().enumerate() {
        *o = match hdr.color_type {
            0 | 4 => (sample(row, d, x * channels) >> 8) as u8,
            2 | 6 => {
                luma16(sample(row, d, x * channels), sample(row, d, x * channels + 1), sample(row, d, x * channels + 2))
            }
            3 => {
                let p = palette.get(index(row, d, x)).copied().unwrap_or([0, 0, 0]);
                super::luma8(p[0], p[1], p[2])
            }
            _ => 0,
        };
    }
}

/// Encode an 8-bit greyscale or RGB image as PNG (test fixtures; filter 0 or Sub).
#[doc(hidden)]
pub fn encode(width: u32, height: u32, channels: u8, pixels: &[u8], sub_filter: bool) -> Vec<u8> {
    let ct = if channels == 3 { 2 } else { 0 };
    let row = width as usize * channels as usize;
    let mut raw = Vec::with_capacity((row + 1) * height as usize);
    for y in 0..height as usize {
        let line = &pixels[y * row..(y + 1) * row];
        if sub_filter {
            raw.push(1);
            for i in 0..row {
                let left = if i >= channels as usize { line[i - channels as usize] } else { 0 };
                raw.push(line[i].wrapping_sub(left));
            }
        } else {
            raw.push(0);
            raw.extend_from_slice(line);
        }
    }
    let mut out = SIGNATURE.to_vec();
    let chunk = |out: &mut Vec<u8>, typ: &[u8; 4], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut c = typ.to_vec();
        c.extend_from_slice(data);
        out.extend_from_slice(&c);
        out.extend_from_slice(&mj_formats::crc32::crc32(&c).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, ct, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6));
    chunk(&mut out, b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::perceptual::fingerprints;

    fn scene(w: usize, h: usize) -> Vec<u8> {
        (0..w * h).map(|i| (((i % w) * 255 / w) as u8) ^ (((i / w) * 7) as u8 & 0x30)).collect()
    }

    #[test]
    fn round_trips_grey_and_rgb_with_filters() {
        let (w, h) = (70, 50);
        let g = scene(w, h);
        let expect = fingerprints(w, h, &g);
        for sub in [false, true] {
            let d = decode(&encode(w as u32, h as u32, 1, &g, sub)).unwrap();
            assert_eq!((d.width, d.height, d.dhash, d.phash), (70, 50, expect.0, expect.1));
            let rgb: Vec<u8> = g.iter().flat_map(|&v| [v, v, v]).collect();
            let d = decode(&encode(w as u32, h as u32, 3, &rgb, sub)).unwrap();
            assert_eq!((d.dhash, d.phash), expect, "grey RGB has the same luma");
        }
    }

    #[test]
    fn unfilters_up_average_paeth() {
        let prev = [10u8, 20, 30, 40];
        for (f, exp) in [(2u8, [11u8, 22, 33, 44]), (3, [6, 15, 25, 36]), (4, [11, 22, 33, 44])] {
            let mut cur = [1u8, 2, 3, 4];
            unfilter(f, &mut cur, &prev, 1).unwrap();
            assert_eq!(cur, exp, "filter {f}");
        }
        assert!(unfilter(9, &mut [0], &[0], 1).is_err());
    }

    #[test]
    fn low_bit_depth_samples_scale() {
        assert_eq!(sample(&[0b1000_0000], 1, 0), 65535);
        assert_eq!(sample(&[0b0100_0000], 2, 0), 65535 / 3);
        assert_eq!(sample(&[0x0F], 4, 1), 65535);
        assert_eq!(index(&[0b0001_1011], 2, 3), 3);
    }

    #[test]
    fn hostile_files_are_errors() {
        assert!(decode(b"").is_err());
        let ok = encode(16, 16, 1, &[7; 256], false);
        for cut in 0..ok.len() {
            let _ = decode(&ok[..cut]);
        }
        let mut huge = ok.clone();
        huge[16..20].copy_from_slice(&100_000u32.to_be_bytes());
        assert_eq!(decode(&huge), Err(DecodeError::TooLarge));
        let mut inter = ok.clone();
        inter[28] = 1;
        assert_eq!(decode(&inter), Err(DecodeError::Unsupported("interlaced PNG")));
        let mut bad = ok.clone();
        bad[25] = 5; // colour type 5 does not exist
        assert!(decode(&bad).is_err());
        let mut x = 3u32;
        for _ in 0..200 {
            let mut m = ok.clone();
            for _ in 0..4 {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                let i = 33 + (x as usize >> 8) % (m.len() - 33);
                m[i] ^= (x >> 3) as u8 | 1;
            }
            let _ = decode(&m);
        }
    }
}
