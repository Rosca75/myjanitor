//! Generic ZIP-based repack at maximum deflate (OPT-OFC-01/03): EPUB, CBZ, JAR/APK, ODF.
//! Owner: W3-A.

use mj_core::{
    ActionKind, Availability, Category, DetectContext, DetectorOutput, Error, Estimate, ExternalTool, FileEntry,
    OptimiseJob, OptimiseOutput, Optimiser, OptimiserId, OptimiserInfo, ParamValues, PlatformSupport, Result,
    SafetyClass,
};

/// ZIP-based files: Repack EPUB, CBZ, JAR and ODF files losslessly
#[derive(Debug, Default, Clone, Copy)]
pub struct ZipRepack;

/// Extensions (lower case) this optimiser targets.
pub const ZIPREPACK_EXTENSIONS: &[&str] = &["epub", "cbz", "jar", "apk", "odt", "ods", "odp"];

impl Optimiser for ZipRepack {
    fn info(&self) -> OptimiserInfo {
        OptimiserInfo {
            id: OptimiserId::ZipRepack,
            title: "ZIP-based files".into(),
            description: "Repack EPUB, CBZ, JAR and ODF files losslessly".into(),
            category: Category::Optimise,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Optimise,
            available_actions: vec![ActionKind::Optimise],
            presets: Vec::new(),
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            tools: vec![],
            feature_ids: vec!["OPT-OFC-01".into(), "OPT-OFC-03".into()],
            unit: "files".into(),
        }
    }

    fn availability(&self, tools: &[ExternalTool]) -> Availability {
        let _ = tools;
        Availability::Available
    }

    fn applies_to(&self, entry: &FileEntry) -> bool {
        entry.is_file() && ZIPREPACK_EXTENSIONS.contains(&entry.extension().as_str())
    }

    fn estimate(&self, entry: &FileEntry, params: &ParamValues) -> Result<Estimate> {
        let _ = (entry, params);
        Err(Error::not_implemented("ZIP-based files estimate (W3-A)"))
    }

    fn optimise(&self, job: &OptimiseJob<'_>) -> Result<OptimiseOutput> {
        let _ = job;
        Err(Error::not_implemented("ZIP-based files (W3-A)"))
    }

    fn analyse(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("ZIP-based files analysis (W3-A)"))
    }
}
