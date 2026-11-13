//! Unused language packs / localisations (CLN-DET-16).
//! Owner: W2-D.

use mj_core::{
    ActionKind, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, Error, PlatformSupport,
    Result, SafetyClass,
};

/// Language packs: Localisation files for languages you do not use
#[derive(Debug, Default, Clone, Copy)]
pub struct LanguagePacks;

impl Detector for LanguagePacks {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::LanguagePacks,
            title: "Language packs".into(),
            description: "Localisation files for languages you do not use".into(),
            category: Category::Clean,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Quarantine,
            available_actions: vec![ActionKind::Quarantine, ActionKind::Trash],
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-16".into()],
            unit: "files".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("Language packs detector (W2-D)"))
    }
}
