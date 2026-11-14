//! Word optimisation (OPT-DOC-01..05, SAF-08).
//! Owner: W3-A.
//!
//! On by default: repack, thumbnail, media dedup. Reported only: tracked
//! changes and comments weight (DOC-04, never auto-accepted). Opt-in: custom
//! XML parts and the attached-template reference (DOC-05), unembedding fonts
//! (DOC-03), embedded chart workbooks (DOC-02 partial). Picture compression
//! (DOC-01), object-to-picture conversion and font subsetting need codecs or
//! renderers that are not bundled and are shown disabled with the reason.

use std::collections::BTreeMap;

use mj_core::{
    ActionKind, Availability, Category, DetectContext, DetectorOutput, Estimate, ExternalTool, FileEntry, OptimiseJob,
    OptimiseOutput, Optimiser, OptimiserId, OptimiserInfo, ParamValues, PlatformSupport, Result, SafetyClass,
};
use mj_formats::sniff::FileKind;
use mj_formats::xml::Edits;

use super::package::{self as pk, disabled, toggle, OpReport, Pkg};
use super::powerpoint::remove_custom_xml;
use crate::framework;

/// Word documents: Compress pictures and remove dead weight
#[derive(Debug, Default, Clone, Copy)]
pub struct Word;

/// Extensions (lower case) this optimiser targets.
pub const WORD_EXTENSIONS: &[&str] = &["docx", "docm", "dotx", "dotm"];

pub const P_CUSTOM_XML: &str = "remove_custom_xml";
pub const P_TEMPLATE: &str = "remove_attached_template";
pub const P_FONTS: &str = "unembed_fonts";

fn settings() -> Vec<mj_core::SettingSpec> {
    let mut v = pk::common_settings();
    v.extend([
        toggle(P_CUSTOM_XML, "Remove custom XML parts", "Never on labelled files; data bound to content controls, bibliography, cover page and SharePoint data is kept (OPT-DOC-05)", false, true),
        toggle(P_TEMPLATE, "Remove the attached template reference", "The document stops pointing at its template file (OPT-DOC-05)", false, true),
        toggle(P_FONTS, "Unembed fonts", "Removes embedded fonts; other computers fall back to installed fonts (OPT-DOC-03)", false, true),
        toggle(pk::P_CHART_DATA, "Remove embedded chart data", "Charts keep their values as shown but their data can no longer be edited (OPT-DOC-02)", false, true),
        disabled("compress_pictures", "Compress pictures (cropped areas, editing history, DPI)", pk::NEEDS_CODEC),
        disabled("objects_to_pictures", "Convert embedded objects to pictures", "Needs an object renderer that is not bundled"),
        disabled("subset_fonts", "Subset embedded fonts", "Needs a font subsetter that is not bundled; unembedding is available"),
    ]);
    v
}

/// DOC-04: weight of tracked changes and comments (report only).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RevisionReport {
    pub insertions: usize,
    pub deletions: usize,
    pub comments: usize,
    pub comment_bytes: u64,
}

fn count_tag(hay: &[u8], tag: &[u8]) -> usize {
    hay.windows(tag.len()).filter(|w| *w == tag).count()
}

fn revision_report(pkg: &mut Pkg, doc: &str) -> Result<RevisionReport> {
    let d = pkg.read(doc)?;
    let mut r = RevisionReport {
        insertions: count_tag(&d, b"<w:ins ") + count_tag(&d, b"<w:ins>"),
        deletions: count_tag(&d, b"<w:del ") + count_tag(&d, b"<w:del>"),
        ..Default::default()
    };
    for (_, c) in pk::targets(pkg, doc, "comments")? {
        let b = pkg.read(&c)?;
        r.comments += count_tag(&b, b"<w:comment ");
        r.comment_bytes += pkg.entry(&c).map(|e| e.compressed_size).unwrap_or(0);
    }
    Ok(r)
}

fn remove_template(pkg: &mut Pkg, doc: &str, rep: &mut OpReport) -> Result<()> {
    let mut n = 0;
    for (_, settings) in pk::targets(pkg, doc, "settings")? {
        let rels: Vec<_> = pkg.rels(&settings)?.into_iter().filter(|r| r.kind() == "attachedTemplate").collect();
        for r in rels {
            pk::remove_rel_and_refs(pkg, &settings, &r.id)?;
            n += 1;
        }
    }
    if n > 0 {
        rep.lossy = true;
        rep.notes.push("attached template reference removed".into());
    }
    Ok(())
}

fn unembed_fonts(pkg: &mut Pkg, doc: &str, rep: &mut OpReport) -> Result<()> {
    let mut n = 0;
    for (_, ft) in pk::targets(pkg, doc, "fontTable")? {
        let mut rids = Vec::new();
        pk::edit_part(pkg, &ft, |_, t| {
            let mut e = Edits::new();
            let mut removed = Vec::new();
            for (i, el) in t.elements.iter().enumerate() {
                if el.local.starts_with("embed") {
                    rids.extend(el.rel_attrs().map(|a| a.value.clone()));
                    t.remove_element(i, &mut e, &mut removed);
                }
            }
            Ok(e)
        })?;
        for id in &rids {
            pkg.remove_relationship(&ft, id)?;
        }
        n += rids.len();
    }
    if n > 0 {
        for (_, st) in pk::targets(pkg, doc, "settings")? {
            pk::edit_part(pkg, &st, |_, t| {
                let mut e = Edits::new();
                let mut removed = Vec::new();
                for (i, el) in t.elements.iter().enumerate() {
                    if matches!(el.local.as_str(), "embedTrueTypeFonts" | "embedSystemFonts" | "saveSubsetFonts") {
                        t.remove_element(i, &mut e, &mut removed);
                    }
                }
                Ok(e)
            })?;
        }
        rep.lossy = true;
        rep.notes.push(format!("{n} embedded font file(s) removed"));
    }
    Ok(())
}

/// True when content controls bind to the custom XML `item` (its datastore GUID
/// appears in a `w:storeItemID` anywhere in the document parts).
fn item_is_bound(pkg: &mut Pkg, item: &str) -> Result<bool> {
    let mut guids = Vec::new();
    for (_, props) in pk::targets(pkg, item, "customXmlProps")? {
        let b = pkg.read(&props)?;
        let text = mj_formats::xml::decode_any(&b).map(|c| c.into_owned()).unwrap_or_default();
        if let Some(i) = text.find("itemID=\"") {
            let rest = &text[i + 8..];
            if let Some(j) = rest.find('"') {
                guids.push(rest[..j].to_ascii_lowercase());
            }
        }
    }
    if guids.is_empty() {
        return Ok(true); // unknown identity: assume bound, keep it
    }
    for p in pkg.parts() {
        if p.to_ascii_lowercase().starts_with("word/") && pk::is_xml_part(&p) {
            let t = String::from_utf8_lossy(&pkg.read(&p)?).to_ascii_lowercase();
            if guids.iter().any(|g| t.contains(g.as_str())) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn ops(pkg: &mut Pkg, p: &ParamValues, rep: &mut OpReport) -> Result<()> {
    let doc = pk::main_part(pkg)?;
    let r = revision_report(pkg, &doc)?;
    if r.insertions + r.deletions + r.comments > 0 {
        rep.notes.push(format!(
            "tracked changes and comments kept (review only): {} insertion(s), {} deletion(s), {} comment(s)",
            r.insertions, r.deletions, r.comments
        ));
    }
    if p.bool(P_CUSTOM_XML, false) {
        let labelled =
            pkg.canonical("docProps/custom.xml").is_some() || pkg.canonical("docMetadata/LabelInfo.xml").is_some();
        remove_custom_xml(pkg, &doc, labelled, &item_is_bound, rep)?;
    }
    if p.bool(P_TEMPLATE, false) {
        remove_template(pkg, &doc, rep)?;
    }
    if p.bool(P_FONTS, false) {
        unembed_fonts(pkg, &doc, rep)?;
    }
    if p.bool(pk::P_CHART_DATA, false) {
        pk::remove_chart_workbooks(pkg, rep)?;
    }
    Ok(())
}

fn columns(e: &FileEntry) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    if let Ok(dir) = pk::directory(&e.path) {
        m.insert("breakdown".into(), pk::breakdown(&dir).summary());
    }
    if let Ok(mut pkg) = mj_formats::ooxml::Package::open_file(&e.path, Default::default()) {
        if let Ok(doc) = pk::main_part(&mut pkg) {
            if let Ok(r) = revision_report(&mut pkg, &doc) {
                if r.insertions + r.deletions > 0 {
                    m.insert("tracked_changes".into(), (r.insertions + r.deletions).to_string());
                }
                if r.comments > 0 {
                    m.insert(
                        "comments".into(),
                        format!("{} ({})", r.comments, mj_core::size::format_bytes(r.comment_bytes)),
                    );
                }
            }
        }
    }
    m
}

impl Optimiser for Word {
    fn info(&self) -> OptimiserInfo {
        OptimiserInfo {
            id: OptimiserId::Word,
            title: "Word documents".into(),
            description: "Compress pictures and remove dead weight".into(),
            category: Category::Optimise,
            safety: SafetyClass::LightCopy,
            default_action: ActionKind::Optimise,
            available_actions: vec![ActionKind::Optimise],
            presets: Vec::new(),
            settings: settings(),
            platform: PlatformSupport::Both,
            tools: vec![],
            feature_ids: (1..=5)
                .map(|i| format!("OPT-DOC-{i:02}"))
                .chain(["OPT-OFC-01".into(), "OPT-OFC-02".into()])
                .collect(),
            unit: "files".into(),
        }
    }

    fn availability(&self, tools: &[ExternalTool]) -> Availability {
        let _ = tools;
        Availability::Available
    }

    fn applies_to(&self, entry: &FileEntry) -> bool {
        entry.is_file() && WORD_EXTENSIONS.contains(&entry.extension().as_str())
    }

    fn estimate(&self, entry: &FileEntry, params: &ParamValues) -> Result<Estimate> {
        let mut extra = 0;
        if params.bool(P_FONTS, false) {
            if let Ok(dir) = pk::directory(&entry.path) {
                extra += dir
                    .iter()
                    .filter(|e| e.name.to_ascii_lowercase().starts_with("word/fonts/"))
                    .map(|e| e.compressed_size)
                    .sum::<u64>();
            }
        }
        pk::estimate(entry, params, extra)
    }

    fn optimise(&self, job: &OptimiseJob<'_>) -> Result<OptimiseOutput> {
        pk::run(job, FileKind::Docx, &ops)
    }

    fn analyse(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        framework::analyse_by_content(
            self,
            ctx,
            &[FileKind::Docx],
            &["pptx", "xlsx", "pptm", "xlsm"],
            |_, _| "Lossless repack, thumbnail and duplicate media".into(),
            columns,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::office::package::fixtures::*;
    use crate::office::package::testkit::*;
    use mj_core::testutil::TempDir;
    use mj_core::ParamValue;

    const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

    fn wml(p: &str) -> String {
        format!("application/vnd.openxmlformats-officedocument.wordprocessingml.{p}+xml")
    }

    /// A document with tracked changes, a comment, two identical pictures, an embedded
    /// font, an attached template, two custom XML items (one bound to a content control)
    /// and a chart with an embedded workbook.
    fn document(labelled: bool) -> Vec<u8> {
        let img = noise(15_000, 21);
        let mut ov = vec![
            ("word/document.xml".to_string(), wml("document.main")),
            ("word/settings.xml".into(), wml("settings")),
            ("word/fontTable.xml".into(), wml("fontTable")),
            ("word/comments.xml".into(), wml("comments")),
            (
                "word/charts/chart1.xml".into(),
                "application/vnd.openxmlformats-officedocument.drawingml.chart+xml".into(),
            ),
            (
                "customXml/itemProps1.xml".into(),
                "application/vnd.openxmlformats-officedocument.customXmlProperties+xml".into(),
            ),
            (
                "customXml/itemProps2.xml".into(),
                "application/vnd.openxmlformats-officedocument.customXmlProperties+xml".into(),
            ),
        ];
        for (p, t) in props_overrides(labelled) {
            ov.push((p.into(), t.into()));
        }
        let ovr: Vec<(&str, &str)> = ov.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        let doc = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="{W}" xmlns:r="{R}"><w:body><w:p><w:ins w:id="1" w:author="a"><w:r><w:t>new</w:t></w:r></w:ins><w:del w:id="2" w:author="a"><w:r><w:delText>old</w:delText></w:r></w:del></w:p><w:sdt><w:sdtPr><w:dataBinding w:xpath="/data/v" w:storeItemID="{{BBBB-1}}"/></w:sdtPr></w:sdt><w:p><w:r><w:drawing><a:blip xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" r:embed="rId5"/></w:drawing><w:drawing><a:blip xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" r:embed="rId6"/></w:drawing></w:r></w:p></w:body></w:document>"#
        );
        let item = |n: u32, guid: &str| -> [(String, Vec<u8>); 3] {
            [
                (format!("customXml/item{n}.xml"), format!("<?xml version=\"1.0\"?><data xmlns=\"urn:c{n}\"><v>{n}</v></data>").into_bytes()),
                (format!("customXml/_rels/item{n}.xml.rels"), rels(&[("rId1", "customXmlProps", &format!("itemProps{n}.xml"))]).into_bytes()),
                (format!("customXml/itemProps{n}.xml"), format!("<?xml version=\"1.0\"?><ds:datastoreItem ds:itemID=\"{guid}\" xmlns:ds=\"http://schemas.openxmlformats.org/officeDocument/2006/customXml\"/>").into_bytes()),
            ]
        };
        let items: Vec<(String, Vec<u8>)> = item(1, "{BBBB-1}").into_iter().chain(item(2, "{CCCC-2}")).collect();
        let mut e: Vec<(&str, Vec<u8>)> = vec![
            ("[Content_Types].xml", content_types(&ovr).into_bytes()),
            ("_rels/.rels", root_rels("word/document.xml", labelled)),
            ("word/document.xml", doc.into_bytes()),
            (
                "word/_rels/document.xml.rels",
                rels(&[("rId1", "settings", "settings.xml"), ("rId2", "fontTable", "fontTable.xml"), ("rId3", "comments", "comments.xml"), ("rId4", "customXml", "../customXml/item1.xml"), ("rId5", "image", "media/image1.png"), ("rId6", "image", "media/image2.png"), ("rId7", "customXml", "../customXml/item2.xml"), ("rId8", "chart", "charts/chart1.xml")]).into_bytes(),
            ),
            ("word/settings.xml", format!(r#"<?xml version="1.0"?><w:settings xmlns:w="{W}" xmlns:r="{R}"><w:embedTrueTypeFonts/><w:saveSubsetFonts/><w:attachedTemplate r:id="rId1"/></w:settings>"#).into_bytes()),
            ("word/_rels/settings.xml.rels", format!(r#"<?xml version="1.0"?><Relationships xmlns="{PKG_R}"><Relationship Id="rId1" Type="{R}/attachedTemplate" Target="file:///C:/Templates/Big.dotm" TargetMode="External"/></Relationships>"#).into_bytes()),
            ("word/fontTable.xml", format!(r#"<?xml version="1.0"?><w:fonts xmlns:w="{W}" xmlns:r="{R}"><w:font w:name="Fixture"><w:embedRegular r:id="rId1" w:fontKey="{{X}}"/></w:font></w:fonts>"#).into_bytes()),
            ("word/_rels/fontTable.xml.rels", rels(&[("rId1", "font", "fonts/font1.odttf")]).into_bytes()),
            ("word/fonts/font1.odttf", noise(8000, 4)),
            ("word/comments.xml", format!(r#"<?xml version="1.0"?><w:comments xmlns:w="{W}"><w:comment w:id="0" w:author="a"><w:p/></w:comment></w:comments>"#).into_bytes()),
            ("word/media/image1.png", img.clone()),
            ("word/media/image2.png", img),
            ("word/charts/chart1.xml", br#"<?xml version="1.0"?><c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><c:chart><c:ser><c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>1</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser></c:chart><c:externalData r:id="rId1"><c:autoUpdate val="0"/></c:externalData></c:chartSpace>"#.to_vec()),
            ("word/charts/_rels/chart1.xml.rels", rels(&[("rId1", "package", "../embeddings/Microsoft_Excel_Worksheet.xlsx")]).into_bytes()),
            ("word/embeddings/Microsoft_Excel_Worksheet.xlsx", noise(12_000, 8)),
        ];
        for (n, d) in &items {
            e.push((n.as_str(), d.clone()));
        }
        e.extend(doc_props(labelled));
        build(&e)
    }

    #[test]
    fn defaults_report_revisions_and_keep_content() {
        let t = TempDir::new("doc").unwrap();
        let (o, dst) = run_on(&Word, &t, "a.docx", &document(false), &ParamValues::new());
        assert!(o.skipped.is_none(), "{o:?}");
        assert!(!o.lossy);
        unzip_ok(&dst);
        let all = read_all(&dst);
        assert!(part(&all, "word/media/image2.png").is_none());
        assert!(part(&all, "docProps/thumbnail.jpeg").is_none());
        assert!(o.notes.iter().any(|n| n.contains("1 insertion(s), 1 deletion(s), 1 comment(s)")), "{:?}", o.notes);
        let src = read_all(&t.join("a.docx"));
        assert_eq!(part(&src, "word/document.xml"), part(&all, "word/document.xml"));
        assert!(part(&all, "word/fonts/font1.odttf").is_some());
    }

    #[test]
    fn opt_ins_remove_unbound_custom_xml_template_fonts_and_chart_data() {
        let t = TempDir::new("doc2").unwrap();
        let mut p = ParamValues::new();
        for k in [P_CUSTOM_XML, P_TEMPLATE, P_FONTS, pk::P_CHART_DATA] {
            p.set(k, ParamValue::Bool(true));
        }
        let (o, dst) = run_on(&Word, &t, "a.docx", &document(false), &p);
        assert!(o.skipped.is_none(), "{o:?}");
        assert!(o.lossy);
        unzip_ok(&dst);
        let all = read_all(&dst);
        assert!(part(&all, "customXml/item1.xml").is_some(), "bound to a content control");
        assert!(part(&all, "customXml/item2.xml").is_none() && part(&all, "customXml/itemProps2.xml").is_none());
        let st = part(&all, "word/settings.xml").unwrap();
        assert!(!st.contains("attachedTemplate") && !st.contains("embedTrueTypeFonts"));
        assert!(part(&all, "word/_rels/settings.xml.rels").unwrap().matches("<Relationship ").count() == 0);
        assert!(part(&all, "word/fonts/font1.odttf").is_none());
        assert!(!part(&all, "word/fontTable.xml").unwrap().contains("embedRegular"));
        let chart = part(&all, "word/charts/chart1.xml").unwrap();
        assert!(!chart.contains("externalData") && chart.contains("numCache"));
        assert!(part(&all, "word/embeddings/Microsoft_Excel_Worksheet.xlsx").is_none());
    }

    #[test]
    fn labelled_document_keeps_custom_parts() {
        let t = TempDir::new("doc3").unwrap();
        let p = ParamValues::new().with(P_CUSTOM_XML, ParamValue::Bool(true));
        let (o, dst) = run_on(&Word, &t, "a.docx", &document(true), &p);
        assert!(o.skipped.is_none(), "{o:?}");
        let all = read_all(&dst);
        assert!(part(&all, "customXml/item2.xml").is_some());
        assert_eq!(part(&all, "docProps/custom.xml"), part(&read_all(&t.join("a.docx")), "docProps/custom.xml"));
    }

    #[test]
    fn columns_and_settings() {
        let t = TempDir::new("doc4").unwrap();
        let p = t.write("a.docx", &document(false)).unwrap();
        let e = FileEntry::new(&p, mj_core::EntryKind::File, std::fs::metadata(&p).unwrap().len());
        let c = columns(&e);
        assert_eq!(c["tracked_changes"], "2");
        assert!(c["comments"].starts_with("1 ("));
        assert!(Word.estimate(&e, &ParamValues::new()).unwrap().saving() > 0);
        assert!(Word.info().settings.iter().any(|s| s.key == "compress_pictures" && s.disabled_reason.is_some()));
    }
}
