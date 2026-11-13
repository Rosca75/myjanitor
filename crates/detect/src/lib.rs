//! # mj-detect
//!
//! Simple cleaning detectors (W2-D). Each detector is a pure function of the
//! `FileIndex` plus bounded reads of file headers; none modifies files.
//! Shared helpers (well-known folders, age tests, topmost-folder reduction)
//! live in `common`; the winapp2.ini parser in `winapp2`.
#![forbid(unsafe_code)]

pub mod broken;
pub mod caches;
pub mod common;
pub mod dev;
pub mod empty;
pub mod fonts;
pub mod inactive;
pub mod installers;
pub mod language;
pub mod links;
pub mod office;
pub mod sidecars;
pub mod stale;
pub mod temp;
pub mod winapp2;
pub mod zeroed;

use mj_core::Detector;

/// Every detector of this crate, in card order. The engine calls this once at start-up.
pub fn detectors() -> Vec<Box<dyn Detector>> {
    vec![
        Box::new(empty::EmptyFiles),
        Box::new(empty::EmptyFolders),
        Box::new(zeroed::ZeroedFiles),
        Box::new(broken::BrokenFiles),
        Box::new(links::BrokenLinks),
        Box::new(temp::TempFiles),
        Box::new(caches::AppCaches),
        Box::new(office::OfficeLeftovers),
        Box::new(dev::DevArtefacts),
        Box::new(installers::OldInstallers),
        Box::new(sidecars::OrphanSidecars),
        Box::new(sidecars::RawJpegPairs),
        Box::new(language::LanguagePacks),
        Box::new(stale::StaleFiles),
        Box::new(inactive::InactiveFolders),
        Box::new(fonts::UnusedFonts),
    ]
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    #[test]
    fn detector_ids_are_unique_and_described() {
        let all = super::detectors();
        let ids: HashSet<_> = all.iter().map(|d| d.info().id).collect();
        assert_eq!(ids.len(), all.len());
        assert!(all.iter().all(|d| !d.info().title.is_empty() && !d.info().feature_ids.is_empty()));
    }
}
