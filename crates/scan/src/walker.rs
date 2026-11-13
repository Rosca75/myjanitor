//! The scan walker (SCAN-01/02/03/11). **Owner: W2-A.**
//!
//! Rules every path through here keeps: links/junctions are recorded as
//! `EntryKind::Link` and never followed; excluded directories are not
//! entered; one-file-system is honoured; every unreadable entry is counted
//! (and sampled) in `ScanStats`, never silently dropped; output is
//! deterministic (the index sorts by `path_key`).
//!
//! Per root: stat the root, then try the fast index (NTFS MFT, Windows) when
//! enabled, else (or when it is refused) the parallel directory walk. All
//! walked roots share one walk, so several small roots still use every worker.

mod fast;
pub mod filter;
mod parallel;
mod progress;

use std::path::PathBuf;
use std::time::Instant;

use mj_core::{EntryKind, Error, FileIndex, Result, Timestamp};

use crate::{effective_roots, ScanRequest};
use filter::ScopeFilter;
use parallel::{Collected, DirJob, WalkCtx};
use progress::Reporter;

/// What happened to one scan root (used by cache pruning).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootState {
    /// Walked (possibly with unreadable holes).
    Walked,
    /// The root does not exist (any longer).
    Gone,
    /// The root could not be read (permission, I/O).
    Unreadable,
    /// The root is excluded by the scope rules.
    Excluded,
}

/// A walk's result plus what the cache needs to prune safely.
pub struct WalkOutcome {
    pub index: FileIndex,
    /// Per effective root (same order as `index.roots`).
    pub roots: Vec<RootState>,
    /// Directories whose content is missing from the index because they could not be read.
    pub holes: Vec<PathBuf>,
    /// True when the index lists every file below each walked root (no include
    /// patterns, no depth limit), so files absent from it are really gone or excluded.
    pub complete: bool,
}

/// Run the walk.
pub fn run(req: &ScanRequest<'_>) -> Result<WalkOutcome> {
    let started = Instant::now();
    let roots = effective_roots(req.scope);
    if roots.is_empty() {
        return Err(Error::InvalidInput("the scan scope has no location".into()));
    }
    let opts = &req.scope.options;
    let filter = ScopeFilter::new(req.scope, req.exclusions);
    let workers = if opts.threads == 0 { mj_core::pool::default_workers() } else { usize::from(opts.threads) };
    let reporter = Reporter::new(req.progress);
    let ctx =
        WalkCtx { platform: req.platform, cancel: req.cancel, filter: &filter, opts, workers, reporter: &reporter };
    let mut out = Collected::default();
    let mut states = Vec::with_capacity(roots.len());
    let mut frontier = Vec::new();
    let mut first_error: Option<Error> = None;

    for (ix, root) in roots.iter().enumerate() {
        req.cancel.checkpoint()?;
        if filter.excludes_root(root) {
            out.stats.excluded += 1;
            states.push(RootState::Excluded);
            continue;
        }
        let mut e = match req.platform.metadata(root) {
            Ok(e) => e,
            Err(err) => {
                let gone = matches!(err, Error::NotFound(_));
                out.unreadable(root.clone(), Some(&err), false);
                states.push(if gone { RootState::Gone } else { RootState::Unreadable });
                first_error.get_or_insert(err);
                continue;
            }
        };
        states.push(RootState::Walked);
        let root_ix = u16::try_from(ix).unwrap_or(u16::MAX);
        e.root = root_ix;
        e.depth = 0;
        if e.kind != EntryKind::Dir {
            // A file root is indexed as itself; a link root is recorded and not followed.
            let job = DirJob { path: root.clone(), depth: 0, root: root_ix, root_dev: None };
            let _ = parallel::admit(&ctx, &mut out, e, 0, &job);
            continue;
        }
        let root_dev = if opts.one_file_system { e.file_id.map(|f| f.volume) } else { None };
        if let Some(id) = e.file_id {
            out.visited.insert(id);
        }
        out.stats.dirs += 1;
        out.entries.push(e);
        if opts.max_depth == Some(0) {
            continue;
        }
        let job = DirJob { path: root.clone(), depth: 0, root: root_ix, root_dev };
        if opts.use_fast_index && fast::try_fast(&ctx, &job, &mut out)? {
            out.stats.used_fast_index = true;
            continue;
        }
        frontier.push(job);
    }

    parallel::walk(&ctx, frontier, &mut out)?;

    if !states.contains(&RootState::Walked) {
        return Err(first_error.unwrap_or_else(|| Error::InvalidInput("every scan location is excluded".into())));
    }
    let mut stats = out.stats;
    stats.elapsed_ms = started.elapsed().as_millis() as u64;
    reporter.finish(out.entries.len() as u64, stats.bytes);
    let complete = !filter.has_includes() && opts.max_depth.is_none();
    Ok(WalkOutcome {
        index: FileIndex::new(roots, out.entries, stats, Timestamp::now()),
        roots: states,
        holes: out.holes,
        complete,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::testutil::TempDir;
    use mj_core::{CancelToken, NullProgress, ScanScope};
    use mj_platform::NativePlatform;

    fn scan(scope: &ScanScope) -> Result<FileIndex> {
        let p = NativePlatform::new();
        let req =
            ScanRequest { scope, exclusions: &[], platform: &p, cancel: &CancelToken::new(), progress: &NullProgress };
        crate::scan(&req)
    }

    #[test]
    fn scans_a_fixture_tree() {
        let t = TempDir::new("walker").unwrap();
        t.write("a/one.txt", b"12345").unwrap();
        t.write("a/b/two.log", b"12").unwrap();
        t.write("skip/three.txt", b"1").unwrap();
        t.write(".hidden/four.txt", b"1").unwrap();
        let mut scope = ScanScope { roots: vec![t.path().to_path_buf()], ..Default::default() };
        scope.exclude.push(t.join("skip").to_string_lossy().into_owned());
        let ix = scan(&scope).unwrap();
        assert_eq!(ix.stats.files, 3);
        assert_eq!(ix.stats.bytes, 8);
        assert_eq!(ix.stats.excluded, 1);
        assert!(ix.get(&t.join("skip/three.txt")).is_none());
        assert!(ix.get(&t.join("skip")).is_none());
        assert_eq!(ix.get(&t.join("a/b/two.log")).unwrap().depth, 3);
        scope.options.include_hidden = false;
        scope.include = vec!["*.txt".into()];
        let ix = scan(&scope).unwrap();
        assert_eq!(ix.stats.files, 1);
        assert!(ix.get(&t.join(".hidden")).is_none());
    }

    #[test]
    fn depth_limit_and_file_roots() {
        let t = TempDir::new("walker-depth").unwrap();
        t.write("a/b/c/d.txt", b"1").unwrap();
        t.write("top.txt", b"22").unwrap();
        let mut scope = ScanScope { roots: vec![t.path().to_path_buf()], ..Default::default() };
        scope.options.max_depth = Some(2);
        let ix = scan(&scope).unwrap();
        assert!(ix.get(&t.join("a/b")).is_some());
        assert!(ix.get(&t.join("a/b/c")).is_none());
        scope.options.max_depth = Some(0);
        assert_eq!(scan(&scope).unwrap().len(), 1);
        let file_root = ScanScope { roots: vec![t.join("top.txt")], ..Default::default() };
        let ix = scan(&file_root).unwrap();
        assert_eq!((ix.len(), ix.stats.files, ix.stats.bytes), (1, 1, 2));
    }

    #[test]
    fn missing_and_excluded_roots() {
        let t = TempDir::new("walker-roots").unwrap();
        let gone = ScanScope { roots: vec![t.join("nope")], ..Default::default() };
        assert_eq!(scan(&gone).unwrap_err().code(), "not_found");
        let sibling = PathBuf::from(format!("{}-missing", t.path().display()));
        let mut both = ScanScope { roots: vec![sibling, t.path().to_path_buf()], ..Default::default() };
        let ix = scan(&both).unwrap();
        assert_eq!(ix.stats.unreadable, 1);
        both.roots.remove(0);
        both.exclude = vec![t.path().to_string_lossy().into_owned()];
        assert!(scan(&both).is_err());
        assert_eq!(scan(&ScanScope::default()).unwrap_err().code(), "invalid_input");
    }
}
