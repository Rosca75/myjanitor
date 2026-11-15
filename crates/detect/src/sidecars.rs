//! Orphan sidecar files (`.xmp`, `.aae`, `.thm`, `.lrv` without a parent media file) and RAW+JPEG
//! pairs (CLN-DET-15).
//!
//! A sidecar is "orphan" only when no media file with the same stem exists beside it. Because the
//! scan may have excluded some file types, the folder is listed again on disk before anything is
//! reported, so a parent hidden by an exclusion never makes a live sidecar look orphaned.
//! Owner: W2-D.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use mj_core::params::ChoiceOption;
use mj_core::pathutil::path_key;
use mj_core::{
    ActionKind, Badges, Candidate, CandidateGroup, Category, DetectContext, Detector, DetectorId, DetectorInfo,
    DetectorOutput, FileEntry, GroupId, GroupKind, PlatformSupport, Result, SafetyClass, SettingKind, SettingSpec,
};

use crate::common::{self, Ticker};

const SIDECARS: &[&str] = &["xmp", "aae", "thm", "lrv"];
/// RAW camera formats.
const RAW: &[&str] = &[
    "cr2", "cr3", "crw", "nef", "nrw", "arw", "srf", "sr2", "dng", "orf", "rw2", "raf", "pef", "srw", "x3f", "3fr",
    "erf", "kdc", "mef", "mrw", "raw", "rwl", "iiq",
];
/// Everything a sidecar can belong to.
const MEDIA: &[&str] = &[
    "jpg", "jpeg", "jpe", "png", "heic", "heif", "tif", "tiff", "gif", "bmp", "webp", "avif", "mov", "mp4", "m4v",
    "avi", "mkv", "mts", "m2ts", "3gp", "mpg", "mpeg", "wmv", "psd",
];
/// Folders whose `.xmp` files are Lightroom/Camera Raw presets, not photo sidecars.
const PRESET_DIRS: &[&str] = &["presets", "settings", "cameraraw", "lightroom", "adobe", "develop presets"];

/// Orphan sidecars: Sidecar files whose photo is gone
#[derive(Debug, Default, Clone, Copy)]
pub struct OrphanSidecars;

impl Detector for OrphanSidecars {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::OrphanSidecars,
            title: "Orphan sidecars".into(),
            description: "Sidecar files whose photo is gone".into(),
            category: Category::Clean,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Trash,
            available_actions: vec![ActionKind::Trash, ActionKind::Quarantine],
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-15".into()],
            unit: "files".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let mut out = DetectorOutput::default();
        let mut tick = Ticker::new(ctx, ctx.index.len() as u64);
        let mut listings: HashMap<PathBuf, Option<HashSet<String>>> = HashMap::new();
        for e in ctx.index.entries() {
            tick.tick()?;
            let ext = e.extension();
            if !e.is_file() || !common::usable(e) || !SIDECARS.contains(&ext.as_str()) {
                continue;
            }
            if ext == "xmp" && common::has_component(&e.path, PRESET_DIRS) {
                out.diagnostics.skip("preset_folder");
                continue;
            }
            let Some(dir) = e.path.parent() else { continue };
            let names = listings.entry(dir.to_path_buf()).or_insert_with(|| list_names(dir));
            let Some(names) = names else {
                out.diagnostics.unreadable(dir);
                continue;
            };
            if has_parent(&common::name_lower(e), &ext, names) {
                continue;
            }
            tick.found(e.reclaimable());
            let reason = format!("Sidecar (.{ext}) whose photo or video is no longer in this folder");
            // An orphan .xmp may describe a photo that was merely moved, so only the disposable kinds are pre-selected.
            let mut c = common::file_candidate(e, DetectorId::OrphanSidecars, reason, ext != "xmp");
            c.columns.insert("type".into(), ext.to_uppercase());
            out.candidates.push(c);
        }
        tick.finish();
        Ok(out)
    }
}

/// Lower-case names in a folder as it is on disk right now.
fn list_names(dir: &Path) -> Option<HashSet<String>> {
    let rd = std::fs::read_dir(dir).ok()?;
    let mut set = HashSet::new();
    for item in rd {
        set.insert(item.ok()?.file_name().to_string_lossy().to_lowercase());
    }
    Some(set)
}

/// Does a media file for this sidecar exist among `names`?
fn has_parent(name_lower: &str, ext: &str, names: &HashSet<String>) -> bool {
    let stem = common::stem(name_lower);
    // `IMG_1.JPG.xmp` names its parent in full.
    if names.contains(stem) && stem != name_lower {
        return true;
    }
    let mut stems = vec![stem.to_string()];
    match ext {
        // iPhone edits: IMG_E1234.AAE / IMG_O1234.AAE belong to IMG_1234 (or IMG_E1234 itself).
        "aae" => {
            for p in ["img_e", "img_o"] {
                if let Some(rest) = stem.strip_prefix(p) {
                    stems.push(format!("img_{rest}"));
                }
            }
        }
        // GoPro: GL011234.LRV pairs with GH011234.MP4.
        "lrv" => {
            if let Some(rest) = stem.strip_prefix("gl") {
                stems.push(format!("gh{rest}"));
            }
        }
        _ => {}
    }
    stems.iter().any(|s| MEDIA.iter().chain(RAW).any(|m| names.contains(&format!("{s}.{m}"))))
}

/// The RAW and JPEG entries found for one shot.
type Shot<'a> = (Option<&'a FileEntry>, Option<&'a FileEntry>);

/// RAW + JPEG pairs: Photos kept both as RAW and JPEG
#[derive(Debug, Default, Clone, Copy)]
pub struct RawJpegPairs;

impl Detector for RawJpegPairs {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::RawJpegPairs,
            title: "RAW + JPEG pairs".into(),
            description: "Photos kept both as RAW and JPEG".into(),
            category: Category::Clean,
            safety: SafetyClass::ReviewOnly,
            default_action: ActionKind::None,
            available_actions: vec![ActionKind::None, ActionKind::Trash, ActionKind::Move],
            settings: vec![SettingSpec {
                key: "keep".into(),
                label: "Keep".into(),
                help: Some("Which file of each pair is kept; the other one is listed for review.".into()),
                kind: SettingKind::Choice {
                    default: "raw".into(),
                    options: vec![
                        ChoiceOption {
                            value: "raw".into(),
                            label: "The RAW file".into(),
                            help: Some("List the JPEGs".into()),
                        },
                        ChoiceOption {
                            value: "jpeg".into(),
                            label: "The JPEG".into(),
                            help: Some("List the RAW files".into()),
                        },
                    ],
                },
                advanced: false,
                lossy: false,
                disabled_reason: None,
            }],
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-15".into()],
            unit: "pairs".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let keep_raw = ctx.params.text("keep", "raw") != "jpeg";
        let mut out = DetectorOutput::default();
        let mut tick = Ticker::new(ctx, ctx.index.len() as u64);
        // (folder key, lower-case stem) -> the RAW and JPEG entries of that shot.
        let mut shots: BTreeMap<(String, String), Shot<'_>> = BTreeMap::new();
        for e in ctx.index.entries() {
            tick.tick()?;
            if !e.is_file() || !common::usable(e) {
                continue;
            }
            let ext = e.extension();
            let is_raw = RAW.contains(&ext.as_str());
            let is_jpeg = matches!(ext.as_str(), "jpg" | "jpeg" | "jpe");
            if !is_raw && !is_jpeg {
                continue;
            }
            let Some(dir) = e.path.parent() else { continue };
            let name = common::name_lower(e);
            let slot = shots.entry((path_key(dir), common::stem(&name).to_string())).or_default();
            if is_raw {
                slot.0.get_or_insert(e);
            } else {
                slot.1.get_or_insert(e);
            }
        }
        for (n, (raw, jpeg)) in shots.values().filter_map(|&(r, j)| r.zip(j)).enumerate() {
            let (keeper, extra) = if keep_raw { (raw, jpeg) } else { (jpeg, raw) };
            let gid = GroupId(n as u64 + 1);
            let kept_what = if keep_raw { "RAW" } else { "JPEG" };
            let dropped_what = if keep_raw { "JPEG" } else { "RAW" };
            let mut listed: Candidate = common::file_candidate(
                extra,
                DetectorId::RawJpegPairs,
                format!("{dropped_what} copy of {}", keeper.name()),
                false,
            );
            let mut kept: Candidate = common::file_candidate(
                keeper,
                DetectorId::RawJpegPairs,
                format!("Kept: the {kept_what} of this pair"),
                false,
            );
            kept.badges |= Badges::KEEPER;
            kept.size_after = kept.size;
            for c in [&mut listed, &mut kept] {
                c.group = Some(gid);
            }
            tick.found(listed.saving());
            out.groups.push(CandidateGroup {
                id: gid,
                kind: GroupKind::Pair,
                label: format!("{} (RAW + JPEG)", common::stem(&keeper.name())),
                confidence: 100.0,
                keeper: Some(keeper.path.clone()),
                members: vec![raw.path.clone(), jpeg.path.clone()],
            });
            out.candidates.push(listed);
            out.candidates.push(kept);
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
    fn orphans_are_sidecars_without_a_same_stem_media_file() {
        let t = TempDir::new("sidecars").unwrap();
        for f in [
            "a/IMG_0001.JPG",
            "a/IMG_0001.xmp", // parent exists
            "a/IMG_0002.xmp", // orphan
            "a/IMG_0003.CR2",
            "a/IMG_0003.xmp", // parent is RAW
            "a/IMG_0004.HEIC",
            "a/IMG_O0004.AAE", // iPhone naming: belongs to IMG_0004
            "a/IMG_0005.AAE",  // orphan
            "a/IMG_0006.JPG",
            "a/IMG_0006.JPG.xmp", // full-name convention, parent exists
            "a/IMG_0007.JPG.xmp", // orphan, full-name convention
            "a/MVI_1.MP4",
            "a/MVI_1.THM",
            "a/MVI_2.THM", // orphan
            "a/GH011234.MP4",
            "a/GL011234.LRV",
            "a/GL019999.LRV", // orphan
            "a/readme.txt",
            "Lightroom/Develop Presets/Warm.xmp", // preset, not a sidecar
        ] {
            t.write(f, b"x").unwrap();
        }
        let out = run(&OrphanSidecars, &index_of(t.path()), &Default::default());
        assert_eq!(
            rel(&out, t.path()),
            vec!["a/GL019999.LRV", "a/IMG_0002.xmp", "a/IMG_0005.AAE", "a/IMG_0007.JPG.xmp", "a/MVI_2.THM"]
        );
        let sel = |n: &str| out.candidates.iter().find(|c| c.path.ends_with(n)).unwrap().default_selected;
        assert!(sel("IMG_0005.AAE") && sel("MVI_2.THM") && !sel("IMG_0002.xmp"));
        assert_eq!(out.diagnostics.skipped["preset_folder"], 1);
    }

    #[test]
    fn a_parent_hidden_from_the_index_is_still_found_on_disk() {
        let t = TempDir::new("sidecars_excl").unwrap();
        t.write("p/IMG_1.JPG", b"x").unwrap();
        t.write("p/IMG_1.xmp", b"x").unwrap();
        let full = index_of(t.path());
        // Simulate a scope that excluded *.jpg: the index lacks the parent.
        let entries: Vec<_> = full.entries().iter().filter(|e| e.extension() != "jpg").cloned().collect();
        let ix = mj_core::FileIndex::new(full.roots.clone(), entries, Default::default(), full.scanned_at);
        assert!(run(&OrphanSidecars, &ix, &Default::default()).candidates.is_empty());
    }

    fn pair_fixture() -> TempDir {
        let t = TempDir::new("pairs").unwrap();
        t.write("s/DSC_1.NEF", &[1u8; 100]).unwrap();
        t.write("s/DSC_1.JPG", &[2u8; 10]).unwrap();
        t.write("s/DSC_2.NEF", &[1u8; 100]).unwrap(); // RAW only
        t.write("s/DSC_3.JPG", &[2u8; 10]).unwrap(); // JPEG only
        t.write("o/DSC_1.JPG", &[2u8; 10]).unwrap(); // same stem, other folder: not a pair
        t
    }

    #[test]
    fn raw_jpeg_pairs_follow_the_keep_rule_and_are_review_only() {
        let t = pair_fixture();
        let ix = index_of(t.path());
        let out = run(&RawJpegPairs, &ix, &Default::default());
        assert_eq!(out.groups.len(), 1);
        assert_eq!(out.groups[0].kind, GroupKind::Pair);
        assert!(out.groups[0].keeper.as_ref().unwrap().ends_with("DSC_1.NEF"));
        let listed = out.candidates.iter().find(|c| !c.badges.contains(Badges::KEEPER)).unwrap();
        assert!(listed.path.ends_with("DSC_1.JPG") && listed.size > 0 && listed.size_after == 0);
        assert!(out.candidates.iter().all(|c| !c.default_selected && c.group == Some(GroupId(1))));
        let kept = out.candidates.iter().find(|c| c.badges.contains(Badges::KEEPER)).unwrap();
        assert_eq!(kept.saving(), 0);

        let mut params = mj_core::ParamValues::new();
        params.set("keep", mj_core::ParamValue::Text("jpeg".into()));
        let out = run(&RawJpegPairs, &ix, &params);
        let listed = out.candidates.iter().find(|c| !c.badges.contains(Badges::KEEPER)).unwrap();
        assert!(listed.path.ends_with("DSC_1.NEF"));
        assert!(listed.size >= 100);
    }
}
