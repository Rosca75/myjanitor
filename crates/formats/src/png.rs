//! PNG chunk reading and writing, and lossless re-deflate of the image data
//! (OPT-IMG-01, OPT-IMG-05). Owner: W3-C.
//!
//! Hardening: every chunk length is checked against the buffer, every CRC is
//! verified (a bad CRC is an error, never "repaired"), the chunk count and the
//! inflated size are bounded, and nothing here panics on malformed input.
//!
//! The re-deflate never changes pixels: the inflated IDAT stream is either
//! byte-identical before and after, or (when rows were re-filtered) identical
//! after *un*filtering; [`recompress`] verifies this on its own output before
//! returning it.

use mj_core::{Error, Result};

/// The eight-byte PNG signature.
pub const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// Chunks walked before giving up (a real file has a handful).
pub const MAX_CHUNKS: usize = 100_000;

/// Largest inflated pixel stream this module will handle (1.5 GiB).
pub const MAX_RAW_BYTES: u64 = 1_610_612_736;

/// Above this size the adaptive re-filter pass is skipped (it needs two extra copies).
const REFILTER_MAX_RAW: usize = 256 * 1024 * 1024;

/// One PNG chunk (type and data; the length and CRC are derived on write).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub kind: [u8; 4],
    pub data: Vec<u8>,
}

impl Chunk {
    /// The type as text (for notes and tests).
    pub fn name(&self) -> String {
        String::from_utf8_lossy(&self.kind).into_owned()
    }
}

/// A parsed PNG: its chunks in file order and any bytes after IEND.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Png {
    pub chunks: Vec<Chunk>,
    /// Bytes after the IEND chunk (not part of the image; kept only under "keep all").
    pub trailing: Vec<u8>,
}

/// Image header facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ihdr {
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub color_type: u8,
    pub interlace: u8,
}

impl Ihdr {
    /// Parse the 13-byte IHDR body.
    pub fn parse(d: &[u8]) -> Result<Ihdr> {
        if d.len() != 13 {
            return Err(Error::parse("PNG", "IHDR chunk has the wrong length"));
        }
        let h = Ihdr {
            width: u32::from_be_bytes([d[0], d[1], d[2], d[3]]),
            height: u32::from_be_bytes([d[4], d[5], d[6], d[7]]),
            bit_depth: d[8],
            color_type: d[9],
            interlace: d[12],
        };
        let depth_ok = match h.color_type {
            0 => matches!(h.bit_depth, 1 | 2 | 4 | 8 | 16),
            3 => matches!(h.bit_depth, 1 | 2 | 4 | 8),
            2 | 4 | 6 => matches!(h.bit_depth, 8 | 16),
            _ => false,
        };
        if !depth_ok
            || h.width == 0
            || h.height == 0
            || h.width > 0x7FFF_FFFF
            || h.height > 0x7FFF_FFFF
            || d[10] != 0
            || d[11] != 0
            || h.interlace > 1
        {
            return Err(Error::parse("PNG", "unsupported or invalid IHDR"));
        }
        Ok(h)
    }

    fn channels(&self) -> u64 {
        match self.color_type {
            0 | 3 => 1,
            4 => 2,
            2 => 3,
            _ => 4,
        }
    }

    /// Bits per pixel.
    pub fn bits_per_pixel(&self) -> u64 {
        self.channels() * u64::from(self.bit_depth)
    }

    /// Filter "bytes per pixel" (at least 1).
    pub fn bpp(&self) -> usize {
        (self.bits_per_pixel() / 8).max(1) as usize
    }

    /// Bytes in one unfiltered scanline of `width` pixels (without the filter byte).
    fn row_bytes(&self, width: u64) -> u64 {
        (width * self.bits_per_pixel()).div_ceil(8)
    }

    /// Length of the inflated IDAT stream: filter bytes plus scanlines, all Adam7 passes included.
    pub fn expected_raw_len(&self) -> Option<u64> {
        let (w, h) = (u64::from(self.width), u64::from(self.height));
        let total = if self.interlace == 0 {
            h.checked_mul(1 + self.row_bytes(w))?
        } else {
            const PASSES: [(u64, u64, u64, u64); 7] =
                [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)];
            let mut t = 0u64;
            for (x0, y0, dx, dy) in PASSES {
                if w <= x0 || h <= y0 {
                    continue;
                }
                let pw = (w - x0).div_ceil(dx);
                let ph = (h - y0).div_ceil(dy);
                t = t.checked_add(ph.checked_mul(1 + self.row_bytes(pw))?)?;
            }
            t
        };
        (total <= MAX_RAW_BYTES).then_some(total)
    }
}

/// Parse and validate a whole PNG (signature, lengths, CRCs, IHDR first, IEND last).
pub fn parse(buf: &[u8]) -> Result<Png> {
    if buf.len() < 8 || buf[..8] != SIGNATURE {
        return Err(Error::parse("PNG", "missing PNG signature"));
    }
    let mut pos = 8usize;
    let mut chunks: Vec<Chunk> = Vec::new();
    loop {
        if chunks.len() >= MAX_CHUNKS {
            return Err(Error::parse("PNG", "too many chunks"));
        }
        let head = buf.get(pos..pos + 8).ok_or_else(|| Error::parse("PNG", "truncated before IEND"))?;
        let len = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
        let kind = [head[4], head[5], head[6], head[7]];
        if !kind.iter().all(u8::is_ascii_alphabetic) {
            return Err(Error::parse("PNG", "invalid chunk type"));
        }
        let body_end = (pos + 8)
            .checked_add(len)
            .filter(|e| e.checked_add(4).is_some_and(|c| c <= buf.len()))
            .ok_or_else(|| Error::parse("PNG", "chunk length exceeds the file"))?;
        let data = &buf[pos + 8..body_end];
        let crc = u32::from_be_bytes([buf[body_end], buf[body_end + 1], buf[body_end + 2], buf[body_end + 3]]);
        let mut c = crate::crc32::Crc32::new();
        c.update(&kind);
        c.update(data);
        if c.finish() != crc {
            return Err(Error::parse("PNG", format!("bad CRC in {} chunk", String::from_utf8_lossy(&kind))));
        }
        if chunks.is_empty() && (&kind != b"IHDR" || len != 13) {
            return Err(Error::parse("PNG", "IHDR is not the first chunk"));
        }
        chunks.push(Chunk { kind, data: data.to_vec() });
        pos = body_end + 4;
        if &kind == b"IEND" {
            break;
        }
    }
    Ok(Png { chunks, trailing: buf[pos..].to_vec() })
}

/// Serialise chunks (signature, then each chunk with its length and CRC) and `trailing`.
pub fn write(chunks: &[Chunk], trailing: &[u8]) -> Vec<u8> {
    let cap: usize = chunks.iter().map(|c| c.data.len() + 12).sum::<usize>() + 8 + trailing.len();
    let mut out = Vec::with_capacity(cap);
    out.extend_from_slice(&SIGNATURE);
    for c in chunks {
        out.extend_from_slice(&(c.data.len() as u32).to_be_bytes());
        out.extend_from_slice(&c.kind);
        out.extend_from_slice(&c.data);
        let mut h = crate::crc32::Crc32::new();
        h.update(&c.kind);
        h.update(&c.data);
        out.extend_from_slice(&h.finish().to_be_bytes());
    }
    out.extend_from_slice(trailing);
    out
}

/// Chunk types and lengths of a file without reading chunk bodies or checking CRCs
/// (cheap: one seek per chunk). Used for estimates.
pub fn summarise<R: std::io::Read + std::io::Seek>(r: &mut R, file_len: u64) -> Result<Vec<([u8; 4], u64)>> {
    let io = |e: std::io::Error| Error::Io { path: None, kind: "read", message: e.to_string() };
    let mut sig = [0u8; 8];
    r.read_exact(&mut sig).map_err(io)?;
    if sig != SIGNATURE {
        return Err(Error::parse("PNG", "missing PNG signature"));
    }
    let mut pos = 8u64;
    let mut out = Vec::new();
    while pos + 12 <= file_len && out.len() < MAX_CHUNKS {
        let mut h = [0u8; 8];
        r.read_exact(&mut h).map_err(io)?;
        let len = u64::from(u32::from_be_bytes([h[0], h[1], h[2], h[3]]));
        let kind = [h[4], h[5], h[6], h[7]];
        out.push((kind, len));
        if &kind == b"IEND" {
            break;
        }
        pos += 12 + len;
        r.seek(std::io::SeekFrom::Start(pos)).map_err(io)?;
    }
    Ok(out)
}

/// What to do with ancillary chunks (OPT-IMG-05).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataPolicy {
    /// Keep every chunk.
    KeepAll,
    /// Drop `tEXt`, `zTXt` and `iTXt` only.
    StripText,
    /// Drop every ancillary chunk except transparency (`tRNS`) and the render-critical set
    /// (colour profile, gamma, chromaticities, sRGB, pixel size, background...) unless `colour` is set.
    StripAll { colour: bool },
}

const TEXT_CHUNKS: [&[u8; 4]; 3] = [b"tEXt", b"zTXt", b"iTXt"];
/// Ancillary chunks that change how pixels look or how big they print.
const RENDER_CHUNKS: [&[u8; 4]; 11] =
    [b"iCCP", b"gAMA", b"cHRM", b"sRGB", b"sBIT", b"cICP", b"mDCV", b"cLLI", b"pHYs", b"bKGD", b"hIST"];
/// Animation chunks: a file holding one is never rewritten.
const APNG_CHUNKS: [&[u8; 4]; 3] = [b"acTL", b"fcTL", b"fdAT"];

/// True when `kind` is a render-critical colour chunk (dropping it can change what the image looks like).
pub fn is_colour_chunk(kind: &[u8; 4]) -> bool {
    RENDER_CHUNKS.contains(&kind)
}

/// True when `kind` must be removed under `policy`.
pub fn chunk_removed(kind: &[u8; 4], policy: MetadataPolicy) -> bool {
    // Critical chunks (upper-case first letter) and transparency are never removed.
    if kind[0].is_ascii_uppercase() || kind == b"tRNS" {
        return false;
    }
    match policy {
        MetadataPolicy::KeepAll => false,
        MetadataPolicy::StripText => TEXT_CHUNKS.contains(&kind),
        MetadataPolicy::StripAll { colour } => colour || !RENDER_CHUNKS.contains(&kind),
    }
}

impl Png {
    /// The header.
    pub fn ihdr(&self) -> Result<Ihdr> {
        Ihdr::parse(&self.chunks.first().ok_or_else(|| Error::parse("PNG", "no chunks"))?.data)
    }

    /// True when any APNG chunk is present.
    pub fn is_animated(&self) -> bool {
        self.chunks.iter().any(|c| APNG_CHUNKS.contains(&&c.kind))
    }

    /// The IDAT chunks concatenated (the zlib stream).
    pub fn idat(&self) -> Vec<u8> {
        let mut v = Vec::new();
        for c in self.chunks.iter().filter(|c| &c.kind == b"IDAT") {
            v.extend_from_slice(&c.data);
        }
        v
    }
}

/// Result of [`recompress`].
#[derive(Debug, Clone)]
pub struct Recompressed {
    pub bytes: Vec<u8>,
    /// Inflated pixel-stream length (identical before and after).
    pub raw_len: usize,
    /// Rows were re-filtered (the pixel stream is equal after unfiltering, not byte-equal).
    pub refiltered: bool,
    /// Chunk types removed by the policy.
    pub removed: Vec<String>,
    /// A colour-critical chunk was removed (output may look different).
    pub colour_changed: bool,
}

fn inflate_idat(png: &Png, ihdr: &Ihdr) -> Result<(Vec<u8>, Vec<u8>)> {
    let expected = ihdr.expected_raw_len().ok_or_else(|| Error::unsupported("PNG", "image too large to recompress"))?;
    let idat = png.idat();
    if idat.is_empty() {
        return Err(Error::parse("PNG", "no image data"));
    }
    let raw = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&idat, expected as usize)
        .map_err(|_| Error::parse("PNG", "image data does not inflate"))?;
    if raw.len() as u64 != expected {
        return Err(Error::parse("PNG", "image data has the wrong length"));
    }
    Ok((idat, raw))
}

/// Re-deflate the image data of `buf` at `level` (1-10; 10 is miniz "uber"), optionally trying
/// adaptive per-row filters, apply the metadata `policy`, and verify the decoded pixels equal the
/// original's. APNG and corrupt files are an error (callers skip them).
pub fn recompress(buf: &[u8], policy: MetadataPolicy, level: u8, try_refilter: bool) -> Result<Recompressed> {
    let png = parse(buf)?;
    if png.is_animated() {
        return Err(Error::unsupported("PNG", "animated PNG (APNG) is left untouched"));
    }
    let ihdr = png.ihdr()?;
    let (old_idat, raw) = inflate_idat(&png, &ihdr)?;
    let level = level.clamp(1, 10);

    // Candidate A: the filtered rows as they are, deflated at the highest level.
    let mut best = miniz_oxide::deflate::compress_to_vec_zlib(&raw, level);
    let mut refiltered = false;
    // Candidate B: per-row adaptive filters (non-interlaced only).
    if try_refilter && ihdr.interlace == 0 && raw.len() <= REFILTER_MAX_RAW {
        let row = ihdr.row_bytes(u64::from(ihdr.width)) as usize;
        if let Ok(pixels) = unfilter(&raw, row, ihdr.bpp()) {
            let cand = miniz_oxide::deflate::compress_to_vec_zlib(&filter_adaptive(&pixels, row, ihdr.bpp()), level);
            if cand.len() < best.len() {
                best = cand;
                refiltered = true;
            }
        }
    }
    // Candidate C: the original stream (already better compressed by another tool).
    let keep_original = old_idat.len() <= best.len();
    let new_idat = if keep_original { old_idat.clone() } else { best };
    if keep_original {
        refiltered = false;
    }

    let mut chunks = Vec::with_capacity(png.chunks.len());
    let (mut removed, mut colour_changed, mut wrote_idat) = (Vec::new(), false, false);
    for c in &png.chunks {
        if &c.kind == b"IDAT" {
            if !wrote_idat {
                chunks.push(Chunk { kind: *b"IDAT", data: new_idat.clone() });
                wrote_idat = true;
            }
        } else if chunk_removed(&c.kind, policy) {
            colour_changed |= is_colour_chunk(&c.kind);
            removed.push(c.name());
        } else {
            chunks.push(c.clone());
        }
    }
    let trailing: &[u8] = if policy == MetadataPolicy::KeepAll { &png.trailing } else { &[] };
    let bytes = write(&chunks, trailing);
    verify_same_pixels(&png, &bytes, &ihdr, refiltered)?;
    Ok(Recompressed { bytes, raw_len: raw.len(), refiltered, removed, colour_changed })
}

/// Re-parse `out` and check header, palette, transparency and pixels equal the original's.
fn verify_same_pixels(orig: &Png, out: &[u8], ihdr: &Ihdr, refiltered: bool) -> Result<()> {
    let bad = |m: &str| Error::Internal(format!("PNG verification failed: {m}"));
    let new = parse(out).map_err(|_| bad("output does not parse"))?;
    if new.ihdr()? != *ihdr || orig.chunks[0] != new.chunks[0] {
        return Err(bad("header changed"));
    }
    for kind in [b"PLTE", b"tRNS"] {
        let a = orig.chunks.iter().find(|c| &c.kind == kind);
        if a != new.chunks.iter().find(|c| &c.kind == kind) {
            return Err(bad("palette or transparency changed"));
        }
    }
    let (_, a) = inflate_idat(orig, ihdr)?;
    let (_, b) = inflate_idat(&new, ihdr)?;
    if a == b {
        return Ok(());
    }
    if refiltered {
        let row = ihdr.row_bytes(u64::from(ihdr.width)) as usize;
        if unfilter(&a, row, ihdr.bpp())? == unfilter(&b, row, ihdr.bpp())? {
            return Ok(());
        }
    }
    Err(bad("pixel data changed"))
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let (ia, ib, ic) = (i32::from(a), i32::from(b), i32::from(c));
    let p = ia + ib - ic;
    let (pa, pb, pc) = ((p - ia).abs(), (p - ib).abs(), (p - ic).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// Undo PNG row filters: `raw` is `height * (1 + row)` bytes; returns `height * row` pixel bytes.
pub fn unfilter(raw: &[u8], row: usize, bpp: usize) -> Result<Vec<u8>> {
    let stride = row + 1;
    if row == 0 || !raw.len().is_multiple_of(stride) {
        return Err(Error::parse("PNG", "scanline data has the wrong length"));
    }
    let rows = raw.len() / stride;
    let mut out = vec![0u8; rows * row];
    for y in 0..rows {
        let ft = raw[y * stride];
        let src = &raw[y * stride + 1..(y + 1) * stride];
        let (prev_rows, cur_rows) = out.split_at_mut(y * row);
        let prev: &[u8] = if y == 0 { &[] } else { &prev_rows[(y - 1) * row..] };
        let cur = &mut cur_rows[..row];
        for x in 0..row {
            let a = if x >= bpp { cur[x - bpp] } else { 0 };
            let b = prev.get(x).copied().unwrap_or(0);
            let c = if x >= bpp { prev.get(x - bpp).copied().unwrap_or(0) } else { 0 };
            let add = match ft {
                0 => 0,
                1 => a,
                2 => b,
                3 => ((u16::from(a) + u16::from(b)) / 2) as u8,
                4 => paeth(a, b, c),
                _ => return Err(Error::parse("PNG", "invalid filter type")),
            };
            cur[x] = src[x].wrapping_add(add);
        }
    }
    Ok(out)
}

/// Filter every row with whichever of the five filters has the smallest sum of absolute
/// (signed) residuals (the classic libpng heuristic).
pub fn filter_adaptive(pixels: &[u8], row: usize, bpp: usize) -> Vec<u8> {
    let rows = pixels.len().checked_div(row).unwrap_or(0);
    let mut out = Vec::with_capacity(rows * (row + 1));
    let mut cand: Vec<Vec<u8>> = vec![vec![0u8; row]; 5];
    for y in 0..rows {
        let cur = &pixels[y * row..(y + 1) * row];
        let prev: &[u8] = if y == 0 { &[] } else { &pixels[(y - 1) * row..y * row] };
        for x in 0..row {
            let a = if x >= bpp { cur[x - bpp] } else { 0 };
            let b = prev.get(x).copied().unwrap_or(0);
            let c = if x >= bpp { prev.get(x - bpp).copied().unwrap_or(0) } else { 0 };
            cand[0][x] = cur[x];
            cand[1][x] = cur[x].wrapping_sub(a);
            cand[2][x] = cur[x].wrapping_sub(b);
            cand[3][x] = cur[x].wrapping_sub(((u16::from(a) + u16::from(b)) / 2) as u8);
            cand[4][x] = cur[x].wrapping_sub(paeth(a, b, c));
        }
        let score = |v: &Vec<u8>| v.iter().map(|&b| u64::from((b as i8).unsigned_abs())).sum::<u64>();
        let best = (0..5).min_by_key(|&f| score(&cand[f])).unwrap_or(0);
        out.push(best as u8);
        out.extend_from_slice(&cand[best]);
    }
    out
}

/// A decoded image, normalised to RGBA with 16 bits per channel (8-bit values are replicated, so
/// the same picture stored at different depths or as palette/greyscale/RGB compares equal).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    pub rgba16: Vec<u16>,
}

/// Largest image [`decode_rgba16`] will decode (32 megapixels = 256 MiB of samples).
pub const MAX_DECODE_PIXELS: u64 = 32 * 1024 * 1024;

fn sample(row: &[u8], n: usize, depth: usize) -> u16 {
    match depth {
        16 => u16::from_be_bytes([row.get(n * 2).copied().unwrap_or(0), row.get(n * 2 + 1).copied().unwrap_or(0)]),
        8 => u16::from(row.get(n).copied().unwrap_or(0)),
        d => {
            let bit = n * d;
            let byte = row.get(bit / 8).copied().unwrap_or(0);
            u16::from((byte >> (8 - d - bit % 8)) & ((1u8 << d) - 1))
        }
    }
}

fn scale16(v: u16, depth: usize) -> u16 {
    match depth {
        16 => v,
        8 => v * 257,
        d => (u32::from(v) * 65_535 / ((1u32 << d) - 1)) as u16,
    }
}

/// Decode to RGBA16 (every colour type, bit depth, `PLTE`, `tRNS` and Adam7). Used to verify that an
/// external optimiser (oxipng) kept every pixel, whatever reductions it applied.
pub fn decode_rgba16(buf: &[u8]) -> Result<Decoded> {
    let png = parse(buf)?;
    if png.is_animated() {
        return Err(Error::unsupported("PNG", "animated PNG"));
    }
    let ih = png.ihdr()?;
    if u64::from(ih.width) * u64::from(ih.height) > MAX_DECODE_PIXELS {
        return Err(Error::unsupported("PNG", "image too large to verify"));
    }
    let (_, raw) = inflate_idat(&png, &ih)?;
    let plte = png.chunks.iter().find(|c| &c.kind == b"PLTE").map(|c| c.data.as_slice()).unwrap_or(&[]);
    let trns = png.chunks.iter().find(|c| &c.kind == b"tRNS").map(|c| c.data.as_slice()).unwrap_or(&[]);
    let (w, h, depth) = (ih.width as usize, ih.height as usize, usize::from(ih.bit_depth));
    let mut out = vec![0u16; w * h * 4];
    let key = |i: usize| -> Option<u16> { (trns.len() >= i * 2 + 2).then(|| u16::from_be_bytes([trns[i * 2], trns[i * 2 + 1]])) };
    let mut put = |x: usize, y: usize, row: &[u8], i: usize| {
        let at = (y * w + x) * 4;
        let px = match ih.color_type {
            0 => {
                let s = sample(row, i, depth);
                let v = scale16(s, depth);
                [v, v, v, if key(0).is_some_and(|k| k & ((1u32 << depth) - 1) as u16 == s) { 0 } else { 65_535 }]
            }
            2 => {
                let s = [sample(row, i * 3, depth), sample(row, i * 3 + 1, depth), sample(row, i * 3 + 2, depth)];
                let transparent = (0..3).all(|c| key(c).is_some_and(|k| k == s[c]));
                [scale16(s[0], depth), scale16(s[1], depth), scale16(s[2], depth), if transparent { 0 } else { 65_535 }]
            }
            3 => {
                let idx = usize::from(sample(row, i, depth));
                let c = |o: usize| u16::from(plte.get(idx * 3 + o).copied().unwrap_or(0)) * 257;
                [c(0), c(1), c(2), trns.get(idx).map_or(65_535, |&a| u16::from(a) * 257)]
            }
            4 => {
                let v = scale16(sample(row, i * 2, depth), depth);
                [v, v, v, scale16(sample(row, i * 2 + 1, depth), depth)]
            }
            _ => [0, 1, 2, 3].map(|c| scale16(sample(row, i * 4 + c, depth), depth)),
        };
        out[at..at + 4].copy_from_slice(&px);
    };
    if ih.interlace == 0 {
        let row_len = ih.row_bytes(w as u64) as usize;
        let pixels = unfilter(&raw, row_len, ih.bpp())?;
        for y in 0..h {
            let row = &pixels[y * row_len..(y + 1) * row_len];
            for x in 0..w {
                put(x, y, row, x);
            }
        }
    } else {
        const PASSES: [(usize, usize, usize, usize); 7] =
            [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)];
        let mut at = 0usize;
        for (x0, y0, dx, dy) in PASSES {
            if w <= x0 || h <= y0 {
                continue;
            }
            let (pw, ph) = ((w - x0).div_ceil(dx), (h - y0).div_ceil(dy));
            let row_len = ih.row_bytes(pw as u64) as usize;
            let len = ph * (1 + row_len);
            let pixels = unfilter(raw.get(at..at + len).ok_or_else(|| Error::parse("PNG", "interlaced data too short"))?, row_len, ih.bpp())?;
            at += len;
            for j in 0..ph {
                let row = &pixels[j * row_len..(j + 1) * row_len];
                for i in 0..pw {
                    put(x0 + i * dx, y0 + j * dy, row, i);
                }
            }
        }
    }
    Ok(Decoded { width: ih.width, height: ih.height, rgba16: out })
}

/// Test-support builders for hand-made PNGs (also used by `mj-optimise` tests).
#[doc(hidden)]
pub mod build {
    use super::*;

    /// A chunk with type `kind`.
    pub fn chunk(kind: &[u8; 4], data: &[u8]) -> Chunk {
        Chunk { kind: *kind, data: data.to_vec() }
    }

    /// IHDR body.
    pub fn ihdr(w: u32, h: u32, depth: u8, color: u8, interlace: u8) -> Chunk {
        let mut d = Vec::new();
        d.extend_from_slice(&w.to_be_bytes());
        d.extend_from_slice(&h.to_be_bytes());
        d.extend_from_slice(&[depth, color, 0, 0, interlace]);
        chunk(b"IHDR", &d)
    }

    /// A deterministic RGB8 gradient image with every row stored with filter 0 and the stream
    /// deflated at a weak level (so recompression has something to win). `extra` chunks go
    /// between IHDR and IDAT; the data is split over `idat_parts` IDAT chunks.
    pub fn rgb_png(w: u32, h: u32, extra: &[Chunk], idat_parts: usize) -> Vec<u8> {
        let mut raw = Vec::new();
        for y in 0..h {
            raw.push(0);
            for x in 0..w {
                raw.extend_from_slice(&[(x * 3) as u8, (y * 2) as u8, ((x + y) / 2) as u8]);
            }
        }
        let z = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 1);
        let mut chunks = vec![ihdr(w, h, 8, 2, 0)];
        chunks.extend_from_slice(extra);
        let part = z.len().div_ceil(idat_parts.max(1)).max(1);
        for p in z.chunks(part) {
            chunks.push(chunk(b"IDAT", p));
        }
        chunks.push(chunk(b"IEND", &[]));
        write(&chunks, &[])
    }
}

#[cfg(test)]
mod tests {
    use super::build::*;
    use super::*;

    fn kinds(p: &Png) -> Vec<String> {
        p.chunks.iter().map(Chunk::name).collect()
    }

    #[test]
    fn round_trip_keeps_pixels_and_is_not_larger() {
        let extra = [chunk(b"gAMA", &[0, 1, 0, 0]), chunk(b"tEXt", b"Comment\0hello")];
        let src = rgb_png(64, 48, &extra, 3);
        let r = recompress(&src, MetadataPolicy::KeepAll, 10, true).unwrap();
        assert!(r.bytes.len() < src.len(), "{} vs {}", r.bytes.len(), src.len());
        let (a, b) = (parse(&src).unwrap(), parse(&r.bytes).unwrap());
        assert_eq!(kinds(&b), ["IHDR", "gAMA", "tEXt", "IDAT", "IEND"]);
        let ih = a.ihdr().unwrap();
        let ra = inflate_idat(&a, &ih).unwrap().1;
        let rb = inflate_idat(&b, &ih).unwrap().1;
        if r.refiltered {
            assert_eq!(unfilter(&ra, 64 * 3, 3).unwrap(), unfilter(&rb, 64 * 3, 3).unwrap());
        } else {
            assert_eq!(ra, rb);
        }
        assert_eq!(r.raw_len, 48 * (1 + 64 * 3));
    }

    #[test]
    fn already_optimal_stream_is_never_made_larger() {
        let src = rgb_png(32, 32, &[], 1);
        let once = recompress(&src, MetadataPolicy::KeepAll, 10, true).unwrap().bytes;
        let twice = recompress(&once, MetadataPolicy::KeepAll, 10, true).unwrap().bytes;
        assert!(twice.len() <= once.len());
    }

    #[test]
    fn metadata_policies() {
        let extra = [
            chunk(b"iCCP", b"p\0\0xx"),
            chunk(b"gAMA", &[0, 0, 1, 0]),
            chunk(b"pHYs", &[0; 9]),
            chunk(b"tIME", &[0; 7]),
            chunk(b"tEXt", b"a\0b"),
            chunk(b"zTXt", b"a\0\0x"),
            chunk(b"vpAg", &[1, 2, 3]),
        ];
        let src = rgb_png(16, 16, &extra, 1);
        let k = |p| kinds(&parse(&recompress(&src, p, 6, false).unwrap().bytes).unwrap());
        assert_eq!(k(MetadataPolicy::KeepAll).len(), 1 + 7 + 1 + 1);
        assert_eq!(k(MetadataPolicy::StripText), ["IHDR", "iCCP", "gAMA", "pHYs", "tIME", "vpAg", "IDAT", "IEND"]);
        let all = recompress(&src, MetadataPolicy::StripAll { colour: false }, 6, false).unwrap();
        assert_eq!(kinds(&parse(&all.bytes).unwrap()), ["IHDR", "iCCP", "gAMA", "pHYs", "IDAT", "IEND"]);
        assert!(!all.colour_changed);
        let col = recompress(&src, MetadataPolicy::StripAll { colour: true }, 6, false).unwrap();
        assert_eq!(kinds(&parse(&col.bytes).unwrap()), ["IHDR", "IDAT", "IEND"]);
        assert!(col.colour_changed && col.removed.contains(&"iCCP".to_string()));
    }

    #[test]
    fn transparency_is_never_removed() {
        let extra = [chunk(b"tRNS", &[0, 0, 0, 0, 0, 0])];
        let src = rgb_png(8, 8, &extra, 1);
        let r = recompress(&src, MetadataPolicy::StripAll { colour: true }, 6, false).unwrap();
        assert!(kinds(&parse(&r.bytes).unwrap()).contains(&"tRNS".to_string()));
    }

    #[test]
    fn corrupt_crc_and_garbage_are_rejected() {
        let mut src = rgb_png(8, 8, &[], 1);
        let n = src.len();
        src[n - 20] ^= 0xFF; // inside the IDAT data or its CRC
        assert!(matches!(parse(&src), Err(Error::Parse { .. })));
        assert!(recompress(&src, MetadataPolicy::KeepAll, 6, false).is_err());
        assert!(parse(b"not a png").is_err());
        let good = rgb_png(8, 8, &[], 1);
        for cut in 0..good.len() {
            let _ = parse(&good[..cut]);
            let _ = recompress(&good[..cut], MetadataPolicy::KeepAll, 1, true);
        }
    }

    #[test]
    fn apng_is_skipped() {
        let extra = [chunk(b"acTL", &[0, 0, 0, 2, 0, 0, 0, 0])];
        let src = rgb_png(8, 8, &extra, 1);
        assert!(matches!(recompress(&src, MetadataPolicy::KeepAll, 6, false), Err(Error::Unsupported { .. })));
    }

    #[test]
    fn wrong_pixel_length_is_rejected_not_rewritten() {
        let mut chunks = vec![ihdr(8, 8, 8, 2, 0)];
        chunks.push(chunk(b"IDAT", &miniz_oxide::deflate::compress_to_vec_zlib(&[0u8; 10], 1)));
        chunks.push(chunk(b"IEND", &[]));
        assert!(recompress(&write(&chunks, &[]), MetadataPolicy::KeepAll, 6, false).is_err());
    }

    #[test]
    fn trailing_bytes_follow_the_policy() {
        let mut src = rgb_png(8, 8, &[], 1);
        src.extend_from_slice(b"TRAILER");
        assert!(recompress(&src, MetadataPolicy::KeepAll, 6, false).unwrap().bytes.ends_with(b"TRAILER"));
        assert!(!recompress(&src, MetadataPolicy::StripText, 6, false).unwrap().bytes.ends_with(b"TRAILER"));
    }

    #[test]
    fn expected_length_for_interlaced_and_packed_images() {
        let i = Ihdr::parse(&ihdr(8, 8, 8, 2, 1).data).unwrap();
        assert_eq!(i.expected_raw_len(), Some(8 * 8 * 3 + 15));
        let p = Ihdr::parse(&ihdr(10, 3, 1, 0, 0).data).unwrap();
        assert_eq!(p.expected_raw_len(), Some(3 * (1 + 2)));
        assert!(Ihdr::parse(&ihdr(0, 3, 8, 2, 0).data).is_err());
        assert!(Ihdr::parse(&ihdr(3, 3, 3, 2, 0).data).is_err());
    }

    #[test]
    fn filters_round_trip() {
        let pixels: Vec<u8> = (0..4 * 30).map(|i| (i * 7 % 251) as u8).collect();
        for bpp in [1, 3, 4] {
            let f = filter_adaptive(&pixels, 30, bpp);
            assert_eq!(unfilter(&f, 30, bpp).unwrap(), pixels);
        }
        assert!(unfilter(&[9, 0, 0], 2, 1).is_err());
    }

    #[test]
    fn decode_normalises_depths_and_palettes() {
        let img = decode_rgba16(&rgb_png(4, 2, &[], 1)).unwrap();
        assert_eq!((img.width, img.height), (4, 2));
        // pixel (x=2, y=1): r = 6, g = 2, b = 1
        let at = (4 + 2) * 4;
        assert_eq!(&img.rgba16[at..at + 4], &[6 * 257, 2 * 257, 257, 65_535]);
        // 2-bit greyscale 3x2 (rows padded to a byte): 0 1 2 / 3 0 1
        let raw = [0u8, 0b0001_1000, 0, 0b1100_0100];
        let z = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6);
        let g = write(&[build::ihdr(3, 2, 2, 0, 0), build::chunk(b"IDAT", &z), build::chunk(b"IEND", &[])], &[]);
        let d = decode_rgba16(&g).unwrap();
        let grey: Vec<u16> = d.rgba16.chunks(4).map(|p| p[0]).collect();
        assert_eq!(grey, [0, 21845, 43690, 65535, 0, 21845]);
        // palette + tRNS: index 1 is transparent
        let pz = miniz_oxide::deflate::compress_to_vec_zlib(&[0u8, 0, 1], 6);
        let p = write(
            &[build::ihdr(2, 1, 8, 3, 0), build::chunk(b"PLTE", &[255, 0, 0, 0, 255, 0]), build::chunk(b"tRNS", &[255, 0]), build::chunk(b"IDAT", &pz), build::chunk(b"IEND", &[])],
            &[],
        );
        let d = decode_rgba16(&p).unwrap();
        assert_eq!(d.rgba16, [65535, 0, 0, 65535, 0, 65535, 0, 0]);
    }

    #[test]
    fn decode_handles_adam7() {
        // 8x8 greyscale where pixel (x, y) = y * 8 + x, stored interlaced with filter 0.
        let mut raw = Vec::new();
        for (x0, y0, dx, dy) in [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)] {
            for y in (y0..8).step_by(dy) {
                raw.push(0);
                for x in (x0..8).step_by(dx) {
                    raw.push((y * 8 + x) as u8);
                }
            }
        }
        let z = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6);
        let png = write(&[build::ihdr(8, 8, 8, 0, 1), build::chunk(b"IDAT", &z), build::chunk(b"IEND", &[])], &[]);
        let d = decode_rgba16(&png).unwrap();
        let grey: Vec<u16> = d.rgba16.chunks(4).map(|p| p[0] / 257).collect();
        assert_eq!(grey, (0..64).collect::<Vec<u16>>());
    }

    #[test]
    fn summarise_lists_chunks() {
        let src = rgb_png(8, 8, &[chunk(b"tEXt", b"a\0b")], 1);
        let got = summarise(&mut std::io::Cursor::new(&src), src.len() as u64).unwrap();
        let k: Vec<_> = got.iter().map(|(k, _)| String::from_utf8_lossy(k).into_owned()).collect();
        assert_eq!(k, ["IHDR", "tEXt", "IDAT", "IEND"]);
    }
}
