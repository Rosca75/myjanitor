//! Invalid symlinks and broken Windows shortcuts (CLN-DET-09).
//!
//! * A symlink or junction is broken when its target does not exist (`NotFound`; any other error
//!   leaves it alone). Targets under removable or network mount points are skipped unless their
//!   parent folder is present, so an unplugged drive never makes every link on it "broken".
//! * A `.lnk` shortcut is parsed in-house (`lnk`) and evaluated only on Windows, only for local
//!   absolute targets, and only when the drive itself exists.
//!
//! Owner: W2-D.

use std::io;
use std::path::Path;

use mj_core::{
    ActionKind, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, EntryKind, FileEntry,
    PlatformSupport, Result, SafetyClass,
};

use crate::common::{self, Ticker};
use crate::fsio;
use crate::lnk::{parse_lnk, LnkTarget, MAX_LNK_BYTES};

/// Mount parents whose children may simply be offline.
const MOUNT_PARENTS: &[&str] = &["/mnt", "/media", "/run/media", "/Volumes", "/net", "/misc"];

/// Verdict for a shortcut after parsing and checking its target.
#[derive(Debug, PartialEq, Eq)]
pub enum LnkVerdict {
    /// The local target is gone while its drive is present.
    Broken(String),
    Fine,
    /// Not evaluated, with a diagnostics key.
    Unknown(&'static str),
}

/// Evaluate shortcut bytes. `exists` answers "does this Windows path exist?" (`None` = unknown);
/// it is injected so the logic is testable on every platform.
pub fn evaluate_lnk(bytes: &[u8], exists: &dyn Fn(&str) -> Option<bool>) -> LnkVerdict {
    match parse_lnk(bytes) {
        Err(_) => LnkVerdict::Unknown("lnk_malformed"),
        Ok(LnkTarget::NotEvaluable(_)) => LnkVerdict::Unknown("lnk_not_local"),
        Ok(LnkTarget::Local(t)) => {
            // The drive must be present, otherwise "missing" only means "unplugged".
            let root = format!("{}\\", &t[..2]);
            if exists(&root) != Some(true) {
                return LnkVerdict::Unknown("lnk_drive_absent");
            }
            match exists(&t) {
                Some(false) => LnkVerdict::Broken(t),
                Some(true) => LnkVerdict::Fine,
                None => LnkVerdict::Unknown("lnk_unknown"),
            }
        }
    }
}

/// Broken links: Shortcuts and links whose target is gone
#[derive(Debug, Default, Clone, Copy)]
pub struct BrokenLinks;

impl Detector for BrokenLinks {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::BrokenLinks,
            title: "Broken links".into(),
            description: "Shortcuts and links whose target is gone".into(),
            category: Category::Clean,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Trash,
            available_actions: vec![ActionKind::Trash, ActionKind::PermanentDelete],
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-09".into()],
            unit: "links".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let mut out = DetectorOutput::default();
        let mut tick = Ticker::new(ctx, ctx.index.len() as u64);
        for e in ctx.index.entries() {
            tick.tick()?;
            if e.depth == 0 {
                continue;
            }
            if e.kind == EntryKind::Link {
                match check_symlink(&e.path) {
                    Ok(Some(target)) => {
                        let reason = format!("Link points to a missing target: {target}");
                        out.candidates.push(common::file_candidate(e, DetectorId::BrokenLinks, reason, true));
                    }
                    Ok(None) => {}
                    Err(why) => out.diagnostics.skip(why),
                }
            } else if e.is_file() && e.extension() == "lnk" && !e.is_online_only() {
                self.check_shortcut(e, &mut out);
            }
        }
        tick.finish();
        Ok(out)
    }
}

impl BrokenLinks {
    fn check_shortcut(&self, e: &FileEntry, out: &mut DetectorOutput) {
        if !cfg!(windows) {
            // Drive letters mean nothing here; never guess.
            out.diagnostics.skip("lnk_not_evaluable_on_this_os");
            return;
        }
        if e.size > MAX_LNK_BYTES {
            out.diagnostics.skip("lnk_malformed");
            return;
        }
        let Ok(bytes) = fsio::read_head(&e.path, e.size as usize) else {
            out.diagnostics.unreadable(&e.path);
            return;
        };
        let exists = |p: &str| Path::new(p).try_exists().ok();
        match evaluate_lnk(&bytes, &exists) {
            LnkVerdict::Broken(t) => {
                let reason = format!("Shortcut points to a missing target: {t}");
                out.candidates.push(common::file_candidate(e, DetectorId::BrokenLinks, reason, true));
            }
            LnkVerdict::Fine => {}
            LnkVerdict::Unknown(why) => out.diagnostics.skip(why),
        }
    }
}

/// `Ok(Some(target))` when the link is provably dangling, `Ok(None)` when fine or unknowable,
/// `Err(key)` when skipped for a reason worth counting.
fn check_symlink(link: &Path) -> std::result::Result<Option<String>, &'static str> {
    let target = std::fs::read_link(link).map_err(|_| "link_unreadable")?;
    match std::fs::metadata(link) {
        Ok(_) => Ok(None),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            if on_removable_mount(&target) {
                return Err("target_on_removable_or_network_mount");
            }
            Ok(Some(target.to_string_lossy().into_owned()))
        }
        Err(_) => Err("link_target_unknown"),
    }
}

/// Absolute target below a mount parent whose own parent folder is missing (mount absent).
fn on_removable_mount(target: &Path) -> bool {
    let t = target.to_string_lossy();
    let under_mount = MOUNT_PARENTS.iter().any(|m| t.starts_with(&format!("{m}/")));
    under_mount && !target.parent().is_some_and(|p| p.exists())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lnk::build_lnk;
    #[cfg(not(windows))]
    use crate::testkit::*;
    #[cfg(not(windows))]
    use mj_core::testutil::TempDir;

    #[cfg(unix)]
    #[test]
    fn dangling_symlinks_are_reported_and_live_ones_are_not() {
        use std::os::unix::fs::symlink;
        let t = TempDir::new("links").unwrap();
        t.write("real.txt", b"data").unwrap();
        t.mkdir("dir").unwrap();
        symlink(t.join("real.txt"), t.join("good_link")).unwrap();
        symlink(t.join("dir"), t.join("good_dir_link")).unwrap();
        symlink(t.join("gone.txt"), t.join("dangling")).unwrap();
        symlink("relative-gone", t.join("dangling_rel")).unwrap();
        symlink("/mnt/definitely-not-mounted-xyz/file", t.join("offline_mount")).unwrap();
        let out = run(&BrokenLinks, &index_of(t.path()), &Default::default());
        assert_eq!(rel(&out, t.path()), vec!["dangling", "dangling_rel"]);
        assert!(out.candidates.iter().all(|c| c.default_selected && c.size == 0));
        assert!(out.candidates[0].reason.contains("missing target"));
        assert_eq!(out.diagnostics.skipped["target_on_removable_or_network_mount"], 1);
        // The detector is read-only.
        assert!(t.join("dangling").symlink_metadata().is_ok());
    }

    #[test]
    fn shortcut_verdicts_with_an_injected_filesystem() {
        let present = |p: &str| match p {
            "C:\\" => Some(true),
            "C:\\Users\\me\\ok.txt" => Some(true),
            "C:\\Users\\me\\gone.txt" => Some(false),
            "C:\\Users\\me\\denied.txt" => None,
            _ => Some(false), // every other drive is absent
        };
        let v = |base: &str| evaluate_lnk(&build_lnk(base, ""), &present);
        assert_eq!(v("C:\\Users\\me\\ok.txt"), LnkVerdict::Fine);
        assert_eq!(v("C:\\Users\\me\\gone.txt"), LnkVerdict::Broken("C:\\Users\\me\\gone.txt".into()));
        assert_eq!(v("C:\\Users\\me\\denied.txt"), LnkVerdict::Unknown("lnk_unknown"));
        // Unplugged drive: never reported.
        assert_eq!(v("E:\\Photos\\a.jpg"), LnkVerdict::Unknown("lnk_drive_absent"));
        assert_eq!(evaluate_lnk(b"garbage", &present), LnkVerdict::Unknown("lnk_malformed"));
        assert_eq!(evaluate_lnk(&build_lnk("\\\\srv\\share\\x", ""), &present), LnkVerdict::Unknown("lnk_not_local"));
    }

    #[cfg(not(windows))]
    #[test]
    fn shortcuts_are_counted_as_not_evaluable_off_windows() {
        let t = TempDir::new("lnk_nonwin").unwrap();
        t.write("a.lnk", &build_lnk("C:\\gone\\x.txt", "")).unwrap();
        let out = run(&BrokenLinks, &index_of(t.path()), &Default::default());
        assert!(out.candidates.is_empty());
        assert_eq!(out.diagnostics.skipped["lnk_not_evaluable_on_this_os"], 1);
    }
}
