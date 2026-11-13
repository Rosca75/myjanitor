//! Zeroed files (CLN-DET-07): non-empty files whose content is all zero bytes (sampled, then confirmed).
//! Owner: W2-D.

use mj_core::{
    ActionKind, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, Error, PlatformSupport,
    Result, SafetyClass,
};

/// Zeroed files: Files filled with zeros, usually corrupted
#[derive(Debug, Default, Clone, Copy)]
pub struct ZeroedFiles;

impl Detector for ZeroedFiles {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::ZeroedFiles,
            title: "Zeroed files".into(),
            description: "Files filled with zeros, usually corrupted".into(),
            category: Category::Clean,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Trash,
            available_actions: vec![ActionKind::Trash, ActionKind::Quarantine],
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-07".into()],
            unit: "files".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("Zeroed files detector (W2-D)"))
    }
}
