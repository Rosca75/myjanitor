//! In-house PNG decoder to luma, for perceptual hashing only.
//!
//! Supports every colour type (grey, RGB, palette, grey+alpha, RGBA) and bit
//! depth (1, 2, 4, 8, 16), non-interlaced. Interlaced (Adam7) files are
//! reported as unsupported (skipped for perceptual matching, still
//! exact-matched). Alpha is ignored and CRCs are not verified (a corrupt
//! stream fails inflate or the size checks). The IDAT stream is inflated one
//! scanline at a time, unfiltered and fed straight into the grid accumulator, so
//! memory is O(width): neither the raw image nor a grey buffer is ever kept. Hardened: chunk lengths are bounds-checked, the inflated size is
//! limited to exactly what the header implies, and dimensions are checked
//! against [`super::within_limits`] before inflating.

use miniz_oxide::inflate::stream::{inflate, InflateState};
use miniz_oxide::{DataFormat, MZError, MZFlush, MZStatus};

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
    let img = Prepared::new(buf, super::MAX_PIXELS)?;
    let (w, h) = (img.hdr.width as usize, img.hdr.height as usize);
    let mut gray = vec![0u8; w];
    let mut acc = GridAccumulator::new(w, h);
    img.for_each_row(|row| {
        row_to_gray(&img.hdr, img.channels, row, &img.palette, &mut gray);
        acc.push_row(&gray);
    })?;
    let g = acc.finish();
    Ok(Decoded { width: img.hdr.width, height: img.hdr.height, dhash: dhash(&g.g9), phash: phash(&g.g32) })
}

/// A PNG whose header, palette and IDAT chunks are located and checked, ready to be
/// inflated and unfiltered row by row (shared by the fingerprint and the thumbnail
/// decoders). Nothing is inflated or copied up front (DUP-5).
pub(crate) struct Prepared<'a> {
    pub hdr: Header,
    pub channels: usize,
    pub palette: Vec<[u8; 3]>,
    /// Palette alpha (`tRNS` of a palette image); missing entries are opaque.
    pub palette_alpha: Vec<u8>,
    /// The IDAT chunk payloads, in file order, borrowed from the file buffer.
    idat: Vec<&'a [u8]>,
    row_bytes: usize,
    bpp: usize,
}

impl<'a> Prepared<'a> {
    /// Check the header against `max_pixels` (and [`within_limits`]) before inflating.
    pub fn new(buf: &'a [u8], max_pixels: u64) -> Result<Prepared<'a>, DecodeError> {
        let hdr = header(buf).ok_or_else(|| DecodeError::Corrupt("missing IHDR".into()))?;
        if !within_limits(hdr.width, hdr.height) || hdr.width as u64 * hdr.height as u64 > max_pixels {
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

        // Locate the IDAT chunks (no copy), PLTE and tRNS.
        let mut pos = 8usize;
        let mut idat: Vec<&[u8]> = Vec::new();
        let mut idat_len = 0usize;
        let mut palette: Vec<[u8; 3]> = Vec::new();
        let mut palette_alpha: Vec<u8> = Vec::new();
        loop {
            let len = buf.get(pos..pos + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize);
            let Some(len) = len else { break };
            let typ = buf.get(pos + 4..pos + 8).ok_or_else(|| DecodeError::Corrupt("truncated chunk".into()))?;
            let data_end = pos.checked_add(8 + len).ok_or_else(|| DecodeError::Corrupt("chunk length".into()))?;
            let data = buf.get(pos + 8..data_end).ok_or_else(|| DecodeError::Corrupt("truncated chunk".into()))?;
            match typ {
                b"IDAT" => {
                    idat_len += data.len();
                    if idat_len > expected.saturating_mul(2) + 1024 * 1024 {
                        return Err(DecodeError::Corrupt("IDAT larger than the image".into()));
                    }
                    idat.push(data);
                }
                b"PLTE" => palette = data.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect(),
                b"tRNS" if hdr.color_type == 3 => palette_alpha = data.iter().take(256).copied().collect(),
                b"IEND" => break,
                _ => {}
            }
            pos = data_end + 4; // skip CRC
        }
        if hdr.color_type == 3 && palette.is_empty() {
            return Err(DecodeError::Corrupt("palette image without PLTE".into()));
        }
        let bpp = bits_per_pixel.div_ceil(8).max(1);
        Ok(Prepared { hdr, channels, palette, palette_alpha, idat, row_bytes, bpp })
    }

    /// Inflate, unfilter and hand every row in order to `f` (raw samples at the file's
    /// depth). DUP-5: one scanline is inflated at a time and unfiltered against the
    /// previous one, so memory is O(width) however large the raw image is (it was the
    /// whole raw image, 8 bytes a pixel for RGBA16, from a file that compresses ~1000:1).
    pub fn for_each_row(&self, mut f: impl FnMut(&[u8])) -> Result<(), DecodeError> {
        let h = self.hdr.height as usize;
        let mut prev = vec![0u8; self.row_bytes];
        let mut line = vec![0u8; self.row_bytes + 1];
        let mut state = InflateState::new_boxed(DataFormat::Zlib);
        let (mut filled, mut y, mut ended) = (0usize, 0usize, false);
        let mut chunks = self.idat.iter().copied();
        let mut input: &[u8] = chunks.next().unwrap_or(&[]);
        while y < h && !ended {
            let r = inflate(&mut state, input, &mut line[filled..], MZFlush::None);
            input = &input[r.bytes_consumed..];
            filled += r.bytes_written;
            if filled == line.len() {
                let (filter, cur) = line.split_at_mut(1);
                unfilter(filter[0], cur, &prev, self.bpp)?;
                f(cur);
                prev.copy_from_slice(cur);
                filled = 0;
                y += 1;
            }
            match r.status {
                Ok(MZStatus::StreamEnd) => ended = true,
                Ok(_) | Err(MZError::Buf) => {}
                Err(e) => return Err(DecodeError::Corrupt(format!("inflate: {e:?}"))),
            }
            if r.bytes_consumed == 0 && r.bytes_written == 0 {
                if !input.is_empty() {
                    return Err(DecodeError::Corrupt("inflate made no progress".into()));
                }
                // Everything buffered is out: feed the next chunk, or stop.
                match chunks.next() {
                    Some(c) => input = c,
                    None => break,
                }
            }
        }
        if y < h {
            return Err(DecodeError::Corrupt("image data truncated".into()));
        }
        Ok(())
    }

    /// One unfiltered row as 8-bit RGBA (palette alpha from `tRNS`).
    pub fn row_to_rgba(&self, row: &[u8], out: &mut [u8]) {
        let d = self.hdr.bit_depth;
        let ch = self.channels;
        for (x, o) in out.chunks_exact_mut(4).enumerate() {
            let s = |i: usize| (sample(row, d, x * ch + i) >> 8) as u8;
            let px = match self.hdr.color_type {
                0 => [s(0), s(0), s(0), 255],
                4 => [s(0), s(0), s(0), s(1)],
                2 => [s(0), s(1), s(2), 255],
                6 => [s(0), s(1), s(2), s(3)],
                _ => {
                    let i = index(row, d, x);
                    let p = self.palette.get(i).copied().unwrap_or([0, 0, 0]);
                    [p[0], p[1], p[2], self.palette_alpha.get(i).copied().unwrap_or(255)]
                }
            };
            o.copy_from_slice(&px);
        }
    }
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

    /// Re-chunk the IDAT stream of `file` into pieces of `n` bytes.
    fn split_idat(file: &[u8], n: usize) -> Vec<u8> {
        let mut pos = 8;
        let (mut head, mut z, mut tail) = (file[..8].to_vec(), Vec::new(), Vec::new());
        while pos + 8 <= file.len() {
            let len = u32::from_be_bytes(file[pos..pos + 4].try_into().unwrap()) as usize;
            let chunk = &file[pos..pos + 12 + len];
            match &file[pos + 4..pos + 8] {
                b"IDAT" => z.extend_from_slice(&chunk[8..8 + len]),
                _ if z.is_empty() => head.extend_from_slice(chunk),
                _ => tail.extend_from_slice(chunk),
            }
            pos += 12 + len;
        }
        for part in z.chunks(n) {
            head.extend_from_slice(&(part.len() as u32).to_be_bytes());
            head.extend_from_slice(b"IDAT");
            head.extend_from_slice(part);
            head.extend_from_slice(&[0; 4]);
        }
        head.extend_from_slice(&tail);
        head
    }

    #[test]
    fn idat_split_anywhere_decodes_the_same() {
        let (w, h) = (70, 50);
        let g = scene(w, h);
        let file = encode(w as u32, h as u32, 1, &g, true);
        let whole = decode(&file).unwrap();
        for n in [1, 7, 100, 4096] {
            assert_eq!(decode(&split_idat(&file, n)).unwrap(), whole, "chunks of {n}");
        }
        // A stream cut short is corrupt, never a panic or a hang.
        let cut = split_idat(&file, 7);
        let short: Vec<u8> = cut[..cut.len() / 2].to_vec();
        assert!(matches!(decode(&short), Err(DecodeError::Corrupt(_))));
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
