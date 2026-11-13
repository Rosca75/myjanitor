//! # mj-dupes
//!
//! Duplicate and similarity engine (W2-B): exact duplicates with staged hashing,
//! perceptual image hashing, version chains, derived files, attachment copies
//! and duplicate fonts. Reads file content only; never modifies files.
//! Shared building blocks live in `staged` (size -> partial -> full hashing
//! over a `FingerprintStore`), `perceptual` (grids, dHash, pHash, BK-tree,
//! Union-Find, leader refinement) and `keep` (keeper choice, CLN-ACT-01).
#![forbid(unsafe_code)]

pub mod attachments;
pub mod derived;
pub mod exact;
pub mod fonts;
pub mod keep;
pub mod perceptual;
pub mod similar_images;
pub mod staged;
pub mod version_chains;

use mj_core::Detector;

/// Every detector of this crate, in card order. The engine calls this once at start-up.
pub fn detectors() -> Vec<Box<dyn Detector>> {
    vec![
        Box::new(exact::ExactDuplicates),
        Box::new(similar_images::SimilarImages),
        Box::new(version_chains::VersionChains),
        Box::new(derived::DerivedFiles),
        Box::new(attachments::AttachmentCopies),
        Box::new(fonts::DuplicateFonts),
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
