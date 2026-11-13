//! Version chains (CLN-DET-18): v1..vN, final, final_v2, date prefixes, initials suffixes; keep latest, review others.
//! Owner: W2-B.

use mj_core::{
    ActionKind, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, Error, PlatformSupport,
    Result, SafetyClass,
};

/// Version chains: Older versions of the same document
#[derive(Debug, Default, Clone, Copy)]
pub struct VersionChains;

impl Detector for VersionChains {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::VersionChains,
            title: "Version chains".into(),
            description: "Older versions of the same document".into(),
            category: Category::Clean,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Trash,
            available_actions: vec![ActionKind::Trash, ActionKind::Quarantine, ActionKind::Move, ActionKind::Archive],
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-18".into()],
            unit: "files".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("Version chains detector (W2-B)"))
    }
}
