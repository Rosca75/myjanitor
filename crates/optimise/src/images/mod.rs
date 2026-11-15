//! Image optimisers (W3-C): in-house lossless PNG and JPEG rewrites, optional
//! external engines (oxipng, jpegtran) and lossy conversion through external
//! encoders (cwebp, avifenc, cjxl).

pub mod adapters;
pub mod common;
pub mod jpeg;
pub mod jpeg_rewrite;
pub mod lossy;
pub mod png;
pub mod tiff_edit;
