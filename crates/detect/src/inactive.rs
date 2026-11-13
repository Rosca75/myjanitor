//! Inactive project folders (CLN-DET-21): top-level folders of a scan root whose newest file is
//! older than N months (default 18), proposed as a whole for archiving. Review-only selection:
//! candidates are never pre-selected. Hidden/system folders and the usual personal and OS folders
//! (Documents, Downloads, AppData, ...) are not "projects" and are skipped.
//! Owner: W2-D.

use mj_core::{
    ActionKind, Attributes, Category, Confidence, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput,
    PlatformSupport, Result, SafetyClass,
};

use crate::common::{self, Ticker};

/// Folder names (lower case) that are never a project.
const NOT_PROJECTS: &[&str] = &[
    "documents",
    "desktop",
    "downloads",
    "pictures",
    "music",
    "videos",
    "movies",
    "public",
    "templates",
    "appdata",
    "application data",
    "library",
    "snap",
    "lost+found",
    "node_modules",
    "$recycle.bin",
    "system volume information",
    "windows",
    "program files",
    "program files (x86)",
    "programdata",
    "users",
    "onedrive",
    "dropbox",
    "recovery",
    "boot",
];

/// Inactive folders: Project folders idle for months
#[derive(Debug, Default, Clone, Copy)]
pub struct InactiveFolders;

impl Detector for InactiveFolders {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::InactiveFolders,
            title: "Inactive folders".into(),
            description: "Project folders idle for months".into(),
            category: Category::Archive,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Archive,
            available_actions: vec![ActionKind::Archive, ActionKind::Move, ActionKind::Trash],
            settings: vec![common::number_setting(
                "months",
                "Idle for",
                "Top-level folders where nothing has changed for this many months.",
                18.0,
                1.0,
                240.0,
                "months",
            )],
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-21".into()],
            unit: "folders".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let months = ctx.params.number("months", 18.0).max(1.0) as i64;
        let cutoff = ctx.now.add_months(-months);
        let ix = ctx.index;
        let mut out = DetectorOutput::default();
        let mut tick = Ticker::new(ctx, ix.len() as u64);
        for (pos, e) in ix.entries().iter().enumerate() {
            tick.tick()?;
            // Direct children of a scan root only.
            if e.depth != 1 || !e.is_dir() || !common::usable(e) {
                continue;
            }
            let name = common::name_lower(e);
            if name.starts_with('.')
                || NOT_PROJECTS.contains(&name.as_str())
                || e.attrs.intersects(Attributes::HIDDEN | Attributes::SYSTEM)
            {
                out.diagnostics.skip("not_a_project_folder");
                continue;
            }
            let t = ix.totals(pos);
            let Some(newest) = t.newest_modified else { continue }; // no files: empty-folders territory
            if newest >= cutoff || t.reclaimable == 0 {
                continue;
            }
            let (y, m, d) = newest.ymd();
            let reason =
                format!("Nothing changed for {} (last {y:04}-{m:02}-{d:02})", common::human_age(newest, ctx.now));
            let mut c = common::folder_candidate(ix, pos, DetectorId::InactiveFolders, reason, false);
            c.size_after = c.size; // the archive ratio is only known once it is made
            c.confidence = Confidence::Unknown;
            c.columns.insert("last_activity".into(), format!("{y:04}-{m:02}-{d:02}"));
            tick.found(c.size);
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
    fn idle_top_level_folders_are_listed_with_their_total_size() {
        let t = TempDir::new("inactive").unwrap();
        for (f, days) in [
            ("ClientA/a.docx", 700),
            ("ClientA/sub/b.xlsx", 650),
            ("ClientB/a.docx", 700),
            ("ClientB/fresh.docx", 5), // one recent file keeps the whole folder active
            ("ClientC/old.txt", 100),
            ("Documents/old.txt", 900), // personal folder, not a project
            (".hidden/old.txt", 900),
            ("loose.txt", 900), // a file, not a folder
            ("ClientA/deep/x/y/z.txt", 640),
        ] {
            t.write(f, b"0123456789").unwrap();
            age(&t.join(f), days);
        }
        let out = run(&InactiveFolders, &index_of(t.path()), &Default::default());
        assert_eq!(rel(&out, t.path()), vec!["ClientA"]);
        let c = &out.candidates[0];
        assert!(c.size >= 30, "size is the allocated total of the three files");
        assert_eq!(c.file_count, Some(3));
        assert!(!c.default_selected && c.target == mj_core::TargetKind::Folder);
        assert!(c.reason.starts_with("Nothing changed for"));
        // A shorter idle period also finds ClientC.
        let out = run(&InactiveFolders, &index_of(t.path()), &p(&[("months", 2.0)]));
        assert_eq!(rel(&out, t.path()), vec!["ClientA", "ClientC"]);
    }
}
