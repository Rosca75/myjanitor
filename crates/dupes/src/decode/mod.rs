//! Pixel sources for perceptual hashing (W2-B).
//!
//! - JPEG: the `zune-jpeg` crate (baseline + progressive, decodes straight to
//!   luma, pure Rust); see `docs/dependencies/W2-B.md` for why it is not in-house.
//! - PNG: in-house on top of `miniz_oxide` inflate ([`png`]).
//!
//! Both return the image size and its two fingerprints computed from the
//! box-averaged grids (`crate::perceptual`). Decoding is bounded: images over
//! [`MAX_PIXELS`] or with a side over [`MAX_SIDE`] are refused before any pixel
//! is decoded, and every failure is an error value, never a panic (the callers
//! additionally isolate each file with `catch_unwind`).

pub mod png;

use zune_jpeg::zune_core::bytestream::ZCursor;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;
use zune_jpeg::JpegDecoder;

use crate::perceptual;

/// Largest image decoded in full (pixels).
pub const MAX_PIXELS: u64 = 64_000_000;
/// Largest side decoded in full.
pub const MAX_SIDE: u32 = 32_768;

/// Why an image yielded no fingerprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// Over [`MAX_PIXELS`] / [`MAX_SIDE`]: skipped by design.
    TooLarge,
    /// A valid file this decoder does not handle (e.g. interlaced PNG).
    Unsupported(&'static str),
    /// Corrupt or truncated data.
    Corrupt(String),
}

/// Size and fingerprints of a decoded image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    pub dhash: u64,
    pub phash: u64,
}

/// True when `w x h` may be decoded in full.
pub fn within_limits(w: u32, h: u32) -> bool {
    w > 0 && h > 0 && w <= MAX_SIDE && h <= MAX_SIDE && (w as u64) * (h as u64) <= MAX_PIXELS
}

/// Decode a JPEG (whole file or embedded thumbnail) to luma and fingerprint it.
pub fn jpeg(bytes: &[u8]) -> Result<Decoded, DecodeError> {
    let opts = DecoderOptions::default()
        .jpeg_set_out_colorspace(ColorSpace::Luma)
        .set_max_width(MAX_SIDE as usize)
        .set_max_height(MAX_SIDE as usize)
        .set_strict_mode(false);
    let mut d = JpegDecoder::new_with_options(ZCursor::new(bytes), opts);
    d.decode_headers().map_err(|e| DecodeError::Corrupt(format!("{e:?}")))?;
    let info = d.info().ok_or_else(|| DecodeError::Corrupt("no frame header".into()))?;
    let (w, h) = (info.width as u32, info.height as u32);
    if !within_limits(w, h) {
        return Err(DecodeError::TooLarge);
    }
    let px = d.decode().map_err(|e| DecodeError::Corrupt(format!("{e:?}")))?;
    let n = w as usize * h as usize;
    let gray: std::borrow::Cow<'_, [u8]> = if px.len() == n {
        px.as_slice().into()
    } else if px.len() == n * 3 {
        // Some colourspaces cannot be converted to luma by the decoder: do it here.
        px.chunks_exact(3).map(|c| luma8(c[0], c[1], c[2])).collect::<Vec<u8>>().into()
    } else {
        return Err(DecodeError::Corrupt(format!("unexpected output size {}", px.len())));
    };
    let (dhash, phash) = perceptual::fingerprints(w as usize, h as usize, &gray);
    Ok(Decoded { width: w, height: h, dhash, phash })
}

/// Luma of an 8-bit RGB pixel with the research formula (computed on 16-bit values).
pub fn luma8(r: u8, g: u8, b: u8) -> u8 {
    luma16(r as u32 * 257, g as u32 * 257, b as u32 * 257)
}

/// `((299 r + 587 g + 114 b) / 1000) >> 8` on 16-bit channel values.
pub fn luma16(r: u32, g: u32, b: u32) -> u8 {
    (((299 * r as u64 + 587 * g as u64 + 114 * b as u64) / 1000) >> 8).min(255) as u8
}
