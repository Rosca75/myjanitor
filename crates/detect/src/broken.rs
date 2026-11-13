//! Broken / corrupted files (CLN-DET-08): truncated images, audio, PDFs and archives (structure checks via mj-formats).
//! Owner: W2-D.

use mj_core::{
    ActionKind, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, Error, PlatformSupport,
    Result, SafetyClass,
};

/// Broken files: Damaged images, documents and archives
#[derive(Debug, Default, Clone, Copy)]
pub struct BrokenFiles;

impl Detector for BrokenFiles {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::BrokenFiles,
            title: "Broken files".into(),
            description: "Damaged images, documents and archives".into(),
            category: Category::Clean,
            safety: SafetyClass::ReviewOnly,
            default_action: ActionKind::None,
            available_actions: vec![ActionKind::None, ActionKind::Trash, ActionKind::Quarantine],
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-08".into()],
            unit: "files".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("Broken files detector (W2-D)"))
    }
}
