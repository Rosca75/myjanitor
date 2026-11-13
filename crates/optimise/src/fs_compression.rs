//! Transparent filesystem compression (OPT-FSC-01..06): WOF on NTFS, Btrfs zstd on Linux; skip poorly compressible files; DirectStorage warning.
//! Owner: W3-B.

use mj_core::{
    ActionKind, Availability, Category, DetectContext, DetectorOutput, Error, Estimate, ExternalTool, FileEntry,
    OptimiseJob, OptimiseOutput, Optimiser, OptimiserId, OptimiserInfo, ParamValues, PlatformSupport, Result,
    SafetyClass,
};

/// Transparent compression: Compress cold folders, files stay usable
#[derive(Debug, Default, Clone, Copy)]
pub struct FsCompression;

/// Extensions (lower case) this optimiser targets.
pub const FSCOMPRESSION_EXTENSIONS: &[&str] = &[];

impl Optimiser for FsCompression {
    fn info(&self) -> OptimiserInfo {
        OptimiserInfo {
            id: OptimiserId::FsCompression,
            title: "Transparent compression".into(),
            description: "Compress cold folders, files stay usable".into(),
            category: Category::Optimise,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::FsCompress,
            available_actions: vec![ActionKind::FsCompress, ActionKind::FsDecompress],
            presets: Vec::new(),
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            tools: vec![],
            feature_ids: vec![
                "OPT-FSC-01".into(),
                "OPT-FSC-02".into(),
                "OPT-FSC-04".into(),
                "OPT-FSC-05".into(),
                "OPT-FSC-06".into(),
            ],
            unit: "folders".into(),
        }
    }

    fn availability(&self, tools: &[ExternalTool]) -> Availability {
        let _ = tools;
        Availability::Available
    }

    fn applies_to(&self, entry: &FileEntry) -> bool {
        entry.is_file() && FSCOMPRESSION_EXTENSIONS.contains(&entry.extension().as_str())
    }

    fn estimate(&self, entry: &FileEntry, params: &ParamValues) -> Result<Estimate> {
        let _ = (entry, params);
        Err(Error::not_implemented("Transparent compression estimate (W3-B)"))
    }

    fn optimise(&self, job: &OptimiseJob<'_>) -> Result<OptimiseOutput> {
        let _ = job;
        Err(Error::not_implemented("Transparent compression (W3-B)"))
    }

    fn analyse(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("Transparent compression analysis (W3-B)"))
    }
}
