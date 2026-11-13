//! Similar images (CLN-DET-02): dHash + pHash from the same decode, BK-tree, Union-Find candidates, leader refinement, diameter confidence (research §4-§7).
//! Owner: W2-B.

use mj_core::{
    ActionKind, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, Error, PlatformSupport,
    Result, SafetyClass,
};

/// Similar images: Near-identical photos and re-saved copies
#[derive(Debug, Default, Clone, Copy)]
pub struct SimilarImages;

impl Detector for SimilarImages {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::SimilarImages,
            title: "Similar images".into(),
            description: "Near-identical photos and re-saved copies".into(),
            category: Category::Clean,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Trash,
            available_actions: vec![ActionKind::Trash, ActionKind::Quarantine, ActionKind::Move],
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-02".into()],
            unit: "images".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("Similar images detector (W2-B)"))
    }
}
