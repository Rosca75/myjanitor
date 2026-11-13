//! File-level tests of the PDF optimisers, including the real-file corpus.

use std::path::{Path, PathBuf};

use mj_core::testutil::TempDir;
use mj_core::{
    Availability, CancelToken, DetectContext, EntryKind, FileEntry, FileIndex, NoFingerprintStore, NullProgress,
    OptimiseJob, Optimiser, ParamValue, ParamValues, ScanStats, Timestamp,
};
use mj_formats::pdf::fixture::{flate_body, stream_body, Builder};

use super::*;

const CONTENT: &[u8] =
    b"BT /F1 12 Tf 72 700 Td (The quick brown fox jumps over the lazy dog, again and again.) Tj ET\n\
BT /F1 12 Tf 72 680 Td (The quick brown fox jumps over the lazy dog, again and again.) Tj ET\n";

fn sample() -> Vec<u8> {
    let mut b = Builder::simple_page(CONTENT);
    b.stream(
        6,
        flate_body(
            "/Type /XObject /Subtype /Image /Width 50 /Height 50 /ColorSpace /DeviceGray /BitsPerComponent 8",
            &[9u8; 2500],
        ),
    )
    .obj(7, "(an unreachable leftover from an older save, an unreachable leftover)");
    b.classic("/Root 1 0 R")
}

fn run(src: &Path, dst: &Path, params: &ParamValues) -> OptimiseOutput {
    let cancel = CancelToken::new();
    let job = OptimiseJob {
        src,
        dst,
        params,
        tools: &[],
        temp_dir: src.parent().unwrap(),
        cancel: &cancel,
        progress: &NullProgress,
    };
    PdfLossless.optimise(&job).unwrap()
}

#[test]
fn optimises_to_dst_and_never_touches_src() {
    let t = TempDir::new("pdf-opt").unwrap();
    let input = sample();
    let src = t.write("in.pdf", &input).unwrap();
    let dst = t.join("out.pdf");
    let out = run(&src, &dst, &ParamValues::new());
    assert_eq!(out.skipped, None, "{:?}", out.notes);
    assert!(!out.lossy);
    assert_eq!(out.bytes_before, input.len() as u64);
    assert_eq!(out.bytes_after, std::fs::metadata(&dst).unwrap().len());
    assert!(out.bytes_after < out.bytes_before);
    assert!(out.notes[0].starts_with("breakdown images="));
    assert_eq!(std::fs::read(&src).unwrap(), input, "source unchanged");
    mj_formats::pdf::verify(&input, &std::fs::read(&dst).unwrap(), &Options::default()).unwrap();
}

#[test]
fn existing_dst_is_never_overwritten() {
    let t = TempDir::new("pdf-exists").unwrap();
    let src = t.write("in.pdf", &sample()).unwrap();
    let dst = t.write("out.pdf", b"keep me").unwrap();
    let cancel = CancelToken::new();
    let params = ParamValues::new();
    let job = OptimiseJob {
        src: &src,
        dst: &dst,
        params: &params,
        tools: &[],
        temp_dir: t.path(),
        cancel: &cancel,
        progress: &NullProgress,
    };
    assert!(PdfLossless.optimise(&job).is_err());
    assert_eq!(std::fs::read(&dst).unwrap(), b"keep me");
}

#[test]
fn encrypted_signed_and_broken_are_skipped_without_output() {
    let t = TempDir::new("pdf-skip").unwrap();
    let enc = Builder::simple_page(CONTENT).classic("/Root 1 0 R /Encrypt << /Filter /Standard >>");
    let mut sb = Builder::simple_page(CONTENT);
    sb.obj(6, "<< /Type /Sig /ByteRange [0 1 2 3] /Contents <00> >>");
    let signed = sb.classic("/Root 1 0 R");
    let broken = sample()[..300].to_vec();
    for (name, bytes) in [("enc.pdf", enc), ("signed.pdf", signed), ("broken.pdf", broken)] {
        let src = t.write(name, &bytes).unwrap();
        let dst = t.join(format!("{name}.out"));
        let out = run(&src, &dst, &ParamValues::new());
        assert!(out.skipped.is_some(), "{name}");
        assert_eq!(out.bytes_after, out.bytes_before);
        assert!(!dst.exists(), "{name}: nothing written");
    }
}

#[test]
fn lossy_removal_marks_output_lossy() {
    let t = TempDir::new("pdf-lossy").unwrap();
    let mut b = Builder::simple_page(CONTENT);
    b.obj(6, "<< /Title (Secret project) /Author (Someone) >>");
    let src = t.write("in.pdf", &b.classic("/Root 1 0 R /Info 6 0 R")).unwrap();
    let dst = t.join("out.pdf");
    let out = run(&src, &dst, &ParamValues::new().with("remove_info", ParamValue::Bool(true)));
    assert_eq!(out.skipped, None);
    assert!(out.lossy);
}

#[test]
fn estimate_and_analyse() {
    let t = TempDir::new("pdf-analyse").unwrap();
    let a = t.write("a.pdf", &sample()).unwrap();
    let enc = t.write("enc.pdf", &Builder::simple_page(CONTENT).classic("/Root 1 0 R /Encrypt 9 0 R")).unwrap();
    let txt = t.write("notes.txt", b"hello").unwrap();
    let entry = |p: &PathBuf| FileEntry::new(p.clone(), EntryKind::File, std::fs::metadata(p).unwrap().len());
    let est = PdfLossless.estimate(&entry(&a), &ParamValues::new()).unwrap();
    assert!(est.bytes_after < est.bytes_before && est.skipped == 0);
    assert_eq!(PdfLossless.estimate(&entry(&enc), &ParamValues::new()).unwrap().skipped, 1);
    assert!(!PdfLossless.applies_to(&entry(&txt)));

    let index = FileIndex::new(
        vec![t.path().to_path_buf()],
        vec![entry(&a), entry(&enc), entry(&txt)],
        ScanStats::default(),
        Timestamp(0),
    );
    let cancel = CancelToken::new();
    let params = ParamValues::new();
    let ctx = DetectContext {
        index: &index,
        params: &params,
        now: Timestamp(0),
        fingerprints: &NoFingerprintStore,
        cancel: &cancel,
        progress: &NullProgress,
        threads: 2,
    };
    let out = PdfLossless.analyse(&ctx).unwrap();
    assert_eq!(out.candidates.len(), 1);
    assert_eq!(out.candidates[0].path, a);
    assert_eq!(out.diagnostics.skipped.get("encrypted"), Some(&1));
}

#[test]
fn lossy_pdf_is_visible_but_disabled() {
    let info = PdfLossy.info();
    assert!(info.settings.iter().all(|s| s.disabled_reason.is_some()));
    assert!(matches!(PdfLossy.availability(&[]), Availability::Disabled { .. }));
    let e = FileEntry::new("/x/a.pdf", EntryKind::File, 10);
    assert!(PdfLossy.estimate(&e, &ParamValues::new()).is_err());
}

/// The read-only 34-file corpus of pdf-to-markdown (bytes copied to a temp dir).
/// Every file must be skipped with a reason, or produce a verified, smaller output.
#[test]
fn corpus_end_to_end() {
    let dir = std::env::var_os("MJ_PDF_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/home/user/pdf-to-markdown/tests/corpus".into());
    let Ok(rd) = std::fs::read_dir(&dir) else {
        eprintln!("PDF corpus not found at {}; skipped", dir.display());
        return;
    };
    let mut paths: Vec<PathBuf> =
        rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "pdf")).collect();
    paths.sort();
    let t = TempDir::new("pdf-corpus").unwrap();
    let (mut optimised, mut skipped, mut saved_pct) = (0usize, 0usize, 0f64);
    for (i, p) in paths.iter().enumerate() {
        let bytes = std::fs::read(p).unwrap();
        let src = t.write(format!("{i}.pdf"), &bytes).unwrap();
        let dst = t.join(format!("{i}.out.pdf"));
        let out = run(&src, &dst, &ParamValues::new());
        let name = p.file_name().unwrap().to_string_lossy();
        match &out.skipped {
            Some(reason) => {
                assert!(!reason.is_empty());
                assert!(!dst.exists(), "{name}: skipped but wrote output");
                skipped += 1;
                eprintln!("{name:40} skipped: {reason}");
            }
            None => {
                let after = std::fs::read(&dst).unwrap();
                assert!(out.bytes_after < out.bytes_before, "{name}: not smaller");
                mj_formats::pdf::verify(&bytes, &after, &Options::default()).unwrap();
                optimised += 1;
                let pct = 100.0 * (1.0 - out.bytes_after as f64 / out.bytes_before as f64);
                saved_pct += pct;
                eprintln!("{name:40} {} -> {} (-{pct:.1}%)", out.bytes_before, out.bytes_after);
            }
        }
        assert_eq!(std::fs::read(&src).unwrap(), bytes, "{name}: source unchanged");
    }
    eprintln!(
        "corpus: {optimised} optimised (average saving {:.1}%), {skipped} skipped",
        saved_pct / optimised.max(1) as f64
    );
    assert_eq!(optimised + skipped, paths.len());
}

#[test]
fn stream_body_helper_is_usable() {
    // Guard against fixture drift: an uncompressed body parses.
    let b = stream_body("/X 1", b"abc");
    assert!(b.starts_with(b"<</X 1 /Length 3>>stream\n"));
}
