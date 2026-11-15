//! In-memory OOXML package model over a capped [`SourceArchive`]: parts are read
//! lazily and changed by replacement or removal; the relationships graph gives
//! reachability from the root relationships; removal is safe (a part still
//! targeted by a relationship of a part that stays cannot be removed, and its
//! own relationships part and content-type override go with it).

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::io::{Read, Seek};

use mj_core::{Error, Result};

use super::content_types::{self, ContentTypes};
use super::rels::{self, Relationship};
use crate::zip::write::repack::{Caps, SourceArchive};
use crate::zip::ZipEntry;

/// Parts whose bytes must never change (SAF-08: sensitivity labels and custom properties).
pub const PROTECTED_PARTS: &[&str] = &["docProps/custom.xml", "docMetadata/LabelInfo.xml"];

/// True for a protected part name.
pub fn is_protected(name: &str) -> bool {
    PROTECTED_PARTS.iter().any(|p| p.eq_ignore_ascii_case(name))
        || name.to_ascii_lowercase().starts_with("_xmlsignatures/")
}

/// One relationship with its source part and resolved internal target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    /// Source part (`""` for the package root).
    pub source: String,
    pub rel: Relationship,
    /// Canonical name of the target part when internal and present.
    pub target: Option<String>,
}

/// An OOXML package being edited.
pub struct Package<R: Read + Seek> {
    src: SourceArchive<R>,
    entries: Vec<ZipEntry>,
    index: HashMap<String, usize>,
    replaced: BTreeMap<String, Vec<u8>>,
    removed: BTreeSet<String>,
    cache: HashMap<String, Vec<u8>>,
}

const CACHE_LIMIT: u64 = 32 << 20;

impl Package<std::io::BufReader<std::fs::File>> {
    /// Open a file.
    pub fn open_file(path: &std::path::Path, caps: Caps) -> Result<Self> {
        Package::new(SourceArchive::open_file(path, caps)?)
    }
}

impl Package<std::io::Cursor<Vec<u8>>> {
    /// Open in-memory bytes.
    pub fn open_bytes(bytes: Vec<u8>, caps: Caps) -> Result<Self> {
        Package::new(SourceArchive::open_bytes(bytes, caps)?)
    }
}

impl<R: Read + Seek> Package<R> {
    /// Wrap an opened archive. It must hold `[Content_Types].xml`.
    pub fn new(src: SourceArchive<R>) -> Result<Self> {
        let entries = src.entries().to_vec();
        let mut index = HashMap::new();
        for (i, e) in entries.iter().enumerate() {
            if index.insert(e.name.to_ascii_lowercase(), i).is_some() {
                return Err(Error::parse("OOXML package", format!("two parts named {}", e.name)));
            }
        }
        if !index.contains_key(&content_types::PART.to_ascii_lowercase()) {
            return Err(Error::parse("OOXML package", "no [Content_Types].xml"));
        }
        Ok(Package { src, entries, index, replaced: BTreeMap::new(), removed: BTreeSet::new(), cache: HashMap::new() })
    }

    /// Source entries in stored order.
    pub fn entries(&self) -> &[ZipEntry] {
        &self.entries
    }

    /// Canonical name of a present part (OPC names compare case-insensitively).
    pub fn canonical(&self, name: &str) -> Option<String> {
        let i = *self.index.get(&name.to_ascii_lowercase())?;
        let n = &self.entries[i].name;
        (!self.removed.contains(n)).then(|| n.clone())
    }

    /// The source entry of a part.
    pub fn entry(&self, name: &str) -> Option<&ZipEntry> {
        self.index.get(&name.to_ascii_lowercase()).map(|&i| &self.entries[i])
    }

    /// Present parts in stored order (directories excluded).
    pub fn parts(&self) -> Vec<String> {
        self.entries.iter().filter(|e| !e.is_dir() && !self.removed.contains(&e.name)).map(|e| e.name.clone()).collect()
    }

    /// Current content of a part.
    pub fn read(&mut self, name: &str) -> Result<Vec<u8>> {
        let canon = self.canonical(name).ok_or_else(|| Error::parse("OOXML package", format!("no part {name}")))?;
        if let Some(d) = self.replaced.get(&canon) {
            return Ok(d.clone());
        }
        if let Some(d) = self.cache.get(&canon) {
            return Ok(d.clone());
        }
        let i = self.index[&canon.to_ascii_lowercase()];
        let d = self.src.read(i)?;
        if d.len() as u64 <= CACHE_LIMIT {
            self.cache.insert(canon, d.clone());
        }
        Ok(d)
    }

    /// Replace a part's content (protected parts are refused).
    pub fn replace(&mut self, name: &str, data: Vec<u8>) -> Result<()> {
        let canon = self.canonical(name).ok_or_else(|| Error::parse("OOXML package", format!("no part {name}")))?;
        if is_protected(&canon) {
            return Err(Error::Internal(format!("{canon} is protected and must not change")));
        }
        self.cache.remove(&canon);
        self.replaced.insert(canon, data);
        Ok(())
    }

    /// Parts replaced so far (by canonical name).
    pub fn replaced(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.replaced
    }

    /// Parts removed so far.
    pub fn removed(&self) -> &BTreeSet<String> {
        &self.removed
    }

    /// True when nothing has changed.
    pub fn is_unchanged(&self) -> bool {
        self.replaced.is_empty() && self.removed.is_empty()
    }

    /// Parsed `[Content_Types].xml`.
    pub fn content_types(&mut self) -> Result<ContentTypes> {
        ContentTypes::parse(&self.read(content_types::PART)?)
    }

    /// Relationships of `source` (`""` = root); empty when it has no rels part.
    pub fn rels(&mut self, source: &str) -> Result<Vec<Relationship>> {
        match self.canonical(&rels::rels_part_of(source)) {
            Some(p) => rels::parse(&self.read(&p)?),
            None => Ok(Vec::new()),
        }
    }

    /// Every relationship of every present part.
    pub fn edges(&mut self) -> Result<Vec<Edge>> {
        let mut out = Vec::new();
        for p in self.parts() {
            let Some(source) = rels::source_of_rels(&p) else { continue };
            if !source.is_empty() && self.canonical(&source).is_none() {
                continue; // relationships of a part that is not there
            }
            for rel in rels::parse(&self.read(&p)?)? {
                let target = if rel.external {
                    None
                } else {
                    rels::resolve(&source, &rel.target).and_then(|t| self.canonical(&t))
                };
                out.push(Edge { source: source.clone(), rel, target });
            }
        }
        Ok(out)
    }

    /// Parts reachable from the root relationships (plus their rels parts and the content types).
    pub fn reachable(&mut self) -> Result<BTreeSet<String>> {
        let edges = self.edges()?;
        let mut by_source: HashMap<String, Vec<String>> = HashMap::new();
        for e in &edges {
            if let Some(t) = &e.target {
                by_source.entry(e.source.to_ascii_lowercase()).or_default().push(t.clone());
            }
        }
        let mut seen = BTreeSet::new();
        let mut q = VecDeque::from([String::new()]);
        while let Some(s) = q.pop_front() {
            if let Some(ts) = by_source.get(&s.to_ascii_lowercase()) {
                for t in ts {
                    if seen.insert(t.clone()) {
                        q.push_back(t.clone());
                    }
                }
            }
        }
        let mut out = seen.clone();
        for s in std::iter::once(String::new()).chain(seen) {
            if let Some(r) = self.canonical(&rels::rels_part_of(&s)) {
                out.insert(r);
            }
        }
        if let Some(ct) = self.canonical(content_types::PART) {
            out.insert(ct);
        }
        Ok(out)
    }

    /// Relationships (from present parts other than `part` itself) that target `part`.
    pub fn incoming(&mut self, part: &str) -> Result<Vec<Edge>> {
        let canon = self.canonical(part);
        Ok(self
            .edges()?
            .into_iter()
            .filter(|e| e.target.is_some() && e.target == canon && !e.source.eq_ignore_ascii_case(part))
            .collect())
    }

    /// Remove relationship `id` of `source`.
    pub fn remove_relationship(&mut self, source: &str, id: &str) -> Result<()> {
        let rp = self
            .canonical(&rels::rels_part_of(source))
            .ok_or_else(|| Error::parse("OOXML package", "no relationships part"))?;
        let b = rels::edit(&self.read(&rp)?, &[id.to_string()], &[])?;
        self.replace(&rp, b)
    }

    /// Point relationship `id` of `source` at part `target`.
    pub fn retarget(&mut self, source: &str, id: &str, target: &str) -> Result<()> {
        let rp = self
            .canonical(&rels::rels_part_of(source))
            .ok_or_else(|| Error::parse("OOXML package", "no relationships part"))?;
        let b = rels::edit(&self.read(&rp)?, &[], &[(id.to_string(), rels::relative(source, target))])?;
        self.replace(&rp, b)
    }

    fn drop_part(&mut self, canon: &str) -> Result<()> {
        if is_protected(canon) {
            return Err(Error::Internal(format!("{canon} is protected and must not be removed")));
        }
        self.removed.insert(canon.to_string());
        self.replaced.remove(canon);
        self.cache.remove(canon);
        if let Some(r) = self.canonical(&rels::rels_part_of(canon)) {
            self.removed.insert(r.clone());
            self.replaced.remove(&r);
        }
        let ctp =
            self.canonical(content_types::PART).ok_or_else(|| Error::parse("OOXML package", "no content types"))?;
        let ct = self.read(&ctp)?;
        if ContentTypes::parse(&ct)?.has_override(canon) {
            let b = content_types::remove_overrides(&ct, &[canon.to_string()])?;
            self.replace(&ctp, b)?;
        }
        Ok(())
    }

    /// Remove a part safely: refused while a part that stays targets it; its rels part
    /// and content-type override are removed with it.
    pub fn remove_part(&mut self, name: &str) -> Result<()> {
        let canon = self.canonical(name).ok_or_else(|| Error::parse("OOXML package", format!("no part {name}")))?;
        if canon.eq_ignore_ascii_case(content_types::PART) || rels::source_of_rels(&canon).is_some() {
            return Err(Error::Internal(format!("{canon} cannot be removed directly")));
        }
        let inc = self.incoming(&canon)?;
        if let Some(e) = inc.first() {
            return Err(Error::Conflict(format!(
                "{canon} is still referenced by {} ({})",
                if e.source.is_empty() { "the package" } else { &e.source },
                e.rel.id
            )));
        }
        self.drop_part(&canon)
    }

    /// Remove every part that was in `before` (a reachability snapshot taken before the
    /// edits) and is no longer reachable, repeatedly. Returns the removed parts.
    /// Parts that were already unreachable are left alone.
    pub fn collect_garbage(&mut self, before: &BTreeSet<String>) -> Result<Vec<String>> {
        let mut out = Vec::new();
        for _ in 0..=self.entries.len() {
            let now = self.reachable()?;
            let dead: Vec<String> = before
                .iter()
                .filter(|p| !now.contains(*p) && self.canonical(p).is_some())
                .filter(|p| rels::source_of_rels(p).is_none() && !p.eq_ignore_ascii_case(content_types::PART))
                .cloned()
                .collect();
            if dead.is_empty() {
                break;
            }
            for d in dead {
                if is_protected(&d) {
                    continue;
                }
                self.drop_part(&d)?;
                out.push(d);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::zip::ZipWriter;

    pub(crate) const CT_HEAD: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/>"#;

    pub(crate) fn rels_xml(rels: &[(&str, &str, &str)]) -> String {
        let mut s = String::from(
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
        );
        for (id, kind, target) in rels {
            s.push_str(&format!(r#"<Relationship Id="{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}" Target="{target}"/>"#));
        }
        s.push_str("</Relationships>");
        s
    }

    fn pkg() -> Vec<u8> {
        let mut w = ZipWriter::new(Vec::new());
        let ct = format!("{CT_HEAD}<Override PartName=\"/a/main.xml\" ContentType=\"main\"/><Override PartName=\"/a/child.xml\" ContentType=\"child\"/></Types>");
        w.add("[Content_Types].xml", ct.as_bytes(), 6, 0, 0).unwrap();
        w.add("_rels/.rels", rels_xml(&[("rId1", "officeDocument", "a/main.xml")]).as_bytes(), 6, 0, 0).unwrap();
        w.add("a/main.xml", b"<m/>", 6, 0, 0).unwrap();
        w.add(
            "a/_rels/main.xml.rels",
            rels_xml(&[("rId1", "child", "child.xml"), ("rId2", "image", "media/p.png")]).as_bytes(),
            6,
            0,
            0,
        )
        .unwrap();
        w.add("a/child.xml", b"<c/>", 6, 0, 0).unwrap();
        w.add("a/_rels/child.xml.rels", rels_xml(&[("rId1", "image", "media/q.png")]).as_bytes(), 6, 0, 0).unwrap();
        w.add("a/media/p.png", b"p", 0, 0, 0).unwrap();
        w.add("a/media/q.png", b"q", 0, 0, 0).unwrap();
        w.add("orphan.xml", b"<o/>", 6, 0, 0).unwrap();
        w.add("docProps/custom.xml", b"<Properties/>", 6, 0, 0).unwrap();
        w.finish().unwrap()
    }

    #[test]
    fn reachability_and_safe_removal() {
        let mut p = Package::open_bytes(pkg(), Caps::default()).unwrap();
        let r = p.reachable().unwrap();
        assert!(r.contains("a/main.xml") && r.contains("a/media/q.png") && r.contains("a/_rels/child.xml.rels"));
        assert!(!r.contains("orphan.xml"));
        assert!(matches!(p.remove_part("a/child.xml"), Err(Error::Conflict(_))), "still referenced");
        assert!(p.remove_part("docProps/custom.xml").is_err(), "protected");
        assert!(p.replace("docProps/custom.xml", b"<x/>".to_vec()).is_err());
        p.remove_relationship("a/main.xml", "rId1").unwrap();
        let gone = p.collect_garbage(&r).unwrap();
        assert_eq!(gone, vec!["a/child.xml".to_string(), "a/media/q.png".to_string()]);
        assert!(p.canonical("a/_rels/child.xml.rels").is_none());
        assert!(p.canonical("orphan.xml").is_some(), "unreachable before: left alone");
        let ct = p.content_types().unwrap();
        assert!(!ct.has_override("a/child.xml"));
        assert!(ct.has_override("a/main.xml"));
        assert_eq!(p.rels("a/main.xml").unwrap().len(), 1);
        // Case-insensitive names and retargeting.
        assert!(p.canonical("A/MEDIA/P.PNG").is_some());
        p.retarget("a/main.xml", "rId2", "a/media/p.png").unwrap();
        assert_eq!(p.rels("a/main.xml").unwrap()[0].target, "media/p.png");
        assert_eq!(p.incoming("a/media/p.png").unwrap().len(), 1);
    }
}
