//! PowerPoint optimisation (OPT-PPT-01..10, OPT-OFC-02, SAF-08).
//! Owner: W3-A.
//!
//! On by default (lossless for what the user sees and edits): repack at the
//! maximum deflate level, thumbnail removal, media deduplication (PPT-08) and
//! removal of slide layouts no slide uses (PPT-04, at least one layout is kept
//! per master). Opt-in: unused masters, speaker notes, comments, hidden slides,
//! custom XML, embedded fonts, embedded chart workbooks. Picture and media
//! re-encoding (PPT-01/02/03/07/09) and font subsetting need codecs that are not
//! bundled and are shown disabled with the reason.

use std::collections::{BTreeMap, BTreeSet};

use mj_core::{
    ActionKind, Availability, Category, DetectContext, DetectorOutput, Estimate, ExternalTool, FileEntry, OptimiseJob,
    OptimiseOutput, Optimiser, OptimiserId, OptimiserInfo, ParamValues, PlatformSupport, Result, SafetyClass,
};
use mj_formats::ooxml::tree::Tree;
use mj_formats::sniff::FileKind;
use mj_formats::xml::{self, Edits};

use super::package::{self as pk, disabled, toggle, OpReport, Pkg};
use crate::framework;

/// Office decks: Lossless first, light copies kept beside originals
#[derive(Debug, Default, Clone, Copy)]
pub struct PowerPoint;

/// Extensions (lower case) this optimiser targets.
pub const POWERPOINT_EXTENSIONS: &[&str] = &["pptx", "pptm", "ppsx", "ppsm", "potx", "potm"];

pub const P_LAYOUTS: &str = "remove_unused_layouts";
pub const P_MASTERS: &str = "remove_unused_masters";
pub const P_NOTES: &str = "remove_notes";
pub const P_COMMENTS: &str = "remove_comments";
pub const P_HIDDEN: &str = "remove_hidden_slides";
pub const P_CUSTOM_XML: &str = "remove_custom_xml";
pub const P_FONTS: &str = "unembed_fonts";

const P14_NS: &str = "http://schemas.microsoft.com/office/powerpoint/2010/main";

fn settings() -> Vec<mj_core::SettingSpec> {
    let mut v = pk::common_settings();
    v.extend([
        toggle(
            P_LAYOUTS,
            "Remove unused slide layouts",
            "Layouts no slide uses; one layout is always kept per master (OPT-PPT-04)",
            true,
            false,
        ),
        toggle(
            P_MASTERS,
            "Remove unused slide masters",
            "Masters none of whose layouts is used by a slide; one master is always kept",
            false,
            false,
        ),
        toggle(P_NOTES, "Remove speaker notes", "Deletes the notes of every slide (OPT-PPT-10)", false, true),
        toggle(P_COMMENTS, "Remove comments", "Deletes review comments on slides (OPT-PPT-10)", false, true),
        toggle(
            P_HIDDEN,
            "Remove hidden slides",
            "Deletes slides marked hidden; slides linked from other slides are kept (OPT-PPT-10)",
            false,
            true,
        ),
        toggle(
            P_CUSTOM_XML,
            "Remove custom XML parts",
            "Never on labelled files; SharePoint and label data is always kept (OPT-PPT-10)",
            false,
            true,
        ),
        toggle(
            P_FONTS,
            "Unembed fonts",
            "Removes embedded fonts; other computers fall back to installed fonts (OPT-PPT-05)",
            false,
            true,
        ),
        toggle(
            pk::P_CHART_DATA,
            "Remove embedded chart data",
            "Charts keep their values as shown but their data can no longer be edited (OPT-PPT-06)",
            false,
            true,
        ),
        disabled("downsample_pictures", "Compress pictures (96 / 150 / 220 ppi)", pk::NEEDS_CODEC),
        disabled("delete_cropped_areas", "Delete cropped areas of pictures", pk::NEEDS_CODEC),
        disabled(
            "delete_editing_history",
            "Delete picture editing history",
            "Removing the originals kept for edited pictures could not be validated against PowerPoint; kept off",
        ),
        disabled(
            "subset_fonts",
            "Subset embedded fonts",
            "Needs a font subsetter that is not bundled; unembedding is available",
        ),
        disabled("chart_to_picture", "Convert charts to pictures", "Needs a chart renderer that is not bundled"),
        disabled("convert_emf_wmf", "Convert large EMF/WMF pictures", "Needs a vector renderer that is not bundled"),
        disabled(
            "reencode_media",
            "Re-encode embedded video and audio",
            "Embedded media re-encoding needs ffmpeg integration inside packages, not available yet",
        ),
    ]);
    v
}

/// Slides, in presentation order: (presentation relationship id, slide part).
fn slides(pkg: &mut Pkg, pres: &str) -> Result<Vec<(String, String)>> {
    Ok(pk::targets(pkg, pres, "slide")?.into_iter().map(|(r, t)| (r.id, t)).collect())
}

/// Layouts no slide uses, per master: (master, layout rel id, layout part, name).
pub fn unused_layouts(pkg: &mut Pkg, pres: &str) -> Result<Vec<(String, String, String, String)>> {
    let mut used = BTreeSet::new();
    for (_, s) in slides(pkg, pres)? {
        for (_, l) in pk::targets(pkg, &s, "slideLayout")? {
            used.insert(l);
        }
    }
    let mut out = Vec::new();
    for (_, master) in pk::targets(pkg, pres, "slideMaster")? {
        let layouts = pk::targets(pkg, &master, "slideLayout")?;
        let mut unused: Vec<_> = layouts.iter().filter(|(_, l)| !used.contains(l)).cloned().collect();
        if unused.len() == layouts.len() && !unused.is_empty() {
            unused.remove(0); // every master keeps at least one layout
        }
        for (r, l) in unused {
            // A layout referenced by anything but its master (another layout, a slide of
            // another kind) is kept.
            let inc = pkg.incoming(&l)?;
            if inc.iter().any(|e| e.source != master) {
                continue;
            }
            let name = layout_name(pkg, &l).unwrap_or_else(|| l.rsplit('/').next().unwrap_or(&l).to_string());
            out.push((master.clone(), r.id, l, name));
        }
    }
    Ok(out)
}

fn layout_name(pkg: &mut Pkg, part: &str) -> Option<String> {
    let b = pkg.read(part).ok()?;
    let x = xml::as_utf8(&b).ok()?;
    let t = Tree::parse(x.text).ok()?;
    let name = t.by_local("cSld").next().and_then(|(_, e)| e.plain("name").map(str::to_string));
    name
}

fn remove_layouts(pkg: &mut Pkg, pres: &str, rep: &mut OpReport) -> Result<()> {
    let list = unused_layouts(pkg, pres)?;
    for (master, rid, _, _) in &list {
        pk::remove_rel_and_refs(pkg, master, rid)?;
    }
    if !list.is_empty() {
        let names: Vec<_> = list.iter().map(|x| x.3.clone()).collect();
        rep.notes.push(format!("{} unused slide layout(s) removed: {}", list.len(), names.join(", ")));
    }
    Ok(())
}

fn remove_masters(pkg: &mut Pkg, pres: &str, rep: &mut OpReport) -> Result<()> {
    let mut used = BTreeSet::new();
    for (_, s) in slides(pkg, pres)? {
        for (_, l) in pk::targets(pkg, &s, "slideLayout")? {
            used.insert(l);
        }
    }
    let masters = pk::targets(pkg, pres, "slideMaster")?;
    let mut keep = 0usize;
    let mut drop = Vec::new();
    for (r, m) in &masters {
        let layouts = pk::targets(pkg, m, "slideLayout")?;
        if layouts.iter().any(|(_, l)| used.contains(l)) {
            keep += 1;
        } else {
            drop.push((r.id.clone(), m.clone()));
        }
    }
    if keep == 0 && !drop.is_empty() {
        drop.remove(0);
    }
    for (rid, _) in &drop {
        pk::remove_rel_and_refs(pkg, pres, rid)?;
    }
    if !drop.is_empty() {
        rep.notes.push(format!("{} unused slide master(s) removed", drop.len()));
    }
    Ok(())
}

fn remove_slide_rels(pkg: &mut Pkg, pres: &str, kind: &str) -> Result<usize> {
    let mut n = 0;
    for (_, s) in slides(pkg, pres)? {
        for (r, _) in pk::targets(pkg, &s, kind)? {
            pk::remove_rel_and_refs(pkg, &s, &r.id)?;
            n += 1;
        }
    }
    Ok(n)
}

fn is_hidden(pkg: &mut Pkg, slide: &str) -> Result<bool> {
    let b = pkg.read(slide)?;
    let x = xml::as_utf8(&b)?;
    let t = Tree::parse(x.text)?;
    Ok(t.root().and_then(|r| r.plain("show")).is_some_and(|v| v == "0" || v == "false"))
}

fn remove_hidden(pkg: &mut Pkg, pres: &str, rep: &mut OpReport) -> Result<()> {
    let all = slides(pkg, pres)?;
    let mut doomed = Vec::new();
    for (rid, s) in &all {
        if !is_hidden(pkg, s)? {
            continue;
        }
        let own_notes: BTreeSet<String> = pk::targets(pkg, s, "notesSlide")?.into_iter().map(|(_, t)| t).collect();
        let inc = pkg.incoming(s)?;
        if inc.iter().any(|e| e.source != pres && !own_notes.contains(&e.source)) {
            rep.notes.push(format!("hidden slide {s} is linked from another slide; kept"));
            continue;
        }
        doomed.push((rid.clone(), s.clone()));
    }
    if doomed.len() == all.len() && !doomed.is_empty() {
        rep.notes.push("every slide is hidden; nothing was removed".into());
        return Ok(());
    }
    if doomed.is_empty() {
        return Ok(());
    }
    let rids: BTreeSet<String> = doomed.iter().map(|d| d.0.clone()).collect();
    pk::edit_part(pkg, pres, |_, t| {
        let mut e = Edits::new();
        let mut removed = Vec::new();
        // Numeric slide ids of the removed slides (used by sections).
        let mut ids = BTreeSet::new();
        for rid in &rids {
            for i in t.referencing(rid) {
                if t.elements[i].local == "sldId" {
                    if let Some(id) = t.elements[i].plain("id") {
                        ids.insert(id.to_string());
                    }
                }
                t.remove_element(i, &mut e, &mut removed);
            }
        }
        for (i, el) in t.by_local("sldId") {
            if el.ns.as_deref() == Some(P14_NS) && el.plain("id").is_some_and(|v| ids.contains(v)) {
                t.remove_element(i, &mut e, &mut removed);
            }
        }
        Ok(e)
    })?;
    for rid in &rids {
        pkg.remove_relationship(pres, rid)?;
    }
    rep.lossy = true;
    rep.notes.push(format!("{} hidden slide(s) removed", doomed.len()));
    Ok(())
}

/// Markers of custom XML that must never be removed (labels, SharePoint, bibliography, cover page).
pub const KEEP_CUSTOM_XML: &[&str] = &[
    "MSIP",
    "LabelInfo",
    "sensitivity",
    "schemas.microsoft.com/office/2006/metadata/properties",
    "contentTypeSchema",
    "officeDocument/2006/bibliography",
    "coverPageProps",
    "schemas.microsoft.com/sharepoint",
];

/// Remove custom XML items attached to `main` (only items nothing else references and
/// that carry no protected data). Returns the number removed.
pub fn remove_custom_xml(
    pkg: &mut Pkg,
    main: &str,
    labelled: bool,
    extra_refs: &dyn Fn(&mut Pkg, &str) -> Result<bool>,
    rep: &mut OpReport,
) -> Result<()> {
    if labelled {
        rep.notes.push("custom XML kept: the file carries a sensitivity label".into());
        return Ok(());
    }
    let mut n = 0;
    for (r, item) in pk::targets(pkg, main, "customXml")? {
        let data = pkg.read(&item)?;
        let text = xml::decode_any(&data).map(|c| c.into_owned()).unwrap_or_default();
        if KEEP_CUSTOM_XML.iter().any(|m| text.contains(m)) {
            continue;
        }
        if pkg.incoming(&item)?.iter().any(|e| e.source != main) || extra_refs(pkg, &item)? {
            continue;
        }
        pk::remove_rel_and_refs(pkg, main, &r.id)?;
        n += 1;
    }
    if n > 0 {
        rep.lossy = true;
        rep.notes.push(format!("{n} custom XML part(s) removed"));
    }
    Ok(())
}

fn unembed_fonts(pkg: &mut Pkg, pres: &str, rep: &mut OpReport) -> Result<()> {
    let mut rids = Vec::new();
    let changed = pk::edit_part(pkg, pres, |text, t| {
        let mut e = Edits::new();
        let mut removed = Vec::new();
        let lists: Vec<usize> = t.by_local("embeddedFontLst").map(|(i, _)| i).collect();
        for i in lists {
            for (j, d) in t.elements.iter().enumerate() {
                if t.contains(i, j) {
                    rids.extend(d.rel_attrs().map(|a| a.value.clone()));
                }
            }
            t.remove_element(i, &mut e, &mut removed);
        }
        if let Some(root) = t.root() {
            if root.plain("embedTrueTypeFonts").is_some() {
                pk::remove_attr(text, t, 0, |a| a.name == "embedTrueTypeFonts" || a.name == "saveSubsetFonts", &mut e);
            }
        }
        Ok(e)
    })?;
    for rid in &rids {
        pkg.remove_relationship(pres, rid)?;
    }
    if changed && !rids.is_empty() {
        rep.lossy = true;
        rep.notes.push(format!("{} embedded font file(s) removed", rids.len()));
    }
    Ok(())
}

fn ops(pkg: &mut Pkg, p: &ParamValues, rep: &mut OpReport) -> Result<()> {
    let pres = pk::main_part(pkg)?;
    if p.bool(P_HIDDEN, false) {
        remove_hidden(pkg, &pres, rep)?;
    }
    if p.bool(P_NOTES, false) {
        let n = remove_slide_rels(pkg, &pres, "notesSlide")?;
        if n > 0 {
            rep.lossy = true;
            rep.notes.push(format!("speaker notes removed from {n} slide(s)"));
        }
    }
    if p.bool(P_COMMENTS, false) {
        let n = remove_slide_rels(pkg, &pres, "comments")?;
        if n > 0 {
            rep.lossy = true;
            rep.notes.push(format!("comments removed from {n} slide(s)"));
        }
    }
    if p.bool(P_MASTERS, false) {
        remove_masters(pkg, &pres, rep)?;
    }
    if p.bool(P_LAYOUTS, true) {
        remove_layouts(pkg, &pres, rep)?;
    }
    if p.bool(P_CUSTOM_XML, false) {
        let labelled =
            pkg.canonical("docProps/custom.xml").is_some() || pkg.canonical("docMetadata/LabelInfo.xml").is_some();
        remove_custom_xml(pkg, &pres, labelled, &|_, _| Ok(false), rep)?;
    }
    if p.bool(P_FONTS, false) {
        unembed_fonts(pkg, &pres, rep)?;
    }
    if p.bool(pk::P_CHART_DATA, false) {
        pk::remove_chart_workbooks(pkg, rep)?;
    }
    Ok(())
}

/// Extra columns for the review table: breakdown and detected unused layouts.
fn columns(e: &FileEntry) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    if let Ok(dir) = pk::directory(&e.path) {
        m.insert("breakdown".into(), pk::breakdown(&dir).summary());
    }
    if let Ok(mut pkg) = mj_formats::ooxml::Package::open_file(&e.path, Default::default()) {
        if let Ok(pres) = pk::main_part(&mut pkg) {
            if let Ok(list) = unused_layouts(&mut pkg, &pres) {
                if !list.is_empty() {
                    m.insert("unused_layouts".into(), list.iter().map(|x| x.3.clone()).collect::<Vec<_>>().join(", "));
                }
            }
        }
    }
    m
}

impl Optimiser for PowerPoint {
    fn info(&self) -> OptimiserInfo {
        OptimiserInfo {
            id: OptimiserId::PowerPoint,
            title: "Office decks".into(),
            description: "Lossless first, light copies kept beside originals".into(),
            category: Category::Optimise,
            safety: SafetyClass::LightCopy,
            default_action: ActionKind::Optimise,
            available_actions: vec![ActionKind::Optimise],
            presets: Vec::new(),
            settings: settings(),
            platform: PlatformSupport::Both,
            tools: vec![],
            feature_ids: (1..=10)
                .map(|i| format!("OPT-PPT-{i:02}"))
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
        entry.is_file() && POWERPOINT_EXTENSIONS.contains(&entry.extension().as_str())
    }

    fn estimate(&self, entry: &FileEntry, params: &ParamValues) -> Result<Estimate> {
        let mut extra = 0;
        if params.bool(P_LAYOUTS, true) {
            if let Ok(mut pkg) = mj_formats::ooxml::Package::open_file(&entry.path, Default::default()) {
                if let Ok(pres) = pk::main_part(&mut pkg) {
                    for (_, _, l, _) in unused_layouts(&mut pkg, &pres).unwrap_or_default() {
                        extra += pkg.entry(&l).map(|e| e.compressed_size).unwrap_or(0);
                    }
                }
            }
        }
        pk::estimate(entry, params, extra)
    }

    fn optimise(&self, job: &OptimiseJob<'_>) -> Result<OptimiseOutput> {
        pk::run(job, FileKind::Pptx, &ops)
    }

    fn analyse(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        framework::analyse_by_content(
            self,
            ctx,
            &[FileKind::Pptx],
            &["docx", "xlsx", "docm", "xlsm"],
            |_, _| "Lossless repack, duplicate media and unused layouts".into(),
            columns,
        )
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::office::package::fixtures::*;
    use crate::office::package::testkit::*;
    use mj_core::testutil::TempDir;
    use mj_core::ParamValue;

    const P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
    const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";

    fn ct(p: &str) -> String {
        format!("application/vnd.openxmlformats-officedocument.presentationml.{p}+xml")
    }

    /// A deck: 1 master with 3 layouts (only layout1 used), slide1 (notes, comments,
    /// image1), slide2 hidden (image2 = copy of image1), an embedded font, custom XML.
    pub fn deck(labelled: bool, signed: bool) -> Vec<u8> {
        let img = noise(20_000, 7);
        let mut ov = vec![
            ("ppt/presentation.xml".to_string(), ct("presentation.main")),
            ("ppt/slideMasters/slideMaster1.xml".into(), ct("slideMaster")),
            ("ppt/slides/slide1.xml".into(), ct("slide")),
            ("ppt/slides/slide2.xml".into(), ct("slide")),
            ("ppt/notesSlides/notesSlide1.xml".into(), ct("notesSlide")),
            ("ppt/comments/comment1.xml".into(), ct("comments")),
            ("ppt/theme/theme1.xml".into(), "application/vnd.openxmlformats-officedocument.theme+xml".into()),
            (
                "customXml/itemProps1.xml".into(),
                "application/vnd.openxmlformats-officedocument.customXmlProperties+xml".into(),
            ),
        ];
        for i in 1..=3 {
            ov.push((format!("ppt/slideLayouts/slideLayout{i}.xml"), ct("slideLayout")));
        }
        for (p, t) in props_overrides(labelled) {
            ov.push((p.into(), t.into()));
        }
        let ovr: Vec<(&str, &str)> = ov.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        let pres = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:a="{A}" xmlns:r="{R}" xmlns:p="{P}" saveSubsetFonts="1" embedTrueTypeFonts="1"><p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst><p:sldIdLst><p:sldId id="256" r:id="rId2"/><p:sldId id="257" r:id="rId3"/></p:sldIdLst><p:sldSz cx="9144000" cy="6858000"/><p:notesSz cx="6858000" cy="9144000"/><p:embeddedFontLst><p:embeddedFont><p:font typeface="Fixture Sans"/><p:regular r:id="rId6"/></p:embeddedFont></p:embeddedFontLst><p:custShowLst><p:custShow name="s" id="0"><p:sldLst><p:sld r:id="rId3"/><p:sld r:id="rId2"/></p:sldLst></p:custShow></p:custShowLst><p:extLst><p:ext uri="{{521415D9-36F7-43E2-AB2F-B90AF26B5E84}}"><p14:sectionLst xmlns:p14="{P14_NS}"><p14:section name="One" id="{{1}}"><p14:sldIdLst><p14:sldId id="256"/><p14:sldId id="257"/></p14:sldIdLst></p14:section></p14:sectionLst></p:ext></p:extLst></p:presentation>"#
        );
        let layout = |n: u32| {
            format!(
                r#"<?xml version="1.0"?><p:sldLayout xmlns:a="{A}" xmlns:r="{R}" xmlns:p="{P}"><p:cSld name="Layout {n}"><p:spTree/></p:cSld></p:sldLayout>"#
            )
        };
        let slide = |hidden: bool, pic: &str| {
            format!(
                r#"<?xml version="1.0"?><p:sld xmlns:a="{A}" xmlns:r="{R}" xmlns:p="{P}"{}><p:cSld><p:spTree><p:pic><p:blipFill><a:blip r:embed="{pic}"/></p:blipFill></p:pic></p:spTree></p:cSld></p:sld>"#,
                if hidden { r#" show="0""# } else { "" }
            )
        };
        let mut e: Vec<(&str, Vec<u8>)> = vec![
            ("[Content_Types].xml", content_types(&ovr).into_bytes()),
            ("_rels/.rels", root_rels("ppt/presentation.xml", labelled)),
            ("ppt/presentation.xml", pres.into_bytes()),
            (
                "ppt/_rels/presentation.xml.rels",
                rels(&[("rId1", "slideMaster", "slideMasters/slideMaster1.xml"), ("rId2", "slide", "slides/slide1.xml"), ("rId3", "slide", "slides/slide2.xml"), ("rId4", "theme", "theme/theme1.xml"), ("rId6", "font", "fonts/font1.fntdata"), ("rId7", "customXml", "../customXml/item1.xml")]).into_bytes(),
            ),
            ("ppt/slideMasters/slideMaster1.xml", format!(r#"<?xml version="1.0"?><p:sldMaster xmlns:a="{A}" xmlns:r="{R}" xmlns:p="{P}"><p:cSld><p:spTree/></p:cSld><p:sldLayoutIdLst><p:sldLayoutId id="2147483649" r:id="rId1"/><p:sldLayoutId id="2147483650" r:id="rId2"/><p:sldLayoutId id="2147483651" r:id="rId3"/></p:sldLayoutIdLst></p:sldMaster>"#).into_bytes()),
            ("ppt/slideMasters/_rels/slideMaster1.xml.rels", rels(&[("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml"), ("rId2", "slideLayout", "../slideLayouts/slideLayout2.xml"), ("rId3", "slideLayout", "../slideLayouts/slideLayout3.xml"), ("rId4", "theme", "../theme/theme1.xml")]).into_bytes()),
            ("ppt/slideLayouts/slideLayout1.xml", layout(1).into_bytes()),
            ("ppt/slideLayouts/_rels/slideLayout1.xml.rels", rels(&[("rId1", "slideMaster", "../slideMasters/slideMaster1.xml")]).into_bytes()),
            ("ppt/slideLayouts/slideLayout2.xml", layout(2).into_bytes()),
            ("ppt/slideLayouts/_rels/slideLayout2.xml.rels", rels(&[("rId1", "slideMaster", "../slideMasters/slideMaster1.xml")]).into_bytes()),
            ("ppt/slideLayouts/slideLayout3.xml", layout(3).into_bytes()),
            ("ppt/slideLayouts/_rels/slideLayout3.xml.rels", rels(&[("rId1", "slideMaster", "../slideMasters/slideMaster1.xml"), ("rId2", "image", "../media/image3.png")]).into_bytes()),
            ("ppt/slides/slide1.xml", slide(false, "rId2").into_bytes()),
            ("ppt/slides/_rels/slide1.xml.rels", rels(&[("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml"), ("rId2", "image", "../media/image1.png"), ("rId3", "notesSlide", "../notesSlides/notesSlide1.xml"), ("rId4", "comments", "../comments/comment1.xml")]).into_bytes()),
            ("ppt/slides/slide2.xml", slide(true, "rId2").into_bytes()),
            ("ppt/slides/_rels/slide2.xml.rels", rels(&[("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml"), ("rId2", "image", "../media/image2.png")]).into_bytes()),
            ("ppt/notesSlides/notesSlide1.xml", format!(r#"<?xml version="1.0"?><p:notes xmlns:a="{A}" xmlns:r="{R}" xmlns:p="{P}"><p:cSld><p:spTree/></p:cSld></p:notes>"#).into_bytes()),
            ("ppt/notesSlides/_rels/notesSlide1.xml.rels", rels(&[("rId1", "slide", "../slides/slide1.xml")]).into_bytes()),
            ("ppt/comments/comment1.xml", format!(r#"<?xml version="1.0"?><p:cmLst xmlns:p="{P}"><p:cm authorId="0"><p:text>Check this</p:text></p:cm></p:cmLst>"#).into_bytes()),
            ("ppt/theme/theme1.xml", format!(r#"<?xml version="1.0"?><a:theme xmlns:a="{A}" name="T">{}</a:theme>"#, "<a:themeElements/>").into_bytes()),
            ("ppt/media/image1.png", img.clone()),
            ("ppt/media/image2.png", img),
            ("ppt/media/image3.png", noise(9000, 3)),
            ("ppt/fonts/font1.fntdata", noise(5000, 5)),
            ("customXml/item1.xml", b"<?xml version=\"1.0\"?><data xmlns=\"urn:custom\"><v>1</v></data>".to_vec()),
            ("customXml/_rels/item1.xml.rels", rels(&[("rId1", "customXmlProps", "itemProps1.xml")]).into_bytes()),
            ("customXml/itemProps1.xml", b"<?xml version=\"1.0\"?><ds:datastoreItem ds:itemID=\"{AAAA}\" xmlns:ds=\"http://schemas.openxmlformats.org/officeDocument/2006/customXml\"/>".to_vec()),
        ];
        e.extend(doc_props(labelled));
        if signed {
            e.push(("_xmlsignatures/sig1.xml", b"<Signature/>".to_vec()));
        }
        build(&e)
    }

    fn names(all: &[(String, Vec<u8>)]) -> Vec<&str> {
        all.iter().map(|(n, _)| n.as_str()).collect()
    }

    #[test]
    fn defaults_are_lossless_dedup_layouts_thumbnail() {
        let t = TempDir::new("ppt").unwrap();
        let (o, dst) = run_on(&PowerPoint, &t, "deck.pptx", &deck(false, false), &ParamValues::new());
        assert!(o.skipped.is_none(), "{o:?}");
        assert!(!o.lossy);
        assert!(o.bytes_after < o.bytes_before);
        unzip_ok(&dst);
        let all = read_all(&dst);
        let n = names(&all);
        assert_eq!(n[0], "[Content_Types].xml");
        assert!(!n.contains(&"docProps/thumbnail.jpeg"));
        assert!(!n.contains(&"ppt/media/image2.png"), "duplicate merged");
        assert!(part(&all, "ppt/slides/_rels/slide2.xml.rels").unwrap().contains("../media/image1.png"));
        assert!(!n.contains(&"ppt/slideLayouts/slideLayout2.xml") && !n.contains(&"ppt/slideLayouts/slideLayout3.xml"));
        assert!(!n.contains(&"ppt/slideLayouts/_rels/slideLayout3.xml.rels"));
        assert!(!n.contains(&"ppt/media/image3.png"), "only used by a removed layout");
        let master = part(&all, "ppt/slideMasters/slideMaster1.xml").unwrap();
        assert_eq!(master.matches("sldLayoutId ").count(), 1);
        assert!(!part(&all, "[Content_Types].xml").unwrap().contains("slideLayout2"));
        // Opt-in content is kept by default.
        for keep in [
            "ppt/notesSlides/notesSlide1.xml",
            "ppt/comments/comment1.xml",
            "ppt/slides/slide2.xml",
            "ppt/fonts/font1.fntdata",
            "customXml/item1.xml",
        ] {
            assert!(n.contains(&keep), "{keep}");
        }
        // Untouched parts are byte-identical.
        let src = read_all(&t.join("deck.pptx"));
        for p in ["ppt/slides/slide1.xml", "ppt/theme/theme1.xml", "ppt/media/image1.png", "ppt/presentation.xml"] {
            assert_eq!(part(&src, p), part(&all, p), "{p}");
        }
        assert!(o.notes.iter().any(|x| x.contains("Layout 2")), "{:?}", o.notes);
    }

    #[test]
    fn opt_in_removals() {
        let t = TempDir::new("ppt2").unwrap();
        let mut p = ParamValues::new();
        for k in [P_NOTES, P_COMMENTS, P_HIDDEN, P_CUSTOM_XML, P_FONTS] {
            p.set(k, ParamValue::Bool(true));
        }
        let (o, dst) = run_on(&PowerPoint, &t, "deck.pptx", &deck(false, false), &p);
        assert!(o.skipped.is_none(), "{o:?}");
        assert!(o.lossy);
        unzip_ok(&dst);
        let all = read_all(&dst);
        let n = names(&all);
        for gone in [
            "ppt/notesSlides/notesSlide1.xml",
            "ppt/comments/comment1.xml",
            "ppt/slides/slide2.xml",
            "ppt/fonts/font1.fntdata",
            "customXml/item1.xml",
            "customXml/itemProps1.xml",
        ] {
            assert!(!n.contains(&gone), "{gone} should be gone");
        }
        assert!(n.contains(&"ppt/media/image1.png"));
        let pres = part(&all, "ppt/presentation.xml").unwrap();
        assert!(!pres.contains("rId3") && !pres.contains("embeddedFontLst") && !pres.contains("embedTrueTypeFonts"));
        assert!(!pres.contains("p14:sldId id=\"257\"") && pres.contains("p14:sldId id=\"256\""));
        assert!(pres.contains("<p:sld r:id=\"rId2\"/>"));
    }

    #[test]
    fn labelled_files_keep_labels_and_custom_xml() {
        let t = TempDir::new("ppt3").unwrap();
        let src = deck(true, false);
        let p = ParamValues::new().with(P_CUSTOM_XML, ParamValue::Bool(true));
        let (o, dst) = run_on(&PowerPoint, &t, "deck.pptx", &src, &p);
        assert!(o.skipped.is_none(), "{o:?}");
        let all = read_all(&dst);
        let before = read_all(&t.join("deck.pptx"));
        assert_eq!(part(&all, "docProps/custom.xml"), part(&before, "docProps/custom.xml"));
        assert!(part(&all, "customXml/item1.xml").is_some());
        assert!(o.notes.iter().any(|n| n.contains("sensitivity label")));
    }

    #[test]
    fn skips_signed_encrypted_and_misnamed() {
        let t = TempDir::new("ppt4").unwrap();
        let (o, _) = run_on(&PowerPoint, &t, "signed.pptx", &deck(false, true), &ParamValues::new());
        assert!(o.skipped.unwrap().contains("signed"));
        let cfb = mj_formats::cfb::build_test_file(&[("EncryptionInfo", b"x"), ("EncryptedPackage", b"y")], &[]);
        let (o, _) = run_on(&PowerPoint, &t, "enc.pptx", &cfb, &ParamValues::new());
        assert!(o.skipped.unwrap().contains("encrypted"));
        let docx =
            build(&[("[Content_Types].xml", content_types(&[]).into_bytes()), ("word/document.xml", b"<w/>".to_vec())]);
        let (o, _) = run_on(&PowerPoint, &t, "renamed.pptx", &docx, &ParamValues::new());
        assert!(o.skipped.unwrap().contains("Word document"));
        let (o, _) = run_on(&PowerPoint, &t, "garbage.pptx", b"not a zip at all", &ParamValues::new());
        assert!(o.skipped.is_some());
    }

    #[test]
    fn estimate_and_settings() {
        let t = TempDir::new("ppt5").unwrap();
        let p = t.write("deck.pptx", &deck(false, false)).unwrap();
        let e = FileEntry::new(&p, mj_core::EntryKind::File, std::fs::metadata(&p).unwrap().len());
        let est = PowerPoint.estimate(&e, &ParamValues::new()).unwrap();
        assert!(est.saving() > 20_000, "{est:?}");
        let info = PowerPoint.info();
        let off: Vec<_> =
            info.settings.iter().filter(|s| s.disabled_reason.is_some()).map(|s| s.key.as_str()).collect();
        assert!(off.contains(&"downsample_pictures") && off.contains(&"subset_fonts"));
        assert!(info
            .settings
            .iter()
            .filter(|s| s.lossy)
            .all(|s| matches!(s.kind, mj_core::SettingKind::Toggle { default: false })));
        let cols = columns(&e);
        assert!(cols["unused_layouts"].contains("Layout 3"));
        assert!(cols["breakdown"].contains("images"));
    }
}
