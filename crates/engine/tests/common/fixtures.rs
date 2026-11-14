//! Generated optimiser fixtures (PNG, JPEG, EPUB, PPTX, PDF, WAV). Every file is built
//! in memory with the in-house writers so the tests need no binary fixture and no tool.
#![allow(dead_code)]

use mj_formats::exif::build::{app1, tiff, Value};
use mj_formats::pdf::fixture::{flate_body, Builder};
use mj_formats::png::build::{chunk, rgb_png};
use mj_formats::zip::ZipWriter;

/// A PNG stored at a weak deflate level with a text chunk: the lossless pass gains.
pub fn png() -> Vec<u8> {
    rgb_png(160, 120, &[chunk(b"tEXt", b"Comment\0generated fixture"), chunk(b"gAMA", &[0, 0, 0xB1, 0x8F])], 3)
}

fn seg(marker: u8, body: &[u8]) -> Vec<u8> {
    let mut v = vec![0xFF, marker];
    v.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
    v.extend_from_slice(body);
    v
}

/// A JPEG with EXIF (orientation, GPS) and a 3 KB embedded thumbnail; the default policy
/// (`strip_thumbnail`) removes the thumbnail and keeps the scan bytes.
pub fn jpeg() -> Vec<u8> {
    let thumb = [vec![0xFF, 0xD8, 0xFF, 0xDB, 0, 4, 1, 2], vec![9; 3000], vec![0xFF, 0xD9]].concat();
    let t = tiff(
        &[
            (0x0112, Value::Short(6)),
            (
                0x8825,
                Value::SubIfd(vec![(1, Value::Ascii("N".into())), (2, Value::Rational(vec![(1, 1), (2, 1), (3, 1)]))]),
            ),
        ],
        Some(&[(0, Value::Thumbnail(thumb))]),
    );
    let mut f = vec![0xFF, 0xD8];
    f.extend(app1(&t));
    f.extend(seg(0xDB, &[0; 65]));
    f.extend(seg(0xC0, &[8, 0, 8, 0, 8, 1, 1, 0x11, 0]));
    f.extend(seg(0xC4, &[0; 20]));
    f.extend(seg(0xDA, &[1, 1, 0, 0, 63, 0]));
    f.extend(vec![0x55u8; 9000]);
    f.extend([0xFF, 0xD9]);
    f
}

/// Prose-like XHTML from a pseudo-random word stream (compresses like real text, not like
/// one repeated line, so deflate levels make a measurable difference).
pub fn prose(seed: u64, words: usize) -> String {
    const VOCAB: [&str; 24] = [
        "lorem",
        "ipsum",
        "dolor",
        "sit",
        "amet",
        "consectetur",
        "adipiscing",
        "elit",
        "sed",
        "do",
        "eiusmod",
        "tempor",
        "incididunt",
        "ut",
        "labore",
        "et",
        "dolore",
        "magna",
        "aliqua",
        "enim",
        "minim",
        "veniam",
        "quis",
        "nostrud",
    ];
    let r = super::bytes(seed, words * 2);
    let mut s = String::from("<html><body><p>");
    for i in 0..words {
        s.push_str(VOCAB[(r[2 * i] as usize) % VOCAB.len()]);
        s.push(if r[2 * i + 1].is_multiple_of(13) { '.' } else { ' ' });
        if r[2 * i + 1].is_multiple_of(97) {
            s.push_str("</p>\n<p>");
        }
    }
    s.push_str("</p></body></html>\n");
    s
}

/// An EPUB whose text entries are deflated at level 1 (a repack at the maximum level gains).
pub fn epub() -> Vec<u8> {
    let body = prose(11, 40_000);
    let mut w = ZipWriter::new(Vec::new());
    w.add("mimetype", b"application/epub+zip", 0, 0x6000, 0x5821).unwrap();
    w.add(
        "META-INF/container.xml",
        br#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#,
        1,
        0x6000,
        0x5821,
    )
    .unwrap();
    w.add(
        "OEBPS/content.opf",
        br#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf"/>"#,
        1,
        0x6000,
        0x5821,
    )
    .unwrap();
    w.add("OEBPS/ch1.xhtml", body.as_bytes(), 1, 0x6000, 0x5821).unwrap();
    w.add("OEBPS/ch2.xhtml", prose(12, 40_000).as_bytes(), 1, 0x6000, 0x5821).unwrap();
    w.finish().unwrap()
}

const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PKG_R: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";

fn rels(list: &[(&str, &str, &str)]) -> Vec<u8> {
    let mut s = format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="{PKG_R}">"#);
    for (id, kind, target) in list {
        let ty = if kind.starts_with("http") { kind.to_string() } else { format!("{R}/{kind}") };
        s.push_str(&format!(r#"<Relationship Id="{id}" Type="{ty}" Target="{target}"/>"#));
    }
    s.push_str("</Relationships>");
    s.into_bytes()
}

fn noise(n: usize, seed: u64) -> Vec<u8> {
    super::bytes(seed, n)
}

/// A two-slide deck whose slides use byte-identical pictures, with a thumbnail. With
/// `labelled` the package carries an MSIP sensitivity label in `docProps/custom.xml`.
pub fn pptx(labelled: bool) -> Vec<u8> {
    let ct = |p: &str| format!("application/vnd.openxmlformats-officedocument.presentationml.{p}+xml");
    let mut types = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Default Extension="jpeg" ContentType="image/jpeg"/>"#,
    );
    let mut overrides = vec![
        ("ppt/presentation.xml".to_string(), ct("presentation.main")),
        ("ppt/slideMasters/slideMaster1.xml".into(), ct("slideMaster")),
        ("ppt/slideLayouts/slideLayout1.xml".into(), ct("slideLayout")),
        ("ppt/slides/slide1.xml".into(), ct("slide")),
        ("ppt/slides/slide2.xml".into(), ct("slide")),
        ("ppt/theme/theme1.xml".into(), "application/vnd.openxmlformats-officedocument.theme+xml".into()),
        ("docProps/core.xml".into(), "application/vnd.openxmlformats-package.core-properties+xml".into()),
        ("docProps/app.xml".into(), "application/vnd.openxmlformats-officedocument.extended-properties+xml".into()),
    ];
    if labelled {
        overrides.push((
            "docProps/custom.xml".into(),
            "application/vnd.openxmlformats-officedocument.custom-properties+xml".into(),
        ));
    }
    for (p, t) in &overrides {
        types.push_str(&format!(r#"<Override PartName="/{p}" ContentType="{t}"/>"#));
    }
    types.push_str("</Types>");
    let mut root = vec![
        ("rId1", "officeDocument", "ppt/presentation.xml"),
        (
            "rId2",
            "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties",
            "docProps/core.xml",
        ),
        ("rId3", "extended-properties", "docProps/app.xml"),
        (
            "rId4",
            "http://schemas.openxmlformats.org/package/2006/relationships/metadata/thumbnail",
            "docProps/thumbnail.jpeg",
        ),
    ];
    if labelled {
        root.push(("rId5", "custom-properties", "docProps/custom.xml"));
    }
    let pres = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><p:presentation xmlns:a="{A}" xmlns:r="{R}" xmlns:p="{P}"><p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst><p:sldIdLst><p:sldId id="256" r:id="rId2"/><p:sldId id="257" r:id="rId3"/></p:sldIdLst><p:sldSz cx="9144000" cy="6858000"/><p:notesSz cx="6858000" cy="9144000"/></p:presentation>"#
    );
    let slide = format!(
        r#"<?xml version="1.0"?><p:sld xmlns:a="{A}" xmlns:r="{R}" xmlns:p="{P}"><p:cSld><p:spTree><p:pic><p:blipFill><a:blip r:embed="rId2"/></p:blipFill></p:pic></p:spTree></p:cSld></p:sld>"#
    );
    let img = noise(30_000, 7);
    let mut entries: Vec<(&str, Vec<u8>)> = vec![
        ("[Content_Types].xml", types.into_bytes()),
        ("_rels/.rels", rels(&root)),
        ("ppt/presentation.xml", pres.into_bytes()),
        (
            "ppt/_rels/presentation.xml.rels",
            rels(&[
                ("rId1", "slideMaster", "slideMasters/slideMaster1.xml"),
                ("rId2", "slide", "slides/slide1.xml"),
                ("rId3", "slide", "slides/slide2.xml"),
                ("rId4", "theme", "theme/theme1.xml"),
            ]),
        ),
        (
            "ppt/slideMasters/slideMaster1.xml",
            format!(r#"<?xml version="1.0"?><p:sldMaster xmlns:a="{A}" xmlns:r="{R}" xmlns:p="{P}"><p:cSld><p:spTree/></p:cSld><p:sldLayoutIdLst><p:sldLayoutId id="2147483649" r:id="rId1"/></p:sldLayoutIdLst></p:sldMaster>"#).into_bytes(),
        ),
        (
            "ppt/slideMasters/_rels/slideMaster1.xml.rels",
            rels(&[("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml"), ("rId2", "theme", "../theme/theme1.xml")]),
        ),
        (
            "ppt/slideLayouts/slideLayout1.xml",
            format!(r#"<?xml version="1.0"?><p:sldLayout xmlns:a="{A}" xmlns:r="{R}" xmlns:p="{P}"><p:cSld name="L"><p:spTree/></p:cSld></p:sldLayout>"#).into_bytes(),
        ),
        ("ppt/slideLayouts/_rels/slideLayout1.xml.rels", rels(&[("rId1", "slideMaster", "../slideMasters/slideMaster1.xml")])),
        ("ppt/slides/slide1.xml", slide.clone().into_bytes()),
        (
            "ppt/slides/_rels/slide1.xml.rels",
            rels(&[("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml"), ("rId2", "image", "../media/image1.png")]),
        ),
        ("ppt/slides/slide2.xml", slide.into_bytes()),
        (
            "ppt/slides/_rels/slide2.xml.rels",
            rels(&[("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml"), ("rId2", "image", "../media/image2.png")]),
        ),
        ("ppt/theme/theme1.xml", format!(r#"<?xml version="1.0"?><a:theme xmlns:a="{A}" name="T"><a:themeElements/></a:theme>"#).into_bytes()),
        ("ppt/media/image1.png", img.clone()),
        ("ppt/media/image2.png", img),
        ("docProps/core.xml", br#"<?xml version="1.0"?><cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Fixture</dc:title></cp:coreProperties>"#.to_vec()),
        ("docProps/app.xml", br#"<?xml version="1.0"?><Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"><Application>Fixture</Application></Properties>"#.to_vec()),
        ("docProps/thumbnail.jpeg", noise(8000, 99)),
    ];
    if labelled {
        entries.push(("docProps/custom.xml", br#"<?xml version="1.0"?><Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/custom-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="2" name="MSIP_Label_0e9d_Enabled"><vt:lpwstr>true</vt:lpwstr></property></Properties>"#.to_vec()));
    }
    let mut w = ZipWriter::new(Vec::new());
    for (n, d) in &entries {
        let level = if n.ends_with(".png") || n.ends_with(".jpeg") { 0 } else { 1 };
        w.add(n, d, level, 0x6000, 0x5821).unwrap();
    }
    w.finish().unwrap()
}

/// A one-page PDF with a weakly deflated image, an uncompressed content stream and an
/// unreachable leftover object: the lossless pass gains.
pub fn pdf() -> Vec<u8> {
    let content =
        b"BT /F1 12 Tf 72 700 Td (The quick brown fox jumps over the lazy dog, again and again.) Tj ET\n".repeat(40);
    let mut b = Builder::simple_page(&content);
    b.stream(
        6,
        flate_body(
            "/Type /XObject /Subtype /Image /Width 80 /Height 80 /ColorSpace /DeviceGray /BitsPerComponent 8",
            &[9u8; 6400],
        ),
    )
    .obj(7, &"(an unreachable leftover from an older save) ".repeat(40));
    b.classic("/Root 1 0 R")
}

/// A 16-bit mono PCM WAV of `secs` seconds at 16 kHz (a quiet tone, so a speech preset
/// has real audio to encode).
pub fn wav(secs: u32) -> Vec<u8> {
    let rate = 16_000u32;
    let n = rate * secs;
    let data_len = n * 2;
    let mut v = Vec::with_capacity(44 + data_len as usize);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data_len).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&rate.to_le_bytes());
    v.extend_from_slice(&(rate * 2).to_le_bytes());
    v.extend_from_slice(&2u16.to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..n {
        let t = i as f64 / rate as f64;
        let s = ((t * 440.0 * std::f64::consts::TAU).sin() * 6000.0) as i16;
        v.extend_from_slice(&s.to_le_bytes());
    }
    v
}
