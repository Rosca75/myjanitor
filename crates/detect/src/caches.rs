//! App caches and junk (CLN-DET-11): browser caches, thumbnail caches, crash dumps, logs, Windows Update leftovers; winapp2.ini compatible (see winapp2.rs).
//! Owner: W2-D.

use mj_core::{
    ActionKind, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, Error, PlatformSupport,
    Result, SafetyClass,
};

/// App caches & junk: Browsers, Teams, crash dumps, logs
#[derive(Debug, Default, Clone, Copy)]
pub struct AppCaches;

impl Detector for AppCaches {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::AppCaches,
            title: "App caches & junk".into(),
            description: "Browsers, Teams, crash dumps, logs".into(),
            category: Category::Clean,
            safety: SafetyClass::Rebuildable,
            default_action: ActionKind::Quarantine,
            available_actions: vec![ActionKind::Quarantine, ActionKind::Trash, ActionKind::PermanentDelete],
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-11".into()],
            unit: "apps".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("App caches & junk detector (W2-D)"))
    }
}
