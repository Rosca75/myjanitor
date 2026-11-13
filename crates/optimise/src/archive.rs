//! Archiving (OPT-ARC-01..05): cold folders to ZIP (in-house) or 7z/zstd (external), integrity test before removing sources; recompress existing archives.
//! Owner: W3-B.

use mj_core::{
    ActionKind, Availability, Category, DetectContext, DetectorOutput, Error, Estimate, ExternalTool, FileEntry,
    OptimiseJob, OptimiseOutput, Optimiser, OptimiserId, OptimiserInfo, ParamValues, PlatformSupport, Result,
    SafetyClass,
};

/// Archive cold folders: Folders not used for months, packed into one archive
#[derive(Debug, Default, Clone, Copy)]
pub struct ArchiveFolders;

/// Extensions (lower case) this optimiser targets.
pub const ARCHIVEFOLDERS_EXTENSIONS: &[&str] = &[];

impl Optimiser for ArchiveFolders {
    fn info(&self) -> OptimiserInfo {
        OptimiserInfo {
            id: OptimiserId::ArchiveFolders,
            title: "Archive cold folders".into(),
            description: "Folders not used for months, packed into one archive".into(),
            category: Category::Archive,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Archive,
            available_actions: vec![ActionKind::Archive],
            presets: Vec::new(),
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            tools: vec![],
            feature_ids: vec!["OPT-ARC-01".into(), "OPT-ARC-02".into(), "OPT-ARC-04".into(), "OPT-ARC-05".into()],
            unit: "folders".into(),
        }
    }

    fn availability(&self, tools: &[ExternalTool]) -> Availability {
        let _ = tools;
        Availability::Available
    }

    fn applies_to(&self, entry: &FileEntry) -> bool {
        entry.is_file() && ARCHIVEFOLDERS_EXTENSIONS.contains(&entry.extension().as_str())
    }

    fn estimate(&self, entry: &FileEntry, params: &ParamValues) -> Result<Estimate> {
        let _ = (entry, params);
        Err(Error::not_implemented("Archive cold folders estimate (W3-B)"))
    }

    fn optimise(&self, job: &OptimiseJob<'_>) -> Result<OptimiseOutput> {
        let _ = job;
        Err(Error::not_implemented("Archive cold folders (W3-B)"))
    }

    fn analyse(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("Archive cold folders analysis (W3-B)"))
    }
}

/// Recompress archives: Existing archives repacked with a stronger setting
#[derive(Debug, Default, Clone, Copy)]
pub struct ArchiveRecompress;

/// Extensions (lower case) this optimiser targets.
pub const ARCHIVERECOMPRESS_EXTENSIONS: &[&str] = &["zip"];

impl Optimiser for ArchiveRecompress {
    fn info(&self) -> OptimiserInfo {
        OptimiserInfo {
            id: OptimiserId::ArchiveRecompress,
            title: "Recompress archives".into(),
            description: "Existing archives repacked with a stronger setting".into(),
            category: Category::Archive,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Optimise,
            available_actions: vec![ActionKind::Optimise],
            presets: Vec::new(),
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            tools: vec![],
            feature_ids: vec!["OPT-ARC-03".into()],
            unit: "files".into(),
        }
    }

    fn availability(&self, tools: &[ExternalTool]) -> Availability {
        let _ = tools;
        Availability::Available
    }

    fn applies_to(&self, entry: &FileEntry) -> bool {
        entry.is_file() && ARCHIVERECOMPRESS_EXTENSIONS.contains(&entry.extension().as_str())
    }

    fn estimate(&self, entry: &FileEntry, params: &ParamValues) -> Result<Estimate> {
        let _ = (entry, params);
        Err(Error::not_implemented("Recompress archives estimate (W3-B)"))
    }

    fn optimise(&self, job: &OptimiseJob<'_>) -> Result<OptimiseOutput> {
        let _ = job;
        Err(Error::not_implemented("Recompress archives (W3-B)"))
    }

    fn analyse(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let _ = ctx;
        Err(Error::not_implemented("Recompress archives analysis (W3-B)"))
    }
}

/// ZIP (in-house) and 7z / tar+zstd (external) archive writer and verifier.
#[derive(Debug, Default, Clone, Copy)]
pub struct Zip7Archiver;

impl mj_core::Archiver for Zip7Archiver {
    fn formats(&self, tools: &[ExternalTool]) -> Vec<mj_core::ArchiveFormat> {
        let mut f = vec![mj_core::ArchiveFormat::Zip];
        if mj_core::tools::find(tools, "7z").is_some() {
            f.push(mj_core::ArchiveFormat::SevenZip);
        }
        if mj_core::tools::find(tools, "zstd").is_some() {
            f.push(mj_core::ArchiveFormat::TarZstd);
        }
        f
    }

    fn create(&self, job: &mj_core::ArchiveJob<'_>) -> Result<u64> {
        let _ = job;
        Err(Error::not_implemented("archive creation (W3-B)"))
    }

    fn verify(
        &self,
        archive: &std::path::Path,
        sources: &[std::path::PathBuf],
        base: &std::path::Path,
        cancel: &mj_core::CancelToken,
    ) -> Result<()> {
        let _ = (archive, sources, base, cancel);
        Err(Error::not_implemented("archive verification (W3-B)"))
    }
}
