//! Exact duplicates (CLN-DET-01): staged size -> partial hash (head/middle/tail 64 KiB, D-005) -> full streaming hash -> SHA-256 or byte confirmation before deletion is offered.
//! Owner: W2-B.

use mj_core::{
    ActionKind, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, Error, PlatformSupport,
    Result, SafetyClass,
};

/// Exact duplicates: Files with identical content
#[derive(Debug, Default, Clone, Copy)]
pub struct ExactDuplicates;

impl Detector for ExactDuplicates {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::ExactDuplicates,
            title: "Exact duplicates".into(),
            description: "Files with identical content".into(),
            category: Category::Clean,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Trash,
            available_actions: vec![
                ActionKind::Trash,
                ActionKind::Quarantine,
                ActionKind::PermanentDelete,
                ActionKind::Move,
                ActionKind::Hardlink,
                ActionKind::Reflink,
            ],
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-01".into(), "CLN-ACT-01".into(), "CLN-ACT-06".into()],
            unit: "files".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("Exact duplicates detector (W2-B)"))
    }
}
