//! Word optimisation (OPT-DOC-01..05, SAF-08).
//! Owner: W3-A.

use mj_core::{
    ActionKind, Availability, Category, DetectContext, DetectorOutput, Error, Estimate, ExternalTool, FileEntry,
    OptimiseJob, OptimiseOutput, Optimiser, OptimiserId, OptimiserInfo, ParamValues, PlatformSupport, Result,
    SafetyClass,
};

/// Word documents: Compress pictures and remove dead weight
#[derive(Debug, Default, Clone, Copy)]
pub struct Word;

/// Extensions (lower case) this optimiser targets.
pub const WORD_EXTENSIONS: &[&str] = &["docx", "docm", "dotx"];

impl Optimiser for Word {
    fn info(&self) -> OptimiserInfo {
        OptimiserInfo {
            id: OptimiserId::Word,
            title: "Word documents".into(),
            description: "Compress pictures and remove dead weight".into(),
            category: Category::Optimise,
            safety: SafetyClass::LightCopy,
            default_action: ActionKind::Optimise,
            available_actions: vec![ActionKind::Optimise],
            presets: Vec::new(),
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            tools: vec![],
            feature_ids: vec![
                "OPT-DOC-01".into(),
                "OPT-DOC-02".into(),
                "OPT-DOC-03".into(),
                "OPT-DOC-04".into(),
                "OPT-DOC-05".into(),
            ],
            unit: "files".into(),
        }
    }

    fn availability(&self, tools: &[ExternalTool]) -> Availability {
        let _ = tools;
        Availability::Available
    }

    fn applies_to(&self, entry: &FileEntry) -> bool {
        entry.is_file() && WORD_EXTENSIONS.contains(&entry.extension().as_str())
    }

    fn estimate(&self, entry: &FileEntry, params: &ParamValues) -> Result<Estimate> {
        let _ = (entry, params);
        Err(Error::not_implemented("Word documents estimate (W3-A)"))
    }

    fn optimise(&self, job: &OptimiseJob<'_>) -> Result<OptimiseOutput> {
        let _ = job;
        Err(Error::not_implemented("Word documents (W3-A)"))
    }

    fn analyse(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("Word documents analysis (W3-A)"))
    }
}
