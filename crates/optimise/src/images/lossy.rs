//! Lossy image recompression, conversion and resize (OPT-IMG-02/03/04) through external encoders (cwebp, avifenc, cjxl).
//! Owner: W3-C.

use mj_core::{
    ActionKind, Availability, Category, DetectContext, DetectorOutput, Error, Estimate, ExternalTool, FileEntry,
    OptimiseJob, OptimiseOutput, Optimiser, OptimiserId, OptimiserInfo, ParamValues, PlatformSupport, Result,
    SafetyClass,
};

/// Photos (lossy): Re-encode to WebP, AVIF or JPEG XL
#[derive(Debug, Default, Clone, Copy)]
pub struct ImageLossy;

/// Extensions (lower case) this optimiser targets.
pub const IMAGELOSSY_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png"];

impl Optimiser for ImageLossy {
    fn info(&self) -> OptimiserInfo {
        OptimiserInfo {
            id: OptimiserId::ImageLossy,
            title: "Photos (lossy)".into(),
            description: "Re-encode to WebP, AVIF or JPEG XL".into(),
            category: Category::Optimise,
            safety: SafetyClass::LossyOptIn,
            default_action: ActionKind::Optimise,
            available_actions: vec![ActionKind::Optimise],
            presets: Vec::new(),
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            tools: vec!["cwebp".into()],
            feature_ids: vec!["OPT-IMG-02".into(), "OPT-IMG-03".into(), "OPT-IMG-04".into()],
            unit: "images".into(),
        }
    }

    fn availability(&self, tools: &[ExternalTool]) -> Availability {
        let _ = tools;
        let needed: &[&str] = &["cwebp"];
        match needed.iter().find(|t| mj_core::tools::find(tools, t).is_none()) {
            Some(t) => Availability::Disabled { reason: format!("Requires {t} on PATH") },
            None => Availability::Available,
        }
    }

    fn applies_to(&self, entry: &FileEntry) -> bool {
        entry.is_file() && IMAGELOSSY_EXTENSIONS.contains(&entry.extension().as_str())
    }

    fn estimate(&self, entry: &FileEntry, params: &ParamValues) -> Result<Estimate> {
        let _ = (entry, params);
        Err(Error::not_implemented("Photos (lossy) estimate (W3-C)"))
    }

    fn optimise(&self, job: &OptimiseJob<'_>) -> Result<OptimiseOutput> {
        let _ = job;
        Err(Error::not_implemented("Photos (lossy) (W3-C)"))
    }

    fn analyse(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("Photos (lossy) analysis (W3-C)"))
    }
}
