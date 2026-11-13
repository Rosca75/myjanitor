//! PowerPoint optimisation (OPT-PPT-01..10, OPT-OFC-02, SAF-08).
//! Owner: W3-A.

use mj_core::{
    ActionKind, Availability, Category, DetectContext, DetectorOutput, Error, Estimate, ExternalTool, FileEntry,
    OptimiseJob, OptimiseOutput, Optimiser, OptimiserId, OptimiserInfo, ParamValues, PlatformSupport, Result,
    SafetyClass,
};

/// Office decks: Lossless first, light copies kept beside originals
#[derive(Debug, Default, Clone, Copy)]
pub struct PowerPoint;

/// Extensions (lower case) this optimiser targets.
pub const POWERPOINT_EXTENSIONS: &[&str] = &["pptx", "ppsx", "potx"];

impl Optimiser for PowerPoint {
    fn info(&self) -> OptimiserInfo {
        OptimiserInfo {
            id: OptimiserId::PowerPoint,
            title: "Office decks".into(),
            description: "Lossless first, light copies kept beside originals".into(),
            category: Category::Optimise,
            safety: SafetyClass::LightCopy,
            default_action: ActionKind::Optimise,
            available_actions: vec![ActionKind::Optimise],
            presets: Vec::new(),
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            tools: vec![],
            feature_ids: vec![
                "OPT-PPT-01".into(),
                "OPT-PPT-02".into(),
                "OPT-PPT-03".into(),
                "OPT-PPT-04".into(),
                "OPT-PPT-05".into(),
                "OPT-PPT-06".into(),
                "OPT-PPT-07".into(),
                "OPT-PPT-08".into(),
                "OPT-PPT-09".into(),
                "OPT-PPT-10".into(),
            ],
            unit: "files".into(),
        }
    }

    fn availability(&self, tools: &[ExternalTool]) -> Availability {
        let _ = tools;
        Availability::Available
    }

    fn applies_to(&self, entry: &FileEntry) -> bool {
        entry.is_file() && POWERPOINT_EXTENSIONS.contains(&entry.extension().as_str())
    }

    fn estimate(&self, entry: &FileEntry, params: &ParamValues) -> Result<Estimate> {
        let _ = (entry, params);
        Err(Error::not_implemented("Office decks estimate (W3-A)"))
    }

    fn optimise(&self, job: &OptimiseJob<'_>) -> Result<OptimiseOutput> {
        let _ = job;
        Err(Error::not_implemented("Office decks (W3-A)"))
    }

    fn analyse(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("Office decks analysis (W3-A)"))
    }
}
