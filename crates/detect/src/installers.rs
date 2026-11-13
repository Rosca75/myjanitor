//! Old installers and disk images (CLN-DET-14): `.msi .exe .msix .iso .img .dmg .deb .rpm .AppImage
//! .pkg` sitting in a Downloads-like folder and older than N days (default 180).
//!
//! "Downloads-like" means some folder on the file's path is named like a downloads folder
//! (English and common localised names, extendable through the `folders` setting): an `.exe`
//! anywhere else is far more likely to be a program than a leftover installer.
//! Portable programs (`.exe`, `.AppImage`) are listed but not pre-selected.
//! Owner: W2-D.

use mj_core::{
    ActionKind, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, PlatformSupport, Result,
    SafetyClass,
};

use crate::common::{self, Ticker};

/// Extensions treated as installers or disk images.
const EXTENSIONS: &[&str] =
    &["msi", "exe", "msix", "msixbundle", "appx", "iso", "img", "dmg", "deb", "rpm", "appimage", "pkg"];
/// Extensions that may be a program someone still runs from there.
const PORTABLE: &[&str] = &["exe", "appimage"];
/// Folder names (lower case) that mean "downloads".
const DOWNLOAD_NAMES: &[&str] = &[
    "downloads",
    "download",
    "telechargements",
    "téléchargements",
    "descargas",
    "transferências",
    "transferencias",
    "scaricati",
    "herunterladen",
];

/// Old installers: Installers and disk images in Downloads
#[derive(Debug, Default, Clone, Copy)]
pub struct OldInstallers;

impl Detector for OldInstallers {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::OldInstallers,
            title: "Old installers".into(),
            description: "Installers and disk images in Downloads".into(),
            category: Category::Clean,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Trash,
            available_actions: vec![
                ActionKind::Trash,
                ActionKind::Quarantine,
                ActionKind::Move,
                ActionKind::PermanentDelete,
            ],
            settings: vec![
                common::number_setting(
                    "min_age_days",
                    "Older than",
                    "Only installers last changed longer ago than this are listed.",
                    180.0,
                    1.0,
                    3650.0,
                    "days",
                ),
                common::text_setting(
                    "folders",
                    "Extra download folder names",
                    "Comma-separated folder names to treat like Downloads.",
                    "",
                    true,
                ),
            ],
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-14".into()],
            unit: "files".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let days = ctx.params.number("min_age_days", 180.0).max(1.0);
        let cutoff = ctx.now.add_days(-(days as i64));
        let mut names: Vec<String> = DOWNLOAD_NAMES.iter().map(|s| s.to_string()).collect();
        names.extend(common::split_list(ctx.params.text("folders", "")));
        let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();

        let mut out = DetectorOutput::default();
        let mut tick = Ticker::new(ctx, ctx.index.len() as u64);
        for e in ctx.index.entries() {
            tick.tick()?;
            if !e.is_file() || !common::usable(e) {
                continue;
            }
            let ext = e.extension();
            if !EXTENSIONS.contains(&ext.as_str()) || !in_downloads(e, &name_refs) {
                continue;
            }
            if !common::before(e.modified, cutoff) {
                out.diagnostics.skip("newer_than_threshold");
                continue;
            }
            let age = e.modified.map(|m| common::human_age(m, ctx.now)).unwrap_or_default();
            let reason = format!(".{ext} installer in Downloads, not touched for {age}");
            tick.found(e.reclaimable());
            let selected = !PORTABLE.contains(&ext.as_str());
            let mut c = common::file_candidate(e, DetectorId::OldInstallers, reason, selected);
            c.columns.insert("type".into(), ext.to_uppercase());
            out.candidates.push(c);
        }
        tick.finish();
        Ok(out)
    }
}

/// True when a folder on the path below the scan root (or the root itself) has a downloads name.
fn in_downloads(e: &mj_core::FileEntry, names: &[&str]) -> bool {
    e.path.parent().is_some_and(|p| {
        p.components().any(|c| {
            let s = c.as_os_str().to_string_lossy().to_lowercase();
            names.iter().any(|n| s == *n)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::*;
    use mj_core::testutil::TempDir;

    #[test]
    fn old_installers_in_downloads_only() {
        let t = TempDir::new("installers").unwrap();
        let old = [
            "Downloads/setup.msi",
            "Downloads/win.ISO",
            "Downloads/tool.exe",
            "Downloads/sub/app.deb",
            "Downloads/Portable.AppImage",
            "Téléchargements/x.pkg",
            "Downloads/readme.txt",
            "Projects/build/installer.exe", // not a downloads folder
        ];
        for f in old {
            t.write(f, b"binary").unwrap();
            age(&t.join(f), 400);
        }
        t.write("Downloads/recent.msi", b"binary").unwrap();
        age(&t.join("Downloads/recent.msi"), 10);
        let out = run(&OldInstallers, &index_of(t.path()), &Default::default());
        assert_eq!(
            rel(&out, t.path()),
            vec![
                "Downloads/Portable.AppImage",
                "Downloads/setup.msi",
                "Downloads/sub/app.deb",
                "Downloads/tool.exe",
                "Downloads/win.ISO",
                "Téléchargements/x.pkg"
            ]
        );
        assert_eq!(out.diagnostics.skipped["newer_than_threshold"], 1);
        let sel = |n: &str| out.candidates.iter().find(|c| c.path.ends_with(n)).unwrap().default_selected;
        assert!(sel("setup.msi") && sel("win.ISO"));
        assert!(!sel("tool.exe") && !sel("Portable.AppImage"));
    }

    #[test]
    fn threshold_and_extra_folders_are_configurable() {
        let t = TempDir::new("installers2").unwrap();
        t.write("Downloads/a.msi", b"x").unwrap();
        age(&t.join("Downloads/a.msi"), 40);
        t.write("Setups/b.msi", b"x").unwrap();
        age(&t.join("Setups/b.msi"), 40);
        let ix = index_of(t.path());
        assert!(run(&OldInstallers, &ix, &Default::default()).candidates.is_empty());
        let out = run(&OldInstallers, &ix, &p(&[("min_age_days", 30.0)]));
        assert_eq!(rel(&out, t.path()), vec!["Downloads/a.msi"]);
        let mut params = p(&[("min_age_days", 30.0)]);
        params.set("folders", mj_core::ParamValue::Text("setups, other".into()));
        assert_eq!(run(&OldInstallers, &ix, &params).candidates.len(), 2);
    }
}
