//! Review thumbnails (W6-A): one small image per row of a duplicates-type step, and the
//! bounded image of the preview pane (`candidate_preview`, review finding IPC-9).
//!
//! Limits, all checked before reading pixels:
//! - online-only files are never read (the scan badge and a fresh attribute check);
//! - JPEG and PNG: input at most [`MAX_INPUT`], decoded pixels at most
//!   [`thumb::THUMB_MAX_PIXELS`], one large decode at a time ([`ThumbCache::gate`]);
//!   row thumbnails of camera JPEGs come from the EXIF embedded thumbnail found in a
//!   [`HEADER_WINDOW`] prefix, without reading the rest of the file;
//! - GIF, WebP and BMP are not decoded here: the original is sent when it is at most
//!   [`PASS_ROW_MAX`] (row) / [`PASS_PREVIEW_MAX`] (preview) and its header gives a sane
//!   size; the webview scales it;
//! - each decode runs under `catch_unwind`, every failure is a reason, never an error.
//!
//! Results (and failures) are kept in a byte-capped LRU keyed by path, size, modification
//! time and requested size; the rows of an exact-duplicate group share one thumbnail.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use mj_core::{lock, Badges, Candidate, CandidateId, Error, GroupId, GroupKind, Result, TargetKind};
use mj_dupes::decode::{thumb, DecodeError};

use crate::api::{CandidateThumbnails, CandidateThumbnailsRequest, Thumbnail};
use crate::results::unknown;
use crate::Engine;

/// Long side of a row thumbnail (shown at 48 px, 96 px on hover: sharp at 2x).
pub(crate) const ROW_SIDE: u32 = 128;
/// Long side of the preview pane image (docs/IPC.md: at most 512 px).
pub(crate) const PREVIEW_SIDE: u32 = 512;
/// Largest JPEG / PNG file read for a thumbnail.
pub(crate) const MAX_INPUT: u64 = 64 << 20;
/// Prefix read first for a JPEG row thumbnail (EXIF + embedded thumbnail).
pub(crate) const HEADER_WINDOW: u64 = 256 << 10;
/// GIF / WebP / BMP sent unchanged up to these sizes (row, preview).
pub(crate) const PASS_ROW_MAX: u64 = 256 << 10;
pub(crate) const PASS_PREVIEW_MAX: u64 = 1 << 20;
/// Images of more pixels than this take the one-at-a-time gate.
const LARGE_PIXELS: u64 = 12_000_000;
/// Bytes of cached data URLs.
const CACHE_BYTES: usize = 32 << 20;
/// Rows per `candidate_thumbnails` call.
pub(crate) const BATCH_MAX: usize = 100;

/// Why an image has no thumbnail (wire codes of `Thumbnail::reason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NoThumb {
    NotImage,
    OnlineOnly,
    TooLarge,
    Unreadable,
    Undecodable,
}

impl NoThumb {
    pub fn code(self) -> &'static str {
        match self {
            NoThumb::NotImage => "not_image",
            NoThumb::OnlineOnly => "online_only",
            NoThumb::TooLarge => "too_large",
            NoThumb::Unreadable => "unreadable",
            NoThumb::Undecodable => "undecodable",
        }
    }

    /// Sentence for the preview pane.
    pub fn sentence(self) -> &'static str {
        match self {
            NoThumb::NotImage => "No preview for this file type",
            NoThumb::OnlineOnly => "Online-only file: a preview would download it",
            NoThumb::TooLarge => "Image too large for a quick preview",
            NoThumb::Unreadable => "The file could not be read",
            NoThumb::Undecodable => "This image could not be decoded for a preview",
        }
    }
}

/// An encoded small image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Encoded {
    pub mime: &'static str,
    pub base64: String,
}

impl Encoded {
    pub fn data_url(&self) -> String {
        format!("data:{};base64,{}", self.mime, self.base64)
    }
}

type Outcome = std::result::Result<Arc<Encoded>, NoThumb>;

/// What a thumbnail needs of a candidate.
type Item = (PathBuf, Badges, TargetKind);
/// A requested row and its exact-duplicate group.
type Row = (Item, Option<GroupId>);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Key {
    path: PathBuf,
    size: u64,
    mtime: i64,
    side: u32,
}

#[derive(Default)]
struct Lru {
    map: HashMap<Key, (Outcome, u64)>,
    bytes: usize,
    tick: u64,
}

fn cost(o: &Outcome) -> usize {
    64 + o.as_ref().map_or(0, |e| e.base64.len())
}

/// Byte-capped LRU of thumbnails plus the gate that serialises large decodes.
#[derive(Default)]
pub(crate) struct ThumbCache {
    lru: Mutex<Lru>,
    gate: Mutex<()>,
}

impl ThumbCache {
    fn get(&self, k: &Key) -> Option<Outcome> {
        let mut l = lock(&self.lru);
        l.tick += 1;
        let t = l.tick;
        l.map.get_mut(k).map(|(o, at)| {
            *at = t;
            o.clone()
        })
    }

    fn put(&self, k: Key, o: Outcome) {
        let mut l = lock(&self.lru);
        l.tick += 1;
        let t = l.tick;
        l.bytes += cost(&o);
        if let Some((old, _)) = l.map.insert(k, (o, t)) {
            l.bytes -= cost(&old);
        }
        while l.bytes > CACHE_BYTES && l.map.len() > 1 {
            let Some(oldest) = l.map.iter().min_by_key(|(_, (_, at))| *at).map(|(k, _)| k.clone()) else { break };
            if let Some((o, _)) = l.map.remove(&oldest) {
                l.bytes -= cost(&o);
            }
        }
    }

    #[cfg(test)]
    fn stats(&self) -> (usize, usize) {
        let l = lock(&self.lru);
        (l.map.len(), l.bytes)
    }
}

/// Minimal standard base64 (no external crate).
pub(crate) fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Jpeg,
    Png,
    Pass,
}

fn kind_of(path: &Path) -> Option<Kind> {
    match mj_core::pathutil::extension_lower(path).as_str() {
        "jpg" | "jpeg" | "jpe" | "jfif" => Some(Kind::Jpeg),
        "png" => Some(Kind::Png),
        "gif" | "webp" | "bmp" => Some(Kind::Pass),
        _ => None,
    }
}

/// True when the extension is one this module makes thumbnails for.
pub(crate) fn is_image(path: &Path) -> bool {
    kind_of(path).is_some()
}

fn read_prefix(path: &Path, n: u64) -> std::io::Result<Vec<u8>> {
    let f = std::fs::File::open(path)?;
    let mut buf = Vec::with_capacity(n.min(1 << 20) as usize);
    f.take(n).read_to_end(&mut buf)?;
    Ok(buf)
}

fn from_decode(e: DecodeError) -> NoThumb {
    match e {
        DecodeError::TooLarge => NoThumb::TooLarge,
        _ => NoThumb::Undecodable,
    }
}

/// Pixel count announced by a JPEG / PNG header (to pick the gate), when readable.
fn announced_pixels(kind: Kind, head: &[u8]) -> Option<u64> {
    match kind {
        Kind::Jpeg => mj_formats::jpeg::walk(head)?.frame.map(|f| f.width as u64 * f.height as u64),
        Kind::Png => mj_dupes::decode::png::header(head).map(|h| h.width as u64 * h.height as u64),
        Kind::Pass => None,
    }
}

impl ThumbCache {
    /// Thumbnail of one local file (no badge checks here: see [`Engine::thumbnail_of`]).
    fn make(&self, path: &Path, len: u64, side: u32) -> Outcome {
        let kind = kind_of(path).ok_or(NoThumb::NotImage)?;
        if kind == Kind::Pass {
            let cap = if side > ROW_SIDE { PASS_PREVIEW_MAX } else { PASS_ROW_MAX };
            if len > cap {
                return Err(NoThumb::TooLarge);
            }
            let data = read_prefix(path, cap + 1).map_err(|_| NoThumb::Unreadable)?;
            if data.len() as u64 > cap {
                return Err(NoThumb::TooLarge);
            }
            let (mime, w, h) = thumb::passthrough_size(&data).ok_or(NoThumb::Undecodable)?;
            if !mj_dupes::decode::within_limits(w, h) {
                return Err(NoThumb::TooLarge);
            }
            return Ok(Arc::new(Encoded { mime, base64: base64(&data) }));
        }
        if len > MAX_INPUT {
            return Err(NoThumb::TooLarge);
        }
        let head = read_prefix(path, HEADER_WINDOW).map_err(|_| NoThumb::Unreadable)?;
        let isolated = |f: &dyn Fn() -> std::result::Result<thumb::Image, DecodeError>| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
                .unwrap_or(Err(DecodeError::Corrupt("panic".into())))
        };
        let fast = if kind == Kind::Jpeg {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| thumb::jpeg_embedded(&head, side))).ok().flatten()
        } else {
            None
        };
        let img = match fast {
            Some(img) => img,
            None => {
                let data = if head.len() as u64 >= HEADER_WINDOW {
                    read_prefix(path, MAX_INPUT + 1).map_err(|_| NoThumb::Unreadable)?
                } else {
                    head
                };
                if data.len() as u64 > MAX_INPUT {
                    return Err(NoThumb::TooLarge);
                }
                let decode = || match kind {
                    Kind::Jpeg => thumb::jpeg(&data, side, thumb::THUMB_MAX_PIXELS),
                    _ => thumb::png(&data, side, thumb::THUMB_MAX_PIXELS),
                };
                let large = announced_pixels(kind, &data).is_none_or(|p| p > LARGE_PIXELS);
                let r = if large {
                    let _one_at_a_time = lock(&self.gate);
                    isolated(&decode)
                } else {
                    isolated(&decode)
                };
                r.map_err(from_decode)?
            }
        };
        let png = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| thumb::encode_png(&img)))
            .map_err(|_| NoThumb::Undecodable)?;
        Ok(Arc::new(Encoded { mime: "image/png", base64: base64(&png) }))
    }
}

impl Engine {
    /// Thumbnail of a candidate path at `side` px: never reads an online-only file,
    /// re-checks the attributes now (the file may have been dehydrated since the scan),
    /// and serves the cache when path, size and modification time are unchanged.
    pub(crate) fn thumbnail_of(&self, path: &Path, badges: Badges, target: TargetKind, side: u32) -> Outcome {
        if target != TargetKind::File || !is_image(path) {
            return Err(NoThumb::NotImage);
        }
        if badges.contains(Badges::ONLINE_ONLY) {
            return Err(NoThumb::OnlineOnly);
        }
        let md = self.cfg.platform.metadata(path).map_err(|_| NoThumb::Unreadable)?;
        if !md.is_file() {
            return Err(NoThumb::NotImage);
        }
        if md.is_online_only() {
            return Err(NoThumb::OnlineOnly);
        }
        let stamp = md.stamp();
        let key = Key { path: path.to_path_buf(), size: stamp.size, mtime: stamp.mtime.0, side };
        if let Some(o) = self.thumbs.get(&key) {
            return o;
        }
        let o = self.thumbs.make(path, stamp.size, side);
        self.thumbs.put(key, o.clone());
        o
    }

    /// Row thumbnails of the visible page (W6-A), in request order. One thumbnail per
    /// exact-duplicate group (any local member of the group serves every row of it).
    pub fn candidate_thumbnails(&self, req: CandidateThumbnailsRequest) -> Result<CandidateThumbnails> {
        if req.candidate_ids.len() > BATCH_MAX {
            return Err(Error::InvalidInput(format!("at most {BATCH_MAX} thumbnails per call")));
        }
        // Snapshot under the read lock; all file access happens after it is released.
        let (rows, exact_members): (Vec<Option<Row>>, HashMap<GroupId, Vec<Item>>) = {
            let results = self.results.read().unwrap_or_else(|e| e.into_inner());
            let rec = results.get(&req.opportunity_id).ok_or_else(|| unknown(&req.opportunity_id))?;
            let exact: std::collections::HashSet<GroupId> =
                rec.groups.iter().filter(|g| g.kind == GroupKind::Exact).map(|g| g.id).collect();
            let by_id: HashMap<CandidateId, &Candidate> = {
                let want: std::collections::HashSet<CandidateId> = req.candidate_ids.iter().copied().collect();
                rec.candidates.iter().filter(|c| want.contains(&c.id)).map(|c| (c.id, c)).collect()
            };
            let rows: Vec<_> = req
                .candidate_ids
                .iter()
                .map(|id| {
                    by_id.get(id).map(|c| {
                        let g = c.group.filter(|g| exact.contains(g));
                        ((c.path.clone(), c.badges, c.target), g)
                    })
                })
                .collect();
            let wanted: std::collections::HashSet<GroupId> = rows.iter().flatten().filter_map(|(_, g)| *g).collect();
            let mut members: HashMap<GroupId, Vec<Item>> = HashMap::new();
            if !wanted.is_empty() {
                for c in &rec.candidates {
                    if let Some(g) = c.group.filter(|g| wanted.contains(g)) {
                        members.entry(g).or_default().push((c.path.clone(), c.badges, c.target));
                    }
                }
            }
            (rows, members)
        };
        let mut per_group: HashMap<GroupId, Outcome> = HashMap::new();
        let mut out = Vec::with_capacity(rows.len());
        for (id, row) in req.candidate_ids.iter().zip(rows) {
            let outcome = match row {
                None => {
                    out.push(Thumbnail { candidate_id: *id, data_url: None, reason: Some("unknown".into()) });
                    continue;
                }
                Some(((path, badges, target), Some(g))) => {
                    if let Some(o) = per_group.get(&g) {
                        o.clone()
                    } else {
                        // This row first, then any other member that is on this disk.
                        let mut o = self.thumbnail_of(&path, badges, target, ROW_SIDE);
                        if matches!(o, Err(NoThumb::OnlineOnly | NoThumb::Unreadable)) {
                            for (p, b, t) in exact_members.get(&g).into_iter().flatten() {
                                if p != &path {
                                    let other = self.thumbnail_of(p, *b, *t, ROW_SIDE);
                                    if other.is_ok() {
                                        o = other;
                                        break;
                                    }
                                }
                            }
                        }
                        per_group.insert(g, o.clone());
                        o
                    }
                }
                Some(((path, badges, target), None)) => self.thumbnail_of(&path, badges, target, ROW_SIDE),
            };
            out.push(match outcome {
                Ok(e) => Thumbnail { candidate_id: *id, data_url: Some(e.data_url()), reason: None },
                Err(r) => Thumbnail { candidate_id: *id, data_url: None, reason: Some(r.code().into()) },
            });
        }
        Ok(CandidateThumbnails { thumbnails: out })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn lru_is_byte_capped_and_evicts_the_least_recent() {
        let c = ThumbCache::default();
        let big = |n: usize| -> Outcome { Ok(Arc::new(Encoded { mime: "image/png", base64: "x".repeat(n) })) };
        let k = |i: u32| Key { path: PathBuf::from(format!("/p{i}")), size: 1, mtime: 0, side: ROW_SIDE };
        for i in 0..5 {
            c.put(k(i), big(10 << 20));
            if i == 1 {
                assert!(c.get(&k(0)).is_some(), "touch 0: it becomes recent");
            }
        }
        let (n, bytes) = c.stats();
        assert!(bytes <= CACHE_BYTES && n == 3, "{n} entries, {bytes} bytes");
        assert!(c.get(&k(0)).is_none() && c.get(&k(1)).is_none());
        assert!(c.get(&k(4)).is_some());
        // Replacing a key does not leak its old cost.
        c.put(k(4), Err(NoThumb::TooLarge));
        assert!(c.stats().1 < 25 << 20);
    }

    #[test]
    fn non_images_and_folders_have_no_thumbnail() {
        assert_eq!(kind_of(Path::new("/a/b.JPG")), Some(Kind::Jpeg));
        assert_eq!(kind_of(Path::new("/a/b.webp")), Some(Kind::Pass));
        assert_eq!(kind_of(Path::new("/a/b.mp4")), None);
        assert_eq!(NoThumb::OnlineOnly.code(), "online_only");
    }
}
