//! Temporary and partial files (CLN-DET-10): `.tmp`, Office `~$` lock files, partial downloads
//! (`.crdownload`, `.part`, `.partial`, `.download`), backups (`.bak`, `.old`, `~` suffix), editor
//! swap files and OS thumbnail databases (`Thumbs.db`, `.DS_Store`). `desktop.ini` is never listed.
//!
//! A file modified within the minimum age (default 24 h) is skipped because a program may still be
//! using it. Files that might hold the only copy of someone's data (backups, partial downloads,
//! swap files) are listed but not pre-selected.
//! Owner: W2-D.

use mj_core::{
    ActionKind, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, EntryKind,
    PlatformSupport, Result, SafetyClass,
};

use crate::common::{self, Ticker};

/// What kind of leftover a file name indicates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TempKind {
    Temporary,
    OfficeLock,
    PartialDownload,
    Backup,
    SwapFile,
    ThumbnailDb,
}

impl TempKind {
    fn label(self) -> &'static str {
        match self {
            TempKind::Temporary => "Temporary file",
            TempKind::OfficeLock => "Office lock file",
            TempKind::PartialDownload => "Unfinished download",
            TempKind::Backup => "Backup copy",
            TempKind::SwapFile => "Editor swap file",
            TempKind::ThumbnailDb => "Thumbnail database",
        }
    }

    /// Pre-selected only when the file carries no user data of its own.
    fn selected(self) -> bool {
        matches!(self, TempKind::Temporary | TempKind::OfficeLock | TempKind::ThumbnailDb)
    }
}

/// Classify a lower-case file name; `None` for anything else (including `desktop.ini`).
pub fn classify(name: &str) -> Option<TempKind> {
    if name == "desktop.ini" {
        return None;
    }
    if matches!(name, "thumbs.db" | "ehthumbs.db" | ".ds_store") {
        return Some(TempKind::ThumbnailDb);
    }
    // `~$report.docx` (Office owner file); skip names that are only the prefix.
    if name.starts_with("~$") && name.len() > 2 {
        return Some(TempKind::OfficeLock);
    }
    if name.ends_with('~') && name.len() > 1 {
        return Some(TempKind::Backup);
    }
    let ext = name.rsplit_once('.').map(|(_, e)| e)?;
    match ext {
        "tmp" | "temp" => Some(TempKind::Temporary),
        "crdownload" | "part" | "partial" | "download" => Some(TempKind::PartialDownload),
        "bak" | "old" => Some(TempKind::Backup),
        "swp" | "swo" => Some(TempKind::SwapFile),
        _ => None,
    }
}

/// Temporary files: Leftover temporary and partial downloads
#[derive(Debug, Default, Clone, Copy)]
pub struct TempFiles;

impl Detector for TempFiles {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::TempFiles,
            title: "Temporary files".into(),
            description: "Leftover temporary and partial downloads".into(),
            category: Category::Clean,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Trash,
            available_actions: vec![ActionKind::Trash, ActionKind::Quarantine, ActionKind::PermanentDelete],
            settings: vec![common::number_setting(
                "min_age_hours",
                "Ignore files changed within",
                "Recently changed files may still be in use by a program.",
                24.0,
                0.0,
                8760.0,
                "hours",
            )],
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-10".into()],
            unit: "files".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let cutoff = common::hours_ago(ctx.now, ctx.params.number("min_age_hours", 24.0));
        let mut out = DetectorOutput::default();
        let mut tick = Ticker::new(ctx, ctx.index.len() as u64);
        for e in ctx.index.entries() {
            tick.tick()?;
            if e.kind != EntryKind::File || !common::usable(e) {
                continue;
            }
            let Some(kind) = classify(&common::name_lower(e)) else { continue };
            // Unknown modification time counts as "maybe in use".
            if !common::before(e.modified, cutoff) {
                out.diagnostics.skip("recently_modified");
                continue;
            }
            let age = e.modified.map(|m| common::human_age(m, ctx.now)).unwrap_or_default();
            let reason = format!("{}, last changed {age} ago", kind.label());
            tick.found(e.reclaimable());
            let mut c = common::file_candidate(e, DetectorId::TempFiles, reason, kind.selected());
            c.columns.insert("kind".into(), kind.label().into());
            out.candidates.push(c);
        }
        tick.finish();
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::*;
    use mj_core::testutil::TempDir;

    #[test]
    fn classification_table() {
        for (n, k) in [
            ("a.tmp", TempKind::Temporary),
            ("~$report.docx", TempKind::OfficeLock),
            ("movie.mkv.crdownload", TempKind::PartialDownload),
            ("x.part", TempKind::PartialDownload),
            ("x.partial", TempKind::PartialDownload),
            ("x.download", TempKind::PartialDownload),
            ("notes.bak", TempKind::Backup),
            ("x.old", TempKind::Backup),
            ("draft.txt~", TempKind::Backup),
            (".readme.swp", TempKind::SwapFile),
            ("thumbs.db", TempKind::ThumbnailDb),
            (".ds_store", TempKind::ThumbnailDb),
        ] {
            assert_eq!(classify(n), Some(k), "{n}");
        }
        for n in ["desktop.ini", "~$", "~", "readme.txt", "tmp", "archive.tmp.zip", "backup"] {
            assert_eq!(classify(n), None, "{n}");
        }
    }

    #[test]
    fn old_temp_files_found_fresh_ones_ignored() {
        let t = TempDir::new("temp").unwrap();
        let old =
            ["old.tmp", "~$doc.docx", "Thumbs.db", ".DS_Store", "dl.crdownload", "x.bak", "desktop.ini", "keep.txt"];
        for f in old {
            t.write(f, b"x").unwrap();
            age(&t.join(f), 3);
        }
        t.write("fresh.tmp", b"x").unwrap(); // modified just now
        t.write("sub/~$new.xlsx", b"x").unwrap();
        let out = run(&TempFiles, &index_of(t.path()), &Default::default());
        assert_eq!(
            rel(&out, t.path()),
            vec![".DS_Store", "Thumbs.db", "dl.crdownload", "old.tmp", "x.bak", "~$doc.docx"]
        );
        assert_eq!(out.diagnostics.skipped["recently_modified"], 2);
        let sel = |n: &str| out.candidates.iter().find(|c| c.path.ends_with(n)).unwrap().default_selected;
        assert!(sel("old.tmp") && sel("Thumbs.db") && sel("~$doc.docx"));
        assert!(!sel("x.bak") && !sel("dl.crdownload"));
    }

    #[test]
    fn minimum_age_is_configurable() {
        let t = TempDir::new("temp_age").unwrap();
        t.write("fresh.tmp", b"x").unwrap();
        let ix = index_of(t.path());
        assert!(run(&TempFiles, &ix, &Default::default()).candidates.is_empty());
        // Zero minimum age: everything with a known modification time before "now" qualifies.
        let later = mj_core::Timestamp::now().add_days(1);
        let out = run_at(&TempFiles, &ix, &p(&[("min_age_hours", 0.0)]), later, &mj_core::CancelToken::new()).unwrap();
        assert_eq!(out.candidates.len(), 1);
    }
}
