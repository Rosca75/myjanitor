//! Email attachment copies (CLN-DET-20): files in Downloads that duplicate synced SharePoint/OneDrive files.
//! Owner: W2-B.

use mj_core::{
    ActionKind, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, Error, PlatformSupport,
    Result, SafetyClass,
};

/// Attachment copies: Downloads that duplicate synced files
#[derive(Debug, Default, Clone, Copy)]
pub struct AttachmentCopies;

impl Detector for AttachmentCopies {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::AttachmentCopies,
            title: "Attachment copies".into(),
            description: "Downloads that duplicate synced files".into(),
            category: Category::Clean,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Trash,
            available_actions: vec![ActionKind::Trash, ActionKind::Quarantine],
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-20".into()],
            unit: "files".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("Attachment copies detector (W2-B)"))
    }
}
