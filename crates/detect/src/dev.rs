//! Developer artefacts (CLN-DET-13): node_modules, .venv, target/, bin/obj, __pycache__, .gradle, Docker build caches; folder candidates, topmost only.
//! Owner: W2-D.

use mj_core::{
    ActionKind, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, Error, PlatformSupport,
    Result, SafetyClass,
};

/// Developer artefacts: node_modules, .venv, target/ and other rebuildable folders
#[derive(Debug, Default, Clone, Copy)]
pub struct DevArtefacts;

impl Detector for DevArtefacts {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::DevArtefacts,
            title: "Developer artefacts".into(),
            description: "node_modules, .venv, target/ and other rebuildable folders".into(),
            category: Category::Clean,
            safety: SafetyClass::Rebuildable,
            default_action: ActionKind::Trash,
            available_actions: vec![ActionKind::Trash, ActionKind::Quarantine, ActionKind::PermanentDelete],
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-13".into()],
            unit: "folders".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("Developer artefacts detector (W2-D)"))
    }
}
