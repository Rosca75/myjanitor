//! Stale files not accessed in N months (CLN-DET-17); feeds archiving.
//! Owner: W2-D.

use mj_core::{
    ActionKind, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, Error, PlatformSupport,
    Result, SafetyClass,
};

/// Stale files: Files not opened for a long time
#[derive(Debug, Default, Clone, Copy)]
pub struct StaleFiles;

impl Detector for StaleFiles {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::StaleFiles,
            title: "Stale files".into(),
            description: "Files not opened for a long time".into(),
            category: Category::Clean,
            safety: SafetyClass::ReviewOnly,
            default_action: ActionKind::None,
            available_actions: vec![ActionKind::None, ActionKind::Archive, ActionKind::Move, ActionKind::Trash],
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-17".into()],
            unit: "files".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("Stale files detector (W2-D)"))
    }
}
