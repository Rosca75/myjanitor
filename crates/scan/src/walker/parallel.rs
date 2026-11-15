//! The parallel directory walk (SCAN-01/11).
//!
//! Level-synchronous breadth-first walk on the in-house pool
//! ([`mj_core::pool::parallel_map`], no rayon). Each level runs two parallel
//! phases:
//!
//! 1. **list**: `read_dir` of every directory of the level, one item each;
//! 2. **stat**: `metadata` (lstat semantics) of every entry those listings
//!    returned, flattened into one list, so the expensive part is balanced
//!    across workers even when one directory holds 100 000 files.
//!
//! Classification between phases is sequential and lock-free (results come
//! back in index-aligned slots). Cancellation and pause are honoured before
//! every item by `parallel_map` and between phases here. Links are recorded
//! and never entered; a directory reached twice (bind mounts, same file id)
//! is not entered again; every failure is counted in `ScanStats`.

use std::collections::HashSet;
use std::path::PathBuf;

use mj_core::pool::parallel_map;
use mj_core::{CancelToken, EntryKind, Error, FileEntry, FileId, Result, ScanOptions, ScanStats};
use mj_platform::ScanPlatform;

use super::filter::ScopeFilter;
use super::progress::Reporter;

/// Below this many items a phase runs on the calling thread (spawning would cost more).
const PAR_MIN: usize = 64;

/// What the walk needs from the request.
pub struct WalkCtx<'a> {
    pub platform: &'a dyn ScanPlatform,
    pub cancel: &'a CancelToken,
    pub filter: &'a ScopeFilter,
    pub opts: &'a ScanOptions,
    pub workers: usize,
    pub reporter: &'a Reporter<'a>,
}

/// Everything collected so far.
#[derive(Default)]
pub struct Collected {
    pub entries: Vec<FileEntry>,
    pub stats: ScanStats,
    /// Directories whose content could not be read (the index is incomplete below them).
    pub holes: Vec<PathBuf>,
    /// Directory ids already entered (loop guard).
    pub visited: HashSet<FileId>,
}

impl Collected {
    /// Count an unreadable path; directories also become holes.
    pub fn unreadable(&mut self, path: PathBuf, err: Option<&Error>, is_dir: bool) {
        let denied = err.is_some_and(|e| e.code() == "permission_denied");
        self.stats.record_unreadable(&path, denied);
        if is_dir {
            self.holes.push(path);
        }
    }
}

/// One directory to list.
#[derive(Debug, Clone)]
pub struct DirJob {
    pub path: PathBuf,
    pub depth: u32,
    pub root: u16,
    /// Volume of the root when `one_file_system` is on.
    pub root_dev: Option<u64>,
}

/// Result of [`admit`].
pub enum Admitted {
    /// Not indexed. `subtree` = a directory whose content must be skipped too.
    Dropped { subtree: bool },
    /// Indexed; nothing below it to walk.
    Leaf,
    /// An indexed directory; `enter` = path to list next (None: depth limit).
    Dir { enter: Option<PathBuf> },
}

/// Apply the scope rules to one stat'ed entry and record it. Shared by the
/// directory walk and the MFT fast path, so both index exactly the same things.
pub fn admit(ctx: &WalkCtx<'_>, out: &mut Collected, mut e: FileEntry, depth: u32, job: &DirJob) -> Admitted {
    let is_dir = e.kind == EntryKind::Dir;
    if ctx.filter.skips_hidden(&e) {
        return Admitted::Dropped { subtree: is_dir };
    }
    if let (Some(dev), Some(id)) = (job.root_dev, e.file_id) {
        if id.volume != dev && e.kind != EntryKind::Link {
            // Another filesystem mounted below the root: not entered, not indexed
            // (an empty-looking mount point must never become a removal candidate).
            out.stats.excluded += 1;
            return Admitted::Dropped { subtree: is_dir };
        }
    }
    e.depth = depth;
    e.root = job.root;
    match e.kind {
        EntryKind::Dir => {
            if let Some(id) = e.file_id {
                if !out.visited.insert(id) {
                    out.stats.excluded += 1;
                    return Admitted::Dropped { subtree: true };
                }
            }
            out.stats.dirs += 1;
            let enter = ctx.opts.max_depth.is_none_or(|m| depth < m).then(|| e.path.clone());
            out.entries.push(e);
            Admitted::Dir { enter }
        }
        EntryKind::File => {
            if !ctx.filter.includes_file(&e.path) {
                return Admitted::Dropped { subtree: false };
            }
            let s = &mut out.stats;
            s.files += 1;
            if e.is_online_only() {
                // SCAN-09: online-only placeholders hold no local bytes; excluded from totals.
                s.online_only_files += 1;
                s.online_only_bytes += e.size;
            } else {
                s.bytes += e.size;
                s.allocated += e.allocated.unwrap_or(e.size);
            }
            out.entries.push(e);
            Admitted::Leaf
        }
        EntryKind::Link => {
            out.stats.links += 1;
            out.entries.push(e);
            Admitted::Leaf
        }
        EntryKind::Other => {
            out.entries.push(e);
            Admitted::Leaf
        }
    }
}

fn par<T: Sync, R: Send>(ctx: &WalkCtx<'_>, items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<Option<R>> {
    let workers = if items.len() < PAR_MIN { 1 } else { ctx.workers };
    parallel_map(items, workers, ctx.cancel, |_, t| f(t))
}

struct StatJob {
    path: PathBuf,
    dir: usize,
}

/// Walk every directory of `frontier` and below.
pub fn walk(ctx: &WalkCtx<'_>, mut frontier: Vec<DirJob>, out: &mut Collected) -> Result<()> {
    while !frontier.is_empty() {
        ctx.cancel.checkpoint()?;
        // Phase 1: list the level.
        let listings = par(ctx, &frontier, |job| ctx.platform.read_dir(&job.path));
        ctx.cancel.checkpoint()?;
        let mut stat_jobs: Vec<StatJob> = Vec::new();
        for (d, (job, listing)) in frontier.iter().zip(listings).enumerate() {
            match listing {
                Some(Ok(items)) => {
                    for item in items {
                        if ctx.filter.skips_hidden_name(&item.name) {
                            continue;
                        }
                        if ctx.filter.excludes(&item.path, &item.name) {
                            out.stats.excluded += 1;
                            continue;
                        }
                        stat_jobs.push(StatJob { path: item.path, dir: d });
                    }
                }
                // The directory vanished after it was listed by its parent.
                Some(Err(Error::NotFound(_))) => {}
                Some(Err(e)) => out.unreadable(job.path.clone(), Some(&e), true),
                // A panic inside the platform call (cancellation was checked above).
                None => out.unreadable(job.path.clone(), None, true),
            }
        }
        // Phase 2: stat the level.
        let results = par(ctx, &stat_jobs, |j| {
            let r = ctx.platform.metadata(&j.path);
            if let Ok(e) = &r {
                ctx.reporter.tick(e.size, &j.path);
            }
            r
        });
        ctx.cancel.checkpoint()?;
        let mut next = Vec::new();
        for (j, res) in stat_jobs.into_iter().zip(results) {
            let parent = &frontier[j.dir];
            let e = match res {
                Some(Ok(e)) => e,
                Some(Err(Error::NotFound(_))) => continue,
                Some(Err(e)) => {
                    out.unreadable(j.path, Some(&e), false);
                    continue;
                }
                None => {
                    out.unreadable(j.path, None, false);
                    continue;
                }
            };
            if let Admitted::Dir { enter: Some(path) } = admit(ctx, out, e, parent.depth + 1, parent) {
                next.push(DirJob { path, depth: parent.depth + 1, root: parent.root, root_dev: parent.root_dev });
            }
        }
        frontier = next;
    }
    Ok(())
}
