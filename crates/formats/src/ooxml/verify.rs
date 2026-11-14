//! Structural verification of a written OOXML package: it opens with the
//! in-house reader, every XML part is well-formed, every internal relationship
//! target exists, content types cover every part, and parts that were not meant
//! to change are CRC-identical. Problems already present in the source are
//! tolerated (they are measured on the source with the same rules), so only a
//! problem *introduced* by the rewrite fails verification.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Seek};

use mj_core::{Error, Result};

use super::content_types::{self, ContentTypes};
use super::rels;
use crate::zip::write::repack::Caps;
use crate::zip::ZipArchive;

/// Structural issues found in a package.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Issues {
    /// XML parts that are not well-formed.
    pub malformed: BTreeSet<String>,
    /// (source part, relationship id) whose internal target is missing.
    pub dangling: BTreeSet<(String, String)>,
    /// Parts without a content type.
    pub uncovered: BTreeSet<String>,
    /// Relationships parts whose source part is missing.
    pub orphan_rels: BTreeSet<String>,
}

impl Issues {
    /// Issues of `self` not present in `baseline`.
    pub fn new_since(&self, baseline: &Issues) -> Vec<String> {
        let mut v = Vec::new();
        v.extend(self.malformed.difference(&baseline.malformed).map(|p| format!("{p} is not well-formed XML")));
        v.extend(
            self.dangling
                .difference(&baseline.dangling)
                .map(|(s, id)| format!("relationship {id} of {s} points to a missing part")),
        );
        v.extend(self.uncovered.difference(&baseline.uncovered).map(|p| format!("{p} has no content type")));
        v.extend(self.orphan_rels.difference(&baseline.orphan_rels).map(|p| format!("{p} has no source part")));
        v
    }
}

fn is_xml_name(n: &str) -> bool {
    let l = n.to_ascii_lowercase();
    l.ends_with(".xml") || l.ends_with(".rels") || l.ends_with(".vml")
}

/// Content of one archive: names in order, (crc, size) by name, and the issues.
pub struct Scan {
    pub names: Vec<String>,
    pub crcs: BTreeMap<String, (u32, u64)>,
    pub issues: Issues,
}

/// Open and check a package.
pub fn scan<R: Read + Seek>(r: R, caps: Caps) -> Result<Scan> {
    let mut z = ZipArchive::new(r)?;
    if z.entries().len() as u64 > caps.max_entries {
        return Err(Error::parse("OOXML package", "too many parts"));
    }
    let names: Vec<String> = z.entries().iter().map(|e| e.name.clone()).collect();
    let lower: BTreeMap<String, String> = names.iter().map(|n| (n.to_ascii_lowercase(), n.clone())).collect();
    let mut crcs = BTreeMap::new();
    let mut issues = Issues::default();
    let mut total = 0u64;
    let mut ct: Option<ContentTypes> = None;
    let mut rels_parts: Vec<(String, Vec<u8>)> = Vec::new();
    for i in 0..names.len() {
        let e = z.entries()[i].clone();
        crcs.insert(e.name.clone(), (e.crc32, e.uncompressed_size));
        if e.is_dir() {
            continue;
        }
        // Every entry is decoded: this checks the CRC of every byte written.
        let d = z.read(i, caps.max_entry.min(caps.max_total.saturating_sub(total)))?;
        total += d.len() as u64;
        if is_xml_name(&e.name) && crate::xml::check_well_formed(&d).is_err() {
            issues.malformed.insert(e.name.clone());
            continue;
        }
        if e.name.eq_ignore_ascii_case(content_types::PART) {
            ct = ContentTypes::parse(&d).ok();
        } else if rels::source_of_rels(&e.name).is_some() {
            rels_parts.push((e.name.clone(), d));
        }
    }
    let ct = ct.ok_or_else(|| Error::parse("OOXML package", "missing or unreadable [Content_Types].xml"))?;
    for n in &names {
        if n.ends_with('/') || n.eq_ignore_ascii_case(content_types::PART) {
            continue;
        }
        if ct.type_of(n).is_none() {
            issues.uncovered.insert(n.clone());
        }
    }
    for (name, d) in rels_parts {
        let source = rels::source_of_rels(&name).unwrap_or_default();
        if !source.is_empty() && !lower.contains_key(&source.to_ascii_lowercase()) {
            issues.orphan_rels.insert(name.clone());
            continue;
        }
        let Ok(list) = rels::parse(&d) else {
            issues.malformed.insert(name.clone());
            continue;
        };
        for r in list.iter().filter(|r| !r.external) {
            let ok = rels::resolve(&source, &r.target).is_some_and(|t| lower.contains_key(&t.to_ascii_lowercase()));
            if !ok {
                issues.dangling.insert((source.clone(), r.id.clone()));
            }
        }
    }
    Ok(Scan { names, crcs, issues })
}

/// Verify `output` against `source`: it opens, it introduces no structural issue,
/// `first` (when given) is its first entry, and every part in `unchanged` has the
/// same CRC-32 and size as in the source.
pub fn verify<R1: Read + Seek, R2: Read + Seek>(
    source: R1,
    output: R2,
    unchanged: &BTreeSet<String>,
    first: Option<&str>,
    caps: Caps,
) -> Result<()> {
    let fail = |m: String| Error::parse("optimised package", format!("verification failed: {m}"));
    let s = scan(source, caps)?;
    let o = scan(output, caps).map_err(|e| fail(e.to_string()))?;
    let new = o.issues.new_since(&s.issues);
    if let Some(m) = new.first() {
        return Err(fail(m.clone()));
    }
    if let Some(f) = first {
        if o.names.first().map(String::as_str) != Some(f) {
            return Err(fail(format!("{f} is no longer the first entry")));
        }
    }
    for n in unchanged {
        if s.crcs.get(n) != o.crcs.get(n) {
            return Err(fail(format!("{n} changed although it should not have")));
        }
    }
    Ok(())
}
