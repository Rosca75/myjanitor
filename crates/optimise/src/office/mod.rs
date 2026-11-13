//! Office (OOXML) optimisers (W3-A). Shared package handling (parts,
//! relationships, media dedup, picture recompression, label preservation
//! SAF-08) goes in `package`.

pub mod excel;
pub mod package;
pub mod powerpoint;
pub mod word;
