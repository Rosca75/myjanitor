//! JPEG lossless optimisation (OPT-IMG-01): metadata stripping in-house, jpegtran when present.
//! Owner: W3-C.

use mj_core::{
    ActionKind, Availability, Category, DetectContext, DetectorOutput, Error, Estimate, ExternalTool, FileEntry,
    OptimiseJob, OptimiseOutput, Optimiser, OptimiserId, OptimiserInfo, ParamValues, PlatformSupport, Result,
    SafetyClass,
};

/// JPEG images: Lossless optimisation and metadata control
#[derive(Debug, Default, Clone, Copy)]
pub struct JpegLossless;

/// Extensions (lower case) this optimiser targets.
pub const JPEGLOSSLESS_EXTENSIONS: &[&str] = &["jpg", "jpeg"];

impl Optimiser for JpegLossless {
    fn info(&self) -> OptimiserInfo {
        OptimiserInfo {
            id: OptimiserId::JpegLossless,
            title: "JPEG images".into(),
            description: "Lossless optimisation and metadata control".into(),
            category: Category::Optimise,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Optimise,
            available_actions: vec![ActionKind::Optimise],
            presets: Vec::new(),
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            tools: vec![],
            feature_ids: vec!["OPT-IMG-01".into(), "OPT-IMG-05".into(), "OPT-IMG-06".into()],
            unit: "images".into(),
        }
    }

    fn availability(&self, tools: &[ExternalTool]) -> Availability {
        let _ = tools;
        Availability::Available
    }

    fn applies_to(&self, entry: &FileEntry) -> bool {
        entry.is_file() && JPEGLOSSLESS_EXTENSIONS.contains(&entry.extension().as_str())
    }

    fn estimate(&self, entry: &FileEntry, params: &ParamValues) -> Result<Estimate> {
        let _ = (entry, params);
        Err(Error::not_implemented("JPEG images estimate (W3-C)"))
    }

    fn optimise(&self, job: &OptimiseJob<'_>) -> Result<OptimiseOutput> {
        let _ = job;
        Err(Error::not_implemented("JPEG images (W3-C)"))
    }

    fn analyse(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("JPEG images analysis (W3-C)"))
    }
}
