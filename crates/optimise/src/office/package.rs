//! Shared OOXML package rewriting for the three Office optimisers (W3-A):
//! read with `mj_formats::zip`, rewrite parts, repack with `ZipWriter`,
//! keep `docProps/custom.xml` sensitivity labels intact (SAF-08), never
//! convert to 97-2003 formats.
//!
//! Pipeline ([`run`]): route by content (a renamed file of another kind is
//! skipped), skip encrypted / RMS-protected and signed files (SAF-07/09), open
//! the package with zip-bomb caps, apply the optimiser's operations to the
//! package model, remove what became unreachable, repack at the maximum
//! deflate level keeping the smaller of old/new per part, then verify the
//! output (opens, XML well-formed, relationship targets exist, content types
//! cover every part, untouched parts CRC-identical, `[Content_Types].xml`
//! still first). Any verification failure means "not optimised".

use std::collections::{BTreeMap, BTreeSet};
use std::io::BufReader;
use std::path::Path;

use mj_core::{
    Confidence, Error, Estimate, FileEntry, OptimiseJob, OptimiseOutput, ParamValues, Result, SettingKind, SettingSpec,
};
use mj_formats::ooxml::tree::Tree;
use mj_formats::ooxml::{self, rels, Package};
use mj_formats::sniff::FileKind;
use mj_formats::xml::{self, Edits};
use mj_formats::zip::write::repack::Caps;
use mj_formats::zip::{ZipArchive, ZipEntry, METHOD_DEFLATE, METHOD_STORED};

use crate::framework::{self, BreakdownCategory, MinGain, SizeBreakdown};
use crate::zip_repack::{repack_file, RepackPlan};

/// The package type the optimisers edit.
pub type Pkg = Package<BufReader<std::fs::File>>;

/// Parameter keys shared by the three optimisers.
pub const P_THUMBNAIL: &str = "remove_thumbnail";
pub const P_DEDUP: &str = "dedup_media";
pub const P_CHART_DATA: &str = "remove_chart_workbooks";

/// Reason shown on controls that need a picture or media codec.
pub const NEEDS_CODEC: &str =
    "Needs an image or video encoder that is not bundled; standalone pictures are handled by the image optimisers";

/// A toggle setting.
pub fn toggle(key: &str, label: &str, help: &str, default: bool, lossy: bool) -> SettingSpec {
    SettingSpec {
        key: key.into(),
        label: label.into(),
        help: Some(help.into()),
        kind: SettingKind::Toggle { default },
        advanced: false,
        lossy,
        disabled_reason: None,
    }
}

/// A visible but disabled toggle with its one-line reason.
pub fn disabled(key: &str, label: &str, reason: &str) -> SettingSpec {
    SettingSpec { disabled_reason: Some(reason.into()), ..toggle(key, label, reason, false, false) }
}

/// The settings every Office optimiser has.
pub fn common_settings() -> Vec<SettingSpec> {
    vec![
        toggle(
            P_THUMBNAIL,
            "Remove the embedded thumbnail",
            "The preview picture Explorer shows; the document content is unchanged (OPT-OFC-02)",
            true,
            false,
        ),
        toggle(
            P_DEDUP,
            "Deduplicate identical media",
            "Byte-identical pictures and media are stored once (OPT-PPT-08)",
            true,
            false,
        ),
    ]
}

fn skipped(before: u64, reason: impl Into<String>) -> OptimiseOutput {
    OptimiseOutput {
        bytes_before: before,
        bytes_after: before,
        lossy: false,
        skipped: Some(reason.into()),
        notes: Vec::new(),
    }
}

/// What an operation pass reports.
#[derive(Debug, Default)]
pub struct OpReport {
    pub notes: Vec<String>,
    /// True when a lossy (content-removing) operation changed something.
    pub lossy: bool,
}

/// Run the pipeline for `job`. `ops` applies the optimiser-specific operations.
pub fn run(
    job: &OptimiseJob<'_>,
    expected: FileKind,
    ops: &dyn Fn(&mut Pkg, &ParamValues, &mut OpReport) -> Result<()>,
) -> Result<OptimiseOutput> {
    let before = std::fs::metadata(job.src).map_err(|e| Error::io(job.src, e))?.len();
    let kind = framework::content_kind(job.src)?;
    if kind == Some(FileKind::Cfb) {
        let i = ooxml::inspect(job.src).unwrap_or_default();
        return Ok(skipped(
            before,
            if i.encrypted {
                "encrypted or rights-protected (RMS) file"
            } else {
                "legacy 97-2003 file (not rewritten)"
            },
        ));
    }
    if kind != Some(expected) {
        let what = kind.map(|k| k.label()).unwrap_or("unrecognised content");
        return Ok(skipped(before, format!("content is {what}, not a {}", expected.label())));
    }
    let insp = match ooxml::inspect(job.src) {
        Ok(i) => i,
        Err(e) => return Ok(skipped(before, format!("not a readable package: {e}"))),
    };
    if insp.encrypted {
        return Ok(skipped(before, "encrypted file"));
    }
    if insp.signed {
        return Ok(skipped(before, "digitally signed file (a rewrite would invalidate the signature)"));
    }
    let mut rep = OpReport::default();
    let attempt = |with_ops: bool, rep: &mut OpReport| -> Result<(Pkg, bool)> {
        let mut pkg = Package::open_file(job.src, Caps::default())?;
        let reach = pkg.reachable()?;
        if job.params.bool(P_THUMBNAIL, true) {
            remove_thumbnail(&mut pkg, rep)?;
        }
        if job.params.bool(P_DEDUP, true) {
            dedup_media(&mut pkg, rep)?;
        }
        if with_ops {
            ops(&mut pkg, job.params, rep)?;
        }
        let gone = pkg.collect_garbage(&reach)?;
        if !gone.is_empty() {
            rep.notes.push(format!("{} unreferenced part(s) removed", gone.len()));
        }
        let first = pkg.entries().first().map(|e| e.name == ooxml::content_types::PART).unwrap_or(false);
        Ok((pkg, first))
    };
    let (pkg, ct_first) = match attempt(true, &mut rep) {
        Ok(x) => x,
        Err(Error::Cancelled) => return Err(Error::Cancelled),
        Err(e) => {
            // An operation could not cope with this file: fall back to the safe subset.
            rep = OpReport::default();
            rep.notes.push(format!("optional operations skipped: {e}"));
            match attempt(false, &mut rep) {
                Ok(x) => x,
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(e) => return Ok(skipped(before, format!("not optimised: {e}"))),
            }
        }
    };
    if insp.labelled {
        rep.notes.push("sensitivity label and custom properties preserved".into());
    }
    let plan = RepackPlan { replace: pkg.replaced().clone(), remove: pkg.removed().clone(), stored: BTreeSet::new() };
    let unchanged: BTreeSet<String> = pkg
        .entries()
        .iter()
        .map(|e| e.name.clone())
        .filter(|n| !plan.replace.contains_key(n) && !plan.remove.contains(n))
        .collect();
    drop(pkg);
    let out = match repack_file(job.src, job.dst, &plan, Caps::default(), job.cancel) {
        Ok(o) => o,
        Err(Error::Cancelled) => return Err(Error::Cancelled),
        Err(e) => return Ok(skipped(before, format!("not optimised: {e}"))),
    };
    let verdict = (|| -> Result<()> {
        let a = BufReader::new(std::fs::File::open(job.src).map_err(|e| Error::io(job.src, e))?);
        let b = BufReader::new(std::fs::File::open(job.dst).map_err(|e| Error::io(job.dst, e))?);
        ooxml::verify::verify(a, b, &unchanged, ct_first.then_some(ooxml::content_types::PART), Caps::default())
    })();
    if let Err(e) = verdict {
        let _ = std::fs::remove_file(job.dst);
        return Ok(skipped(before, format!("not optimised: {e}")));
    }
    if let Some(reason) = MinGain::default().reject_reason(before, out.bytes_after) {
        let _ = std::fs::remove_file(job.dst);
        return Ok(skipped(before, reason));
    }
    rep.notes.insert(0, "repacked at the maximum deflate level".into());
    Ok(OptimiseOutput {
        bytes_before: before,
        bytes_after: out.bytes_after,
        lossy: rep.lossy,
        skipped: None,
        notes: rep.notes,
    })
}

// ---------------------------------------------------------------- helpers

/// The main document part (target of the root `officeDocument` relationship).
pub fn main_part(pkg: &mut Pkg) -> Result<String> {
    pkg.rels("")?
        .into_iter()
        .filter(|r| !r.external && r.kind() == "officeDocument")
        .find_map(|r| rels::resolve("", &r.target).and_then(|t| pkg.canonical(&t)))
        .ok_or_else(|| Error::parse("OOXML package", "no main document part"))
}

/// Internal relationships of `source` of a kind, with their resolved targets.
pub fn targets(pkg: &mut Pkg, source: &str, kind: &str) -> Result<Vec<(rels::Relationship, String)>> {
    let list = pkg.rels(source)?;
    Ok(list
        .into_iter()
        .filter(|r| !r.external && r.kind() == kind)
        .filter_map(|r| {
            let t = rels::resolve(source, &r.target).and_then(|t| pkg.canonical(&t))?;
            Some((r, t))
        })
        .collect())
}

/// Read a part as UTF-8 XML, let `f` compute edits, and store the result when it changed.
pub fn edit_part(pkg: &mut Pkg, part: &str, f: impl FnOnce(&str, &Tree) -> Result<Edits>) -> Result<bool> {
    let bytes = pkg.read(part)?;
    let x = xml::as_utf8(&bytes)?;
    let tree = Tree::parse(x.text)?;
    let edits = f(x.text, &tree)?;
    if edits.is_empty() {
        return Ok(false);
    }
    let out = xml::apply_edits(&x, edits)?;
    xml::check_well_formed(&out)?;
    pkg.replace(part, out)?;
    Ok(true)
}

/// Remove relationship `rid` of `source` and every element of `source` that references
/// it (with empty `ext` / `extLst` wrappers). Binary sources only lose the relationship.
pub fn remove_rel_and_refs(pkg: &mut Pkg, source: &str, rid: &str) -> Result<()> {
    if is_xml_part(source) {
        edit_part(pkg, source, |_, t| {
            let mut e = Edits::new();
            let mut removed = Vec::new();
            for i in t.referencing(rid) {
                t.remove_element(i, &mut e, &mut removed);
            }
            Ok(e)
        })?;
    }
    pkg.remove_relationship(source, rid)
}

/// True for parts with XML content.
pub fn is_xml_part(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    l.ends_with(".xml") || l.ends_with(".rels") || l.ends_with(".vml")
}

/// Set (or add) an unprefixed attribute on the start tag of element `i`.
pub fn set_attr(text: &str, t: &Tree, i: usize, name: &str, value: &str, e: &mut Edits) {
    let el = &t.elements[i];
    if let Some(a) = el.attrs.iter().find(|a| a.name == name) {
        if a.value != value {
            e.replace(a.value_span.clone(), xml::escape_attr(value).into_owned());
        }
        return;
    }
    let tag = &text[el.start_tag.clone()];
    let at = el.start_tag.end - if tag.ends_with("/>") { 2 } else { 1 };
    e.insert(at, format!(" {name}=\"{}\"", xml::escape_attr(value)));
}

/// Remove an attribute (and the whitespace before it) from element `i`.
pub fn remove_attr(text: &str, t: &Tree, i: usize, pred: impl Fn(&ooxml::tree::Attr) -> bool, e: &mut Edits) {
    for a in t.elements[i].attrs.iter().filter(|a| pred(a)) {
        let mut s = a.span.start;
        while s > 0 && matches!(text.as_bytes()[s - 1], b' ' | b'\t' | b'\r' | b'\n') {
            s -= 1;
        }
        e.delete(s..a.span.end);
    }
}

// ---------------------------------------------------------------- common operations

/// OPT-OFC-02: drop the root thumbnail relationship (the part goes by garbage collection).
pub fn remove_thumbnail(pkg: &mut Pkg, rep: &mut OpReport) -> Result<()> {
    let thumbs: Vec<String> = pkg.rels("")?.into_iter().filter(|r| r.kind() == "thumbnail").map(|r| r.id).collect();
    for id in &thumbs {
        pkg.remove_relationship("", id)?;
    }
    if !thumbs.is_empty() {
        rep.notes.push("embedded thumbnail removed".into());
    }
    Ok(())
}

fn is_media_name(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    !is_xml_part(&l) && (l.contains("/media/") || l.ends_with(".bin") && l.contains("/embeddings/"))
}

/// OPT-PPT-08 (all formats): byte-identical media parts are stored once; every
/// relationship to a duplicate is pointed at the first copy.
pub fn dedup_media(pkg: &mut Pkg, rep: &mut OpReport) -> Result<()> {
    let mut groups: BTreeMap<(u64, u32), Vec<String>> = BTreeMap::new();
    for e in pkg.entries().to_vec() {
        if !e.is_dir() && e.uncompressed_size > 0 && is_media_name(&e.name) && pkg.canonical(&e.name).is_some() {
            groups.entry((e.uncompressed_size, e.crc32)).or_default().push(e.name.clone());
        }
    }
    let mut merged = 0usize;
    let mut saved = 0u64;
    let mut edges = None;
    for (_, names) in groups.into_iter().filter(|(_, v)| v.len() > 1) {
        let keeper = names[0].clone();
        let kdata = pkg.read(&keeper)?;
        let kext = keeper.rsplit_once('.').map(|(_, x)| x.to_ascii_lowercase());
        for dup in &names[1..] {
            // Same content type (extension) only, and byte-identical.
            if dup.rsplit_once('.').map(|(_, x)| x.to_ascii_lowercase()) != kext || pkg.read(dup)? != kdata {
                continue;
            }
            if edges.is_none() {
                edges = Some(pkg.edges()?);
            }
            let incoming: Vec<_> = edges
                .as_ref()
                .into_iter()
                .flatten()
                .filter(|e| e.target.as_deref() == Some(dup.as_str()))
                .cloned()
                .collect();
            for e in incoming {
                pkg.retarget(&e.source, &e.rel.id, &keeper)?;
            }
            merged += 1;
            saved += pkg.entry(dup).map(|e| e.compressed_size).unwrap_or(0);
        }
    }
    if merged > 0 {
        rep.notes.push(format!("{merged} duplicate media part(s) merged ({})", mj_core::size::format_bytes(saved)));
    }
    Ok(())
}

/// OPT-PPT-06 / OPT-DOC-02 (partial): remove the embedded workbook behind each chart,
/// keeping the chart's cached values; charts without cached values are left alone.
pub fn remove_chart_workbooks(pkg: &mut Pkg, rep: &mut OpReport) -> Result<()> {
    let charts: Vec<String> = pkg.parts().into_iter().filter(|p| is_chart_part(p)).collect();
    let mut n = 0;
    for c in charts {
        let packs = targets(pkg, &c, "package")?;
        if packs.is_empty() {
            continue;
        }
        let text = pkg.read(&c)?;
        let cached = text.windows(9).any(|w| w == b"numCache>" || w == b"strCache>");
        if !cached {
            rep.notes.push(format!("{c}: no cached values, embedded workbook kept"));
            continue;
        }
        for (r, _) in packs {
            remove_rel_and_refs(pkg, &c, &r.id)?;
            n += 1;
        }
    }
    if n > 0 {
        rep.lossy = true;
        rep.notes.push(format!(
            "{n} embedded chart workbook(s) removed (charts keep their values; data can no longer be edited)"
        ));
    }
    Ok(())
}

fn is_chart_part(p: &str) -> bool {
    let l = p.to_ascii_lowercase();
    l.contains("/charts/chart") && l.ends_with(".xml")
}

// ---------------------------------------------------------------- breakdown and estimates

/// Category of a part (OPT-ENG-09).
pub fn category(name: &str) -> BreakdownCategory {
    let l = name.to_ascii_lowercase();
    let ext = l.rsplit_once('.').map(|(_, x)| x).unwrap_or("");
    if l.contains("thumbnail") || l.contains("pivotcache") {
        return BreakdownCategory::Caches;
    }
    if l.contains("/fonts/") || matches!(ext, "fntdata" | "odttf" | "ttf" | "otf") {
        return BreakdownCategory::Fonts;
    }
    if l.contains("/embeddings/") || l.contains("/activex/") || ext == "bin" {
        return BreakdownCategory::Objects;
    }
    if matches!(ext, "png" | "jpg" | "jpeg" | "gif" | "bmp" | "tif" | "tiff" | "emf" | "wmf" | "svg" | "wdp" | "jfif") {
        return BreakdownCategory::Images;
    }
    if matches!(ext, "mp4" | "m4v" | "mov" | "wmv" | "avi" | "mp3" | "m4a" | "wav" | "wma" | "mpg" | "mpeg") {
        return BreakdownCategory::Media;
    }
    if l.ends_with("styles.xml") || l.contains("/theme/") {
        return BreakdownCategory::Styles;
    }
    if l.contains("/slides/")
        || l.contains("/worksheets/")
        || l.ends_with("document.xml")
        || l.contains("sharedstrings")
    {
        return BreakdownCategory::Content;
    }
    BreakdownCategory::Other
}

/// Stored bytes per category, from the ZIP directory.
pub fn breakdown(entries: &[ZipEntry]) -> SizeBreakdown {
    let mut b = SizeBreakdown::default();
    for e in entries.iter().filter(|e| !e.is_dir()) {
        b.add(category(&e.name), e.compressed_size);
    }
    b
}

/// Open the ZIP directory of a file (cheap: the central directory only).
pub fn directory(path: &Path) -> Result<Vec<ZipEntry>> {
    let f = std::fs::File::open(path).map_err(|e| Error::io(path, e))?;
    Ok(ZipArchive::new(BufReader::new(f))?.entries().to_vec())
}

/// Cheap estimate model shared by the three optimisers: recompression of XML parts,
/// the thumbnail and duplicate media; `extra` adds optimiser-specific savings.
pub fn estimate(entry: &FileEntry, params: &ParamValues, extra: u64) -> Result<Estimate> {
    let mut est = Estimate {
        files: 1,
        bytes_before: entry.size,
        bytes_after: entry.size,
        confidence: Confidence::Estimated,
        skipped: 0,
    };
    let insp = ooxml::inspect(&entry.path)?;
    if insp.encrypted || insp.signed || !insp.zip {
        est.skipped = 1;
        return Ok(est);
    }
    let entries = directory(&entry.path)?;
    let mut saving = extra;
    let mut seen: BTreeSet<(u64, u32)> = BTreeSet::new();
    for e in entries.iter().filter(|e| !e.is_dir()) {
        let l = e.name.to_ascii_lowercase();
        if params.bool(P_THUMBNAIL, true) && l.starts_with("docprops/thumbnail") {
            saving += e.compressed_size;
            continue;
        }
        if params.bool(P_DEDUP, true) && is_media_name(&l) && !seen.insert((e.uncompressed_size, e.crc32)) {
            saving += e.compressed_size;
            continue;
        }
        if is_xml_part(&l) {
            saving += match e.method {
                METHOD_STORED if e.uncompressed_size > 256 => e.uncompressed_size - e.uncompressed_size / 4,
                METHOD_DEFLATE => e.compressed_size / 25,
                _ => 0,
            };
        }
    }
    est.bytes_after = entry.size.saturating_sub(saving);
    if !MinGain::default().accepts(est.bytes_before, est.bytes_after) {
        est.bytes_after = est.bytes_before;
    }
    Ok(est)
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! Programmatic OOXML fixtures built with the in-house writer.

    use mj_formats::zip::ZipWriter;

    pub const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    pub const PKG_R: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

    /// `(id, kind, target)` relationships; kind is the last segment of the type.
    pub fn rels(list: &[(&str, &str, &str)]) -> String {
        let mut s = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>{}<Relationships xmlns="{PKG_R}">"#,
            "\r\n"
        );
        for (id, kind, target) in list {
            let base = if kind.starts_with("http") {
                kind.to_string()
            } else if *kind == "thumbnail" {
                "http://schemas.openxmlformats.org/package/2006/relationships/metadata/thumbnail".into()
            } else {
                format!("{R}/{kind}")
            };
            s.push_str(&format!(r#"<Relationship Id="{id}" Type="{base}" Target="{target}"/>"#));
        }
        s.push_str("</Relationships>");
        s
    }

    /// Content types with the usual defaults plus overrides `(part, type)`.
    pub fn content_types(overrides: &[(&str, &str)]) -> String {
        let mut s = String::from(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Default Extension="jpeg" ContentType="image/jpeg"/><Default Extension="bin" ContentType="application/vnd.ms-office.vbaProject"/><Default Extension="fntdata" ContentType="application/x-fontdata"/><Default Extension="xlsx" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"/>"#,
        );
        for (p, t) in overrides {
            s.push_str(&format!(r#"<Override PartName="/{p}" ContentType="{t}"/>"#));
        }
        s.push_str("</Types>");
        s
    }

    /// Incompressible bytes (a stand-in for a JPEG/PNG payload).
    pub fn noise(n: usize, seed: u64) -> Vec<u8> {
        let mut x = seed | 1;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x >> 24) as u8
            })
            .collect()
    }

    /// Build a package; entries are written in order at deflate level 1 (so a repack gains).
    pub fn build(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut w = ZipWriter::new(Vec::new());
        for (n, d) in entries {
            let level = if n.ends_with(".png") || n.ends_with(".jpeg") { 0 } else { 1 };
            w.add(n, d, level, 0x6000, 0x5821).unwrap();
        }
        w.finish().unwrap()
    }

    /// Standard docProps parts and their root relationships.
    pub fn doc_props(labelled: bool) -> Vec<(&'static str, Vec<u8>)> {
        let mut v = vec![
            ("docProps/core.xml", br#"<?xml version="1.0"?><cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Fixture</dc:title></cp:coreProperties>"#.to_vec()),
            ("docProps/app.xml", br#"<?xml version="1.0"?><Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"><Application>Fixture</Application></Properties>"#.to_vec()),
            ("docProps/thumbnail.jpeg", noise(6000, 99)),
        ];
        if labelled {
            v.push(("docProps/custom.xml", br#"<?xml version="1.0"?><Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/custom-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="2" name="MSIP_Label_0e9d_Enabled"><vt:lpwstr>true</vt:lpwstr></property></Properties>"#.to_vec()));
        }
        v
    }

    /// Root relationships for a main part and the docProps.
    pub fn root_rels(main: &str, labelled: bool) -> Vec<u8> {
        let mut l = vec![
            ("rId1", "officeDocument", main),
            (
                "rId2",
                "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties",
                "docProps/core.xml",
            ),
            ("rId3", "extended-properties", "docProps/app.xml"),
            ("rId4", "thumbnail", "docProps/thumbnail.jpeg"),
        ];
        if labelled {
            l.push(("rId5", "custom-properties", "docProps/custom.xml"));
        }
        rels(&l).into_bytes()
    }

    /// The standard overrides for docProps.
    pub fn props_overrides(labelled: bool) -> Vec<(&'static str, &'static str)> {
        let mut v = vec![
            ("docProps/core.xml", "application/vnd.openxmlformats-package.core-properties+xml"),
            ("docProps/app.xml", "application/vnd.openxmlformats-officedocument.extended-properties+xml"),
        ];
        if labelled {
            v.push(("docProps/custom.xml", "application/vnd.openxmlformats-officedocument.custom-properties+xml"));
        }
        v
    }
}

#[cfg(test)]
pub(crate) mod testkit {
    use super::*;
    use mj_core::{CancelToken, NullProgress};

    /// Run an optimiser on bytes written to `dir/name`; returns the output and its path.
    pub fn run_on(
        opt: &dyn mj_core::Optimiser,
        dir: &mj_core::testutil::TempDir,
        name: &str,
        bytes: &[u8],
        params: &ParamValues,
    ) -> (OptimiseOutput, std::path::PathBuf) {
        let src = dir.write(name, bytes).unwrap();
        let dst = dir.join(format!("out-{name}"));
        let _ = std::fs::remove_file(&dst);
        let cancel = CancelToken::new();
        let tmp = dir.path().to_path_buf();
        let o = opt
            .optimise(&OptimiseJob {
                src: &src,
                dst: &dst,
                params,
                tools: &[],
                temp_dir: &tmp,
                cancel: &cancel,
                progress: &NullProgress,
            })
            .unwrap();
        if o.skipped.is_some() {
            assert!(!dst.exists(), "a skipped run must not leave an output");
        }
        (o, dst)
    }

    /// Names and contents of an output package.
    pub fn read_all(path: &Path) -> Vec<(String, Vec<u8>)> {
        let mut z = ZipArchive::new(std::io::Cursor::new(std::fs::read(path).unwrap())).unwrap();
        (0..z.entries().len()).map(|i| (z.entries()[i].name.clone(), z.read(i, 1 << 30).unwrap())).collect()
    }

    /// Content of one part, if present.
    pub fn part(all: &[(String, Vec<u8>)], name: &str) -> Option<String> {
        all.iter().find(|(n, _)| n == name).map(|(_, d)| String::from_utf8_lossy(d).into_owned())
    }

    /// Independent check with `unzip -t` when available.
    pub fn unzip_ok(path: &Path) {
        if let Ok(out) = std::process::Command::new("unzip").arg("-t").arg(path).output() {
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stdout));
        }
    }
}
