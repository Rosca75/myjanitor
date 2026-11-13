//! Orphan sidecar files (.xmp, .aae, .thm without parent) and RAW+JPEG pairs (CLN-DET-15).
//! Owner: W2-D.

use mj_core::{
    ActionKind, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, Error, PlatformSupport,
    Result, SafetyClass,
};

/// Orphan sidecars: Sidecar files whose photo is gone
#[derive(Debug, Default, Clone, Copy)]
pub struct OrphanSidecars;

impl Detector for OrphanSidecars {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::OrphanSidecars,
            title: "Orphan sidecars".into(),
            description: "Sidecar files whose photo is gone".into(),
            category: Category::Clean,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Trash,
            available_actions: vec![ActionKind::Trash, ActionKind::Quarantine],
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-15".into()],
            unit: "files".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("Orphan sidecars detector (W2-D)"))
    }
}

/// RAW + JPEG pairs: Photos kept both as RAW and JPEG
#[derive(Debug, Default, Clone, Copy)]
pub struct RawJpegPairs;

impl Detector for RawJpegPairs {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::RawJpegPairs,
            title: "RAW + JPEG pairs".into(),
            description: "Photos kept both as RAW and JPEG".into(),
            category: Category::Clean,
            safety: SafetyClass::ReviewOnly,
            default_action: ActionKind::None,
            available_actions: vec![ActionKind::None, ActionKind::Trash, ActionKind::Move],
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-15".into()],
            unit: "pairs".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("RAW + JPEG pairs detector (W2-D)"))
    }
}
