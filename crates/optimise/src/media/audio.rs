//! Audio and meeting recordings (OPT-AUD-01..08) through external ffmpeg.
//! Owner: W3-C.

use mj_core::{
    ActionKind, Availability, Category, DetectContext, DetectorOutput, Error, Estimate, ExternalTool, FileEntry,
    OptimiseJob, OptimiseOutput, Optimiser, OptimiserId, OptimiserInfo, ParamValues, PlatformSupport, Result,
    SafetyClass,
};

/// Meeting recordings: Speech preset, silences trimmed
#[derive(Debug, Default, Clone, Copy)]
pub struct Audio;

/// Extensions (lower case) this optimiser targets.
pub const AUDIO_EXTENSIONS: &[&str] = &["wav", "m4a", "mp3", "aiff", "aif", "flac", "wma"];

impl Optimiser for Audio {
    fn info(&self) -> OptimiserInfo {
        OptimiserInfo {
            id: OptimiserId::Audio,
            title: "Meeting recordings".into(),
            description: "Speech preset, silences trimmed".into(),
            category: Category::Optimise,
            safety: SafetyClass::LossyOptIn,
            default_action: ActionKind::Optimise,
            available_actions: vec![ActionKind::Optimise],
            presets: Vec::new(),
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            tools: vec!["ffmpeg".into()],
            feature_ids: vec![
                "OPT-AUD-01".into(),
                "OPT-AUD-02".into(),
                "OPT-AUD-03".into(),
                "OPT-AUD-04".into(),
                "OPT-AUD-05".into(),
                "OPT-AUD-06".into(),
                "OPT-AUD-07".into(),
                "OPT-AUD-08".into(),
            ],
            unit: "files".into(),
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
        entry.is_file() && AUDIO_EXTENSIONS.contains(&entry.extension().as_str())
    }

    fn estimate(&self, entry: &FileEntry, params: &ParamValues) -> Result<Estimate> {
        let _ = (entry, params);
        Err(Error::not_implemented("Meeting recordings estimate (W3-C)"))
    }

    fn optimise(&self, job: &OptimiseJob<'_>) -> Result<OptimiseOutput> {
        let _ = job;
        Err(Error::not_implemented("Meeting recordings (W3-C)"))
    }

    fn analyse(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("Meeting recordings analysis (W3-C)"))
    }
}
