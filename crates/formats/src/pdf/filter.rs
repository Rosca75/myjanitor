//! Stream filters: FlateDecode (zlib, through `miniz_oxide`) with the PNG and
//! TIFF predictors (ISO 32000-1 §7.4.4). Every other filter (DCT, JPX, JBIG2,
//! CCITT, LZW, ASCII85...) is *not* implemented: such streams are copied byte
//! for byte and compared raw.
//!
//! The deflate helpers are public so other in-house code (the transparent
//! compression probe) can reuse the one bundled codec.

use mj_core::{Error, Result};

use super::object::{Dict, Object};

/// Maximum decoded size of one stream (hostile "zip bomb" guard).
pub const MAX_DECODED: usize = 256 * 1024 * 1024;

fn err(msg: impl Into<String>) -> Error {
    Error::parse("PDF stream", msg)
}

/// Inflate a zlib stream completely (Adler-32 verified), at most `limit` bytes.
pub fn inflate(data: &[u8], limit: usize) -> Result<Vec<u8>> {
    miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(data, limit)
        .map_err(|e| err(format!("FlateDecode failed: {:?}", e.status)))
}

/// Deflate `data` into a zlib stream at `level` (0-10; 10 = best).
pub fn deflate(data: &[u8], level: u8) -> Vec<u8> {
    miniz_oxide::deflate::compress_to_vec_zlib(data, level.min(10))
}

/// Raw deflate (no zlib header), for size probes.
pub fn deflate_raw(data: &[u8], level: u8) -> Vec<u8> {
    miniz_oxide::deflate::compress_to_vec(data, level.min(10))
}

/// What the `/Filter` entry of a stream says, as far as this module cares.
#[derive(Debug, Clone, PartialEq)]
pub enum FilterKind {
    /// No `/Filter` (or `null`).
    None,
    /// Exactly one filter, FlateDecode; the `/DecodeParms` dictionary if any.
    Flate(Option<Dict>),
    /// Anything else (unknown filters, chains, malformed entries).
    Other,
}

/// Classify a stream dictionary's filters.
pub fn filter_kind(dict: &Dict) -> FilterKind {
    let filter = match dict.get(b"Filter") {
        None | Some(Object::Null) => {
            return if dict.get(b"DecodeParms").is_some_and(|p| *p != Object::Null) {
                FilterKind::Other
            } else {
                FilterKind::None
            }
        }
        Some(Object::Name(n)) => n.as_slice(),
        Some(Object::Array(a)) if a.len() == 1 => match &a[0] {
            Object::Name(n) => n.as_slice(),
            _ => return FilterKind::Other,
        },
        _ => return FilterKind::Other,
    };
    if filter != b"FlateDecode" && filter != b"Fl" {
        return FilterKind::Other;
    }
    let parms = match dict.get(b"DecodeParms") {
        None | Some(Object::Null) => None,
        Some(Object::Dict(d)) => Some(d.clone()),
        Some(Object::Array(a)) if a.len() == 1 => match &a[0] {
            Object::Dict(d) => Some(d.clone()),
            Object::Null => None,
            _ => return FilterKind::Other,
        },
        _ => return FilterKind::Other,
    };
    FilterKind::Flate(parms)
}

/// Fully decode a stream that uses only FlateDecode (with any predictor) or no filter.
/// Used for cross-reference and object streams, whose content must be read.
pub fn decode_full(dict: &Dict, data: &[u8]) -> Result<Vec<u8>> {
    match filter_kind(dict) {
        FilterKind::None => Ok(data.to_vec()),
        FilterKind::Flate(parms) => {
            let raw = inflate(data, MAX_DECODED)?;
            match parms {
                Some(p) => unpredict(&raw, &p),
                None => Ok(raw),
            }
        }
        FilterKind::Other => Err(err("unsupported filter")),
    }
}

fn int_param(p: &Dict, key: &[u8], default: i64) -> i64 {
    p.get(key).and_then(Object::as_int).unwrap_or(default)
}

/// Undo a PNG (10-15) or TIFF (2) predictor; predictor 1 (or absent) is the identity.
pub fn unpredict(data: &[u8], parms: &Dict) -> Result<Vec<u8>> {
    let predictor = int_param(parms, b"Predictor", 1);
    if predictor == 1 {
        return Ok(data.to_vec());
    }
    let colors = int_param(parms, b"Colors", 1);
    let bpc = int_param(parms, b"BitsPerComponent", 8);
    let columns = int_param(parms, b"Columns", 1);
    if !(1..=32).contains(&colors) || ![1, 2, 4, 8, 16].contains(&bpc) || !(1..=1 << 24).contains(&columns) {
        return Err(err("invalid predictor parameters"));
    }
    let bits_per_pixel = (colors * bpc) as usize;
    let bpp = bits_per_pixel.div_ceil(8);
    let row = (bits_per_pixel * columns as usize).div_ceil(8);
    match predictor {
        2 => {
            if bpc != 8 {
                return Err(err("TIFF predictor with other than 8 bits per component is not supported"));
            }
            let mut out = data.to_vec();
            for line in out.chunks_mut(row) {
                for i in bpp..line.len() {
                    line[i] = line[i].wrapping_add(line[i - bpp]);
                }
            }
            Ok(out)
        }
        10..=15 => {
            let mut out = Vec::with_capacity(data.len());
            let mut prev = vec![0u8; row];
            let mut cur = vec![0u8; row];
            for chunk in data.chunks(row + 1) {
                let ty = chunk[0];
                let src = &chunk[1..];
                cur[..src.len()].copy_from_slice(src);
                for v in cur[src.len()..].iter_mut() {
                    *v = 0;
                }
                for i in 0..row {
                    let left = if i >= bpp { cur[i - bpp] } else { 0 };
                    let up = prev[i];
                    let ul = if i >= bpp { prev[i - bpp] } else { 0 };
                    cur[i] = match ty {
                        0 => cur[i],
                        1 => cur[i].wrapping_add(left),
                        2 => cur[i].wrapping_add(up),
                        3 => cur[i].wrapping_add(((left as u16 + up as u16) / 2) as u8),
                        4 => cur[i].wrapping_add(paeth(left, up, ul)),
                        _ => return Err(err("invalid PNG predictor row type")),
                    };
                }
                out.extend_from_slice(&cur[..src.len()]);
                std::mem::swap(&mut prev, &mut cur);
            }
            Ok(out)
        }
        _ => Err(err(format!("unknown predictor {predictor}"))),
    }
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i16 + b as i16 - c as i16;
    let pa = (p - a as i16).abs();
    let pb = (p - b as i16).abs();
    let pc = (p - c as i16).abs();
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// The comparable form of a stream's content, used by verification and hashing.
///
/// Two streams are equivalent when their canonical forms are equal: plain
/// streams compare decoded bytes; FlateDecode with parameters compares the
/// inflated bytes plus the parameters (the predictor is never changed);
/// everything else compares raw bytes plus the filter entries.
#[derive(Debug, Clone, PartialEq)]
pub enum Canonical {
    /// No filter, or FlateDecode without parameters: the decoded bytes.
    Plain(Vec<u8>),
    /// FlateDecode with `/DecodeParms`: inflated (still predicted) bytes and the parameters.
    FlateParms(Vec<u8>, Dict),
    /// Unsupported or undecodable: raw bytes, `/Filter`, `/DecodeParms`.
    Raw(Vec<u8>, Option<Object>, Option<Object>),
}

/// Canonical form of a stream.
pub fn canonical(dict: &Dict, data: &[u8]) -> Canonical {
    let raw = || Canonical::Raw(data.to_vec(), dict.get(b"Filter").cloned(), dict.get(b"DecodeParms").cloned());
    match filter_kind(dict) {
        FilterKind::None => Canonical::Plain(data.to_vec()),
        FilterKind::Flate(parms) => match inflate(data, MAX_DECODED) {
            Ok(d) => match parms {
                None => Canonical::Plain(d),
                Some(p) => Canonical::FlateParms(d, p),
            },
            Err(_) => raw(),
        },
        FilterKind::Other => raw(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parms(pred: i64, colors: i64, columns: i64) -> Dict {
        let mut d = Dict::new();
        d.set(b"Predictor", Object::Int(pred));
        d.set(b"Colors", Object::Int(colors));
        d.set(b"Columns", Object::Int(columns));
        d
    }

    #[test]
    fn flate_round_trip_and_limit() {
        let data: Vec<u8> = (0..10_000u32).map(|i| (i % 7) as u8).collect();
        let z = deflate(&data, 10);
        assert!(z.len() < data.len());
        assert_eq!(inflate(&z, usize::MAX).unwrap(), data);
        assert!(inflate(&z, 100).is_err());
        let mut bad = z.clone();
        let n = bad.len();
        bad[n - 1] ^= 0xff; // Adler-32 mismatch
        assert!(inflate(&bad, usize::MAX).is_err());
        assert!(inflate(&z[..z.len() / 2], usize::MAX).is_err());
    }

    #[test]
    fn png_up_predictor() {
        // Two rows of 3 bytes: row 1 "None", row 2 "Up" (+1 each).
        let enc = [0, 1, 2, 3, 2, 1, 1, 1];
        assert_eq!(unpredict(&enc, &parms(12, 1, 3)).unwrap(), vec![1, 2, 3, 2, 3, 4]);
    }

    #[test]
    fn png_sub_average_paeth() {
        let enc = [1, 5, 1, 1, 3, 0, 4, 4, 4, 0, 0, 0];
        let out = unpredict(&enc, &parms(15, 1, 3)).unwrap();
        // Row 1 Sub: 5, 6, 7. Row 2 Average: 0+5/2=2, 4+(2+6)/2=8, 4+(8+7)/2=11.
        // Row 3 Paeth: left 0 -> up 2 => 2; ...
        assert_eq!(&out[..6], &[5, 6, 7, 2, 8, 11]);
        assert_eq!(out.len(), 9);
    }

    #[test]
    fn tiff_predictor() {
        assert_eq!(unpredict(&[1, 1, 1, 5, 1], &parms(2, 1, 5)).unwrap(), vec![1, 2, 3, 8, 9]);
        assert!(unpredict(&[0; 4], &parms(7, 1, 4)).is_err());
        assert!(unpredict(&[0; 4], &parms(12, 0, 4)).is_err());
    }

    #[test]
    fn filter_classification() {
        let mut d = Dict::new();
        assert_eq!(filter_kind(&d), FilterKind::None);
        d.set(b"Filter", Object::Array(vec![Object::Name(b"FlateDecode".to_vec())]));
        assert_eq!(filter_kind(&d), FilterKind::Flate(None));
        d.set(b"Filter", Object::Name(b"DCTDecode".to_vec()));
        assert_eq!(filter_kind(&d), FilterKind::Other);
        d.set(
            b"Filter",
            Object::Array(vec![Object::Name(b"ASCII85Decode".to_vec()), Object::Name(b"FlateDecode".to_vec())]),
        );
        assert_eq!(filter_kind(&d), FilterKind::Other);
    }
}
