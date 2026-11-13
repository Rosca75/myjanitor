//! PDF optimisation: lossless structural pass (OPT-PDF-03/05) and image downsampling presets (OPT-PDF-01/02/04).
//! Owner: W3-B.

use mj_core::{
    ActionKind, Availability, Category, DetectContext, DetectorOutput, Error, Estimate, ExternalTool, FileEntry,
    OptimiseJob, OptimiseOutput, Optimiser, OptimiserId, OptimiserInfo, ParamValues, PlatformSupport, Result,
    SafetyClass,
};

/// PDF files: Lossless structural optimisation
#[derive(Debug, Default, Clone, Copy)]
pub struct PdfLossless;

/// Extensions (lower case) this optimiser targets.
pub const PDFLOSSLESS_EXTENSIONS: &[&str] = &["pdf"];

impl Optimiser for PdfLossless {
    fn info(&self) -> OptimiserInfo {
        OptimiserInfo {
            id: OptimiserId::PdfLossless,
            title: "PDF files".into(),
            description: "Lossless structural optimisation".into(),
            category: Category::Optimise,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Optimise,
            available_actions: vec![ActionKind::Optimise],
            presets: Vec::new(),
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            tools: vec![],
            feature_ids: vec!["OPT-PDF-03".into(), "OPT-PDF-05".into()],
            unit: "files".into(),
        }
    }

    fn availability(&self, tools: &[ExternalTool]) -> Availability {
        let _ = tools;
        Availability::Available
    }

    fn applies_to(&self, entry: &FileEntry) -> bool {
        entry.is_file() && PDFLOSSLESS_EXTENSIONS.contains(&entry.extension().as_str())
    }

    fn estimate(&self, entry: &FileEntry, params: &ParamValues) -> Result<Estimate> {
        let _ = (entry, params);
        Err(Error::not_implemented("PDF files estimate (W3-B)"))
    }

    fn optimise(&self, job: &OptimiseJob<'_>) -> Result<OptimiseOutput> {
        let _ = job;
        Err(Error::not_implemented("PDF files (W3-B)"))
    }

    fn analyse(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("PDF files analysis (W3-B)"))
    }
}

/// PDF images: Downsample images to a target DPI
#[derive(Debug, Default, Clone, Copy)]
pub struct PdfLossy;

/// Extensions (lower case) this optimiser targets.
pub const PDFLOSSY_EXTENSIONS: &[&str] = &["pdf"];

impl Optimiser for PdfLossy {
    fn info(&self) -> OptimiserInfo {
        OptimiserInfo {
            id: OptimiserId::PdfLossy,
            title: "PDF images".into(),
            description: "Downsample images to a target DPI".into(),
            category: Category::Optimise,
            safety: SafetyClass::LossyOptIn,
            default_action: ActionKind::Optimise,
            available_actions: vec![ActionKind::Optimise],
            presets: Vec::new(),
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            tools: vec![],
            feature_ids: vec!["OPT-PDF-01".into(), "OPT-PDF-02".into(), "OPT-PDF-04".into()],
            unit: "files".into(),
        }
    }

    fn availability(&self, tools: &[ExternalTool]) -> Availability {
        let _ = tools;
        Availability::Available
    }

    fn applies_to(&self, entry: &FileEntry) -> bool {
        entry.is_file() && PDFLOSSY_EXTENSIONS.contains(&entry.extension().as_str())
    }

    fn estimate(&self, entry: &FileEntry, params: &ParamValues) -> Result<Estimate> {
        let _ = (entry, params);
        Err(Error::not_implemented("PDF images estimate (W3-B)"))
    }

    fn optimise(&self, job: &OptimiseJob<'_>) -> Result<OptimiseOutput> {
        let _ = job;
        Err(Error::not_implemented("PDF images (W3-B)"))
    }

    fn analyse(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("PDF images analysis (W3-B)"))
    }
}
