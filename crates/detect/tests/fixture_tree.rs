//! Integration test: one generated fixture tree, every detector of the crate run through the
//! public `detectors()` list, plus cross-cutting guarantees (read-only, cancel, live progress).

#[path = "../src/testkit.rs"]
mod testkit;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

use mj_core::testutil::{snapshot, TempDir};
use mj_core::{
    Attributes, CancelToken, DetectContext, DetectorId, DetectorOutput, Error, FileIndex, NoFingerprintStore,
    ParamValues, Progress, ProgressSink, Timestamp,
};
use testkit::{age, age_dir, index_of, rel};

/// Minimal files that pass the structural checks.
fn good_png() -> Vec<u8> {
    let mut v = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0];
    v.extend_from_slice(b"IEND\xae\x42\x60\x82");
    v
}

fn good_jpeg() -> Vec<u8> {
    vec![0xff, 0xd8, 0xff, 0xe0, 0, 2, 0xff, 0xd9]
}

/// Build the tree. Every detector has at least one positive and one negative case in here.
fn build(t: &TempDir) {
    let w = |p: &str, c: &[u8]| {
        t.write(p, c).unwrap();
    };
    // CLN-DET-06 empty files and folders.
    w("empties/zero.txt", b"");
    t.mkdir("empties/tree/a/b").unwrap();
    // CLN-DET-07 zeroed.
    w("zeroed/lost.dat", &vec![0u8; 8192]);
    w("zeroed/real.dat", b"data");
    // CLN-DET-08 broken.
    w("broken/cut.png", &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3]);
    w("broken/ok.pdf", b"%PDF-1.4\nbody\n%%EOF\n");
    w("broken/cut.pdf", b"%PDF-1.4\nbody");
    // CLN-DET-10 temp.
    w("temp/old.tmp", b"x");
    w("temp/fresh.tmp", b"x");
    // CLN-DET-11 caches (under a fake profile).
    w("home/u/AppData/Local/Google/Chrome/User Data/Default/Cache/f_1", b"cache");
    w("home/u/.cache/thumbnails/large/a.png", &good_png());
    // CLN-DET-12 office (Windows layout, exercised through `scan` on this platform).
    w("home/u/AppData/Roaming/Microsoft/Teams/Cache/data_0", b"teams");
    // CLN-DET-13 dev.
    w("proj/rs/Cargo.toml", b"[package]");
    w("proj/rs/target/debug/app", b"bin");
    w("proj/other/target/x", b"not a cargo target");
    w("proj/web/package.json", b"{}");
    w("proj/web/node_modules/p/index.js", b"js");
    // CLN-DET-14 installers.
    w("Downloads/setup.msi", b"msi");
    // CLN-DET-15 sidecars and pairs.
    w("photos/IMG_1.JPG", &good_jpeg());
    w("photos/IMG_1.CR2", b"rawraw");
    w("photos/IMG_2.xmp", b"orphan");
    w("photos/IMG_3.AAE", b"orphan");
    // CLN-DET-16 language packs.
    w("app/locale/de/LC_MESSAGES/a.mo", b"de");
    w("app/locale/en/LC_MESSAGES/a.mo", b"en");
    // CLN-DET-17 stale and CLN-DET-21 inactive.
    w("Archive2023/doc.txt", b"old");
    // OPT-FNT-02.
    w("home/u/.cache/fontconfig/x.cache-7", b"fc");

    for f in ["temp/old.tmp", "zeroed/lost.dat", "zeroed/real.dat", "Downloads/setup.msi", "Archive2023/doc.txt"] {
        age(&t.join(f), if f.starts_with("Archive") { 900 } else { 400 });
    }
    age(&t.join("Downloads/setup.msi"), 400);
}

fn params_for(id: DetectorId) -> ParamValues {
    use mj_core::ParamValue::{Number, Text};
    let mut p = ParamValues::new();
    match id {
        DetectorId::StaleFiles => p.set("min_size_mb", Number(0.0)),
        DetectorId::LanguagePacks => p.set("kept_languages", Text("en".into())),
        _ => {}
    }
    p
}

fn run_all(ix: &FileIndex) -> BTreeMap<DetectorId, DetectorOutput> {
    let cancel = CancelToken::new();
    let mut out = BTreeMap::new();
    for d in mj_detect::detectors() {
        let info = d.info();
        let params = mj_core::params::defaults(&info.settings).merged(&params_for(info.id));
        let ctx = DetectContext {
            index: ix,
            params: &params,
            now: Timestamp::now(),
            fingerprints: &NoFingerprintStore,
            cancel: &cancel,
            progress: &mj_core::NullProgress,
            threads: 2,
        };
        // The Windows-only Office detector returns nothing on Linux; run its platform-independent core.
        let res = if info.id == DetectorId::OfficeLeftovers {
            mj_detect::office::OfficeLeftovers.scan(&ctx)
        } else {
            d.run(&ctx)
        };
        out.insert(info.id, res.unwrap_or_else(|e| panic!("{:?} failed: {e}", info.id)));
    }
    out
}

#[test]
fn every_detector_finds_its_cases_on_the_fixture_tree() {
    let t = TempDir::new("fixture_tree").unwrap();
    build(&t);
    age_dir(&t.join("empties/tree"), 1);
    let ix = index_of(t.path());
    let before = snapshot(t.path()).unwrap();
    let all = run_all(&ix);
    let r = |id| rel(&all[&id], t.path());

    assert_eq!(r(DetectorId::EmptyFiles), vec!["empties/zero.txt"]);
    assert_eq!(r(DetectorId::EmptyFolders), vec!["empties/tree"]);
    assert_eq!(r(DetectorId::ZeroedFiles), vec!["zeroed/lost.dat"]);
    assert_eq!(r(DetectorId::BrokenFiles), vec!["broken/cut.pdf", "broken/cut.png"]);
    assert_eq!(r(DetectorId::BrokenLinks), Vec::<String>::new());
    assert_eq!(r(DetectorId::TempFiles), vec!["temp/old.tmp"]);
    let caches = r(DetectorId::AppCaches);
    assert!(caches.contains(&"home/u/AppData/Local/Google/Chrome/User Data/Default/Cache".to_string()), "{caches:?}");
    assert!(caches.contains(&"home/u/.cache/thumbnails".to_string()));
    assert_eq!(r(DetectorId::OfficeLeftovers), vec!["home/u/AppData/Roaming/Microsoft/Teams/Cache"]);
    assert_eq!(r(DetectorId::DevArtefacts), vec!["proj/rs/target", "proj/web/node_modules"]);
    assert_eq!(r(DetectorId::OldInstallers), vec!["Downloads/setup.msi"]);
    assert_eq!(r(DetectorId::OrphanSidecars), vec!["photos/IMG_2.xmp", "photos/IMG_3.AAE"]);
    assert_eq!(r(DetectorId::RawJpegPairs), vec!["photos/IMG_1.CR2", "photos/IMG_1.JPG"]);
    assert_eq!(r(DetectorId::LanguagePacks), vec!["app/locale/de"]);
    assert!(r(DetectorId::StaleFiles).contains(&"Archive2023/doc.txt".to_string()));
    assert!(r(DetectorId::InactiveFolders).contains(&"Archive2023".to_string()));
    assert_eq!(r(DetectorId::UnusedFonts), vec!["home/u/.cache/fontconfig"]);

    // Detectors are read-only: the tree is byte-for-byte what it was.
    assert_eq!(snapshot(t.path()).unwrap(), before);
}

#[test]
fn candidates_respect_the_contract() {
    let t = TempDir::new("fixture_contract").unwrap();
    build(&t);
    let ix = index_of(t.path());
    for (id, out) in run_all(&ix) {
        let review_only = mj_detect::detectors().iter().find(|d| d.info().id == id).unwrap().info().safety
            == mj_core::SafetyClass::ReviewOnly;
        for c in &out.candidates {
            assert!(c.path.starts_with(t.path()), "{id:?}: {:?} is outside the scan root", c.path);
            assert_ne!(c.path, t.path(), "{id:?}: the scan root itself must never be a candidate");
            assert!(!c.reason.is_empty(), "{id:?}: every candidate needs a one-line reason");
            assert!(!c.reason.contains('\n'));
            assert_eq!(c.source, mj_core::Source::Detector(id));
            assert!(!c.badges.contains(mj_core::Badges::ONLINE_ONLY));
            if review_only {
                assert!(!c.default_selected, "{id:?}: review-only candidates are never pre-selected");
            }
        }
        // Topmost: no candidate lies inside another folder candidate of the same detector.
        let folders: Vec<&Path> = out
            .candidates
            .iter()
            .filter(|c| c.target == mj_core::TargetKind::Folder)
            .map(|c| c.path.as_path())
            .collect();
        for c in &out.candidates {
            assert!(
                !folders.iter().any(|f| c.path != *f && c.path.starts_with(f)),
                "{id:?}: nested candidate {:?}",
                c.path
            );
        }
    }
}

#[test]
fn online_only_placeholders_are_never_returned() {
    let t = TempDir::new("fixture_online").unwrap();
    build(&t);
    let full = index_of(t.path());
    let entries: Vec<_> = full
        .entries()
        .iter()
        .cloned()
        .map(|mut e| {
            if e.is_file() {
                e.attrs |= Attributes::ONLINE_ONLY;
            }
            e
        })
        .collect();
    let ix = FileIndex::new(full.roots.clone(), entries, Default::default(), full.scanned_at);
    for (id, out) in run_all(&ix) {
        let files: Vec<_> = out.candidates.iter().filter(|c| c.target == mj_core::TargetKind::File).collect();
        assert!(
            files.is_empty(),
            "{id:?} returned online-only files: {:?}",
            files.iter().map(|c| &c.path).collect::<Vec<_>>()
        );
    }
}

#[test]
fn cancelled_token_stops_every_detector() {
    let t = TempDir::new("fixture_cancel").unwrap();
    build(&t);
    let ix = index_of(t.path());
    let cancel = CancelToken::new();
    cancel.cancel();
    for d in mj_detect::detectors() {
        let info = d.info();
        if !info.platform.here() {
            continue; // Windows-only detectors return early off Windows
        }
        let params = mj_core::params::defaults(&info.settings);
        let ctx = DetectContext {
            index: &ix,
            params: &params,
            now: Timestamp::now(),
            fingerprints: &NoFingerprintStore,
            cancel: &cancel,
            progress: &mj_core::NullProgress,
            threads: 2,
        };
        assert_eq!(d.run(&ctx).unwrap_err(), Error::Cancelled, "{:?} ignored the cancel token", info.id);
    }
}

#[derive(Default)]
struct Recorder(Mutex<Vec<Progress>>);

impl ProgressSink for Recorder {
    fn report(&self, p: &Progress) {
        self.0.lock().unwrap().push(p.clone());
    }
}

#[test]
fn progress_carries_live_running_totals() {
    let t = TempDir::new("fixture_progress").unwrap();
    build(&t);
    // Enough entries to cross the periodic reporting interval.
    for i in 0..700 {
        t.write(format!("bulk/f{i}.txt"), b"x").unwrap();
    }
    t.write("bulk/empty.txt", b"").unwrap();
    let ix = index_of(t.path());
    let rec = Recorder::default();
    let params = ParamValues::new();
    let cancel = CancelToken::new();
    let ctx = DetectContext {
        index: &ix,
        params: &params,
        now: Timestamp::now(),
        fingerprints: &NoFingerprintStore,
        cancel: &cancel,
        progress: &rec,
        threads: 2,
    };
    let out = mj_detect::empty::EmptyFiles.run_for_test(&ctx);
    let seen = rec.0.lock().unwrap();
    assert!(seen.len() >= 2, "periodic and final reports expected, got {}", seen.len());
    assert!(seen.windows(2).all(|w| w[0].done <= w[1].done && w[0].bytes <= w[1].bytes), "totals only grow");
    let last = seen.last().unwrap();
    assert!(last.is_final());
    assert_eq!(last.bytes, out.candidates.iter().map(|c| c.saving()).sum::<u64>());
}

trait RunForTest {
    fn run_for_test(&self, ctx: &DetectContext<'_>) -> DetectorOutput;
}

impl<D: mj_core::Detector> RunForTest for D {
    fn run_for_test(&self, ctx: &DetectContext<'_>) -> DetectorOutput {
        self.run(ctx).unwrap()
    }
}
