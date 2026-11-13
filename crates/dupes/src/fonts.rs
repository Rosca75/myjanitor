//! Duplicate installed fonts (OPT-FNT-01) in user and system font folders.
//! Owner: W2-B.

use mj_core::{
    ActionKind, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, Error, PlatformSupport,
    Result, SafetyClass,
};

/// Duplicate fonts: The same font installed more than once
#[derive(Debug, Default, Clone, Copy)]
pub struct DuplicateFonts;

impl Detector for DuplicateFonts {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::DuplicateFonts,
            title: "Duplicate fonts".into(),
            description: "The same font installed more than once".into(),
            category: Category::Optimise,
            safety: SafetyClass::ReviewOnly,
            default_action: ActionKind::None,
            available_actions: vec![ActionKind::None, ActionKind::Trash],
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            feature_ids: vec!["OPT-FNT-01".into()],
            unit: "fonts".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("Duplicate fonts detector (W2-B)"))
    }
}
