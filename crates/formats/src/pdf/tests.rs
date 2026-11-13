//! End-to-end tests of the PDF reader, the structural pass and verification
//! on programmatically built fixtures.

use super::document::Document;
use super::fixture::{flate_body, incremental_update, stream_body, Builder};
use super::object::Object;
use super::structural::{optimise, Options, Outcome, SkipReason};
use super::verify::verify;

const CONTENT: &[u8] = b"BT /F1 24 Tf 72 700 Td (Hello, world. Hello, world. Hello, world.) Tj ET\n\
BT /F1 24 Tf 72 650 Td (Hello, world. Hello, world. Hello, world.) Tj ET\n\
BT /F1 24 Tf 72 600 Td (Hello, world. Hello, world. Hello, world.) Tj ET\n";

fn optimised(input: &[u8], opts: &Options) -> Vec<u8> {
    match optimise(input, opts) {
        Outcome::Optimised { bytes, .. } => {
            verify(input, &bytes, opts).expect("output verifies");
            assert!(bytes.len() < input.len());
            bytes
        }
        Outcome::Skipped(r) => panic!("skipped: {r:?}"),
    }
}

fn skipped(input: &[u8]) -> SkipReason {
    match optimise(input, &Options::default()) {
        Outcome::Skipped(r) => r,
        Outcome::Optimised { .. } => panic!("expected a skip"),
    }
}

/// A richer document: two pages, a poorly compressed image, an unreachable object,
/// two identical streams, a JPEG-like DCT stream, a thumbnail and an info dictionary.
fn rich() -> Builder {
    let image: Vec<u8> = (0..20_000u32).map(|i| ((i / 40) % 256) as u8).collect();
    let mut b = Builder::new();
    b.obj(1, "<< /Type /Catalog /Pages 2 0 R /PageMode /UseNone >>")
        .obj(2, "<< /Type /Pages /Kids [3 0 R 10 0 R] /Count 2 >>")
        .obj(3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Thumb 9 0 R /Resources << /Font << /F1 5 0 R >> /XObject << /Im1 6 0 R /Im2 7 0 R /Im3 8 0 R >> >> >>")
        .stream(4, flate_body("", CONTENT))
        .obj(5, "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>")
        .stream(6, flate_body("/Type /XObject /Subtype /Image /Width 100 /Height 200 /ColorSpace /DeviceGray /BitsPerComponent 8", &image))
        .stream(7, flate_body("/Type /XObject /Subtype /Image /Width 100 /Height 200 /ColorSpace /DeviceGray /BitsPerComponent 8", &image))
        .stream(8, stream_body("/Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode", b"\xff\xd8\xff\xe0fake-jpeg-bytes\xff\xd9"))
        .stream(9, stream_body("/Width 4 /Height 4 /ColorSpace /DeviceRGB /BitsPerComponent 8", &[7u8; 48]))
        .obj(10, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents [11 0 R 12 0 R] >>")
        .stream(11, stream_body("", CONTENT))
        .stream(12, stream_body("", b"q 1 0 0 1 0 0 cm Q"))
        .obj(13, "(unreachable garbage that nobody references, unreachable garbage)")
        .obj(14, "<< /Title (A title) /Producer (fixture) >>");
    b
}

const TRAILER: &str =
    "/Root 1 0 R /Info 14 0 R /ID [<00112233445566778899aabbccddeeff> <00112233445566778899aabbccddeeff>]";

#[test]
fn classic_xref_is_optimised_and_verifies() {
    let input = rich().classic(TRAILER);
    let out = optimised(&input, &Options::default());
    let doc = Document::load(&out).unwrap();
    assert!(doc.xref.has_xref_stream, "object streams in use");
    assert_eq!(doc.version, (1, 5));
    assert_eq!(doc.pages().unwrap().len(), 2);
    // The unreachable string, the duplicate image and the thumbnail are gone.
    assert!(!super::document::contains(&out, b"unreachable garbage"));
    let images = doc
        .objects
        .values()
        .filter(|o| o.as_dict().is_some_and(|d| d.get(b"Subtype").is_some_and(|s| s.is_name(b"Image"))))
        .count();
    assert_eq!(images, 2, "duplicate image merged, DCT kept");
    // The DCT stream is byte for byte identical.
    assert!(super::document::contains(&out, b"\xff\xd8\xff\xe0fake-jpeg-bytes\xff\xd9"));
    // /ID and /Info preserved.
    let info = doc.trailer.get(b"Info").map(|i| doc.resolve(i)).and_then(Object::as_dict).unwrap();
    assert_eq!(info.get(b"Title"), Some(&Object::Str(b"A title".to_vec())));
    assert_eq!(doc.trailer.get(b"ID"), Document::load(&input).unwrap().trailer.get(b"ID"));
}

#[test]
fn report_counts_the_work() {
    let input = rich().classic(TRAILER);
    let Outcome::Optimised { report, .. } = optimise(&input, &Options::default()) else { panic!() };
    assert!(report.streams_recompressed >= 2);
    assert!(report.streams_compressed >= 1, "uncompressed content stream compressed");
    assert_eq!(report.streams_merged, 1);
    assert!(report.objects_after < report.objects_before);
    assert!(report.breakdown.images > 0 && report.breakdown.content > 0);
    assert_eq!(report.removed[0].0, "thumbnails");
    assert_eq!(report.removed[0].1, 48);
}

#[test]
fn classic_layout_when_object_streams_off() {
    let input = rich().classic(TRAILER);
    let opts = Options { object_streams: false, ..Options::default() };
    let out = optimised(&input, &opts);
    let doc = Document::load(&out).unwrap();
    assert!(!doc.xref.has_xref_stream);
    assert_eq!(doc.version, (1, 4));
}

#[test]
fn xref_stream_and_object_streams_are_read() {
    for objstm in [false, true] {
        let mut b = rich();
        b.version = "1.5".into();
        let input = b.xref_stream(TRAILER, objstm);
        let doc = Document::load(&input).unwrap();
        assert_eq!(doc.pages().unwrap().len(), 2);
        let out = optimised(&input, &Options::default());
        assert_eq!(Document::load(&out).unwrap().pages().unwrap().len(), 2);
    }
}

#[test]
fn incremental_update_uses_the_newest_objects() {
    let base = Builder::simple_page(CONTENT).classic("/Root 1 0 R");
    let updated = incremental_update(
        &base,
        &[(5, "<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>"), (6, "<< /Title (Updated) >>")],
        7,
        "/Root 1 0 R /Info 6 0 R",
    );
    let doc = Document::load(&updated).unwrap();
    assert_eq!(doc.xref.sections, 2);
    let font = doc.objects.get(&5).and_then(Object::as_dict).unwrap();
    assert!(font.get(b"BaseFont").unwrap().is_name(b"Courier"));
    let out = optimised(&updated, &Options::default());
    let d2 = Document::load(&out).unwrap();
    assert_eq!(d2.xref.sections, 1);
    let fonts: Vec<_> = d2.objects.values().filter_map(Object::as_dict).filter_map(|d| d.get(b"BaseFont")).collect();
    assert_eq!(fonts, vec![&Object::Name(b"Courier".to_vec())], "superseded revision dropped");
}

#[test]
fn hybrid_xref_stream_is_followed() {
    // Classic table lists object 5 as free; the /XRefStm stream holds it.
    let mut b = Builder::simple_page(CONTENT);
    b.objects.retain(|o| o.num != 5);
    let mut file = b.classic("/Root 1 0 R");
    // Append object 5 and an xref stream for it, then a classic table with /XRefStm.
    let off5 = file.len();
    file.extend_from_slice(b"5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman >>\nendobj\n");
    let xs = file.len();
    let mut rows = vec![1u8];
    rows.extend_from_slice(&(off5 as u32).to_be_bytes());
    rows.extend_from_slice(&[0, 0]);
    let body = stream_body("/Type /XRef /Size 6 /Index [5 1] /W [1 4 2]", &rows);
    file.extend_from_slice(b"7 0 obj\n");
    file.extend_from_slice(&body);
    file.extend_from_slice(b"\nendobj\n");
    let prev = super::fixture::last_startxref(&file[..off5]);
    let xref = file.len();
    file.extend_from_slice(format!("xref\n5 1\n0000000000 00000 f \ntrailer\n<< /Size 8 /Root 1 0 R /Prev {prev} /XRefStm {xs} >>\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    let doc = Document::load(&file).unwrap();
    assert!(doc.objects.get(&5).and_then(Object::as_dict).unwrap().get(b"BaseFont").unwrap().is_name(b"Times-Roman"));
    optimised(&file, &Options::default());
}

#[test]
fn indirect_length_is_resolved() {
    let mut b = Builder::new();
    b.obj(1, "<< /Type /Catalog /Pages 2 0 R >>")
        .obj(2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
        .obj(3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] /Contents 4 0 R >>");
    let mut body = b"<< /Length 5 0 R >>stream\n".to_vec();
    body.extend_from_slice(CONTENT);
    body.extend_from_slice(b"\nendstream");
    b.stream(4, body).obj(5, &CONTENT.len().to_string());
    let input = b.classic("/Root 1 0 R");
    let out = optimised(&input, &Options::default());
    // The length object is no longer needed.
    assert_eq!(Document::load(&out).unwrap().objects.values().filter(|o| matches!(o, Object::Int(_))).count(), 0);
}

#[test]
fn encrypted_and_signed_are_refused() {
    let enc = Builder::simple_page(CONTENT).classic("/Root 1 0 R /Encrypt << /Filter /Standard /V 2 /R 3 >>");
    assert_eq!(skipped(&enc), SkipReason::Encrypted);

    let mut b = Builder::simple_page(CONTENT);
    b.obj(6, "<< /Type /Sig /Filter /Adobe.PPKLite /Contents <00> >>");
    b.objects[0].body = b"<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [] /SigFlags 3 /X 6 0 R >> >>".to_vec();
    assert_eq!(skipped(&b.classic("/Root 1 0 R")).code(), "signed");

    let mut b = Builder::simple_page(CONTENT);
    b.obj(6, "<< /Type /Sig /ByteRange [0 10 20 30] >>");
    assert_eq!(skipped(&b.classic("/Root 1 0 R")).code(), "signed", "raw /ByteRange anywhere");

    let mut b = Builder::simple_page(CONTENT);
    b.objects[0].body = b"<< /Type /Catalog /Pages 2 0 R /Perms << /DocMDP 6 0 R >> >>".to_vec();
    b.obj(6, "<< /Type /Foo >>");
    assert_eq!(skipped(&b.classic("/Root 1 0 R")).code(), "signed");
}

#[test]
fn malformed_files_are_skipped_not_panicking() {
    let good = Builder::simple_page(CONTENT).classic("/Root 1 0 R");
    let cases: Vec<Vec<u8>> = vec![
        Vec::new(),
        b"not a pdf at all".to_vec(),
        good[..good.len() / 2].to_vec(),
        good[..good.len() - 10].to_vec(),
        [b"garbage\n".as_slice(), &good].concat(),
        String::from_utf8_lossy(&good).replace("startxref", "startxrex").into_bytes(),
        String::from_utf8_lossy(&good).replace("/Length 2", "/Length 9").into_bytes(),
    ];
    for c in cases {
        assert_eq!(skipped(&c).code(), "unparseable");
    }
}

#[test]
fn wrong_xref_offset_is_refused() {
    let good = Builder::simple_page(CONTENT).classic("/Root 1 0 R");
    let s = String::from_utf8_lossy(&good).into_owned();
    // Shift every object by inserting bytes after the header: offsets now lie.
    let bad = s.replacen("1 0 obj", "\n\n\n1 0 obj", 1).into_bytes();
    assert_eq!(skipped(&bad).code(), "unparseable");
}

#[test]
fn xref_loop_is_detected() {
    let base = Builder::simple_page(CONTENT).classic("/Root 1 0 R");
    let xref = super::fixture::last_startxref(&base);
    // An update whose /Prev points to itself.
    let mut file = base.clone();
    let at = file.len();
    file.extend_from_slice(
        format!(
            "xref\n0 1\n0000000000 65535 f \ntrailer\n<< /Size 6 /Root 1 0 R /Prev {at} >>\nstartxref\n{at}\n%%EOF\n"
        )
        .as_bytes(),
    );
    assert!(Document::load(&file).unwrap_err().to_string().contains("loop"));
    let _ = xref;
}

#[test]
fn deep_nesting_is_refused() {
    let deep = format!("<< /Type /Catalog /Pages 2 0 R /X {}1{} >>", "[".repeat(5000), "]".repeat(5000));
    let mut b = Builder::simple_page(CONTENT);
    b.objects[0].body = deep.into_bytes();
    assert_eq!(skipped(&b.classic("/Root 1 0 R")).code(), "unparseable");
    // A deep (but legal) page tree is walked iteratively.
    let mut b = Builder::new();
    let depth = 200u32;
    b.obj(1, "<< /Type /Catalog /Pages 10 0 R >>");
    for i in 0..depth {
        let n = 10 + i;
        let kid = if i + 1 == depth { 2 } else { n + 1 };
        b.obj(n, &format!("<< /Type /Pages /Kids [{kid} 0 R] /Count 1 >>"));
    }
    b.obj(2, "<< /Type /Page /MediaBox [0 0 10 10] /Contents 3 0 R >>").stream(3, stream_body("", CONTENT));
    let input = b.classic("/Root 1 0 R");
    assert_eq!(Document::load(&input).unwrap().pages().unwrap().len(), 1);
    optimised(&input, &Options::default());
}

#[test]
fn page_tree_cycle_is_refused() {
    let mut b = Builder::simple_page(CONTENT);
    b.objects[1].body = b"<< /Type /Pages /Kids [3 0 R 2 0 R] /Count 1 >>".to_vec();
    assert_eq!(skipped(&b.classic("/Root 1 0 R")).code(), "unparseable");
}

#[test]
fn removals_are_opt_in_and_verified() {
    let mut b = rich();
    b.objects[0].body = b"<< /Type /Catalog /Pages 2 0 R /Metadata 20 0 R /Names << /JavaScript 21 0 R /EmbeddedFiles 22 0 R >> /OpenAction << /S /JavaScript /JS (app.alert(1)) >> /PieceInfo << /Illustrator << /Private 24 0 R >> >> >>".to_vec();
    let xmp =
        b"<?xpacket begin='' id='W5M0MpCehiHzreSzNTczkc9d'?><x:xmpmeta xmlns:x='adobe:ns:meta/'/><?xpacket end='w'?>";
    b.stream(20, stream_body("/Type /Metadata /Subtype /XML", xmp))
        .obj(21, "<< /Names [(js) << /S /JavaScript /JS (1) >>] >>")
        .obj(22, "<< /Names [(a.txt) << /Type /Filespec /F (a.txt) /EF << /F 23 0 R >> >>] >>")
        .stream(23, stream_body("/Type /EmbeddedFile", &[b'a'; 5000]))
        .stream(24, stream_body("", &[b'p'; 3000]));
    let input = b.classic(TRAILER);

    // Default: everything but the thumbnail is kept.
    let out = optimised(&input, &Options::default());
    let doc = Document::load(&out).unwrap();
    let cat = doc.catalog().unwrap();
    for k in [&b"Metadata"[..], b"Names", b"OpenAction", b"PieceInfo"] {
        assert!(cat.contains(k));
    }
    assert!(super::document::contains(&out, b"xpacket"), "XMP left uncompressed");

    let all = Options {
        remove_xmp: true,
        remove_info: true,
        remove_attachments: true,
        remove_javascript: true,
        remove_piece_info: true,
        ..Options::default()
    };
    let Outcome::Optimised { bytes, report } = optimise(&input, &all) else { panic!() };
    verify(&input, &bytes, &all).unwrap();
    let doc = Document::load(&bytes).unwrap();
    let cat = doc.catalog().unwrap();
    assert!(!cat.contains(b"Metadata") && !cat.contains(b"OpenAction") && !cat.contains(b"PieceInfo"));
    assert!(doc.trailer.get(b"Info").is_none());
    let removed: std::collections::HashMap<_, _> = report.removed.iter().copied().collect();
    assert_eq!(removed["attachments"], 5000);
    assert_eq!(removed["piece_info"], 3000);
    assert_eq!(removed["xmp"], xmp.len() as u64);
    // Verifying the stripped output against the default options must fail.
    assert!(verify(&input, &bytes, &Options::default()).is_err());
}

#[test]
fn archival_files_keep_layout_and_metadata() {
    let mut b = rich();
    b.objects[0].body = b"<< /Type /Catalog /Pages 2 0 R /Metadata 20 0 R >>".to_vec();
    b.stream(20, stream_body("/Type /Metadata /Subtype /XML", b"<x:xmpmeta><pdfaid:part>1</pdfaid:part></x:xmpmeta>"));
    let input = b.classic(TRAILER);
    let opts = Options { remove_xmp: true, ..Options::default() };
    let out = optimised(&input, &opts);
    let doc = Document::load(&out).unwrap();
    assert!(!doc.xref.has_xref_stream);
    assert_eq!(doc.version, (1, 4));
    assert!(doc.catalog().unwrap().contains(b"Metadata"));
}

#[test]
fn verification_catches_tampering() {
    let input = rich().classic(TRAILER);
    let opts = Options { object_streams: false, recompress: false, ..Options::default() };
    let Outcome::Optimised { bytes, .. } = optimise(&input, &opts) else { panic!() };
    verify(&input, &bytes, &opts).unwrap();
    // Change one glyph inside the uncompressed content stream of page 2.
    let s = String::from_utf8_lossy(&bytes).into_owned();
    let tampered = s.replacen("q 1 0 0 1 0 0 cm Q", "q 1 0 0 1 0 9 cm Q", 1).into_bytes();
    assert_ne!(tampered, bytes);
    assert!(verify(&input, &tampered, &opts).is_err());
    // Drop a page dictionary key.
    let tampered = s.replacen("/PageMode/UseNone", "/PageMode/UseOutl", 1).into_bytes();
    assert!(verify(&input, &tampered, &opts).is_err());
    // Garbage output.
    assert!(verify(&input, b"%PDF-1.4\n", &opts).is_err());
}

#[test]
fn already_optimal_output_is_not_smaller() {
    let input = rich().classic(TRAILER);
    let out = optimised(&input, &Options::default());
    let Outcome::Optimised { bytes, .. } = optimise(&out, &Options::default()).clone() else {
        // A second pass can legitimately find nothing to gain.
        return;
    };
    assert!(bytes.len() < out.len());
    verify(&out, &bytes, &Options::default()).unwrap();
}

#[test]
fn mutated_files_never_panic() {
    // Deterministic xorshift mutations of a valid file: every outcome is fine
    // except a panic or an output that fails verification.
    let input = rich().xref_stream(TRAILER, true);
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut rnd = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..300 {
        let mut m = input.clone();
        for _ in 0..(1 + rnd() % 4) {
            let i = (rnd() % m.len() as u64) as usize;
            match rnd() % 3 {
                0 => m[i] = rnd() as u8,
                1 => m.truncate(i.max(1)),
                _ => m.insert(i, b"[<("[(rnd() % 3) as usize]),
            }
        }
        if let Outcome::Optimised { bytes, .. } = optimise(&m, &Options::default()) {
            verify(&m, &bytes, &Options::default()).expect("optimised output verifies");
        }
    }
}

/// Real-file corpus (read-only; skipped when absent): every file either is
/// skipped with a reason or produces a smaller output that verifies.
#[test]
fn corpus_outcomes() {
    let dir = std::env::var_os("MJ_PDF_CORPUS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/home/user/pdf-to-markdown/tests/corpus".into());
    let Ok(rd) = std::fs::read_dir(&dir) else {
        eprintln!("PDF corpus not found at {}; skipped", dir.display());
        return;
    };
    let mut paths: Vec<_> =
        rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "pdf")).collect();
    paths.sort();
    for p in paths {
        let input = std::fs::read(&p).unwrap();
        match optimise(&input, &Options::default()) {
            Outcome::Optimised { bytes, report } => {
                let v = verify(&input, &bytes, &Options::default());
                eprintln!(
                    "{:40} {:>8} -> {:>8} {:?} merged={} re={} co={}",
                    p.file_name().unwrap().to_string_lossy(),
                    input.len(),
                    bytes.len(),
                    v,
                    report.streams_merged,
                    report.streams_recompressed,
                    report.streams_compressed
                );
                v.unwrap();
                assert!(bytes.len() < input.len());
            }
            Outcome::Skipped(r) => {
                eprintln!("{:40} {:>8} skipped: {}", p.file_name().unwrap().to_string_lossy(), input.len(), r.message())
            }
        }
    }
}
