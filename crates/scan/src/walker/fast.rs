//! The fast-index path (SCAN-03): entries from `ScanPlatform::fast_enumerate`
//! (NTFS MFT on Windows), filtered with exactly the rules of the directory walk.
//!
//! The platform returns the whole subtree at once, so exclusions, hidden
//! entries, the depth limit and other filesystems are applied afterwards: the
//! entries are sorted by `path_key`, which makes every subtree contiguous, and
//! a dropped directory skips its whole range. Any platform error (no access,
//! not NTFS, a failure midway) makes the caller walk instead: nothing from a
//! failed enumeration is kept.

use mj_core::pathutil::{is_within, normalize_lexical, path_key};
use mj_core::{EntryKind, Error, FileEntry, Result};

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
    let mut skip: Option<String> = None;
    for (n, (key, e)) in keyed.into_iter().enumerate() {
        if n % 4096 == 0 {
            ctx.cancel.checkpoint()?;
        }
        if let Some(prefix) = &skip {
            if key.starts_with(prefix.as_str()) {
                continue;
            }
            skip = None;
        }
        let depth = (normalize_lexical(&e.path).components().count().saturating_sub(root_depth)) as u32;
        let is_dir = e.kind == EntryKind::Dir;
        let subtree = |key: &str| Some(if key.ends_with('/') { key.to_string() } else { format!("{key}/") });
        let name = e.path.file_name().unwrap_or(e.path.as_os_str()).to_owned();
        if ctx.filter.skips_hidden_name(&name) {
            skip = subtree(&key);
            continue;
        }
        if ctx.filter.excludes(&e.path, &name) {
            out.stats.excluded += 1;
            skip = subtree(&key);
            continue;
        }
        if ctx.opts.max_depth.is_some_and(|m| depth > m) {
            skip = subtree(&key);
            continue;
        }
        ctx.reporter.tick(e.size, &e.path);
        match admit(ctx, out, e, depth, root) {
            Admitted::Dropped { subtree: true } => skip = subtree(&key),
            // A link's target is never part of the scan, whatever the platform returned.
            Admitted::Leaf | Admitted::Dropped { subtree: false } if !is_dir => skip = subtree(&key),
            _ => {}
        }
    }
    Ok(true)
}
