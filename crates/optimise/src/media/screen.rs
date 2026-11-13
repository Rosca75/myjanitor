//! Screen recordings (OPT-SCR-01..05) through external ffmpeg.
//! Owner: W3-C.

use mj_core::{
    ActionKind, Availability, Category, DetectContext, DetectorOutput, Error, Estimate, ExternalTool, FileEntry,
    OptimiseJob, OptimiseOutput, Optimiser, OptimiserId, OptimiserInfo, ParamValues, PlatformSupport, Result,
    SafetyClass,
};

/// Screen recordings: Low-motion video at a low frame rate, text stays sharp
#[derive(Debug, Default, Clone, Copy)]
pub struct ScreenRecordings;

/// Extensions (lower case) this optimiser targets.
pub const SCREENRECORDINGS_EXTENSIONS: &[&str] = &["mp4", "mkv", "webm", "mov"];

impl Optimiser for ScreenRecordings {
    fn info(&self) -> OptimiserInfo {
        OptimiserInfo {
            id: OptimiserId::ScreenRecordings,
            title: "Screen recordings".into(),
            description: "Low-motion video at a low frame rate, text stays sharp".into(),
            category: Category::Optimise,
            safety: SafetyClass::LossyOptIn,
            default_action: ActionKind::Optimise,
            available_actions: vec![ActionKind::Optimise],
            presets: Vec::new(),
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            tools: vec!["ffmpeg".into()],
            feature_ids: vec![
                "OPT-SCR-01".into(),
                "OPT-SCR-02".into(),
                "OPT-SCR-03".into(),
                "OPT-SCR-04".into(),
                "OPT-SCR-05".into(),
            ],
            unit: "videos".into(),
        }
    }

    fn availability(&self, tools: &[ExternalTool]) -> Availability {
        let _ = tools;
        let needed: &[&str] = &["ffmpeg"];
        match needed.iter().find(|t| mj_core::tools::find(tools, t).is_none()) {
            Some(t) => Availability::Disabled { reason: format!("Requires {t} on PATH") },
            None => Availability::Available,
        }
    }

    fn applies_to(&self, entry: &FileEntry) -> bool {
        entry.is_file() && SCREENRECORDINGS_EXTENSIONS.contains(&entry.extension().as_str())
    }

    fn estimate(&self, entry: &FileEntry, params: &ParamValues) -> Result<Estimate> {
        let _ = (entry, params);
        Err(Error::not_implemented("Screen recordings estimate (W3-C)"))
    }

    fn optimise(&self, job: &OptimiseJob<'_>) -> Result<OptimiseOutput> {
        let _ = job;
        Err(Error::not_implemented("Screen recordings (W3-C)"))
    }

    fn analyse(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("Screen recordings analysis (W3-C)"))
    }
}
