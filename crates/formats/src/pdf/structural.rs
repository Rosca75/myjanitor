//! The lossless structural pass (OPT-PDF-03) and dead-weight removal (OPT-PDF-05).
//!
//! Steps: refuse what must not be touched (encrypted, signed, certified,
//! unparseable) -> optional removals -> drop unreachable objects -> recompress
//! FlateDecode streams at the best level when smaller and identical once
//! inflated, compress safe uncompressed streams -> merge byte-identical
//! streams -> renumber -> write one clean revision (object streams + xref
//! stream when allowed, else a classic table). Verification is separate
//! ([`super::verify`]) and runs on the bytes read back from disk.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use mj_core::hash::Xxh64;

use super::document::{contains, Document, MAX_FILE};
use super::filter::{self, FilterKind};
use super::object::{for_each_ref, map_refs, Dict, ObjRef, Object};
use super::writer::{self, Layout};

/// Options of the pass. `Default` is the safe default: lossless rewrite and
/// thumbnail removal on, every other removal off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// Pack objects into object streams with a cross-reference stream (PDF 1.5+).
    pub object_streams: bool,
    /// Recompress Flate streams and compress uncompressed ones.
    pub recompress: bool,
    /// Merge byte-identical streams.
    pub dedupe: bool,
    /// Remove page thumbnails (`/Thumb`); viewers regenerate them.
    pub remove_thumbnails: bool,
    /// Remove XMP metadata streams (`/Metadata`).
    pub remove_xmp: bool,
    /// Remove the document information dictionary (`/Info`).
    pub remove_info: bool,
    /// Remove embedded files (`/Names /EmbeddedFiles`, catalog `/AF`).
    pub remove_attachments: bool,
    /// Remove document and page JavaScript (`/Names /JavaScript`, JavaScript `/OpenAction`, `/AA`).
    pub remove_javascript: bool,
    /// Remove private application data (`/PieceInfo`).
    pub remove_piece_info: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            object_streams: true,
            recompress: true,
            dedupe: true,
            remove_thumbnails: true,
            remove_xmp: false,
            remove_info: false,
            remove_attachments: false,
            remove_javascript: false,
            remove_piece_info: false,
        }
    }
}

impl Options {
    /// True when a removal that discards user-visible information is on.
    pub fn is_lossy(&self) -> bool {
        self.remove_xmp
            || self.remove_info
            || self.remove_attachments
            || self.remove_javascript
            || self.remove_piece_info
    }
}

/// Why a file was not rewritten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    Encrypted,
    Signed(String),
    TooLarge,
    /// Could not be parsed completely (damaged, unsupported structure).
    Unparseable(String),
    NotSmaller,
}

impl SkipReason {
    /// Stable code for diagnostics.
    pub fn code(&self) -> &'static str {
        match self {
            SkipReason::Encrypted => "encrypted",
            SkipReason::Signed(_) => "signed",
            SkipReason::TooLarge => "too_large",
            SkipReason::Unparseable(_) => "unparseable",
            SkipReason::NotSmaller => "not_smaller",
        }
    }

    /// One-line human reason.
    pub fn message(&self) -> String {
        match self {
            SkipReason::Encrypted => "Encrypted or password-protected PDF: left unchanged".into(),
            SkipReason::Signed(why) => format!("Digitally signed or certified PDF ({why}): left unchanged"),
            SkipReason::TooLarge => "PDF larger than 512 MB: left unchanged".into(),
            SkipReason::Unparseable(m) => format!("PDF structure could not be fully read ({m}): left unchanged"),
            SkipReason::NotSmaller => "Already compact: a rewrite would not be smaller".into(),
        }
    }
}

/// Bytes of the input by kind (OPT-ENG-09).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Breakdown {
    pub images: u64,
    pub fonts: u64,
    pub content: u64,
    pub metadata: u64,
    pub other: u64,
}

/// What the pass did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Report {
    pub breakdown: Breakdown,
    /// Bytes no longer referenced after each removal (`"thumbnails"`, `"xmp"`, ...).
    pub removed: Vec<(&'static str, u64)>,
    pub objects_before: usize,
    pub objects_after: usize,
    pub streams_recompressed: usize,
    pub streams_compressed: usize,
    pub streams_merged: usize,
    pub object_streams: bool,
    pub notes: Vec<String>,
}

/// Result of [`optimise`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Optimised { bytes: Vec<u8>, report: Report },
    Skipped(SkipReason),
}

/// Run the pass over a whole file.
pub fn optimise(input: &[u8], opts: &Options) -> Outcome {
    if input.len() > MAX_FILE {
        return Outcome::Skipped(SkipReason::TooLarge);
    }
    let mut doc = match prepare(input) {
        Ok(d) => d,
        Err(r) => return Outcome::Skipped(r),
    };
    let mut report = Report { breakdown: breakdown(&doc, input.len() as u64), ..Report::default() };
    report.objects_before = doc.objects.len();
    let archival = is_archival(&doc);
    report.removed = apply_removals(&mut doc, opts, archival, &mut report.notes);
    drop_unreachable(&mut doc);
    if opts.recompress {
        let (re, co) = recompress(&mut doc);
        report.streams_recompressed = re;
        report.streams_compressed = co;
    }
    if opts.dedupe {
        report.streams_merged = dedupe(&mut doc);
    }
    let (objects, trailer) = renumber(&doc);
    report.objects_after = objects.len();
    let use_objstm = opts.object_streams && !archival;
    if opts.object_streams && archival {
        report.notes.push("PDF/A or PDF/X file: object streams not used, version kept".into());
    }
    let version = if use_objstm { doc.version.max((1, 5)) } else { doc.version };
    report.object_streams = use_objstm;
    let layout = if use_objstm { Layout::ObjectStreams } else { Layout::Classic };
    let bytes = writer::write_file(version, &objects, &trailer, layout);
    if bytes.len() >= input.len() {
        return Outcome::Skipped(SkipReason::NotSmaller);
    }
    Outcome::Optimised { bytes, report }
}

/// Load and run the refusal checks.
pub fn prepare(input: &[u8]) -> Result<Document, SkipReason> {
    // Cheap byte-level net first: a signature anywhere (even in an old revision) means hands off.
    if contains(input, b"/ByteRange") {
        return Err(SkipReason::Signed("signature byte range".into()));
    }
    let doc = Document::load(input).map_err(|e| SkipReason::Unparseable(error_text(&e)))?;
    if doc.xref.encrypted {
        return Err(SkipReason::Encrypted);
    }
    if doc.catalog().is_none() {
        return Err(SkipReason::Unparseable("no document catalog".into()));
    }
    if let Some(why) = signature(&doc) {
        return Err(SkipReason::Signed(why.into()));
    }
    if doc.objects.values().any(|o| o.as_dict().is_some_and(|d| d.type_name() == Some(b"XRef"))) {
        return Err(SkipReason::Unparseable("cross-reference stream referenced as an object".into()));
    }
    doc.pages().map_err(|e| SkipReason::Unparseable(error_text(&e)))?;
    Ok(doc)
}

fn error_text(e: &mj_core::Error) -> String {
    match e {
        mj_core::Error::Parse { message, .. } => message.clone(),
        other => other.to_string(),
    }
}

/// Every dictionary in the document, direct ones included (iterative walk).
fn each_dict<'a>(doc: &'a Document, mut f: impl FnMut(&'a Dict)) {
    let mut stack: Vec<&Object> = doc.objects.values().collect();
    while let Some(o) = stack.pop() {
        match o {
            Object::Dict(d) => {
                f(d);
                stack.extend(d.0.iter().map(|(_, v)| v));
            }
            Object::Stream(s) => {
                f(&s.dict);
                stack.extend(s.dict.0.iter().map(|(_, v)| v));
            }
            Object::Array(a) => stack.extend(a.iter()),
            _ => {}
        }
    }
}

/// Why the document counts as signed or certified, if it does.
pub fn signature(doc: &Document) -> Option<&'static str> {
    if let Some(cat) = doc.catalog() {
        if cat.contains(b"Perms") {
            return Some("usage rights or certification");
        }
        if let Some(Object::Dict(af)) = doc.get(cat, b"AcroForm") {
            if doc.get(af, b"SigFlags").and_then(Object::as_int).is_some_and(|f| f & 1 != 0) {
                return Some("signature fields");
            }
        }
    }
    let mut why = None;
    each_dict(doc, |d| {
        if why.is_some() {
            return;
        }
        if d.contains(b"ByteRange") || matches!(d.type_name(), Some(b"Sig") | Some(b"DocTimeStamp")) {
            why = Some("signature dictionary");
        } else if d.get(b"FT").is_some_and(|v| v.is_name(b"Sig")) && d.get(b"V").is_some_and(|v| *v != Object::Null) {
            why = Some("signed signature field");
        } else if d.contains(b"DocMDP") {
            why = Some("certification");
        }
    });
    why
}

/// PDF/A or PDF/X: keep the classic layout, the version and the archival metadata.
pub fn is_archival(doc: &Document) -> bool {
    if let Some(Object::Dict(info)) = doc.trailer.get(b"Info").map(|o| doc.resolve(o)) {
        if info.contains(b"GTS_PDFXVersion") {
            return true;
        }
    }
    let Some(cat) = doc.catalog() else { return false };
    match doc.get(cat, b"Metadata") {
        Some(Object::Stream(s)) => match filter::canonical(&s.dict, &s.data) {
            filter::Canonical::Plain(x) | filter::Canonical::FlateParms(x, _) => {
                contains(&x, b"pdfaid:") || contains(&x, b"pdfxid:") || contains(&x, b"GTS_PDFX")
            }
            // Cannot read the metadata: assume archival (the conservative choice).
            filter::Canonical::Raw(..) => true,
        },
        _ => false,
    }
}

fn stream_size(o: &Object) -> u64 {
    match o {
        Object::Stream(s) => s.data.len() as u64,
        _ => 0,
    }
}

/// Classify the encoded size of the input by kind.
pub fn breakdown(doc: &Document, file_size: u64) -> Breakdown {
    let mut fonts = HashSet::new();
    let mut content = HashSet::new();
    each_dict(doc, |d| {
        for k in [&b"FontFile"[..], b"FontFile2", b"FontFile3"] {
            if let Some(r) = d.get(k).and_then(Object::as_ref) {
                fonts.insert(r.num);
            }
        }
        if d.type_name() == Some(b"Page") || (d.contains(b"MediaBox") && d.contains(b"Contents")) {
            match d.get(b"Contents") {
                Some(Object::Ref(r)) => {
                    content.insert(r.num);
                }
                Some(Object::Array(a)) => content.extend(a.iter().filter_map(Object::as_ref).map(|r| r.num)),
                _ => {}
            }
        }
    });
    let mut b = Breakdown::default();
    for (num, o) in &doc.objects {
        let Object::Stream(s) = o else { continue };
        let size = s.data.len() as u64;
        let sub = s.dict.get(b"Subtype").and_then(Object::as_name);
        if sub == Some(b"Image") {
            b.images += size;
        } else if fonts.contains(num) {
            b.fonts += size;
        } else if content.contains(num) || sub == Some(b"Form") {
            b.content += size;
        } else if s.dict.type_name() == Some(b"Metadata") {
            b.metadata += size;
        }
    }
    b.other = file_size.saturating_sub(b.images + b.fonts + b.content + b.metadata);
    b
}

/// Number of every object reachable from the trailer.
pub fn reachable(doc: &Document) -> HashSet<u32> {
    let mut seen = HashSet::new();
    let mut stack: Vec<ObjRef> = Vec::new();
    for (_, v) in &doc.trailer.0 {
        for_each_ref(v, |r| stack.push(r));
    }
    while let Some(r) = stack.pop() {
        if !doc.objects.contains_key(&r.num) || !seen.insert(r.num) {
            continue;
        }
        for_each_ref(&doc.objects[&r.num], |c| {
            if !seen.contains(&c.num) {
                stack.push(c)
            }
        });
    }
    seen
}

fn drop_unreachable(doc: &mut Document) {
    let keep = reachable(doc);
    doc.objects.retain(|n, _| keep.contains(n));
}

fn reachable_bytes(doc: &Document) -> u64 {
    reachable(doc).iter().map(|n| stream_size(&doc.objects[n])).sum()
}

/// Apply the enabled removals in place. Returns the stream bytes each one
/// made unreachable. Archival (PDF/A, PDF/X) files only lose thumbnails.
pub fn apply_removals(
    doc: &mut Document,
    opts: &Options,
    archival: bool,
    notes: &mut Vec<String>,
) -> Vec<(&'static str, u64)> {
    let mut out = Vec::new();
    let Some(root) = doc.trailer.get(b"Root").and_then(Object::as_ref).map(|r| r.num) else {
        return out;
    };
    let mut before = reachable_bytes(doc);
    let mut step = |doc: &mut Document, name: &'static str, out: &mut Vec<(&'static str, u64)>| {
        let after = reachable_bytes(doc);
        out.push((name, before.saturating_sub(after)));
        before = after;
    };
    let pages = doc.pages().unwrap_or_default();
    if opts.remove_thumbnails {
        for p in &pages {
            if let Some(d) = doc.objects.get_mut(p).and_then(Object::as_dict_mut) {
                d.remove(b"Thumb");
            }
        }
        step(doc, "thumbnails", &mut out);
    }
    if archival && opts.is_lossy() {
        notes.push("PDF/A or PDF/X file: metadata, attachments, JavaScript and private data kept".into());
        return out;
    }
    if opts.remove_xmp {
        let metadata_streams: HashSet<u32> = doc
            .objects
            .iter()
            .filter(|(_, o)| matches!(o, Object::Stream(s) if s.dict.type_name() == Some(b"Metadata")))
            .map(|(n, _)| *n)
            .collect();
        for o in doc.objects.values_mut() {
            edit_dicts(o, &mut |d| {
                if d.get(b"Metadata").and_then(Object::as_ref).is_some_and(|r| metadata_streams.contains(&r.num)) {
                    d.remove(b"Metadata");
                }
            });
        }
        step(doc, "xmp", &mut out);
    }
    if opts.remove_info && doc.trailer.remove(b"Info").is_some() {
        step(doc, "info", &mut out);
    }
    if opts.remove_attachments {
        edit_names(doc, root, b"EmbeddedFiles");
        if let Some(c) = doc.objects.get_mut(&root).and_then(Object::as_dict_mut) {
            c.remove(b"AF");
        }
        notes.push("File attachment annotations on pages are kept".into());
        step(doc, "attachments", &mut out);
    }
    if opts.remove_javascript {
        edit_names(doc, root, b"JavaScript");
        let open_js = match doc.objects.get(&root).and_then(Object::as_dict).and_then(|c| c.get(b"OpenAction")) {
            Some(o) => {
                matches!(doc.resolve(o), Object::Dict(a) if a.get(b"S").is_some_and(|s| s.is_name(b"JavaScript")))
            }
            None => false,
        };
        if let Some(c) = doc.objects.get_mut(&root).and_then(Object::as_dict_mut) {
            if open_js {
                c.remove(b"OpenAction");
            }
            c.remove(b"AA");
        }
        for p in &pages {
            if let Some(d) = doc.objects.get_mut(p).and_then(Object::as_dict_mut) {
                d.remove(b"AA");
            }
        }
        notes.push("JavaScript on form fields and links is kept".into());
        step(doc, "javascript", &mut out);
    }
    if opts.remove_piece_info {
        for o in doc.objects.values_mut() {
            edit_dicts(o, &mut |d| {
                d.remove(b"PieceInfo");
            });
        }
        step(doc, "piece_info", &mut out);
    }
    out
}

/// Remove `key` from the catalog's `/Names` dictionary (direct or indirect).
fn edit_names(doc: &mut Document, root: u32, key: &[u8]) {
    let names = doc.objects.get(&root).and_then(Object::as_dict).and_then(|c| c.get(b"Names")).cloned();
    match names {
        Some(Object::Ref(r)) => {
            if let Some(d) = doc.objects.get_mut(&r.num).and_then(Object::as_dict_mut) {
                d.remove(key);
            }
        }
        Some(Object::Dict(_)) => {
            if let Some(Object::Dict(d)) =
                doc.objects.get_mut(&root).and_then(Object::as_dict_mut).and_then(|c| c.get_mut(b"Names"))
            {
                d.remove(key);
            }
        }
        _ => {}
    }
}

/// Call `f` on every dictionary inside `o` (iterative).
fn edit_dicts(o: &mut Object, f: &mut dyn FnMut(&mut Dict)) {
    let mut stack: Vec<&mut Object> = vec![o];
    while let Some(o) = stack.pop() {
        match o {
            Object::Dict(d) => {
                f(d);
                stack.extend(d.0.iter_mut().map(|(_, v)| v));
            }
            Object::Stream(s) => {
                f(&mut s.dict);
                stack.extend(s.dict.0.iter_mut().map(|(_, v)| v));
            }
            Object::Array(a) => stack.extend(a.iter_mut()),
            _ => {}
        }
    }
}

/// Streams that may gain a FlateDecode filter: not XMP (kept readable by
/// non-PDF tools and required unfiltered by PDF/A), not external (`/F`), not
/// already parameterised.
fn safe_to_compress(d: &Dict) -> bool {
    !matches!(d.type_name(), Some(b"Metadata") | Some(b"XRef") | Some(b"ObjStm"))
        && !d.contains(b"F")
        && !d.contains(b"DecodeParms")
        && !d.contains(b"FFilter")
}

/// Recompress in place. Returns (Flate streams recompressed, plain streams compressed).
fn recompress(doc: &mut Document) -> (usize, usize) {
    let mut re = 0;
    let mut co = 0;
    for o in doc.objects.values_mut() {
        let Object::Stream(s) = o else { continue };
        match filter::filter_kind(&s.dict) {
            FilterKind::None => {
                if s.data.len() < 32 || !safe_to_compress(&s.dict) {
                    continue;
                }
                let z = filter::deflate(&s.data, 10);
                // "/Filter/FlateDecode" costs 19 bytes.
                if z.len() + 19 < s.data.len() && filter::inflate(&z, s.data.len()).as_deref() == Ok(&s.data[..]) {
                    s.data = z;
                    s.dict.set(b"Filter", Object::Name(b"FlateDecode".to_vec()));
                    co += 1;
                }
            }
            FilterKind::Flate(_) => {
                let Ok(raw) = filter::inflate(&s.data, filter::MAX_DECODED) else { continue };
                let z = filter::deflate(&raw, 10);
                if z.len() < s.data.len() && filter::inflate(&z, raw.len()).as_deref() == Ok(&raw[..]) {
                    s.data = z;
                    re += 1;
                }
            }
            FilterKind::Other => {}
        }
    }
    (re, co)
}

/// Merge byte-identical streams (dictionary and data). Repeats while merging
/// makes more streams identical (an image whose masks were merged), at most 4 rounds.
fn dedupe(doc: &mut Document) -> usize {
    let mut merged = 0;
    for _ in 0..4 {
        // Bucket by (hash of dictionary + data); store only the dictionary bytes.
        let mut by_key: HashMap<u64, Vec<u32>> = HashMap::new();
        let mut dicts: HashMap<u32, Vec<u8>> = HashMap::new();
        for (n, o) in &doc.objects {
            let Object::Stream(s) = o else { continue };
            let mut d = s.dict.clone();
            d.remove(b"Length");
            let k = writer::to_bytes(&Object::Dict(d));
            let mut h = Xxh64::new(0);
            h.update(&k);
            h.update(&s.data);
            by_key.entry(h.finish()).or_default().push(*n);
            dicts.insert(*n, k);
        }
        let same = |a: u32, b: u32| {
            dicts[&a] == dicts[&b]
                && matches!((&doc.objects[&a], &doc.objects[&b]), (Object::Stream(x), Object::Stream(y)) if x.data == y.data)
        };
        let mut remap: HashMap<u32, u32> = HashMap::new();
        for nums in by_key.values() {
            if nums.len() < 2 {
                continue;
            }
            // Bounded: a hash bucket larger than 64 is compared against its first members only.
            for (i, &n) in nums.iter().enumerate().take(64) {
                if remap.contains_key(&n) {
                    continue;
                }
                for &m in &nums[i + 1..] {
                    if !remap.contains_key(&m) && same(n, m) {
                        remap.insert(m, n);
                    }
                }
            }
        }
        if remap.is_empty() {
            break;
        }
        merged += remap.len();
        let mut f = |r: ObjRef| Some(ObjRef::new(remap.get(&r.num).copied().unwrap_or(r.num)));
        for o in doc.objects.values_mut() {
            map_refs(o, &mut f);
        }
        for (_, v) in doc.trailer.0.iter_mut() {
            map_refs(v, &mut f);
        }
        for m in remap.keys() {
            doc.objects.remove(m);
        }
    }
    merged
}

/// Trailer keys that describe the old file layout and are never copied.
pub const LAYOUT_KEYS: &[&[u8]] = &[
    b"Size",
    b"Prev",
    b"XRefStm",
    b"Type",
    b"W",
    b"Index",
    b"Filter",
    b"DecodeParms",
    b"Length",
    b"DL",
    b"F",
    b"FFilter",
    b"FDecodeParms",
];

/// Breadth-first renumbering from the trailer; returns objects `1..=n` and the new trailer.
fn renumber(doc: &Document) -> (BTreeMap<u32, Object>, Dict) {
    let mut trailer = Dict::new();
    for (k, v) in &doc.trailer.0 {
        if !LAYOUT_KEYS.contains(&k.as_slice()) {
            trailer.0.push((k.clone(), v.clone()));
        }
    }
    let mut order: Vec<u32> = Vec::new();
    let mut index: HashMap<u32, u32> = HashMap::new();
    let mut queue: VecDeque<u32> = VecDeque::new();
    let visit = |r: ObjRef, index: &mut HashMap<u32, u32>, queue: &mut VecDeque<u32>, order: &mut Vec<u32>| {
        if doc.objects.contains_key(&r.num) && !index.contains_key(&r.num) {
            order.push(r.num);
            index.insert(r.num, order.len() as u32);
            queue.push_back(r.num);
        }
    };
    for (_, v) in &trailer.0 {
        let mut refs = Vec::new();
        for_each_ref(v, |r| refs.push(r));
        for r in refs {
            visit(r, &mut index, &mut queue, &mut order);
        }
    }
    while let Some(n) = queue.pop_front() {
        let mut refs = Vec::new();
        collect_refs_in_order(&doc.objects[&n], &mut refs);
        for r in refs {
            visit(r, &mut index, &mut queue, &mut order);
        }
    }
    let mut map = |r: ObjRef| index.get(&r.num).map(|n| ObjRef::new(*n));
    let mut objects = BTreeMap::new();
    for num in &order {
        let mut o = doc.objects[num].clone();
        map_refs(&mut o, &mut map);
        objects.insert(index[num], o);
    }
    for (_, v) in trailer.0.iter_mut() {
        map_refs(v, &mut map);
    }
    (objects, trailer)
}

/// References in reading order (for a deterministic, locality-friendly numbering).
fn collect_refs_in_order(o: &Object, out: &mut Vec<ObjRef>) {
    let mut stack: Vec<&Object> = vec![o];
    while let Some(o) = stack.pop() {
        match o {
            Object::Ref(r) => out.push(*r),
            Object::Array(a) => stack.extend(a.iter().rev()),
            Object::Dict(d) => stack.extend(d.0.iter().rev().map(|(_, v)| v)),
            Object::Stream(s) => stack.extend(s.dict.0.iter().rev().map(|(_, v)| v)),
            _ => {}
        }
    }
}
