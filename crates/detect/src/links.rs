//! Invalid symlinks and broken Windows shortcuts (CLN-DET-09).
//!
//! * A symlink or junction is broken when its target does not exist (`NotFound`; any other error
//!   leaves it alone). Targets that may simply be offline are skipped: a network path, a drive
//!   letter whose root is absent, or a Unix mount point that is missing or empty, so an
//!   unplugged drive never makes every link on it "broken" (RF-6).
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
            let t = target.to_string_lossy();
            let drive_present = |root: &str| Path::new(root).try_exists().ok() == Some(true);
            let populated = |dir: &str| std::fs::read_dir(dir).is_ok_and(|mut d| d.next().is_some());
            if target_may_be_offline(&t, &drive_present, &populated) {
                return Err("target_on_removable_or_network_mount");
            }
            Ok(Some(t.into_owned()))
        }
        Err(_) => Err("link_target_unknown"),
    }
}

/// RF-6: true when a missing link target may simply be offline, so the link is not called
/// broken. `drive_present("E:\\")` says whether a drive root exists; `populated(dir)` says
/// whether a mount directory exists and has something in it. Both are injected so the
/// Windows rules are tested on every platform.
///
/// * A network target (`\\server\share`, `\\?\UNC\…`, `//server/share`) may be offline.
/// * A target on a drive letter may be offline when that drive's root is absent (a
///   junction to `E:\Videos` with the USB disk unplugged reports "not found").
/// * Below a Unix mount parent (`/mnt`, `/media`, `/run/media`, `/Volumes`, `/net`,
///   `/misc`), the target may be offline unless its mount directory (`/mnt/<x>`,
///   `/media/<user>/<x>`, `/run/media/<user>/<x>`) exists and is not empty: an empty
///   folder there is usually an unmounted mount point.
fn target_may_be_offline(target: &str, drive_present: &dyn Fn(&str) -> bool, populated: &dyn Fn(&str) -> bool) -> bool {
    let t = target.replace('/', "\\");
    // Verbatim (`\\?\`) and NT (`\??\`, junction targets) prefixes: `UNC\` means network.
    let t = match t.strip_prefix("\\\\?\\").or_else(|| t.strip_prefix("\\??\\")) {
        Some(r) => match r.get(..4) {
            Some(unc) if unc.eq_ignore_ascii_case("UNC\\") => return true,
            _ => r.to_string(),
        },
        None => t,
    };
    if t.starts_with("\\\\") {
        return true;
    }
    let b = t.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        return !drive_present(&format!("{}:\\", b[0] as char));
    }
    // Unix spellings below a mount parent.
    let Some(parent) = MOUNT_PARENTS.iter().find(|m| target.starts_with(&format!("{m}/"))) else {
        return false;
    };
    let rest: Vec<&str> = target[parent.len() + 1..].split('/').filter(|c| !c.is_empty()).collect();
    let depth = if matches!(*parent, "/media" | "/run/media") { 2 } else { 1 };
    if rest.len() < depth {
        return true;
    }
    !populated(&format!("{parent}/{}", rest[..depth].join("/")))
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

    /// RF-6: a link to an unplugged drive, a network share or an unmounted mount point is
    /// not "broken"; one to a missing file on a present drive is.
    #[test]
    fn rf6_offline_targets_are_not_broken() {
        let drives = |root: &str| root == "C:\\";
        let mounts = |dir: &str| dir == "/mnt/nas" || dir == "/media/me/USB";
        let off = |t: &str| target_may_be_offline(t, &drives, &mounts);
        assert!(off("E:\\Videos\\a.mp4"));
        assert!(off("\\\\?\\E:\\Videos"));
        assert!(off("\\??\\E:\\Videos"));
        assert!(!off("C:\\Users\\me\\gone.txt"));
        assert!(!off("\\\\?\\C:\\gone"));
        assert!(off("\\\\srv\\share\\x"));
        assert!(off("\\\\?\\UNC\\srv\\share\\x"));
        assert!(off("//srv/share/x"));
        assert!(off("/mnt/backup/file"), "missing or empty mount directory");
        assert!(!off("/mnt/nas/file"), "mounted: the target is really gone");
        assert!(off("/media/me/OTHER/a"));
        assert!(!off("/media/me/USB/a"));
        assert!(off("/media/me"));
        assert!(!off("/home/me/gone.txt"));
        assert!(!off("relative-gone"));
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
