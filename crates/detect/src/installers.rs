//! Old installers and disk images (CLN-DET-14): .msi .exe .iso in Downloads older than X days.
//! Owner: W2-D.

use mj_core::{
    ActionKind, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, Error, PlatformSupport,
    Result, SafetyClass,
};

/// Old installers: Installers and disk images in Downloads
#[derive(Debug, Default, Clone, Copy)]
pub struct OldInstallers;

impl Detector for OldInstallers {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::OldInstallers,
            title: "Old installers".into(),
            description: "Installers and disk images in Downloads".into(),
            category: Category::Clean,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Trash,
            available_actions: vec![
                ActionKind::Trash,
                ActionKind::Quarantine,
                ActionKind::Move,
                ActionKind::PermanentDelete,
            ],
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-14".into()],
            unit: "files".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("Old installers detector (W2-D)"))
    }
}
