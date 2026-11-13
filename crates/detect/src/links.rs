//! Invalid symlinks and broken Windows shortcuts (.lnk) (CLN-DET-09). Never follows a link to decide; reads the target path only.
//! Owner: W2-D.

use mj_core::{
    ActionKind, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, Error, PlatformSupport,
    Result, SafetyClass,
};

/// Broken links: Shortcuts and links whose target is gone
#[derive(Debug, Default, Clone, Copy)]
pub struct BrokenLinks;

impl Detector for BrokenLinks {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::BrokenLinks,
            title: "Broken links".into(),
            description: "Shortcuts and links whose target is gone".into(),
            category: Category::Clean,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Trash,
            available_actions: vec![ActionKind::Trash, ActionKind::PermanentDelete],
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-09".into()],
            unit: "links".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("Broken links detector (W2-D)"))
    }
}
