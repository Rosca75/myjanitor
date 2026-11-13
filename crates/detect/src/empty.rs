//! Empty files and empty folders, nested empty trees reported topmost (CLN-DET-06).
//! Owner: W2-D.

use mj_core::{
    ActionKind, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, Error, PlatformSupport,
    Result, SafetyClass,
};

/// Empty files: Zero-byte files
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyFiles;

impl Detector for EmptyFiles {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::EmptyFiles,
            title: "Empty files".into(),
            description: "Zero-byte files".into(),
            category: Category::Clean,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Trash,
            available_actions: vec![ActionKind::Trash, ActionKind::Quarantine, ActionKind::PermanentDelete],
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-06".into()],
            unit: "files".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("Empty files detector (W2-D)"))
    }
}

/// Empty folders: Folders with nothing inside, including nested empty trees
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyFolders;

impl Detector for EmptyFolders {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::EmptyFolders,
            title: "Empty folders".into(),
            description: "Folders with nothing inside, including nested empty trees".into(),
            category: Category::Clean,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Trash,
            available_actions: vec![ActionKind::Trash, ActionKind::PermanentDelete],
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-06".into()],
            unit: "folders".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("Empty folders detector (W2-D)"))
    }
}
