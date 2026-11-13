//! Email attachment copies (CLN-DET-20): files in Downloads that duplicate synced SharePoint/OneDrive files.
//! Owner: W2-B.
//!
//! The caller says which folders are Downloads-like and which are synced
//! through two text parameters (one path per line, `;` also separates):
//! [`DOWNLOADS`] and [`SYNCED`]. When a parameter is empty a conservative
//! name heuristic applies: Downloads = any folder named `Downloads`; synced =
//! any folder whose name starts with `OneDrive` (`OneDrive`, `OneDrive -
//! Contoso`) or is `Dropbox`, `Google Drive` or `iCloudDrive`. SharePoint
//! libraries synced under a tenant-named folder are only found when the engine
//! passes the real sync roots (contract change request in the W2-B notes).
//!
//! Files are compared by content with the staged exact lane (size, partial,
//! full xxh64 + SHA-256). A group qualifies when it holds at least one synced
//! file outside the Downloads folders and at least one Downloads file; the
//! Downloads copies are the candidates (pre-selected, sent to the Trash by
//! default) and the first synced file is shown as the keeper. Online-only
//! synced files are never read, so they cannot be compared; they are counted.

use std::path::{Path, PathBuf};

use mj_core::{
    ActionKind, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, Diagnostics, FileEntry,
    GroupId, PlatformSupport, Result, SafetyClass, SettingKind, SettingSpec,
};

use crate::exact::emit_exact;
use crate::keep::Selection;
use crate::staged;

/// Parameter key: Downloads-like folders.
pub const DOWNLOADS: &str = "downloads_folders";
/// Parameter key: synced folders (OneDrive, SharePoint, Dropbox...).
pub const SYNCED: &str = "synced_folders";

/// Attachment copies: Downloads that duplicate synced files
#[derive(Debug, Default, Clone, Copy)]
pub struct AttachmentCopies;

impl Detector for AttachmentCopies {
    fn info(&self) -> DetectorInfo {
        let text = |key: &str, label: &str, help: &str| SettingSpec {
            key: key.into(),
            label: label.into(),
            help: Some(help.into()),
            kind: SettingKind::Text { default: String::new(), placeholder: Some("One folder per line".into()) },
            advanced: true,
            lossy: false,
            disabled_reason: None,
        };
        DetectorInfo {
            id: DetectorId::AttachmentCopies,
            title: "Attachment copies".into(),
            description: "Downloads that duplicate synced files".into(),
            category: Category::Clean,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Trash,
            available_actions: vec![ActionKind::Trash, ActionKind::Quarantine],
            settings: vec![
                text(DOWNLOADS, "Downloads folders", "Empty: every folder named Downloads"),
                text(SYNCED, "Synced folders", "Empty: OneDrive, Dropbox, Google Drive and iCloud Drive folders"),
            ],
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-20".into()],
            unit: "files".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let downloads = crate::path_list(ctx.params.text(DOWNLOADS, ""));
        let synced = crate::path_list(ctx.params.text(SYNCED, ""));
        let mut set = Vec::new();
        let mut kinds = Vec::new();
        let mut online_synced = 0u64;
        for e in ctx.index.files() {
            let d = in_any(&e.path, &downloads, is_downloads_name);
            let s = !d && in_any(&e.path, &synced, is_synced_name);
            if !(d || s) {
                continue;
            }
            if s && e.is_online_only() {
                online_synced += 1;
            }
            set.push(e);
            kinds.push(d);
        }
        let mut diag = Diagnostics::default();
        let groups = staged::exact_groups(ctx, &set, &mut diag)?;
        if online_synced > 0 {
            diag.notes.push(format!(
                "{online_synced} synced file{} online-only and could not be compared",
                if online_synced == 1 { " is" } else { "s are" }
            ));
        }
        let mut out = DetectorOutput::default();
        let mut n = 0u64;
        for g in &groups {
            let original = g.members.iter().copied().find(|&m| !kinds[m]);
            let copies: Vec<usize> = g.members.iter().copied().filter(|&m| kinds[m]).collect();
            let Some(original) = original else { continue };
            if copies.is_empty() {
                continue;
            }
            n += 1;
            let mut members: Vec<&FileEntry> = vec![set[original]];
            members.extend(copies.iter().map(|&c| set[c]));
            emit_exact(GroupId(n), DetectorId::AttachmentCopies, members, 0, g, Selection::AllButKeeper, &mut out);
        }
        for c in &mut out.candidates {
            if c.default_selected {
                c.reason = "Downloaded copy of a file in a synced folder".into();
            } else {
                c.reason = "Synced original (kept)".into();
            }
        }
        out.diagnostics = diag;
        Ok(out)
    }
}

fn in_any(path: &Path, roots: &[PathBuf], heuristic: fn(&str) -> bool) -> bool {
    if roots.is_empty() {
        let parent = path.parent().unwrap_or(path);
        parent.components().any(|c| heuristic(&c.as_os_str().to_string_lossy()))
    } else {
        roots.iter().any(|r| mj_core::pathutil::is_within(r, path))
    }
}

/// Folder names treated as Downloads when no folder is configured.
pub fn is_downloads_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("downloads")
}

/// Folder names treated as synced when no folder is configured.
pub fn is_synced_name(name: &str) -> bool {
    let l = name.to_lowercase();
    l.starts_with("onedrive") || matches!(l.as_str(), "dropbox" | "google drive" | "icloud drive" | "iclouddrive")
}
