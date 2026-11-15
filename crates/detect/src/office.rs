//! Office leftovers (CLN-DET-12): AutoRecover and UnsavedFiles folders, Teams and Outlook local
//! caches. These locations only exist on Windows, so the detector is marked Windows-only and
//! returns nothing elsewhere.
//!
//! AutoRecover and UnsavedFiles can hold the *only* copy of unsaved work, and the Outlook offline
//! cache (OST) can hold mail that has not synced yet: those are listed but never pre-selected.
//! Teams and Office file caches are rebuilt automatically and are pre-selected.
//! Owner: W2-D.

use mj_core::{
    ActionKind, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, PlatformSupport, Result,
    SafetyClass,
};

use crate::rules::{self, rule, RuleDef, RuleSet, Target};

/// The Windows rule table.
pub fn office_rules() -> Vec<RuleDef> {
    let d = Target::Dir;
    let f = Target::File;
    let mut v = vec![
        rule(
            "Office",
            "Unsaved files",
            "may be the only copy of unsaved work",
            "**/AppData/Local/Microsoft/Office/UnsavedFiles",
            d,
        )
        .review(),
        rule(
            "Office",
            "AutoRecover files",
            "may be the only copy of unsaved work",
            "**/AppData/Roaming/Microsoft/Word/*.asd",
            f,
        )
        .review(),
        rule(
            "Office",
            "AutoRecover files",
            "may be the only copy of unsaved work",
            "**/AppData/Roaming/Microsoft/Excel/*.xar",
            f,
        )
        .review(),
        rule(
            "Office",
            "AutoRecover files",
            "may be the only copy of unsaved work",
            "**/AppData/Roaming/Microsoft/PowerPoint/*.asd",
            f,
        )
        .review(),
        rule(
            "Office",
            "Office file cache",
            "rebuilt when documents are opened",
            "**/AppData/Local/Microsoft/Office/*/OfficeFileCache",
            d,
        ),
        rule(
            "Outlook",
            "Offline address books",
            "downloaded again on next sync",
            "**/AppData/Local/Microsoft/Outlook/Offline Address Books",
            d,
        ),
        rule("Outlook", "Roaming cache", "rebuilt on next start", "**/AppData/Local/Microsoft/Outlook/RoamCache", d),
        rule(
            "Outlook",
            "Offline mail cache",
            "re-downloaded from the server; unsynced mail would be lost",
            "**/AppData/Local/Microsoft/Outlook/*.ost",
            f,
        )
        .review(),
        rule(
            "Outlook",
            "Offline mail cache",
            "re-downloaded from the server; unsynced mail would be lost",
            "**/AppData/Local/Microsoft/Outlook/*.nst",
            f,
        )
        .review(),
        rule(
            "Teams (new)",
            "Teams cache",
            "rebuilt on next start",
            "**/AppData/Local/Packages/MSTeams_*/LocalCache",
            d,
        ),
    ];
    for sub in ["Cache", "Code Cache", "GPUCache", "blob_storage", "tmp", "Service Worker/CacheStorage", "IndexedDB"] {
        v.push(rule(
            "Teams (classic)",
            "Teams cache",
            "rebuilt on next start",
            format!("**/AppData/Roaming/Microsoft/Teams/{sub}"),
            d,
        ));
    }
    v
}

/// Office leftovers: AutoRecover files and Office caches
#[derive(Debug, Default, Clone, Copy)]
pub struct OfficeLeftovers;

impl OfficeLeftovers {
    /// The detection logic without the platform gate (used by `run` on Windows and by tests).
    pub fn scan(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let set = RuleSet::new(office_rules());
        rules::run_rules(ctx, &set, DetectorId::OfficeLeftovers, Vec::new(), false, 0.0)
    }
}

impl Detector for OfficeLeftovers {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::OfficeLeftovers,
            title: "Office leftovers".into(),
            description: "AutoRecover files and Office caches".into(),
            category: Category::Clean,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Quarantine,
            available_actions: vec![ActionKind::Quarantine, ActionKind::Trash],
            settings: Vec::new(),
            platform: PlatformSupport::WindowsOnly,
            feature_ids: vec!["CLN-DET-12".into()],
            unit: "files".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        if !self.info().platform.here() {
            return Ok(DetectorOutput::default());
        }
        self.scan(ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::*;
    use mj_core::testutil::TempDir;

    fn fixture() -> TempDir {
        let t = TempDir::new("office").unwrap();
        for p in [
            "u/AppData/Local/Microsoft/Office/UnsavedFiles/Book1((Unsaved-1)).xlsb",
            "u/AppData/Roaming/Microsoft/Word/AutoRecovery save of Doc.asd",
            "u/AppData/Roaming/Microsoft/Word/Normal.dotm",
            "u/AppData/Local/Microsoft/Outlook/me@x.ost",
            "u/AppData/Local/Microsoft/Outlook/RoamCache/Stream_1.dat",
            "u/AppData/Roaming/Microsoft/Teams/Cache/data_0",
            "u/AppData/Roaming/Microsoft/Teams/settings.json",
            "u/AppData/Local/Microsoft/Office/16.0/OfficeFileCache/fsd.bin",
            "u/Documents/Report.docx",
        ] {
            t.write(p, b"0123456789").unwrap();
        }
        t
    }

    #[test]
    fn finds_office_leftovers_and_protects_unsaved_work() {
        let t = fixture();
        let ix = index_of(t.path());
        let merged = mj_core::params::defaults(&[]);
        let ctx = DetectContext {
            index: &ix,
            params: &merged,
            now: mj_core::Timestamp::now(),
            fingerprints: &mj_core::NoFingerprintStore,
            cancel: &mj_core::CancelToken::new(),
            progress: &mj_core::NullProgress,
            threads: 1,
        };
        let out = OfficeLeftovers.scan(&ctx).unwrap();
        assert_eq!(
            rel(&out, t.path()),
            vec![
                "u/AppData/Local/Microsoft/Office/16.0/OfficeFileCache",
                "u/AppData/Local/Microsoft/Office/UnsavedFiles",
                "u/AppData/Local/Microsoft/Outlook/RoamCache",
                "u/AppData/Local/Microsoft/Outlook/me@x.ost",
                "u/AppData/Roaming/Microsoft/Teams/Cache",
                "u/AppData/Roaming/Microsoft/Word/AutoRecovery save of Doc.asd",
            ]
        );
        let sel = |n: &str| out.candidates.iter().find(|c| c.path.ends_with(n)).unwrap().default_selected;
        assert!(sel("Cache") && sel("RoamCache") && sel("OfficeFileCache"));
        assert!(!sel("UnsavedFiles") && !sel("me@x.ost") && !sel("AutoRecovery save of Doc.asd"));
    }

    #[cfg(not(windows))]
    #[test]
    fn returns_nothing_off_windows() {
        let t = fixture();
        let out = run(&OfficeLeftovers, &index_of(t.path()), &Default::default());
        assert!(out.candidates.is_empty());
        assert_eq!(OfficeLeftovers.info().platform, PlatformSupport::WindowsOnly);
    }
}
