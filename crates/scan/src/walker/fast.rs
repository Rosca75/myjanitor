//! The fast-index path (SCAN-03): entries from `ScanPlatform::fast_enumerate`
//! (NTFS MFT on Windows), filtered with exactly the rules of the directory walk.
//!
//! The platform returns the whole subtree at once, so exclusions, hidden
//! entries, the depth limit and other filesystems are applied afterwards: the
//! entries are sorted by `path_key` (a parent before its children) and an entry
//! is kept only when its parent directory was admitted and entered, so a dropped
//! directory drops everything below it. Any platform error (no access,
//! not NTFS, a failure midway) makes the caller walk instead: nothing from a
//! failed enumeration is kept.

use std::collections::HashSet;

use mj_core::pathutil::{is_within, normalize_lexical, path_key};
use mj_core::{Error, FileEntry, Result};

use super::parallel::{admit, Admitted, Collected, DirJob, WalkCtx};

/// Try the fast path for one root. `Ok(true)` when it was used.
pub fn try_fast(ctx: &WalkCtx<'_>, root: &DirJob, out: &mut Collected) -> Result<bool> {
    let mut got: Vec<FileEntry> = Vec::new();
    match ctx.platform.fast_enumerate(&root.path, &mut |e| got.push(e), ctx.cancel) {
        Ok(()) => {}
        Err(Error::Cancelled) => return Err(Error::Cancelled),
        Err(_) => return Ok(false),
    }
    ctx.cancel.checkpoint()?;
    let root_key = path_key(&root.path);
    let root_depth = normalize_lexical(&root.path).components().count();
    let mut keyed: Vec<(String, FileEntry)> = got
        .into_iter()
        .filter(|e| is_within(&root.path, &e.path))
        .map(|e| (path_key(&e.path), e))
        .filter(|(k, _)| *k != root_key)
        .collect();
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    keyed.dedup_by(|a, b| a.0 == b.0);
    // SCAN-1: an entry is considered only when its parent folder was admitted and entered.
    // Sorting by key puts a parent before its children, but not a subtree right after its
    // folder: `skip.zip` and `skip - Copy` sort between `skip` and `skip/x`, so a "skip
    // everything that follows with this prefix" scan let an excluded folder's content in.
    let mut entered: HashSet<String> = HashSet::new();
    entered.insert(root_key.trim_end_matches('/').to_string());
    for (n, (key, e)) in keyed.into_iter().enumerate() {
        if n % 4096 == 0 {
            ctx.cancel.checkpoint()?;
        }
        let parent = key.rfind('/').map(|i| &key[..i]);
        if !parent.is_some_and(|p| entered.contains(p)) {
            continue;
        }
        let depth = (normalize_lexical(&e.path).components().count().saturating_sub(root_depth)) as u32;
        let name = e.path.file_name().unwrap_or(e.path.as_os_str()).to_owned();
        if ctx.filter.skips_hidden_name(&name) {
            continue;
        }
        if ctx.filter.excludes(&e.path, &name) {
            out.stats.excluded += 1;
            continue;
        }
        if ctx.opts.max_depth.is_some_and(|m| depth > m) {
            continue;
        }
        ctx.reporter.tick(e.size, &e.path);
        // A link's target is never part of the scan, whatever the platform returned: only
        // an admitted directory with something to list opens its children.
        if let Admitted::Dir { enter: Some(_) } = admit(ctx, out, e, depth, root) {
            entered.insert(key.trim_end_matches('/').to_string());
        }
    }
    Ok(true)
}
