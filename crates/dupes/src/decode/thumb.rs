//! Bounded thumbnails for the duplicate review (W6-A): JPEG and PNG decoded with the
//! decoders this crate already has, box-downscaled, EXIF orientation applied, and
//! encoded as a small PNG.
//!
//! Every input is hostile: the pixel count is checked against the caller's cap before
//! any pixel is decoded, every failure is a [`DecodeError`] value, and the caller
//! isolates each file with `catch_unwind` as well. Nothing here touches the file
//! system: the engine decides what may be read (never an online-only file) and how much.

use zune_jpeg::zune_core::bytestream::ZCursor;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;
use zune_jpeg::JpegDecoder;

use super::{png, within_limits, DecodeError, MAX_SIDE};

/// Largest image decoded in full for a thumbnail (pixels): a 48 MP photo. The RGB
/// buffer of such a JPEG is 144 MB, so callers also run at most one large decode at a time.
pub const THUMB_MAX_PIXELS: u64 = 48_000_000;

/// Long side of the EXIF embedded thumbnails that the fast path accepts as a source
/// (cameras write 160 x 120); a request for a larger thumbnail decodes the full image.
pub const EMBEDDED_MAX_SIDE: u32 = 160;

/// A small decoded image, 8 bits per channel, 3 (RGB) or 4 (RGBA) channels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub channels: u8,
    pub pixels: Vec<u8>,
}

/// Width and height scaled so the long side is at most `max_side` (never upscaled,
/// never below 1).
pub fn fit(w: u32, h: u32, max_side: u32) -> (u32, u32) {
    let max_side = max_side.max(1);
    if w <= max_side && h <= max_side {
        return (w.max(1), h.max(1));
    }
    let (w64, h64, m) = (w as u64, h as u64, max_side as u64);
    if w >= h {
        (max_side, ((h64 * m + w64 / 2) / w64).clamp(1, m) as u32)
    } else {
        (((w64 * m + h64 / 2) / h64).clamp(1, m) as u32, max_side)
    }
}

/// Streaming box filter: rows go in one at a time, the small image comes out. Alpha is
/// premultiplied while averaging so transparent pixels do not bleed their colour.
struct BoxScaler {
    w: usize,
    h: usize,
    tw: usize,
    th: usize,
    ch: usize,
    sums: Vec<u64>,
    counts: Vec<u64>,
    y: usize,
}

impl BoxScaler {
    fn new(w: u32, h: u32, ch: u8, max_side: u32) -> BoxScaler {
        let (tw, th) = fit(w, h, max_side);
        let (tw, th) = (tw as usize, th as usize);
        BoxScaler {
            w: w as usize,
            h: h as usize,
            tw,
            th,
            ch: ch as usize,
            sums: vec![0; tw * th * ch as usize],
            counts: vec![0; tw * th],
            y: 0,
        }
    }

    fn push_row(&mut self, row: &[u8]) {
        if self.y >= self.h {
            return;
        }
        let ty = self.y * self.th / self.h;
        let ch = self.ch;
        for (x, px) in row.chunks_exact(ch).take(self.w).enumerate() {
            let t = ty * self.tw + x * self.tw / self.w;
            let s = &mut self.sums[t * ch..(t + 1) * ch];
            if ch == 4 {
                let a = px[3] as u64;
                for i in 0..3 {
                    s[i] += px[i] as u64 * a;
                }
                s[3] += a;
            } else {
                for i in 0..ch {
                    s[i] += px[i] as u64;
                }
            }
            self.counts[t] += 1;
        }
        self.y += 1;
    }

    fn finish(self) -> Image {
        let ch = self.ch;
        let mut pixels = vec![0u8; self.tw * self.th * ch];
        for (t, &n) in self.counts.iter().enumerate() {
            if n == 0 {
                continue;
            }
            let s = &self.sums[t * ch..(t + 1) * ch];
            let o = &mut pixels[t * ch..(t + 1) * ch];
            if ch == 4 {
                for i in 0..3 {
                    o[i] = s[i].checked_div(s[3]).unwrap_or(0).min(255) as u8;
                }
                o[3] = (s[3] / n).min(255) as u8;
            } else {
                for i in 0..ch {
                    o[i] = (s[i] / n).min(255) as u8;
                }
            }
        }
        Image { width: self.tw as u32, height: self.th as u32, channels: ch as u8, pixels }
    }
}

/// Apply an EXIF/TIFF orientation (1..=8) to a decoded image; other values leave it as is.
pub fn orient(img: Image, orientation: u16) -> Image {
    if !(2..=8).contains(&orientation) {
        return img;
    }
    let (w, h, ch) = (img.width as usize, img.height as usize, img.channels as usize);
    let swap = orientation >= 5;
    let (ow, oh) = if swap { (h, w) } else { (w, h) };
    let mut out = vec![0u8; img.pixels.len()];
    for dy in 0..oh {
        for dx in 0..ow {
            let (sx, sy) = match orientation {
                2 => (w - 1 - dx, dy),
                3 => (w - 1 - dx, h - 1 - dy),
                4 => (dx, h - 1 - dy),
                5 => (dy, dx),
                6 => (dy, h - 1 - dx),
                7 => (w - 1 - dy, h - 1 - dx),
                _ => (w - 1 - dy, dx),
            };
            let s = (sy * w + sx) * ch;
            let d = (dy * ow + dx) * ch;
            out[d..d + ch].copy_from_slice(&img.pixels[s..s + ch]);
        }
    }
    Image { width: ow as u32, height: oh as u32, channels: img.channels, pixels: out }
}

/// Decode a JPEG to RGB (bounded by `max_pixels`) and downscale it.
fn jpeg_scaled(bytes: &[u8], max_side: u32, max_pixels: u64) -> Result<Image, DecodeError> {
    let opts = DecoderOptions::default()
        .jpeg_set_out_colorspace(ColorSpace::RGB)
        .set_max_width(MAX_SIDE as usize)
        .set_max_height(MAX_SIDE as usize)
        .set_strict_mode(false);
    let mut d = JpegDecoder::new_with_options(ZCursor::new(bytes), opts);
    d.decode_headers().map_err(|e| DecodeError::Corrupt(format!("{e:?}")))?;
    let info = d.info().ok_or_else(|| DecodeError::Corrupt("no frame header".into()))?;
    let (w, h) = (info.width as u32, info.height as u32);
    if !within_limits(w, h) || w as u64 * h as u64 > max_pixels {
        return Err(DecodeError::TooLarge);
    }
    let px = d.decode().map_err(|e| DecodeError::Corrupt(format!("{e:?}")))?;
    let row = w as usize * 3;
    if px.len() != row * h as usize {
        return Err(DecodeError::Corrupt(format!("unexpected output size {}", px.len())));
    }
    let mut s = BoxScaler::new(w, h, 3, max_side);
    for r in px.chunks_exact(row) {
        s.push_row(r);
    }
    Ok(s.finish())
}

/// EXIF orientation, embedded thumbnail and main image size of a JPEG buffer.
type JpegFacts<'a> = (u16, Option<&'a [u8]>, Option<(u32, u32)>);

/// EXIF facts of a JPEG buffer (a header window is enough): orientation, and the
/// embedded thumbnail when it lies entirely inside `bytes`.
fn jpeg_exif(bytes: &[u8]) -> JpegFacts<'_> {
    let Some(hdr) = mj_formats::jpeg::walk(bytes) else { return (1, None, None) };
    let frame = hdr.frame.map(|f| (f.width as u32, f.height as u32));
    let Some(range) = hdr.exif else { return (1, None, frame) };
    let Some(x) = bytes.get(range.clone()).and_then(|t| mj_formats::exif::parse(t).ok()) else {
        return (1, None, frame);
    };
    let thumb = bytes.get(range.start..).and_then(|tiff| x.thumbnail_bytes(tiff));
    (x.orientation.unwrap_or(1), thumb, frame.or(x.width.zip(x.height)))
}

/// Fast path: the EXIF embedded thumbnail, when `bytes` (a header window or the whole
/// file) holds one with the image's aspect ratio (a letterboxed thumbnail is refused)
/// and the request is small enough for it. `None` means "decode the full image".
pub fn jpeg_embedded(bytes: &[u8], max_side: u32) -> Option<Image> {
    if max_side > EMBEDDED_MAX_SIDE {
        return None;
    }
    let (orientation, thumb, main) = jpeg_exif(bytes);
    let img = jpeg_scaled(thumb?, max_side, EMBEDDED_MAX_SIDE as u64 * EMBEDDED_MAX_SIDE as u64 * 4).ok()?;
    let (mw, mh) = main?;
    if mw == 0 || mh == 0 {
        return None;
    }
    // Same aspect ratio within 3 % (thumbnails are computed from the main image).
    let main_ratio = mw as f64 / mh as f64;
    let ratio = img.width as f64 / img.height as f64;
    if (ratio / main_ratio - 1.0).abs() > 0.03 || img.width.max(img.height) < max_side.min(96) {
        return None;
    }
    Some(orient(img, orientation))
}

/// Thumbnail of a whole JPEG file: the embedded thumbnail when it fits the request,
/// otherwise the full image decoded (bounded by `max_pixels`) and downscaled.
pub fn jpeg(bytes: &[u8], max_side: u32, max_pixels: u64) -> Result<Image, DecodeError> {
    if let Some(img) = jpeg_embedded(bytes, max_side) {
        return Ok(img);
    }
    let (orientation, _, _) = jpeg_exif(bytes);
    Ok(orient(jpeg_scaled(bytes, max_side, max_pixels)?, orientation))
}

/// Thumbnail of a whole PNG file (RGBA, every colour type and depth, not interlaced).
pub fn png(bytes: &[u8], max_side: u32, max_pixels: u64) -> Result<Image, DecodeError> {
    let img = png::Prepared::new(bytes, max_pixels)?;
    let (w, h) = (img.hdr.width, img.hdr.height);
    let mut s = BoxScaler::new(w, h, 4, max_side);
    let mut rgba = vec![0u8; w as usize * 4];
    img.for_each_row(|row| {
        img.row_to_rgba(row, &mut rgba);
        s.push_row(&rgba);
    })?;
    Ok(s.finish())
}

/// Encode a small image as PNG (adaptive filters, deflate level 6). Opaque RGBA is
/// written as RGB.
pub fn encode_png(img: &Image) -> Vec<u8> {
    let opaque = img.channels == 4 && img.pixels.chunks_exact(4).all(|p| p[3] == 255);
    let (ch, pixels): (usize, std::borrow::Cow<'_, [u8]>) = if opaque {
        (3, img.pixels.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect::<Vec<u8>>().into())
    } else {
        (img.channels as usize, img.pixels.as_slice().into())
    };
    let color_type = match ch {
        1 => 0,
        3 => 2,
        _ => 6,
    };
    let row = img.width as usize * ch;
    let raw = mj_formats::png::filter_adaptive(&pixels, row, ch);
    let mut out = png::SIGNATURE.to_vec();
    let chunk = |out: &mut Vec<u8>, typ: &[u8; 4], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut c = typ.to_vec();
        c.extend_from_slice(data);
        out.extend_from_slice(&c);
        out.extend_from_slice(&mj_formats::crc32::crc32(&c).to_be_bytes());
    };
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&img.width.to_be_bytes());
    ihdr.extend_from_slice(&img.height.to_be_bytes());
    ihdr.extend_from_slice(&[8, color_type, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6));
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Pixel size of a GIF, BMP or WebP from its header, for the formats the engine passes
/// through to the webview unchanged (bounded by file size and these dimensions).
pub fn passthrough_size(bytes: &[u8]) -> Option<(&'static str, u32, u32)> {
    let le16 = |i: usize| bytes.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as u32);
    let le32 = |i: usize| bytes.get(i..i + 4).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let le24 = |i: usize| bytes.get(i..i + 3).map(|b| u32::from_le_bytes([b[0], b[1], b[2], 0]));
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some(("image/gif", le16(6)?, le16(8)?));
    }
    if bytes.starts_with(b"BM") {
        let (w, h) = (le32(18)?, le32(22)?);
        return Some(("image/bmp", w.unsigned_abs(), h.unsigned_abs()));
    }
    if bytes.len() >= 30 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        let (w, h) = match &bytes[12..16] {
            b"VP8 " if bytes.get(23..26) == Some(&[0x9d, 0x01, 0x2a]) => (le16(26)? & 0x3fff, le16(28)? & 0x3fff),
            b"VP8L" if bytes[20] == 0x2f => {
                let b = |i: usize| bytes.get(i).copied().unwrap_or(0) as u32;
                (1 + (b(21) | ((b(22) & 0x3f) << 8)), 1 + ((b(22) >> 6) | (b(23) << 2) | ((b(24) & 0x0f) << 10)))
            }
            b"VP8X" => (1 + le24(24)?, 1 + le24(27)?),
            _ => return None,
        };
        return Some(("image/webp", w, h));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(w: u32, h: u32) -> Vec<u8> {
        (0..w * h).flat_map(|i| [(i % w * 255 / w) as u8, (i / w * 255 / h) as u8, 90]).collect()
    }

    #[test]
    fn fit_keeps_the_aspect_and_never_upscales() {
        assert_eq!(fit(4000, 3000, 128), (128, 96));
        assert_eq!(fit(3000, 4000, 128), (96, 128));
        assert_eq!(fit(50, 20, 128), (50, 20));
        assert_eq!(fit(32_768, 1, 128), (128, 1));
        assert_eq!(fit(0, 0, 128), (1, 1));
    }

    #[test]
    fn png_round_trip_downscales_and_reencodes() {
        let (w, h) = (300, 200);
        let src = png::encode(w, h, 3, &gradient(w, h), true);
        let t = super::png(&src, 128, THUMB_MAX_PIXELS).unwrap();
        assert_eq!((t.width, t.height, t.channels), (128, 85, 4));
        let enc = encode_png(&t);
        // Opaque: written as RGB, and our own decoder reads it back.
        let back = super::png(&enc, 512, THUMB_MAX_PIXELS).unwrap();
        assert_eq!((back.width, back.height), (128, 85));
        assert_eq!(back.pixels, t.pixels);
        // Box average of a horizontal gradient: left is dark, right is bright.
        assert!(t.pixels[0] < 10 && t.pixels[(127) * 4] > 245);
    }

    #[test]
    fn transparent_png_keeps_alpha_without_colour_bleed() {
        let mut s = BoxScaler::new(2, 1, 4, 1);
        s.push_row(&[255, 0, 0, 255, 0, 255, 0, 0]);
        let i = s.finish();
        assert_eq!(i.pixels, vec![255, 0, 0, 127]);
        let enc = encode_png(&i);
        assert_eq!(enc[25], 6, "colour type RGBA");
    }

    #[test]
    fn orientation_rotates_and_flips() {
        // 2 x 1 image: A B.
        let img = Image { width: 2, height: 1, channels: 1, pixels: vec![1, 2] };
        assert_eq!(orient(img.clone(), 1).pixels, vec![1, 2]);
        assert_eq!(orient(img.clone(), 2).pixels, vec![2, 1]);
        let r6 = orient(img.clone(), 6);
        assert_eq!((r6.width, r6.height, r6.pixels.clone()), (1, 2, vec![1, 2]));
        let r8 = orient(img.clone(), 8);
        assert_eq!((r8.width, r8.height, r8.pixels), (1, 2, vec![2, 1]));
        // 2 x 2: 1 2 / 3 4.
        let sq = Image { width: 2, height: 2, channels: 1, pixels: vec![1, 2, 3, 4] };
        assert_eq!(orient(sq.clone(), 3).pixels, vec![4, 3, 2, 1]);
        assert_eq!(orient(sq.clone(), 4).pixels, vec![3, 4, 1, 2]);
        assert_eq!(orient(sq.clone(), 5).pixels, vec![1, 3, 2, 4]);
        assert_eq!(orient(sq.clone(), 6).pixels, vec![3, 1, 4, 2]);
        assert_eq!(orient(sq.clone(), 7).pixels, vec![4, 2, 3, 1]);
        assert_eq!(orient(sq.clone(), 8).pixels, vec![2, 4, 1, 3]);
        assert_eq!(orient(sq.clone(), 99).pixels, vec![1, 2, 3, 4]);
    }

    #[test]
    fn pixel_cap_is_checked_before_decoding() {
        let src = png::encode(300, 200, 3, &gradient(300, 200), false);
        assert_eq!(super::png(&src, 128, 10_000), Err(DecodeError::TooLarge));
        let mut huge = src.clone();
        huge[16..20].copy_from_slice(&40_000u32.to_be_bytes());
        assert_eq!(super::png(&huge, 128, THUMB_MAX_PIXELS), Err(DecodeError::TooLarge));
    }

    #[test]
    fn hostile_inputs_are_errors_not_panics() {
        assert!(super::jpeg(b"", 128, THUMB_MAX_PIXELS).is_err());
        assert!(super::jpeg(&[0xFF, 0xD8, 0xFF, 0xE1, 0, 2], 128, THUMB_MAX_PIXELS).is_err());
        assert!(super::png(b"\x89PNG\r\n\x1a\n", 128, THUMB_MAX_PIXELS).is_err());
        let ok = png::encode(40, 30, 3, &gradient(40, 30), true);
        for cut in 0..ok.len() {
            let _ = super::png(&ok[..cut], 64, THUMB_MAX_PIXELS);
        }
        let mut x = 7u32;
        for _ in 0..200 {
            let mut m = ok.clone();
            for _ in 0..4 {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                let i = 33 + (x as usize >> 8) % (m.len() - 33);
                m[i] ^= (x >> 3) as u8 | 1;
            }
            let _ = super::png(&m, 64, THUMB_MAX_PIXELS);
            let _ = super::jpeg(&m, 64, THUMB_MAX_PIXELS);
            let _ = passthrough_size(&m);
        }
        let webp = b"RIFF\0\0\0\0WEBPVP8X\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";
        for n in 0..=webp.len() {
            let _ = passthrough_size(&webp[..n]);
        }
    }

    #[test]
    fn passthrough_sizes_from_headers() {
        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&[10, 0, 20, 0]);
        assert_eq!(passthrough_size(&gif), Some(("image/gif", 10, 20)));
        let mut bmp = vec![0u8; 26];
        bmp[..2].copy_from_slice(b"BM");
        bmp[18..22].copy_from_slice(&640i32.to_le_bytes());
        bmp[22..26].copy_from_slice(&(-480i32).to_le_bytes());
        assert_eq!(passthrough_size(&bmp), Some(("image/bmp", 640, 480)));
        let mut webp = b"RIFF\0\0\0\0WEBPVP8X".to_vec();
        webp.extend_from_slice(&[0u8; 8]);
        webp.extend_from_slice(&[0xFF, 0x01, 0, 0xDF, 0x00, 0]); // 512 x 224
        webp.extend_from_slice(&[0, 0]);
        assert_eq!(passthrough_size(&webp), Some(("image/webp", 512, 224)));
        assert_eq!(passthrough_size(b"not an image"), None);
    }
}
