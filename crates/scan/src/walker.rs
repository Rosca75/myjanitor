//! Directory walker. **Owner: W2-A.** Wave 1: sequential, std-only, correct.
//!
//! Rules every implementation must keep: links/junctions recorded as
//! `EntryKind::Link` and never followed; excluded directories not entered;
//! one-file-system honoured; every unreadable entry counted (and sampled) in
//! `ScanStats`; deterministic output (the index sorts).

use std::path::Path;
use std::time::Instant;

use mj_core::glob::{Glob, GlobMode};
use mj_core::{Attributes, EntryKind, Error, FileEntry, FileIndex, Phase, Progress, Result, ScanStats, Timestamp};

use crate::{effective_roots, ScanRequest};

fn compile(patterns: &[String]) -> Vec<Glob> {
    patterns.iter().filter(|p| !p.trim().is_empty()).map(|p| Glob::platform(p.trim(), GlobMode::Path)).collect()
}

fn excluded(globs: &[Glob], path: &Path) -> bool {
    globs.iter().any(|g| {
        if g.is_wildcard() {
            g.matches_path(path)
        } else {
            mj_core::pathutil::is_within(Path::new(g.pattern()), path)
        }
    })
}

/// Sequential scan.
pub fn scan(req: &ScanRequest<'_>) -> Result<FileIndex> {
    let started = Instant::now();
    let roots = effective_roots(req.scope);
    if roots.is_empty() {
        return Err(Error::InvalidInput("the scan scope has no location".into()));
    }
    let mut excl: Vec<String> = req.scope.exclude.clone();
    excl.extend(req.exclusions.iter().cloned());
    let excl = compile(&excl);
    let incl = compile(&req.scope.include);
    let opts = &req.scope.options;
    let mut stats = ScanStats::default();
    let mut entries: Vec<FileEntry> = Vec::new();
    let mut readable_roots = 0;

    for (root_ix, root) in roots.iter().enumerate() {
        req.cancel.checkpoint()?;
        let root_entry = match req.platform.metadata(root) {
            Ok(e) => e,
            Err(e) => {
                stats.record_unreadable(root, e.code() == "permission_denied");
                continue;
            }
        };
        readable_roots += 1;
        let root_volume = root_entry.file_id.map(|f| f.volume);
        let root_ix = root_ix as u16;
        let mut stack = vec![(root.clone(), 0u32)];
        let mut first = Some(root_entry);
        while let Some((dir, depth)) = stack.pop() {
            req.cancel.checkpoint()?;
            let mut entry = match first.take() {
                Some(e) => e,
                None => match req.platform.metadata(&dir) {
                    Ok(e) => e,
                    Err(e) => {
                        stats.record_unreadable(&dir, e.code() == "permission_denied");
                        continue;
                    }
                },
            };
            entry.depth = depth;
            entry.root = root_ix;
            let is_dir = entry.kind == EntryKind::Dir;
            if !is_dir {
                continue;
            }
            stats.dirs += 1;
            entries.push(entry);
            if opts.max_depth.is_some_and(|m| depth >= m) {
                continue;
            }
            let items = match req.platform.read_dir(&dir) {
                Ok(i) => i,
                Err(e) => {
                    stats.record_unreadable(&dir, e.code() == "permission_denied");
                    continue;
                }
            };
            for item in items {
                if excluded(&excl, &item.path) {
                    stats.excluded += 1;
                    continue;
                }
                let hidden_name = item.name.to_string_lossy().starts_with('.');
                if !opts.include_hidden && hidden_name {
                    continue;
                }
                match item.kind {
                    EntryKind::Dir => {
                        // Stat later (when popped) so one-file-system and hidden attrs can be checked.
                        match req.platform.metadata(&item.path) {
                            Ok(md) => {
                                if !opts.include_hidden && md.attrs.contains(Attributes::HIDDEN) {
                                    continue;
                                }
                                if opts.one_file_system
                                    && root_volume.is_some()
                                    && md.file_id.map(|f| f.volume) != root_volume
                                {
                                    stats.excluded += 1;
                                    continue;
                                }
                                if md.kind == EntryKind::Dir {
                                    stack.push((item.path.clone(), depth + 1));
                                } else {
                                    let mut md = md;
                                    md.depth = depth + 1;
                                    md.root = root_ix;
                                    record(&mut stats, &mut entries, md, &incl);
                                }
                            }
                            Err(e) => stats.record_unreadable(&item.path, e.code() == "permission_denied"),
                        }
                    }
                    _ => match req.platform.metadata(&item.path) {
                        Ok(mut md) => {
                            if !opts.include_hidden && md.attrs.contains(Attributes::HIDDEN) {
                                continue;
                            }
                            md.depth = depth + 1;
                            md.root = root_ix;
                            record(&mut stats, &mut entries, md, &incl);
                        }
                        Err(e) => stats.record_unreadable(&item.path, e.code() == "permission_denied"),
                    },
                }
            }
            req.progress.report(&Progress {
                phase: Phase::Walking,
                done: entries.len() as u64,
                total: None,
                bytes: stats.bytes,
                current: Some(dir),
            });
        }
    }
    if readable_roots == 0 {
        return Err(Error::NotFound(roots[0].clone()));
    }
    stats.elapsed_ms = started.elapsed().as_millis() as u64;
    Ok(FileIndex::new(roots, entries, stats, Timestamp::now()))
}

fn record(stats: &mut ScanStats, entries: &mut Vec<FileEntry>, e: FileEntry, incl: &[Glob]) {
    match e.kind {
        EntryKind::File => {
            if !incl.is_empty() && !incl.iter().any(|g| g.matches_path(&e.path)) {
                return;
            }
            stats.files += 1;
            stats.bytes += e.size;
            stats.allocated += e.allocated.unwrap_or(e.size);
            if e.is_online_only() {
                stats.online_only_files += 1;
                stats.online_only_bytes += e.size;
            }
        }
        EntryKind::Link => stats.links += 1,
        _ => {}
    }
    entries.push(e);
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::testutil::TempDir;
    use mj_core::{CancelToken, NullProgress, ScanScope};
    use mj_platform::NativePlatform;

    #[test]
    fn scans_a_fixture_tree() {
        let t = TempDir::new("walker").unwrap();
        t.write("a/one.txt", b"12345").unwrap();
        t.write("a/b/two.log", b"12").unwrap();
        t.write("skip/three.txt", b"1").unwrap();
        t.write(".hidden/four.txt", b"1").unwrap();
        let mut scope = ScanScope { roots: vec![t.path().to_path_buf()], ..Default::default() };
        scope.exclude.push(t.join("skip").to_string_lossy().into_owned());
        let p = NativePlatform::new();
        let req = ScanRequest {
            scope: &scope,
            exclusions: &[],
            platform: &p,
            cancel: &CancelToken::new(),
            progress: &NullProgress,
        };
        let ix = crate::scan(&req).unwrap();
        assert_eq!(ix.stats.files, 3);
        assert_eq!(ix.stats.bytes, 8);
        assert!(ix.get(&t.join("skip/three.txt")).is_none());
        assert_eq!(ix.get(&t.join("a/b/two.log")).unwrap().depth, 3);
        scope.options.include_hidden = false;
        scope.include = vec!["*.txt".into()];
        let req = ScanRequest {
            scope: &scope,
            exclusions: &[],
            platform: &p,
            cancel: &CancelToken::new(),
            progress: &NullProgress,
        };
        let ix = crate::scan(&req).unwrap();
        assert_eq!(ix.stats.files, 1);
    }

    #[test]
    fn effective_roots_drop_nested() {
        let scope =
            ScanScope { roots: vec!["/a/b".into(), "/a".into(), "/c".into(), "/a/".into()], ..Default::default() };
        assert_eq!(effective_roots(&scope), vec![std::path::PathBuf::from("/a"), "/c".into()]);
    }
}
