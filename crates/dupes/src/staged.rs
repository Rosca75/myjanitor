//! Staged content hashing shared by the exact-duplicate lane (W2-B).
//!
//! Contract (ARCHITECTURE.md §6): bucket **all** files by size (cache hits
//! included, D-006); partial hash = xxh64 over head + middle + tail windows of
//! `PARTIAL_WINDOW` bytes (whole file when `size <= PARTIAL_FULL_THRESHOLD`);
//! full hash streamed in 1 MiB buffers; every stage read through and written
//! to the `FingerprintStore`; every failed read recorded in `Diagnostics`.
//! Owner: W2-B.
//!
//! Stages:
//! 1. **Eligibility**: regular files with `size > 0` (zero-byte files belong to
//!    the empty-files detector), never online-only placeholders (reading one
//!    would download it), one path per [`FileId`] (hard links are one file).
//! 2. **Size buckets** over every eligible file, cache state ignored.
//! 3. **Partial hash** for members of same-size buckets (cache first). Small
//!    files are read whole here, so their partial *is* their full hash and
//!    their SHA-256 is computed from the same bytes.
//! 4. **Full hash + SHA-256** in one streaming pass, only for members of
//!    `(size, partial)` collisions that lack them in the cache.
//! 5. **Groups** = equal `(size, full xxh64, SHA-256)`: the SHA-256 is the
//!    confirmation before a deletion is offered (research §3.2), and it is cached.
//!
//! All reads go through [`adaptive::run`] (parallel, then serial on a flaky
//! filesystem). A file that fails is counted in `Diagnostics::unreadable`; a
//! file whose size changed during the scan is counted under
//! `skipped["changed_during_scan"]`. Neither is ever treated as "unique".

pub mod adaptive;

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use mj_core::fingerprint::{PARTIAL_FULL_THRESHOLD, PARTIAL_WINDOW};
use mj_core::hash::{Sha256, Xxh64};
use mj_core::{DetectContext, Diagnostics, FileEntry, FileId, Fingerprints, Phase, Progress, Result};

use adaptive::ReadError;

/// Streaming buffer for full hashes.
pub const FULL_BUFFER: usize = 1024 * 1024;

/// Skip reasons recorded in `Diagnostics::skipped`.
pub mod skip {
    /// Cloud placeholder whose content is not local: never read.
    pub const ONLINE_ONLY: &str = "online_only";
    /// Another path of the same file (hard link): not a duplicate.
    pub const HARDLINK: &str = "hardlink";
    /// Size changed between the scan and the read.
    pub const CHANGED: &str = "changed_during_scan";
}

/// A group of byte-identical files: indexes into the slice given to [`exact_groups`],
/// sorted by path key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExactGroup {
    pub members: Vec<usize>,
    pub size: u64,
    pub full: u64,
    pub sha256: [u8; 32],
}

#[derive(Debug, Clone, Default)]
struct State {
    partial: Option<u64>,
    full: Option<u64>,
    sha: Option<[u8; 32]>,
}

/// True when the exact lane may read this entry (regular, non-empty, local content).
pub fn readable_candidate(e: &FileEntry) -> bool {
    e.is_file() && e.size > 0 && !e.is_online_only()
}

/// Drop hard-link aliases: keep the first path (in slice order) of every `FileId`.
/// Returns the kept indexes.
pub fn dedup_hardlinks(files: &[&FileEntry], diag: &mut Diagnostics) -> Vec<usize> {
    let mut seen: HashMap<FileId, usize> = HashMap::new();
    let mut out = Vec::with_capacity(files.len());
    for (i, e) in files.iter().enumerate() {
        if let Some(id) = e.file_id {
            if seen.insert(id, i).is_some() {
                diag.skip(skip::HARDLINK);
                continue;
            }
        }
        out.push(i);
    }
    out
}

/// Find groups of byte-identical files among `files`. Ineligible files are filtered
/// (and counted) here, so callers may pass any subset of the index.
pub fn exact_groups(ctx: &DetectContext<'_>, files: &[&FileEntry], diag: &mut Diagnostics) -> Result<Vec<ExactGroup>> {
    // Stage 1: eligibility.
    let mut eligible: Vec<&FileEntry> = Vec::new();
    let mut origin: Vec<usize> = Vec::new();
    for (i, e) in files.iter().enumerate() {
        if !e.is_file() || e.size == 0 {
            continue;
        }
        if e.is_online_only() {
            diag.skip(skip::ONLINE_ONLY);
            continue;
        }
        eligible.push(e);
        origin.push(i);
    }
    let kept = dedup_hardlinks(&eligible, diag);

    // Stage 2: size buckets across all files.
    let mut by_size: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
    for &k in &kept {
        by_size.entry(eligible[k].size).or_default().push(k);
    }
    let candidates: Vec<usize> = by_size.into_values().filter(|v| v.len() > 1).flatten().collect();
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let mut state: HashMap<usize, State> = HashMap::with_capacity(candidates.len());
    for &k in &candidates {
        let e = eligible[k];
        let fp = ctx.fingerprints.get(&e.path, e.stamp()).unwrap_or_default();
        state.insert(k, State { partial: fp.partial, full: fp.full, sha: fp.sha256 });
    }

    // Stage 3: partial hashes.
    ctx.cancel.checkpoint()?;
    let need: Vec<usize> = candidates.iter().copied().filter(|k| state[k].partial.is_none()).collect();
    let results = read_all(ctx, &need, &eligible, diag, |e| read_partial(&e.path, e.size))?;
    for (k, r) in need.iter().zip(results) {
        let e = eligible[*k];
        match r {
            Ok(st) => {
                let fp = Fingerprints { partial: st.partial, full: st.full, sha256: st.sha, ..Default::default() };
                ctx.fingerprints.put(&e.path, e.stamp(), fp);
                state.insert(*k, st);
            }
            Err(err) => {
                report(diag, &e.path, &err);
                state.remove(k);
            }
        }
    }

    // Stage 4: full hash + SHA-256 for partial collisions.
    let mut by_partial: BTreeMap<(u64, u64), Vec<usize>> = BTreeMap::new();
    for &k in &candidates {
        if let Some(p) = state.get(&k).and_then(|s| s.partial) {
            by_partial.entry((eligible[k].size, p)).or_default().push(k);
        }
    }
    let colliding: Vec<usize> = by_partial.into_values().filter(|v| v.len() > 1).flatten().collect();
    ctx.cancel.checkpoint()?;
    let need: Vec<usize> =
        colliding.iter().copied().filter(|k| state[k].full.is_none() || state[k].sha.is_none()).collect();
    let results = read_all(ctx, &need, &eligible, diag, |e| hash_full(&e.path, e.size))?;
    for (k, r) in need.iter().zip(results) {
        let e = eligible[*k];
        match r {
            Ok((full, sha)) => {
                let fp = Fingerprints { full: Some(full), sha256: Some(sha), ..Default::default() };
                ctx.fingerprints.put(&e.path, e.stamp(), fp);
                let s = state.entry(*k).or_default();
                s.full = Some(full);
                s.sha = Some(sha);
            }
            Err(err) => {
                report(diag, &e.path, &err);
                state.remove(k);
            }
        }
    }

    // Stage 5: confirmed groups.
    let mut by_full: BTreeMap<(u64, u64, [u8; 32]), Vec<usize>> = BTreeMap::new();
    for &k in &colliding {
        if let Some(State { full: Some(f), sha: Some(s), .. }) = state.get(&k) {
            by_full.entry((eligible[k].size, *f, *s)).or_default().push(k);
        }
    }
    let mut groups: Vec<ExactGroup> = by_full
        .into_iter()
        .filter(|(_, v)| v.len() > 1)
        .map(|((size, full, sha256), v)| {
            let mut members: Vec<usize> = v.into_iter().map(|k| origin[k]).collect();
            members.sort_by_cached_key(|&i| mj_core::pathutil::path_key(&files[i].path));
            ExactGroup { members, size, full, sha256 }
        })
        .collect();
    groups.sort_by(|a, b| {
        let wa = a.size * (a.members.len() as u64 - 1);
        let wb = b.size * (b.members.len() as u64 - 1);
        wb.cmp(&wa).then_with(|| files[a.members[0]].path.cmp(&files[b.members[0]].path))
    });
    Ok(groups)
}

/// Record a failed read in the diagnostics.
pub fn report(diag: &mut Diagnostics, path: &Path, err: &ReadError) {
    match err {
        ReadError::Changed => diag.skip(skip::CHANGED),
        ReadError::Io(_) | ReadError::Panicked => diag.unreadable(path),
    }
}

/// Run `work` over the entries `idx` with adaptive concurrency and hashing progress.
pub(crate) fn read_all<R: Send>(
    ctx: &DetectContext<'_>,
    idx: &[usize],
    files: &[&FileEntry],
    diag: &mut Diagnostics,
    work: impl Fn(&FileEntry) -> std::result::Result<R, ReadError> + Sync,
) -> Result<Vec<std::result::Result<R, ReadError>>> {
    if idx.is_empty() {
        return Ok(Vec::new());
    }
    let total = idx.len() as u64;
    let done = AtomicU64::new(0);
    let bytes = AtomicU64::new(0);
    let r = adaptive::run(idx, ctx.threads, ctx.cancel, |&k| {
        let e = files[k];
        let out = work(e);
        let d = done.fetch_add(1, Ordering::Relaxed) + 1;
        let b = bytes.fetch_add(e.size, Ordering::Relaxed) + e.size;
        ctx.progress.report(&Progress {
            phase: Phase::Hashing,
            done: d.min(total),
            total: Some(total),
            bytes: b,
            current: None,
        });
        out
    })?;
    if r.fell_back {
        note_fell_back(diag);
    }
    Ok(r.results)
}

/// Note added to diagnostics when concurrent reads failed and serial reads were used.
pub const FELL_BACK_NOTE: &str = "Concurrent reads failed on this filesystem; files were re-read one at a time";

/// Add [`FELL_BACK_NOTE`] once.
pub fn note_fell_back(diag: &mut Diagnostics) {
    if !diag.notes.iter().any(|n| n == FELL_BACK_NOTE) {
        diag.notes.push(FELL_BACK_NOTE.to_string());
    }
}

/// Open a file for reading and check it is still a regular file of the scanned size.
pub fn open_checked(path: &Path, size: u64) -> std::result::Result<File, ReadError> {
    let f = File::open(path)?;
    let md = f.metadata()?;
    if !md.is_file() {
        return Err(ReadError::Io(std::io::Error::other("not a regular file")));
    }
    if md.len() != size {
        return Err(ReadError::Changed);
    }
    Ok(f)
}

/// Stage-3 read: whole file when small, else head + middle + tail windows.
fn read_partial(path: &Path, size: u64) -> std::result::Result<State, ReadError> {
    let mut f = open_checked(path, size)?;
    if size <= PARTIAL_FULL_THRESHOLD {
        let mut buf = Vec::with_capacity(size as usize);
        f.take(size + 1).read_to_end(&mut buf)?;
        if buf.len() as u64 != size {
            return Err(ReadError::Changed);
        }
        let h = mj_core::hash::xxh64(&buf, 0);
        return Ok(State { partial: Some(h), full: Some(h), sha: Some(mj_core::hash::sha256(&buf)) });
    }
    Ok(State { partial: Some(partial_windows(&mut f, size)?), full: None, sha: None })
}

/// xxh64 over the three windows (head, middle at `size/2 - W/2`, tail), fed in order.
pub fn partial_windows<R: Read + Seek>(f: &mut R, size: u64) -> std::io::Result<u64> {
    let w = PARTIAL_WINDOW;
    let mut buf = vec![0u8; w as usize];
    let mut h = Xxh64::new(0);
    for off in [0, size / 2 - w / 2, size - w] {
        f.seek(SeekFrom::Start(off))?;
        f.read_exact(&mut buf)?;
        h.update(&buf);
    }
    Ok(h.finish())
}

/// Stage-4 read: stream the whole file once into xxh64 and SHA-256.
fn hash_full(path: &Path, size: u64) -> std::result::Result<(u64, [u8; 32]), ReadError> {
    let mut f = open_checked(path, size)?;
    let mut buf = vec![0u8; FULL_BUFFER];
    let mut x = Xxh64::new(0);
    let mut s = Sha256::new();
    let mut total = 0u64;
    loop {
        let n = match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        };
        total += n as u64;
        if total > size {
            return Err(ReadError::Changed);
        }
        x.update(&buf[..n]);
        s.update(&buf[..n]);
    }
    if total != size {
        return Err(ReadError::Changed);
    }
    Ok((x.finish(), s.finish()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn partial_windows_cover_head_middle_tail() {
        let size = 4 * PARTIAL_WINDOW as usize;
        let base: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
        let h0 = partial_windows(&mut Cursor::new(&base), size as u64).unwrap();
        for (pos, changes) in [(0, true), (size / 2, true), (size - 1, true), (PARTIAL_WINDOW as usize + 10, false)] {
            let mut m = base.clone();
            m[pos] ^= 1;
            let h = partial_windows(&mut Cursor::new(&m), size as u64).unwrap();
            assert_eq!(h != h0, changes, "byte {pos}");
        }
    }
}
