//! Verification of a rewritten PDF against its original.
//!
//! Two independent checks, both required:
//!
//! 1. **Whole-graph equivalence.** The original, with the same removals
//!    applied, is walked in lock step with the output from the trailer:
//!    every reachable object must match (references through a consistent
//!    mapping, dictionaries key by key, streams by canonical content with
//!    `/Length`, `/Filter` and `/DecodeParms` free to differ only as the pass
//!    is allowed to change them).
//! 2. **Targeted invariants** against the untouched original: same page
//!    count and leaf order, every page's content streams identical once
//!    decoded, the same sets of embedded fonts and images by decoded-content
//!    hash, and the catalog keys preserved except those a removal targets.

use std::collections::{BTreeSet, HashMap};

use mj_core::hash::Xxh64;

use super::document::Document;
use super::filter::{self, Canonical};
use super::object::{Dict, Object};
use super::structural::{self, Options, LAYOUT_KEYS};

/// Verify `output` against `original` for a pass run with `opts`.
pub fn verify(original: &[u8], output: &[u8], opts: &Options) -> Result<(), String> {
    let orig = Document::load(original).map_err(|e| format!("original no longer parses: {e}"))?;
    let out = Document::load(output).map_err(|e| format!("output does not parse: {e}"))?;
    if out.xref.encrypted {
        return Err("output is encrypted".into());
    }
    if out.xref.sections != 1 {
        return Err("output has more than one revision".into());
    }
    let mut expected = orig.clone();
    let archival = structural::is_archival(&orig);
    structural::apply_removals(&mut expected, opts, archival, &mut Vec::new());
    graph_equal(&expected, &out)?;
    invariants(&orig, &out, opts, archival)
}

/// Lock-step comparison of two documents from their trailers.
pub fn graph_equal(exp: &Document, act: &Document) -> Result<(), String> {
    let mut c = Cmp { exp, act, map: HashMap::new(), work: Vec::new() };
    let keys = |d: &Dict| -> BTreeSet<Vec<u8>> {
        d.0.iter().map(|(k, _)| k.clone()).filter(|k| !LAYOUT_KEYS.contains(&k.as_slice())).collect()
    };
    if keys(&exp.trailer) != keys(&act.trailer) {
        return Err("trailer keys differ".into());
    }
    for k in keys(&exp.trailer) {
        c.value(exp.trailer.get(&k).unwrap_or(&Object::Null), act.trailer.get(&k).unwrap_or(&Object::Null), 0)
            .map_err(|e| format!("trailer /{}: {e}", String::from_utf8_lossy(&k)))?;
    }
    while let Some((a, b)) = c.work.pop() {
        let (oa, ob) = (&exp.objects[&a], &act.objects[&b]);
        c.object(oa, ob).map_err(|e| format!("object {a} (now {b}): {e}"))?;
    }
    Ok(())
}

struct Cmp<'a> {
    exp: &'a Document,
    act: &'a Document,
    /// Expected object number -> output object number.
    map: HashMap<u32, u32>,
    work: Vec<(u32, u32)>,
}

impl Cmp<'_> {
    fn deref_dangling<'o>(doc: &Document, o: &'o Object) -> &'o Object {
        match o {
            Object::Ref(r) if !doc.objects.contains_key(&r.num) => &Object::Null,
            other => other,
        }
    }

    fn value(&mut self, a: &Object, b: &Object, depth: usize) -> Result<(), String> {
        if depth > 128 {
            return Err("values nested too deeply".into());
        }
        let a = Self::deref_dangling(self.exp, a);
        let b = Self::deref_dangling(self.act, b);
        match (a, b) {
            (Object::Ref(x), Object::Ref(y)) => {
                match self.map.get(&x.num) {
                    Some(m) if *m == y.num => {}
                    Some(_) => return Err("reference maps to two different objects".into()),
                    None => {
                        self.map.insert(x.num, y.num);
                        self.work.push((x.num, y.num));
                    }
                }
                Ok(())
            }
            (Object::Array(x), Object::Array(y)) => {
                if x.len() != y.len() {
                    return Err("array lengths differ".into());
                }
                for (u, v) in x.iter().zip(y) {
                    self.value(u, v, depth + 1)?;
                }
                Ok(())
            }
            (Object::Dict(x), Object::Dict(y)) => self.dict(x, y, &[], depth),
            (Object::Stream(_), _) | (_, Object::Stream(_)) => Err("stream where a direct value was expected".into()),
            (x, y) if x == y => Ok(()),
            _ => Err("values differ".into()),
        }
    }

    fn dict(&mut self, x: &Dict, y: &Dict, ignore: &[&[u8]], depth: usize) -> Result<(), String> {
        let keys = |d: &Dict| -> BTreeSet<Vec<u8>> {
            d.0.iter().map(|(k, _)| k.clone()).filter(|k| !ignore.contains(&k.as_slice())).collect()
        };
        let (kx, ky) = (keys(x), keys(y));
        if kx != ky {
            let missing: Vec<String> = kx.difference(&ky).map(|k| String::from_utf8_lossy(k).into_owned()).collect();
            let extra: Vec<String> = ky.difference(&kx).map(|k| String::from_utf8_lossy(k).into_owned()).collect();
            return Err(format!("dictionary keys differ (missing {missing:?}, extra {extra:?})"));
        }
        for k in kx {
            let (u, v) = (x.get(&k).unwrap_or(&Object::Null), y.get(&k).unwrap_or(&Object::Null));
            self.value(u, v, depth + 1).map_err(|e| format!("/{}: {e}", String::from_utf8_lossy(&k)))?;
        }
        Ok(())
    }

    fn object(&mut self, a: &Object, b: &Object) -> Result<(), String> {
        match (a, b) {
            (Object::Stream(x), Object::Stream(y)) => {
                self.dict(&x.dict, &y.dict, &[b"Length", b"Filter", b"DecodeParms"], 0)?;
                match (filter::canonical(&x.dict, &x.data), filter::canonical(&y.dict, &y.data)) {
                    (Canonical::Plain(p), Canonical::Plain(q)) if p == q => Ok(()),
                    (Canonical::FlateParms(p, dp), Canonical::FlateParms(q, dq)) if p == q => {
                        self.dict(&dp, &dq, &[], 0)
                    }
                    (Canonical::Raw(p, f, dp), Canonical::Raw(q, g, dq)) if p == q => {
                        self.value(f.as_ref().unwrap_or(&Object::Null), g.as_ref().unwrap_or(&Object::Null), 0)?;
                        self.value(dp.as_ref().unwrap_or(&Object::Null), dq.as_ref().unwrap_or(&Object::Null), 0)
                    }
                    _ => Err("stream content differs".into()),
                }
            }
            _ => self.value(a, b, 0),
        }
    }
}

/// Hash of a canonical stream form.
fn canonical_hash(c: &Canonical) -> u64 {
    let mut h = Xxh64::new(7);
    match c {
        Canonical::Plain(b) => {
            h.update(b"P");
            h.update(b);
        }
        Canonical::FlateParms(b, p) => {
            h.update(b"F");
            h.update(&super::writer::to_bytes(&Object::Dict(p.clone())));
            h.update(b);
        }
        Canonical::Raw(b, f, p) => {
            h.update(b"R");
            for o in [f, p] {
                if let Some(o) = o {
                    h.update(&super::writer::to_bytes(o));
                }
                h.update(b"|");
            }
            h.update(b);
        }
    }
    h.finish()
}

/// Canonical content of each content stream of a page.
fn page_contents(doc: &Document, page: u32) -> Result<Vec<u64>, String> {
    let d = doc.objects.get(&page).and_then(Object::as_dict).ok_or("missing page")?;
    let items: Vec<&Object> = match d.get(b"Contents").map(|o| doc.resolve(o)) {
        None | Some(Object::Null) => Vec::new(),
        Some(s @ Object::Stream(_)) => vec![s],
        Some(Object::Array(a)) => a.iter().map(|o| doc.resolve(o)).collect(),
        // Malformed /Contents: compared by the graph check.
        Some(_) => Vec::new(),
    };
    items
        .into_iter()
        .filter(|o| **o != Object::Null)
        .map(|o| match o {
            Object::Stream(s) => Ok(canonical_hash(&filter::canonical(&s.dict, &s.data))),
            // Not a stream: viewers ignore it; the graph check compares the value itself.
            _ => Ok(0),
        })
        .collect()
}

/// Fonts (by `/FontFile*`) and images (`/Subtype /Image`) as content-hash sets.
fn fonts_and_images(doc: &Document) -> (BTreeSet<u64>, BTreeSet<u64>) {
    let mut fonts = BTreeSet::new();
    let mut images = BTreeSet::new();
    for o in doc.objects.values() {
        match o {
            Object::Stream(s) if s.dict.get(b"Subtype").is_some_and(|v| v.is_name(b"Image")) => {
                images.insert(canonical_hash(&filter::canonical(&s.dict, &s.data)));
            }
            Object::Dict(d) => {
                for k in [&b"FontFile"[..], b"FontFile2", b"FontFile3"] {
                    if let Some(Object::Stream(s)) = d.get(k).map(|v| doc.resolve(v)) {
                        fonts.insert(canonical_hash(&filter::canonical(&s.dict, &s.data)));
                    }
                }
            }
            _ => {}
        }
    }
    (fonts, images)
}

fn invariants(orig: &Document, out: &Document, opts: &Options, archival: bool) -> Result<(), String> {
    let pa = orig.pages().map_err(|e| e.to_string())?;
    let pb = out.pages().map_err(|e| e.to_string())?;
    if pa.len() != pb.len() {
        return Err(format!("page count changed ({} -> {})", pa.len(), pb.len()));
    }
    for (i, (a, b)) in pa.iter().zip(&pb).enumerate() {
        if page_contents(orig, *a)? != page_contents(out, *b)? {
            return Err(format!("content of page {} changed", i + 1));
        }
    }
    let (fa, ia) = fonts_and_images(orig);
    let (fb, ib) = fonts_and_images(out);
    if fa != fb {
        return Err("embedded fonts changed".into());
    }
    if !opts.remove_attachments && !opts.remove_piece_info && ia != ib {
        return Err("images changed".into());
    }
    if !ib.is_subset(&ia) {
        return Err("images changed".into());
    }
    let ka: BTreeSet<Vec<u8>> = orig.catalog().ok_or("no catalog")?.0.iter().map(|(k, _)| k.clone()).collect();
    let kb: BTreeSet<Vec<u8>> = out.catalog().ok_or("no catalog in output")?.0.iter().map(|(k, _)| k.clone()).collect();
    let mut may_go: Vec<&[u8]> = Vec::new();
    if !archival {
        if opts.remove_xmp {
            may_go.push(b"Metadata");
        }
        if opts.remove_attachments {
            may_go.push(b"AF");
        }
        if opts.remove_javascript {
            may_go.extend([&b"OpenAction"[..], b"AA"]);
        }
        if opts.remove_piece_info {
            may_go.push(b"PieceInfo");
        }
    }
    if !kb.is_subset(&ka) || ka.difference(&kb).any(|k| !may_go.contains(&k.as_slice())) {
        return Err("document catalog keys changed".into());
    }
    if orig.trailer.get(b"ID") != out.trailer.get(b"ID") {
        return Err("document /ID changed".into());
    }
    Ok(())
}
