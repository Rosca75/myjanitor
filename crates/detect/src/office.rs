//! Office leftovers (CLN-DET-12): AutoRecover / UnsavedFiles folders, Teams and Outlook local caches.
//! Owner: W2-D.

use mj_core::{
    ActionKind, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, Error, PlatformSupport,
    Result, SafetyClass,
};

/// Office leftovers: AutoRecover files and Office caches
#[derive(Debug, Default, Clone, Copy)]
pub struct OfficeLeftovers;

impl Detector for OfficeLeftovers {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::OfficeLeftovers,
            title: "Office leftovers".into(),
            description: "AutoRecover files and Office caches".into(),
            category: Category::Clean,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Quarantine,
            available_actions: vec![ActionKind::Quarantine, ActionKind::Trash],
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-12".into()],
            unit: "files".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("Office leftovers detector (W2-D)"))
    }
}
