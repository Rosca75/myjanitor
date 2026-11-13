//! Excel optimisation (OPT-XLS-01..10, SAF-08).
//! Owner: W3-A.

use mj_core::{
    ActionKind, Availability, Category, DetectContext, DetectorOutput, Error, Estimate, ExternalTool, FileEntry,
    OptimiseJob, OptimiseOutput, Optimiser, OptimiserId, OptimiserInfo, ParamValues, PlatformSupport, Result,
    SafetyClass,
};

/// Excel workbooks: Trim bloated used ranges, styles and pivot caches
#[derive(Debug, Default, Clone, Copy)]
pub struct Excel;

/// Extensions (lower case) this optimiser targets.
pub const EXCEL_EXTENSIONS: &[&str] = &["xlsx", "xlsm"];

impl Optimiser for Excel {
    fn info(&self) -> OptimiserInfo {
        OptimiserInfo {
            id: OptimiserId::Excel,
            title: "Excel workbooks".into(),
            description: "Trim bloated used ranges, styles and pivot caches".into(),
            category: Category::Optimise,
            safety: SafetyClass::LightCopy,
            default_action: ActionKind::Optimise,
            available_actions: vec![ActionKind::Optimise],
            presets: Vec::new(),
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            tools: vec![],
            feature_ids: vec![
                "OPT-XLS-01".into(),
                "OPT-XLS-02".into(),
                "OPT-XLS-03".into(),
                "OPT-XLS-04".into(),
                "OPT-XLS-05".into(),
                "OPT-XLS-06".into(),
                "OPT-XLS-07".into(),
                "OPT-XLS-08".into(),
                "OPT-XLS-09".into(),
                "OPT-XLS-10".into(),
            ],
            unit: "files".into(),
        }
    }

    fn availability(&self, tools: &[ExternalTool]) -> Availability {
        let _ = tools;
        Availability::Available
    }

    fn applies_to(&self, entry: &FileEntry) -> bool {
        entry.is_file() && EXCEL_EXTENSIONS.contains(&entry.extension().as_str())
    }

    fn estimate(&self, entry: &FileEntry, params: &ParamValues) -> Result<Estimate> {
        let _ = (entry, params);
        Err(Error::not_implemented("Excel workbooks estimate (W3-A)"))
    }

    fn optimise(&self, job: &OptimiseJob<'_>) -> Result<OptimiseOutput> {
        let _ = job;
        Err(Error::not_implemented("Excel workbooks (W3-A)"))
    }

    fn analyse(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("Excel workbooks analysis (W3-A)"))
    }
}
