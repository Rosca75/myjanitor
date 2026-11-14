//! OPC relationships parts (`_rels/*.rels`) and part-name arithmetic.

use std::ops::Range;

use mj_core::{Error, Result};

use super::tree::Tree;
use crate::xml::{as_utf8, escape_attr, Edits};

/// One `<Relationship>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relationship {
    pub id: String,
    pub rel_type: String,
    pub target: String,
    pub external: bool,
    /// Span of the whole element in the rels part text (after the BOM).
    pub span: Range<usize>,
    /// Span of the `Target` value.
    pub target_span: Range<usize>,
}

impl Relationship {
    /// The last segment of the type URI (`slideLayout`, `image`...), which is the same in
    /// the transitional and strict conformance classes.
    pub fn kind(&self) -> &str {
        self.rel_type.rsplit('/').next().unwrap_or("")
    }
}

/// Parse a relationships part.
pub fn parse(bytes: &[u8]) -> Result<Vec<Relationship>> {
    let x = as_utf8(bytes)?;
    let t = Tree::parse(x.text)?;
    let mut out = Vec::new();
    for (_, e) in t.by_local("Relationship") {
        let id = e.plain("Id").ok_or_else(|| Error::parse("relationships", "Relationship without Id"))?.to_string();
        let target_attr = e.attrs.iter().find(|a| a.name == "Target");
        let (target, target_span) = match target_attr {
            Some(a) => (a.value.clone(), a.value_span.clone()),
            None => return Err(Error::parse("relationships", format!("relationship {id} without Target"))),
        };
        out.push(Relationship {
            id,
            rel_type: e.plain("Type").unwrap_or_default().to_string(),
            target,
            external: e.plain("TargetMode").is_some_and(|m| m.eq_ignore_ascii_case("External")),
            span: e.span.clone(),
            target_span,
        });
    }
    if out.iter().enumerate().any(|(i, r)| out[..i].iter().any(|o| o.id == r.id)) {
        return Err(Error::parse("relationships", "duplicate relationship id"));
    }
    Ok(out)
}

/// The relationships part of `source` (`""` = the package root).
pub fn rels_part_of(source: &str) -> String {
    match source.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None if source.is_empty() => "_rels/.rels".into(),
        None => format!("_rels/{source}.rels"),
    }
}

/// The source part of a relationships part, or `None` when `name` is not one.
pub fn source_of_rels(name: &str) -> Option<String> {
    let (dir, file) = match name.rsplit_once('/') {
        Some((d, f)) => (d, f),
        None => return None,
    };
    let file = file.strip_suffix(".rels")?;
    let base = if dir == "_rels" { "" } else { dir.strip_suffix("/_rels")? };
    if dir != "_rels" && !dir.ends_with("/_rels") {
        return None;
    }
    if file.is_empty() {
        return if base.is_empty() { Some(String::new()) } else { None };
    }
    Some(if base.is_empty() { file.to_string() } else { format!("{base}/{file}") })
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let h = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(v) = h {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// Resolve an internal relationship target against its source part into a part
/// name (no leading `/`). `None` when it escapes the package or is empty.
pub fn resolve(source: &str, target: &str) -> Option<String> {
    let target = target.split('#').next().unwrap_or("");
    if target.is_empty() || target.contains('\\') || target.contains("://") {
        return None;
    }
    let target = percent_decode(target);
    let mut parts: Vec<&str> = Vec::new();
    if !target.starts_with('/') {
        if let Some((dir, _)) = source.rsplit_once('/') {
            parts.extend(dir.split('/'));
        }
    }
    for seg in target.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

/// A relative target from `source` to part `target` (`../media/image1.png`).
pub fn relative(source: &str, target: &str) -> String {
    let sdir: Vec<&str> = match source.rsplit_once('/') {
        Some((d, _)) => d.split('/').collect(),
        None => Vec::new(),
    };
    let t: Vec<&str> = target.split('/').collect();
    let (tdir, tfile) = t.split_at(t.len() - 1);
    let common = sdir.iter().zip(tdir).take_while(|(a, b)| a == b).count();
    let mut out = String::new();
    for _ in common..sdir.len() {
        out.push_str("../");
    }
    for d in &tdir[common..] {
        out.push_str(d);
        out.push('/');
    }
    out.push_str(tfile[0]);
    out
}

/// Edit a rels part: remove relationships by id and retarget others.
pub fn edit(bytes: &[u8], remove: &[String], retarget: &[(String, String)]) -> Result<Vec<u8>> {
    let x = as_utf8(bytes)?;
    let rels = parse(bytes)?;
    let mut e = Edits::new();
    for r in &rels {
        if remove.contains(&r.id) {
            e.delete(r.span.clone());
        } else if let Some((_, t)) = retarget.iter().find(|(id, _)| *id == r.id) {
            e.replace(r.target_span.clone(), escape_attr(t).into_owned());
        }
    }
    crate::xml::apply_edits(&x, e)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_name_arithmetic() {
        assert_eq!(rels_part_of(""), "_rels/.rels");
        assert_eq!(rels_part_of("ppt/slides/slide1.xml"), "ppt/slides/_rels/slide1.xml.rels");
        assert_eq!(rels_part_of("a.xml"), "_rels/a.xml.rels");
        assert_eq!(source_of_rels("_rels/.rels").as_deref(), Some(""));
        assert_eq!(source_of_rels("ppt/slides/_rels/slide1.xml.rels").as_deref(), Some("ppt/slides/slide1.xml"));
        assert_eq!(source_of_rels("ppt/slides/slide1.xml"), None);
        assert_eq!(source_of_rels("x/_rels/.rels"), None);
        assert_eq!(resolve("ppt/slides/slide1.xml", "../media/image1.png").as_deref(), Some("ppt/media/image1.png"));
        assert_eq!(resolve("", "ppt/presentation.xml").as_deref(), Some("ppt/presentation.xml"));
        assert_eq!(resolve("ppt/a.xml", "/docProps/app.xml").as_deref(), Some("docProps/app.xml"));
        assert_eq!(resolve("ppt/a.xml", "media/my%20pic.png#x").as_deref(), Some("ppt/media/my pic.png"));
        assert_eq!(resolve("a.xml", "../../etc"), None);
        assert_eq!(resolve("a.xml", "http://x/y"), None);
        assert_eq!(relative("ppt/slides/slide1.xml", "ppt/media/image1.png"), "../media/image1.png");
        assert_eq!(relative("ppt/presentation.xml", "ppt/slides/slide2.xml"), "slides/slide2.xml");
        assert_eq!(relative("", "docProps/core.xml"), "docProps/core.xml");
        assert_eq!(relative("word/document.xml", "word/media/a.png"), "media/a.png");
    }

    #[test]
    fn parse_and_edit_rels() {
        let x = br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://x/slideLayout" Target="../slideLayouts/slideLayout1.xml"/><Relationship Id="rId2" Type="http://x/hyperlink" Target="https://example.com" TargetMode="External"/><Relationship Id="rId3" Type="http://x/image" Target="../media/a&amp;b.png"/></Relationships>"#;
        let r = parse(x).unwrap();
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].kind(), "slideLayout");
        assert!(r[1].external);
        assert_eq!(r[2].target, "../media/a&b.png");
        let out = edit(x, &["rId1".into()], &[("rId3".into(), "../media/c&d.png".into())]).unwrap();
        let r2 = parse(&out).unwrap();
        assert_eq!(r2.len(), 2);
        assert_eq!(r2[1].target, "../media/c&d.png");
        assert!(String::from_utf8(out).unwrap().contains("c&amp;d.png"));
        let dup =
            br#"<Relationships><Relationship Id="a" Target="x"/><Relationship Id="a" Target="y"/></Relationships>"#;
        assert!(parse(dup).is_err());
    }
}
