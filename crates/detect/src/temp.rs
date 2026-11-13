//! Temporary and partial files (CLN-DET-10): .tmp, ~$ Office locks, .crdownload, .part, .bak.
//! Owner: W2-D.

use mj_core::{
    ActionKind, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, Error, PlatformSupport,
    Result, SafetyClass,
};

/// Temporary files: Leftover temporary and partial downloads
#[derive(Debug, Default, Clone, Copy)]
pub struct TempFiles;

impl Detector for TempFiles {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::TempFiles,
            title: "Temporary files".into(),
            description: "Leftover temporary and partial downloads".into(),
            category: Category::Clean,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Trash,
            available_actions: vec![ActionKind::Trash, ActionKind::Quarantine, ActionKind::PermanentDelete],
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-10".into()],
            unit: "files".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("Temporary files detector (W2-D)"))
    }
}
