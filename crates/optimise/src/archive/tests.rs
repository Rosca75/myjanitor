//! Archiving tests on generated trees (only inside `TempDir`s).

use std::collections::BTreeMap;
use std::fs;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use mj_core::hash::{sha256, to_hex};
use mj_core::testutil::TempDir;
use mj_core::{
    Archiver, CancelToken, DetectContext, EntryKind, FileEntry, FileIndex, NoFingerprintStore, NullProgress, Progress,
    ProgressSink, ScanStats, Timestamp,
};
use mj_formats::zip::read::safe_relative_path;
use mj_formats::zip::ZipArchive;

use super::*;

fn noise(n: usize, seed: u64) -> Vec<u8> {
    let mut s = seed | 1;
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s as u8
        })
        .collect()
}

/// `base/proj/...` with nested folders, an empty folder, a Unicode name, text,
/// binary and (on Unix) a symlink.
fn tree(t: &TempDir) -> (PathBuf, PathBuf) {
    let base = t.mkdir("base").unwrap();
    t.write("base/proj/readme.txt", &b"hello archive\n".repeat(2000)).unwrap();
    t.write("base/proj/src/main.rs", b"fn main() { println!(\"hi\"); }\n").unwrap();
    t.write("base/proj/data/blob.bin", &noise(70_000, 7)).unwrap();
    t.write("base/proj/data/r\u{e9}sum\u{e9} \u{65e5}\u{672c}.txt", "caf\u{e9}".as_bytes()).unwrap();
    t.write("base/proj/zero.txt", b"").unwrap();
    t.mkdir("base/proj/empty").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("../outside", base.join("proj/link")).unwrap();
    (base.clone(), base.join("proj"))
}

#[allow(clippy::too_many_arguments)]
fn job<'a>(
    src: &'a [PathBuf],
    base: &'a Path,
    dst: &'a Path,
    fmt: ArchiveFormat,
    level: u8,
    tools: &'a [ExternalTool],
    cancel: &'a CancelToken,
    progress: &'a dyn ProgressSink,
) -> ArchiveJob<'a> {
    ArchiveJob { sources: src, base, dst, format: fmt, level, tools, cancel, progress }
}

/// path -> sha256 of every regular file under `root` (relative, `/`-separated).
fn snapshot(root: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).unwrap().flatten() {
            let md = fs::symlink_metadata(e.path()).unwrap();
            let rel = e.path().strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            if md.is_dir() {
                out.insert(format!("{rel}/"), String::new());
                stack.push(e.path());
            } else if md.is_file() {
                out.insert(rel, to_hex(&sha256(&fs::read(e.path()).unwrap())));
            }
        }
    }
    out
}

fn extract(zip: &Path, to: &Path) {
    let mut z = ZipArchive::new(BufReader::new(fs::File::open(zip).unwrap())).unwrap();
    for i in 0..z.entries().len() {
        let e = z.entries()[i].clone();
        let rel = safe_relative_path(&e.name).expect("safe name");
        let p = to.join(rel);
        if e.is_dir() {
            fs::create_dir_all(&p).unwrap();
        } else {
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, z.read(i, 1 << 30).unwrap()).unwrap();
        }
    }
}

#[test]
fn zip_round_trip_verify_extract_and_index() {
    let t = TempDir::new("arc-zip").unwrap();
    let (base, proj) = tree(&t);
    let dst = t.join("out/proj.zip");
    fs::create_dir_all(dst.parent().unwrap()).unwrap();
    let cancel = CancelToken::new();
    let src = vec![proj.clone()];
    let size =
        Zip7Archiver.create(&job(&src, &base, &dst, ArchiveFormat::Zip, 9, &[], &cancel, &NullProgress)).unwrap();
    assert_eq!(size, fs::metadata(&dst).unwrap().len());
    Zip7Archiver.verify(&dst, &src, &base, &cancel).unwrap();

    // Extract with the in-house reader and compare trees by path and hash.
    let out = t.mkdir("extracted").unwrap();
    extract(&dst, &out);
    let mut expected = snapshot(&base);
    let mut got = snapshot(&out);
    assert!(got.remove(INDEX_NAME).is_some());
    expected.retain(|k, _| !k.ends_with("link"));
    assert_eq!(got, expected);

    // Index: first entry, one row per item, the link listed as not archived; sidecar identical.
    let mut z = ZipArchive::new(BufReader::new(fs::File::open(&dst).unwrap())).unwrap();
    assert_eq!(z.entries()[0].name, INDEX_NAME);
    let idx = String::from_utf8(z.read(0, 1 << 20).unwrap()).unwrap();
    assert!(idx.starts_with("path,size,mtime,sha256,note\n"));
    assert!(idx.contains("proj/readme.txt,28000,"));
    assert!(idx.contains(&to_hex(&sha256(&b"hello archive\n".repeat(2000)))));
    #[cfg(unix)]
    assert!(idx.contains("proj/link,,") && idx.contains("symlink not archived -> ../outside"));
    assert_eq!(fs::read(sidecar_path(&dst)).unwrap(), idx.as_bytes());
    // Times survive (UT extra field present on entries).
    let readme = z.entries().iter().find(|e| e.name == "proj/readme.txt").unwrap();
    assert!(readme.extra.windows(2).any(|w| w == [0x55, 0x54]));
    #[cfg(unix)]
    assert_eq!(skipped_links(&src, &base).unwrap(), vec![proj.join("link")]);
}

#[test]
fn corrupted_or_changed_archive_fails_the_integrity_test() {
    let t = TempDir::new("arc-corrupt").unwrap();
    let (base, proj) = tree(&t);
    let src = vec![proj.clone()];
    let cancel = CancelToken::new();
    for level in [0u8, 9] {
        let dst = t.join(format!("p{level}.zip"));
        Zip7Archiver.create(&job(&src, &base, &dst, ArchiveFormat::Zip, level, &[], &cancel, &NullProgress)).unwrap();
        Zip7Archiver.verify(&dst, &src, &base, &cancel).unwrap();
        // Flip one byte inside the stored/deflated data of the first file after the index.
        let mut bytes = fs::read(&dst).unwrap();
        let z = ZipArchive::new(std::io::Cursor::new(bytes.clone())).unwrap();
        let e = z.entries().iter().find(|e| e.name == "proj/readme.txt").unwrap().clone();
        let at = (e.local_header_offset + 30 + e.name.len() as u64 + 9 + e.compressed_size / 2) as usize;
        bytes[at] ^= 0x40;
        fs::write(&dst, &bytes).unwrap();
        assert!(Zip7Archiver.verify(&dst, &src, &base, &cancel).is_err(), "level {level}");
        // A failed verification removes our sidecar.
        assert!(!sidecar_path(&dst).exists());
    }
    // A source changed after archiving also fails.
    let dst = t.join("changed.zip");
    Zip7Archiver.create(&job(&src, &base, &dst, ArchiveFormat::Zip, 9, &[], &cancel, &NullProgress)).unwrap();
    fs::write(proj.join("src/main.rs"), b"changed").unwrap();
    assert!(Zip7Archiver.verify(&dst, &src, &base, &cancel).is_err());
    // Garbage is not an archive.
    let junk = t.write("junk.zip", b"not an archive").unwrap();
    assert!(Zip7Archiver.verify(&junk, &src, &base, &cancel).is_err());
}

/// Cancels its token at the first progress report (i.e. mid-archive).
struct CancelOnProgress<'a>(&'a CancelToken);
impl ProgressSink for CancelOnProgress<'_> {
    fn report(&self, _: &Progress) {
        self.0.cancel();
    }
}

#[test]
fn cancel_mid_archive_leaves_nothing() {
    let t = TempDir::new("arc-cancel").unwrap();
    let (base, proj) = tree(&t);
    let src = vec![proj];
    let cancel = CancelToken::new();
    let sink = CancelOnProgress(&cancel);
    let dst = t.join("p.zip");
    let r = Zip7Archiver.create(&job(&src, &base, &dst, ArchiveFormat::Zip, 9, &[], &cancel, &sink));
    assert_eq!(r, Err(Error::Cancelled));
    assert!(!dst.exists());
    assert!(!sidecar_path(&dst).exists());
    // Existing destinations are never overwritten.
    let taken = t.write("taken.zip", b"keep").unwrap();
    let c2 = CancelToken::new();
    assert!(Zip7Archiver.create(&job(&src, &base, &taken, ArchiveFormat::Zip, 9, &[], &c2, &NullProgress)).is_err());
    assert_eq!(fs::read(&taken).unwrap(), b"keep");
}

#[test]
fn external_formats_need_their_tools() {
    let t = TempDir::new("arc-tools").unwrap();
    let (base, proj) = tree(&t);
    let src = vec![proj];
    let cancel = CancelToken::new();
    assert_eq!(Zip7Archiver.formats(&[]), vec![ArchiveFormat::Zip]);
    for fmt in [ArchiveFormat::SevenZip, ArchiveFormat::TarZstd] {
        let dst = t.join("x.out");
        let r = Zip7Archiver.create(&job(&src, &base, &dst, fmt, 9, &[], &cancel, &NullProgress));
        assert!(matches!(r, Err(Error::ToolMissing(_))));
        assert!(!dst.exists() && !ext::partial_tar(&dst).exists());
    }
}

#[test]
fn tar_round_trip_and_system_tar_accepts_it() {
    let t = TempDir::new("arc-tar").unwrap();
    let (base, proj) = tree(&t);
    let long = format!("{}/{}.txt", "d".repeat(60), "n".repeat(80));
    t.write(format!("base/proj/{long}"), b"long name").unwrap();
    let cancel = CancelToken::new();
    let p = prepare(std::slice::from_ref(&proj), &base, &cancel).unwrap();
    let dst = t.join("p.tar");
    tar::create(&p.items, &p.hashes, &p.index_bytes, &p.index_name, &dst, &cancel, &NullProgress).unwrap();
    let entries = tar::read(&dst, &p.index_name, &cancel).unwrap();
    check_tar(&entries, &p).unwrap();
    assert!(entries.iter().any(|e| e.name == format!("proj/{long}")));
    // Corruption is detected.
    let mut b = fs::read(&dst).unwrap();
    let pos = b.windows(13).position(|w| w == b"hello archive").unwrap();
    b[pos] ^= 1;
    let bad = t.write("bad.tar", &b).unwrap();
    assert!(tar::read(&bad, &p.index_name, &cancel).and_then(|e| check_tar(&e, &p)).is_err());
    // GNU/BSD tar lists and extracts it identically.
    if let Some(tool) = crate::tools::which("tar") {
        let out = t.mkdir("tar-x").unwrap();
        let o = std::process::Command::new(tool).arg("-xf").arg(&dst).arg("-C").arg(&out).output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let mut got = snapshot(&out);
        got.remove(INDEX_NAME);
        let mut want = snapshot(&base);
        want.retain(|k, _| !k.ends_with("link"));
        assert_eq!(got, want);
    }
}

#[test]
fn unzip_accepts_the_archive_when_present() {
    let Some(unzip) = crate::tools::which("unzip") else { return };
    let t = TempDir::new("arc-unzip").unwrap();
    let (base, proj) = tree(&t);
    let cancel = CancelToken::new();
    for level in [0u8, 1, 9] {
        let dst = t.join(format!("p{level}.zip"));
        Zip7Archiver
            .create(&job(
                std::slice::from_ref(&proj),
                &base,
                &dst,
                ArchiveFormat::Zip,
                level,
                &[],
                &cancel,
                &NullProgress,
            ))
            .unwrap();
        let o = std::process::Command::new(&unzip).arg("-t").arg(&dst).output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stdout));
    }
}

#[test]
fn external_tools_integration_when_present() {
    let tools = crate::tools::detect(&Default::default());
    let t = TempDir::new("arc-ext").unwrap();
    let (base, proj) = tree(&t);
    let src = vec![proj];
    let cancel = CancelToken::new();
    for (id, fmt, name) in [("7z", ArchiveFormat::SevenZip, "p.7z"), ("zstd", ArchiveFormat::TarZstd, "p.tar.zst")] {
        if mj_core::tools::find(&tools, id).is_none() {
            eprintln!("{id} not found: integration test skipped");
            continue;
        }
        let dst = t.join(name);
        Zip7Archiver.create(&job(&src, &base, &dst, fmt, 5, &tools, &cancel, &NullProgress)).unwrap();
        Zip7Archiver.verify(&dst, &src, &base, &cancel).unwrap();
    }
}

#[test]
fn estimates_are_sane() {
    let t = TempDir::new("arc-est").unwrap();
    t.write("text/a.txt", &b"the same line again and again\n".repeat(10_000)).unwrap();
    t.write("rand/a.bin", &noise(300_000, 3)).unwrap();
    t.write("rand/b.jpg", &noise(100_000, 5)).unwrap();
    let text = folders::estimate_folder(&t.join("text"));
    assert!(text.bytes_after < text.bytes_before / 5, "{text:?}");
    let rand = folders::estimate_folder(&t.join("rand"));
    assert!(rand.bytes_after > rand.bytes_before * 95 / 100, "{rand:?}");
    // The estimate is close to a real archive for the text folder.
    let cancel = CancelToken::new();
    let dst = t.join("text.zip");
    let real = Zip7Archiver
        .create(&job(&[t.join("text")], t.path(), &dst, ArchiveFormat::Zip, 9, &[], &cancel, &NullProgress))
        .unwrap();
    assert!(real < text.bytes_before / 5);
    assert_eq!(level_from(&ParamValues::new()), 9);
}

#[test]
fn cold_folders_are_found_topmost_only() {
    let t = TempDir::new("arc-cold").unwrap();
    let now = Timestamp::from_ymd_hms(2026, 6, 1, 0, 0, 0);
    let old = Timestamp::from_ymd_hms(2024, 1, 1, 0, 0, 0);
    let root = t.path().to_path_buf();
    let f = |rel: &str, size: u64, m: Timestamp, depth: u32| {
        let p = root.join(rel);
        if size > 0 {
            t.write(rel, &b"x".repeat(size as usize)).unwrap();
        }
        let mut e = FileEntry::new(p, if size > 0 { EntryKind::File } else { EntryKind::Dir }, size);
        e.modified = Some(m);
        e.depth = depth;
        e
    };
    let entries = vec![
        f("cold", 0, old, 1),
        f("cold/sub", 0, old, 2),
        f("cold/sub/a.txt", 50_000, old, 3),
        f("warm", 0, old, 1),
        f("warm/b.txt", 50_000, now, 2),
    ];
    let index = FileIndex::new(vec![root.clone()], entries, ScanStats::default(), now);
    let cancel = CancelToken::new();
    let params = ParamValues::new().with("min_size_mib", mj_core::ParamValue::Number(0.0));
    let ctx = DetectContext {
        index: &index,
        params: &params,
        now,
        fingerprints: &NoFingerprintStore,
        cancel: &cancel,
        progress: &NullProgress,
        threads: 1,
    };
    let out = ArchiveFolders.analyse(&ctx).unwrap();
    assert_eq!(out.candidates.len(), 1);
    assert_eq!(out.candidates[0].path, root.join("cold"));
    assert_eq!(out.candidates[0].target, mj_core::TargetKind::Folder);
    assert!(out.candidates[0].size_after < out.candidates[0].size);
}

#[test]
fn recompress_zip_verifies_and_shrinks() {
    let t = TempDir::new("arc-recompress").unwrap();
    // A ZIP with stored text entries (plenty to gain).
    let mut w = mj_formats::zip::ZipWriter::new(Vec::new());
    w.add("a.txt", &b"recompress me please\n".repeat(5000), 0, 0, 33).unwrap();
    w.add("b/c.csv", &b"1,2,3\n".repeat(3000), 1, 0, 33).unwrap();
    let src = t.write("in.zip", &w.finish().unwrap()).unwrap();
    let dst = t.join("out.zip");
    let cancel = CancelToken::new();
    let params = ParamValues::new();
    let j = OptimiseJob {
        src: &src,
        dst: &dst,
        params: &params,
        tools: &[],
        temp_dir: t.path(),
        cancel: &cancel,
        progress: &NullProgress,
    };
    let out = ArchiveRecompress.optimise(&j).unwrap();
    assert_eq!(out.skipped, None, "{:?}", out);
    assert!(out.bytes_after < out.bytes_before);
    let caps = mj_formats::zip::write::repack::Caps::default();
    assert_eq!(
        recompress::content_digest(&src, caps, &cancel).unwrap(),
        recompress::content_digest(&dst, caps, &cancel).unwrap()
    );
    let e = FileEntry::new(src.clone(), EntryKind::File, out.bytes_before);
    assert!(ArchiveRecompress.estimate(&e, &params).unwrap().bytes_after < out.bytes_before);
    // A second pass on the result finds nothing to gain and writes nothing.
    let dst2 = t.join("out2.zip");
    let j2 = OptimiseJob {
        src: &dst,
        dst: &dst2,
        params: &params,
        tools: &[],
        temp_dir: t.path(),
        cancel: &cancel,
        progress: &NullProgress,
    };
    let again = ArchiveRecompress.optimise(&j2).unwrap();
    assert!(again.skipped.is_some());
    assert!(!dst2.exists());
    // Not a ZIP: skipped.
    let docx = t.write("x.zip", b"plain text").unwrap();
    let j3 = OptimiseJob {
        src: &docx,
        dst: &t.join("o3.zip"),
        params: &params,
        tools: &[],
        temp_dir: t.path(),
        cancel: &cancel,
        progress: &NullProgress,
    };
    assert!(ArchiveRecompress.optimise(&j3).unwrap().skipped.is_some());
}

#[test]
fn settings_show_disabled_reasons() {
    let s = folder_settings(&[]);
    assert_eq!(s.iter().find(|x| x.key == "encrypt").unwrap().disabled_reason.as_deref(), Some(ENCRYPTION_REASON));
    assert_eq!(s.iter().find(|x| x.key == "split").unwrap().disabled_reason.as_deref(), Some(SPLIT_REASON));
    assert!(s[0].help.as_deref().unwrap().contains("Requires 7z and zstd"));
    assert!(ArchiveRecompress.info().settings[0].disabled_reason.is_some());
}
