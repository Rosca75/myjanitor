//! Derived files (CLN-DET-19): a PDF exported from a .pptx/.docx still present in the same folder.
//! Owner: W2-B.

use mj_core::{
    ActionKind, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, Error, PlatformSupport,
    Result, SafetyClass,
};

/// Derived files: PDF exports next to their source documents
#[derive(Debug, Default, Clone, Copy)]
pub struct DerivedFiles;

impl Detector for DerivedFiles {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::DerivedFiles,
            title: "Derived files".into(),
            description: "PDF exports next to their source documents".into(),
            category: Category::Clean,
            safety: SafetyClass::ReviewOnly,
            default_action: ActionKind::None,
            available_actions: vec![ActionKind::None, ActionKind::Trash, ActionKind::Quarantine, ActionKind::Move],
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-19".into()],
            unit: "files".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("Derived files detector (W2-B)"))
    }
}
