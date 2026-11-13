//! # mj-formats
//!
//! In-house parsers and writers for the containers and metadata MyJanitor
//! reads or rewrites. Everything here treats input as **hostile**: bounded
//! recursion, bounded allocations (never trust a declared size), total-order
//! comparisons, no panics on malformed input (return `Error::Parse`).
//!
//! Module ownership (ARCHITECTURE.md §11):
//! | module | owner | purpose |
//! |---|---|---|
//! | `crc32` | W1 | CRC-32 (ZIP, PNG) |
//! | `sniff` | W2-C | content-type detection (CLN-CNT-02, OPT-ENG-02) |
//! | `zip::read` | W2-E | ZIP central directory + entry reading |
//! | `zip::write` | W3-A | ZIP writing / repack |
//! | `cfb` | W2-E | OLE compound files (encrypted Office detection) |
//! | `xml` | W3-A | minimal XML pull parser |
//! | `ooxml` | W3-A | OOXML package helpers |
//! | `pdf` | W3-B | PDF objects, signatures, structural rewrite |
//! | `jpeg`, `exif` | W2-B | JPEG segments, EXIF/TIFF and embedded thumbnails |
//! | `png` | W3-C | PNG chunks |
//! | `media` | W3-C | ID3, FLAC, MP4/MOV box metadata |
#![forbid(unsafe_code)]

pub mod cfb;
pub mod crc32;
pub mod exif;
pub mod jpeg;
pub mod media;
pub mod ooxml;
pub mod pdf;
pub mod png;
pub mod sniff;
pub mod xml;
pub mod zip;
