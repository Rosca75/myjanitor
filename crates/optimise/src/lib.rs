//! # mj-optimise
//!
//! Optimisation framework and optimisers (W3-A/B/C). Every optimiser is a pure
//! `src -> dst` transform behind `mj_core::Optimiser`; replacing originals,
//! keep-if-not-smaller, light copies and journaling are done by `mj-actions`.
//! External encoders (`ffmpeg`, `cwebp`, `7z`...) are detected at runtime by
//! [`tools::detect`], run as separate processes ([`external`]), never
//! downloaded, and their absence disables the feature with a one-line reason.
#![forbid(unsafe_code)]

pub mod archive;
pub mod external;
pub mod framework;
pub mod fs_compression;
pub mod images;
pub mod media;
pub mod office;
pub mod pdf;
pub mod tools;
pub mod zip_repack;

use mj_core::{Archiver, Optimiser};

/// Every optimiser, in card order. The engine calls this once at start-up.
pub fn optimisers() -> Vec<Box<dyn Optimiser>> {
    vec![
        Box::new(office::powerpoint::PowerPoint),
        Box::new(office::excel::Excel),
        Box::new(office::word::Word),
        Box::new(zip_repack::ZipRepack),
        Box::new(pdf::PdfLossless),
        Box::new(pdf::PdfLossy),
        Box::new(archive::ArchiveFolders),
        Box::new(archive::ArchiveRecompress),
        Box::new(fs_compression::FsCompression),
        Box::new(images::png::PngLossless),
        Box::new(images::jpeg::JpegLossless),
        Box::new(images::lossy::ImageLossy),
        Box::new(media::video::Video),
        Box::new(media::screen::ScreenRecordings),
        Box::new(media::audio::Audio),
    ]
}

/// The archiver used by `Action::Archive`.
pub fn archiver() -> Box<dyn Archiver> {
    Box::new(archive::Zip7Archiver)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    #[test]
    fn optimiser_ids_are_unique() {
        let all = super::optimisers();
        let ids: HashSet<_> = all.iter().map(|o| o.info().id).collect();
        assert_eq!(ids.len(), all.len());
    }

    #[test]
    fn tool_gated_optimisers_say_why() {
        for o in super::optimisers() {
            if !o.info().tools.is_empty() {
                match o.availability(&[]) {
                    mj_core::Availability::Disabled { reason } => assert!(reason.starts_with("Requires")),
                    mj_core::Availability::Available => panic!("{:?} available without tools", o.info().id),
                }
            }
        }
    }
}
