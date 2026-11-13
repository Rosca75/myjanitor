//! Closed engagements / inactive project folders, inactive for N months (CLN-DET-21).
//! Owner: W2-D.

use mj_core::{
    ActionKind, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, Error, PlatformSupport,
    Result, SafetyClass,
};

/// Inactive folders: Project folders idle for months
#[derive(Debug, Default, Clone, Copy)]
pub struct InactiveFolders;

impl Detector for InactiveFolders {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::InactiveFolders,
            title: "Inactive folders".into(),
            description: "Project folders idle for months".into(),
            category: Category::Archive,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Archive,
            available_actions: vec![ActionKind::Archive, ActionKind::Move, ActionKind::Trash],
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-21".into()],
            unit: "folders".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("Inactive folders detector (W2-D)"))
    }
}
