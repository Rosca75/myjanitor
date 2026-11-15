//! Empty files and empty folders, nested empty trees reported topmost (CLN-DET-06).
//! Owner: W2-D.

use std::collections::HashSet;
use std::path::Path;

use mj_core::{
    ActionKind, Attributes, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, EntryKind,
    PlatformSupport, Result, SafetyClass,
};

use crate::common::{self, Ticker};

/// Zero-byte files that exist on purpose (keep-the-folder markers, Python packages, ...).
const INTENTIONAL: &[&str] = &[
    ".gitkeep",
    ".keep",
    ".gitignore",
    ".gitattributes",
    ".nojekyll",
    ".nomedia",
    ".placeholder",
    ".empty",
    "__init__.py",
    "py.typed",
    ".dockerignore",
];

/// Folders whose (empty) subfolders are part of a tool's own layout; never proposed.
const TOOL_DIRS: &[&str] = &[".git", ".svn", ".hg", ".bzr", "node_modules", ".idea", ".vscode"];

/// Empty files: Zero-byte files
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyFiles;

impl Detector for EmptyFiles {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::EmptyFiles,
            title: "Empty files".into(),
            description: "Zero-byte files".into(),
            category: Category::Clean,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Trash,
            available_actions: vec![ActionKind::Trash, ActionKind::Quarantine, ActionKind::PermanentDelete],
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-06".into()],
            unit: "files".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let mut out = DetectorOutput::default();
        let mut tick = Ticker::new(ctx, ctx.index.len() as u64);
        for e in ctx.index.entries() {
            tick.tick()?;
            if e.kind != EntryKind::File || e.size != 0 || !common::usable(e) {
                continue;
            }
            if INTENTIONAL.contains(&common::name_lower(e).as_str()) || common::has_component(&e.path, TOOL_DIRS) {
                out.diagnostics.skip("intentionally_empty");
                continue;
            }
            tick.found(e.reclaimable());
            out.candidates.push(common::file_candidate(e, DetectorId::EmptyFiles, "Empty file (0 bytes)", true));
        }
        tick.finish();
        Ok(out)
    }
}

/// Empty folders: Folders with nothing inside, including nested empty trees
#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyFolders;

impl Detector for EmptyFolders {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::EmptyFolders,
            title: "Empty folders".into(),
            description: "Folders with nothing inside, including nested empty trees".into(),
            category: Category::Clean,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Trash,
            available_actions: vec![ActionKind::Trash, ActionKind::PermanentDelete],
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-06".into()],
            unit: "folders".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let ix = ctx.index;
        let mut out = DetectorOutput::default();
        let mut tick = Ticker::new(ctx, ix.len() as u64 * 2);

        // Pass 1: every entry that is not a directory makes all of its ancestors non-empty. A link
        // or device node counts as content, so a folder holding only a symlink is not empty.
        let mut non_empty: HashSet<usize> = HashSet::new();
        for (i, e) in ix.entries().iter().enumerate() {
            tick.tick()?;
            if e.kind == EntryKind::Dir {
                continue;
            }
            let mut cur = ix.parent(i);
            while let Some(a) = cur {
                if !non_empty.insert(a) {
                    break; // ancestors above are already marked
                }
                cur = ix.parent(a);
            }
        }

        // Pass 2: empty directories, then keep only the topmost of each nested empty tree.
        let mut empty: Vec<usize> = Vec::new();
        for (i, e) in ix.entries().iter().enumerate() {
            tick.tick()?;
            let skip = e.kind != EntryKind::Dir
                || !common::usable(e)
                || non_empty.contains(&i)
                || e.attrs.contains(Attributes::SYSTEM)
                || common::has_component(&e.path, TOOL_DIRS);
            if !skip {
                empty.push(i);
            }
        }
        for pos in common::topmost(ix, &empty) {
            let e = ix.entry(pos);
            // The index may have missed content (unreadable folder); confirm against the disk now.
            match confirm_empty_tree(&e.path, 64) {
                Some(true) => {}
                Some(false) => {
                    out.diagnostics.skip("not_empty_on_disk");
                    continue;
                }
                None => {
                    out.diagnostics.unreadable(&e.path);
                    continue;
                }
            }
            let inner = ix.totals(pos).dirs.saturating_sub(1); // totals count the folder itself
            let reason = if inner == 0 {
                "Empty folder".to_string()
            } else {
                format!("Empty folder tree ({inner} empty subfolder{} inside)", if inner == 1 { "" } else { "s" })
            };
            let mut c = common::file_candidate(e, DetectorId::EmptyFolders, reason, true);
            c.size = 0;
            c.file_count = Some(0);
            out.candidates.push(c);
        }
        tick.finish();
        Ok(out)
    }
}

/// `Some(true)` when `dir` holds only (recursively) empty directories, `Some(false)` when anything
/// else is inside, `None` when it cannot be read. Never follows links: a link counts as content.
fn confirm_empty_tree(dir: &Path, depth_left: u32) -> Option<bool> {
    let rd = std::fs::read_dir(dir).ok()?;
    for item in rd {
        let item = item.ok()?;
        let ft = item.file_type().ok()?;
        if !ft.is_dir() {
            return Some(false);
        }
        if depth_left == 0 {
            return Some(false); // too deep to prove empty: stay safe
        }
        if !confirm_empty_tree(&item.path(), depth_left - 1)? {
            return Some(false);
        }
    }
    Some(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::*;
    use mj_core::testutil::TempDir;

    #[test]
    fn empty_files_found_but_markers_kept() {
        let t = TempDir::new("emptyfiles").unwrap();
        t.write("a/zero.txt", b"").unwrap();
        t.write("a/full.txt", b"x").unwrap();
        t.write("a/.gitkeep", b"").unwrap();
        t.write("pkg/__init__.py", b"").unwrap();
        t.write("repo/.git/HEAD_EMPTY", b"").unwrap();
        let out = run(&EmptyFiles, &index_of(t.path()), &Default::default());
        assert_eq!(rel(&out, t.path()), vec!["a/zero.txt"]);
        assert!(out.candidates[0].default_selected);
        assert_eq!(out.diagnostics.skipped["intentionally_empty"], 3);
    }

    #[test]
    fn nested_empty_tree_is_reported_at_the_top_only() {
        let t = TempDir::new("emptydirs").unwrap();
        t.mkdir("junk/a/b/c").unwrap();
        t.mkdir("junk/d").unwrap();
        t.mkdir("keep/empty_inside").unwrap();
        t.write("keep/file.txt", b"hi").unwrap();
        t.mkdir("lonely").unwrap();
        let out = run(&EmptyFolders, &index_of(t.path()), &Default::default());
        // `keep` has a file, so only `keep/empty_inside` is empty; `junk` and `lonely` are topmost.
        assert_eq!(rel(&out, t.path()), vec!["junk", "keep/empty_inside", "lonely"]);
        let junk = out.candidates.iter().find(|c| c.path.ends_with("junk")).unwrap();
        assert!(junk.reason.contains("4 empty subfolders"), "{}", junk.reason);
        assert_eq!(junk.size, 0);
    }

    #[test]
    fn root_and_vcs_folders_are_never_reported() {
        let t = TempDir::new("emptyroot").unwrap();
        t.mkdir("repo/.git/refs/tags").unwrap();
        t.write("repo/a.txt", b"a").unwrap();
        let out = run(&EmptyFolders, &index_of(t.path()), &Default::default());
        assert!(out.candidates.is_empty(), "{:?}", rel(&out, t.path()));
        // A fully empty scan root is not a candidate either.
        let e = TempDir::new("emptyroot2").unwrap();
        assert!(run(&EmptyFolders, &index_of(e.path()), &Default::default()).candidates.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn folder_with_only_a_link_is_not_empty() {
        let t = TempDir::new("emptylink").unwrap();
        t.mkdir("d").unwrap();
        std::os::unix::fs::symlink("/nonexistent-target", t.join("d/l")).unwrap();
        assert!(run(&EmptyFolders, &index_of(t.path()), &Default::default()).candidates.is_empty());
    }

    #[test]
    fn stale_index_is_confirmed_against_the_disk() {
        let t = TempDir::new("emptystale").unwrap();
        t.mkdir("d").unwrap();
        let ix = index_of(t.path());
        t.write("d/late.txt", b"x").unwrap(); // appears after the scan
        let out = run(&EmptyFolders, &ix, &Default::default());
        assert!(out.candidates.is_empty());
        assert_eq!(out.diagnostics.skipped["not_empty_on_disk"], 1);
    }

    #[test]
    fn cancelled_run_returns_cancelled() {
        let t = TempDir::new("emptycancel").unwrap();
        t.write("a", b"").unwrap();
        let c = mj_core::CancelToken::new();
        c.cancel();
        let r = run_at(&EmptyFiles, &index_of(t.path()), &Default::default(), mj_core::Timestamp::now(), &c);
        assert_eq!(r.unwrap_err(), mj_core::Error::Cancelled);
    }
}
