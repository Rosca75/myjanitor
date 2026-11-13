//! Video re-encode (OPT-VID-01..06) through external ffmpeg.
//! Owner: W3-C.

use mj_core::{
    ActionKind, Availability, Category, DetectContext, DetectorOutput, Error, Estimate, ExternalTool, FileEntry,
    OptimiseJob, OptimiseOutput, Optimiser, OptimiserId, OptimiserInfo, ParamValues, PlatformSupport, Result,
    SafetyClass,
};

/// Videos: Re-encode to HEVC or AV1
#[derive(Debug, Default, Clone, Copy)]
pub struct Video;

/// Extensions (lower case) this optimiser targets.
pub const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mov", "mkv", "avi", "wmv", "m4v"];

impl Optimiser for Video {
    fn info(&self) -> OptimiserInfo {
        OptimiserInfo {
            id: OptimiserId::Video,
            title: "Videos".into(),
            description: "Re-encode to HEVC or AV1".into(),
            category: Category::Optimise,
            safety: SafetyClass::LossyOptIn,
            default_action: ActionKind::Optimise,
            available_actions: vec![ActionKind::Optimise],
            presets: Vec::new(),
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            tools: vec!["ffmpeg".into()],
            feature_ids: vec![
                "OPT-VID-01".into(),
                "OPT-VID-02".into(),
                "OPT-VID-03".into(),
                "OPT-VID-04".into(),
                "OPT-VID-05".into(),
                "OPT-VID-06".into(),
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
        entry.is_file() && VIDEO_EXTENSIONS.contains(&entry.extension().as_str())
    }

    fn estimate(&self, entry: &FileEntry, params: &ParamValues) -> Result<Estimate> {
        let _ = (entry, params);
        Err(Error::not_implemented("Videos estimate (W3-C)"))
    }

    fn optimise(&self, job: &OptimiseJob<'_>) -> Result<OptimiseOutput> {
        let _ = job;
        Err(Error::not_implemented("Videos (W3-C)"))
    }

    fn analyse(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("Videos analysis (W3-C)"))
    }
}
