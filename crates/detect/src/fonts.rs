//! Unused font families and font caches, review only (OPT-FNT-02).
//! Owner: W2-D.

use mj_core::{
    ActionKind, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, Error, PlatformSupport,
    Result, SafetyClass,
};

/// Unused fonts: Font families and font caches you may not need
#[derive(Debug, Default, Clone, Copy)]
pub struct UnusedFonts;

impl Detector for UnusedFonts {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::UnusedFonts,
            title: "Unused fonts".into(),
            description: "Font families and font caches you may not need".into(),
            category: Category::Optimise,
            safety: SafetyClass::ReviewOnly,
            default_action: ActionKind::None,
            available_actions: vec![ActionKind::None],
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            feature_ids: vec!["OPT-FNT-02".into()],
            unit: "fonts".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("Unused fonts detector (W2-D)"))
    }
}
