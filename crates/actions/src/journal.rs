//! The journal store. **Owner: W2-E.** Format (frozen, ARCHITECTURE.md §7):
//! `<data dir>/journal/entries-YYYY-MM.jsonl` (one `JournalEntry` snapshot per
//! line, last line per id wins, torn last line ignored, `sync_data` after every
//! `intent`) and `<data dir>/journal/runs.jsonl` (`RunRecord`s, same rules).
//!
//! Durability rules implemented here:
//!
//! - every record is one `write` of a complete line followed by `sync_data`
//!   (intent records *and* completion records; the cost is one flush per record,
//!   which is negligible next to the file operation it describes);
//! - the in-memory view is updated only after the line is durable, so what
//!   callers read is always what a restart would replay;
//! - a newly created journal file is followed by a directory sync;
//! - a torn final line found on open (no trailing newline) is left in place
//!   (it is ignored on replay, never deleted: the journal is an audit record)
//!   and terminated with a newline so later appends start on a fresh line;
//! - records go to the file of their month, but never to a month earlier than
//!   the newest file already written, so "later line wins" stays true across
//!   files even when the clock goes backwards.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use mj_core::{lock, EntryId, Error, JournalEntry, JournalState, Result, RunId, RunRecord, Timestamp};

/// Entries per run: `EntryId = run_id * ENTRIES_PER_RUN + index`.
pub const ENTRIES_PER_RUN: u64 = 1_000_000;

/// What replaying the files found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReplayStats {
    /// Lines parsed.
    pub lines: u64,
    /// Lines ignored (torn or not valid JSON for the record type).
    pub ignored: u64,
    /// Files whose last line was torn (repaired by appending a newline).
    pub repaired: u64,
}

#[derive(Debug)]
struct FileState {
    /// Newest month written or found (`"2026-11"`).
    month: Option<String>,
    /// Open handle of `entries-<month>.jsonl`.
    entries: Option<File>,
    runs: Option<File>,
}

/// Journal backend.
#[derive(Debug)]
enum Backend {
    Memory,
    Files { dir: PathBuf, state: Mutex<FileState> },
}

/// The undo journal.
#[derive(Debug)]
pub struct Journal {
    backend: Backend,
    entries: Mutex<BTreeMap<EntryId, JournalEntry>>,
    runs: Mutex<BTreeMap<RunId, RunRecord>>,
    /// Next free entry index per run (reservations of running jobs included).
    next_index: Mutex<BTreeMap<RunId, u64>>,
    stats: ReplayStats,
}

fn month_of(t: Timestamp) -> String {
    let (y, m, _) = t.ymd();
    format!("{y:04}-{m:02}")
}

fn entry_file_name(month: &str) -> String {
    format!("entries-{month}.jsonl")
}

/// `entries-YYYY-MM.jsonl` → `YYYY-MM`.
fn month_of_file(name: &str) -> Option<String> {
    let m = name.strip_prefix("entries-")?.strip_suffix(".jsonl")?;
    let ok =
        m.len() == 7 && m.as_bytes()[4] == b'-' && m.chars().enumerate().all(|(i, c)| i == 4 || c.is_ascii_digit());
    ok.then(|| m.to_string())
}

/// Read a JSONL file, calling `f` for every line; returns (lines, ignored, torn).
fn replay_file(path: &Path, mut f: impl FnMut(&str) -> bool) -> Result<(u64, u64, bool)> {
    let mut text = String::new();
    let mut bytes = Vec::new();
    File::open(path).and_then(|mut h| h.read_to_end(&mut bytes)).map_err(|e| Error::io(path, e))?;
    text.push_str(&String::from_utf8_lossy(&bytes));
    let torn = !text.is_empty() && !text.ends_with('\n');
    let (mut lines, mut ignored) = (0u64, 0u64);
    for line in text.split('\n') {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        if f(line) {
            lines += 1;
        } else {
            ignored += 1;
        }
    }
    Ok((lines, ignored, torn))
}

fn open_append(path: &Path) -> Result<(File, bool)> {
    let existed = path.exists();
    let f = OpenOptions::new().create(true).append(true).open(path).map_err(|e| Error::io(path, e))?;
    Ok((f, !existed))
}

fn write_line(f: &mut File, path: &Path, line: &str) -> Result<()> {
    let mut buf = String::with_capacity(line.len() + 1);
    buf.push_str(line);
    buf.push('\n');
    f.write_all(buf.as_bytes()).map_err(|e| Error::io(path, e))?;
    f.sync_data().map_err(|e| Error::io(path, e))
}

impl Journal {
    /// Open (or create) the journal under `dir`, replaying it.
    pub fn open(dir: &Path) -> Result<Journal> {
        std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        let mut stats = ReplayStats::default();
        let mut entries = BTreeMap::new();
        let mut runs = BTreeMap::new();
        let mut months: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
            .map_err(|e| Error::io(dir, e))?
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                month_of_file(&name).map(|m| (m, e.path()))
            })
            .collect();
        months.sort();
        let mut repair: Vec<PathBuf> = Vec::new();
        for (_, path) in &months {
            let (l, i, torn) = replay_file(path, |line| match serde_json::from_str::<JournalEntry>(line) {
                Ok(e) => {
                    entries.insert(e.id, e);
                    true
                }
                Err(_) => false,
            })?;
            stats.lines += l;
            stats.ignored += i;
            if torn {
                repair.push(path.clone());
            }
        }
        let runs_path = dir.join("runs.jsonl");
        if runs_path.exists() {
            let (l, i, torn) = replay_file(&runs_path, |line| match serde_json::from_str::<RunRecord>(line) {
                Ok(r) => {
                    runs.insert(r.run_id, r);
                    true
                }
                Err(_) => false,
            })?;
            stats.lines += l;
            stats.ignored += i;
            if torn {
                repair.push(runs_path.clone());
            }
        }
        for p in &repair {
            let (mut f, _) = open_append(p)?;
            f.write_all(b"\n").and_then(|_| f.sync_data()).map_err(|e| Error::io(p, e))?;
            stats.repaired += 1;
        }
        let mut next_index = BTreeMap::new();
        for id in entries.keys() {
            let run = RunId(id.0 / ENTRIES_PER_RUN);
            let idx = id.0 % ENTRIES_PER_RUN + 1;
            let slot = next_index.entry(run).or_insert(0);
            *slot = (*slot).max(idx);
        }
        Ok(Journal {
            backend: Backend::Files {
                dir: dir.to_path_buf(),
                state: Mutex::new(FileState {
                    month: months.last().map(|(m, _)| m.clone()),
                    entries: None,
                    runs: None,
                }),
            },
            entries: Mutex::new(entries),
            runs: Mutex::new(runs),
            next_index: Mutex::new(next_index),
            stats,
        })
    }

    /// A journal that is never persisted (tests, dry runs).
    pub fn in_memory() -> Journal {
        Journal {
            backend: Backend::Memory,
            entries: Mutex::new(BTreeMap::new()),
            runs: Mutex::new(BTreeMap::new()),
            next_index: Mutex::new(BTreeMap::new()),
            stats: ReplayStats::default(),
        }
    }

    /// True when backed by files.
    pub fn is_persistent(&self) -> bool {
        !matches!(self.backend, Backend::Memory)
    }

    /// The journal directory (file backend).
    pub fn dir(&self) -> Option<&Path> {
        match &self.backend {
            Backend::Memory => None,
            Backend::Files { dir, .. } => Some(dir),
        }
    }

    /// What `open` replayed.
    pub fn replay_stats(&self) -> ReplayStats {
        self.stats
    }

    /// Append an entry snapshot (durable before returning for file backends).
    pub fn append(&self, entry: &JournalEntry) -> Result<()> {
        if let Backend::Files { dir, state } = &self.backend {
            let line = serde_json::to_string(entry).map_err(|e| Error::Internal(format!("journal encode: {e}")))?;
            let mut st = lock(state);
            let mut month = month_of(entry.at);
            if let Some(cur) = &st.month {
                if *cur > month {
                    month = cur.clone();
                }
            }
            let path = dir.join(entry_file_name(&month));
            if st.month.as_deref() != Some(month.as_str()) || st.entries.is_none() {
                let (f, created) = open_append(&path)?;
                if created {
                    mj_platform::action::sync_dir(dir).map_err(|e| Error::io(dir, e))?;
                }
                st.entries = Some(f);
                st.month = Some(month);
            }
            let f = st.entries.as_mut().ok_or_else(|| Error::Internal("journal file not open".into()))?;
            write_line(f, &path, &line)?;
        }
        lock(&self.entries).insert(entry.id, entry.clone());
        let run = RunId(entry.id.0 / ENTRIES_PER_RUN);
        let idx = entry.id.0 % ENTRIES_PER_RUN + 1;
        let mut ni = lock(&self.next_index);
        let slot = ni.entry(run).or_insert(0);
        *slot = (*slot).max(idx);
        Ok(())
    }

    /// Append or update a run header.
    pub fn put_run(&self, run: &RunRecord) -> Result<()> {
        if let Backend::Files { dir, state } = &self.backend {
            let line = serde_json::to_string(run).map_err(|e| Error::Internal(format!("journal encode: {e}")))?;
            let mut st = lock(state);
            let path = dir.join("runs.jsonl");
            if st.runs.is_none() {
                let (f, created) = open_append(&path)?;
                if created {
                    mj_platform::action::sync_dir(dir).map_err(|e| Error::io(dir, e))?;
                }
                st.runs = Some(f);
            }
            let f = st.runs.as_mut().ok_or_else(|| Error::Internal("runs file not open".into()))?;
            write_line(f, &path, &line)?;
        }
        lock(&self.runs).insert(run.run_id, run.clone());
        Ok(())
    }

    /// Latest state of one entry.
    pub fn entry(&self, id: EntryId) -> Option<JournalEntry> {
        lock(&self.entries).get(&id).cloned()
    }

    /// Latest state of every entry of a run, in id order.
    pub fn entries_of(&self, run: RunId) -> Vec<JournalEntry> {
        let lo = EntryId(run.0.saturating_mul(ENTRIES_PER_RUN));
        let hi = EntryId(run.0.saturating_add(1).saturating_mul(ENTRIES_PER_RUN));
        lock(&self.entries).range(lo..hi).map(|(_, e)| e.clone()).filter(|e| e.run_id == run).collect()
    }

    /// Latest state of every entry, in id order.
    pub fn all_entries(&self) -> Vec<JournalEntry> {
        lock(&self.entries).values().cloned().collect()
    }

    /// Entries whose latest record is `intent` (left by an interrupted run).
    pub fn pending_intents(&self) -> Vec<JournalEntry> {
        lock(&self.entries).values().filter(|e| e.state == JournalState::Intent).cloned().collect()
    }

    /// Run headers, newest first.
    pub fn runs(&self) -> Vec<RunRecord> {
        let mut v: Vec<RunRecord> = lock(&self.runs).values().cloned().collect();
        v.sort_by(|a, b| b.started.cmp(&a.started).then(b.run_id.cmp(&a.run_id)));
        v
    }

    /// One run header.
    pub fn run(&self, id: RunId) -> Option<RunRecord> {
        lock(&self.runs).get(&id).cloned()
    }

    /// A new run id: milliseconds of `now`, bumped past every existing id.
    pub fn next_run_id(&self, now: Timestamp) -> RunId {
        let max_run = lock(&self.runs).keys().next_back().map(|r| r.0).unwrap_or(0);
        let max_entry = lock(&self.entries).keys().next_back().map(|e| e.0 / ENTRIES_PER_RUN).unwrap_or(0);
        RunId((now.millis().max(0) as u64).max(max_run.max(max_entry) + 1))
    }

    /// Reserve `n` consecutive entry ids in `run` (several jobs of one run never collide).
    pub fn reserve_entry_ids(&self, run: RunId, n: u64) -> Result<EntryId> {
        let mut ni = lock(&self.next_index);
        let slot = ni.entry(run).or_insert(0);
        let first = *slot;
        if first.saturating_add(n) > ENTRIES_PER_RUN {
            return Err(Error::InvalidInput(format!("a run holds at most {ENTRIES_PER_RUN} items")));
        }
        *slot = first + n;
        Ok(EntryId(run.0.saturating_mul(ENTRIES_PER_RUN) + first))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::testutil::TempDir;
    use mj_core::{ActionKind, FileStamp, JobId, RunKind, TargetKind};

    pub(crate) fn entry(id: u64, run: u64, state: JournalState, at: Timestamp) -> JournalEntry {
        JournalEntry {
            id: EntryId(id),
            run_id: RunId(run),
            job_id: JobId(1),
            step_id: None,
            at,
            state,
            action: ActionKind::Trash,
            target: TargetKind::File,
            original: PathBuf::from("/x/a"),
            original_stamp: FileStamp { size: 1, mtime: Timestamp(2) },
            original_sha256: None,
            bytes_before: 1,
            bytes_after: 0,
            retained_at: None,
            trash: None,
            created: vec![],
            link_source: None,
            reversible: true,
            undo_deadline: None,
            lossy: false,
            error: None,
        }
    }

    #[test]
    fn persists_and_replays_last_wins() {
        let t = TempDir::new("journal").unwrap();
        let dir = t.join("journal");
        let nov = Timestamp::from_ymd_hms(2026, 11, 13, 0, 0, 0);
        let dec = Timestamp::from_ymd_hms(2026, 12, 1, 0, 0, 0);
        {
            let j = Journal::open(&dir).unwrap();
            assert!(j.is_persistent());
            let run = j.next_run_id(nov);
            let first = j.reserve_entry_ids(run, 2).unwrap();
            assert_eq!(first.0, run.0 * ENTRIES_PER_RUN);
            j.append(&entry(first.0, run.0, JournalState::Intent, nov)).unwrap();
            j.append(&entry(first.0, run.0, JournalState::Done, dec)).unwrap();
            // Clock went back: the record still goes to the newest file and still wins.
            j.append(&entry(first.0 + 1, run.0, JournalState::Intent, nov)).unwrap();
            j.append(&entry(first.0 + 1, run.0, JournalState::Failed, nov)).unwrap();
            j.put_run(&RunRecord {
                run_id: run,
                kind: RunKind::Apply,
                title: "t".into(),
                started: nov,
                finished: None,
                steps: vec![],
                undo_of: None,
            })
            .unwrap();
            assert_eq!(j.reserve_entry_ids(run, 1).unwrap().0, first.0 + 2);
        }
        assert!(dir.join("entries-2026-11.jsonl").exists());
        assert!(dir.join("entries-2026-12.jsonl").exists());
        let j = Journal::open(&dir).unwrap();
        let es = j.all_entries();
        assert_eq!(es.len(), 2);
        assert_eq!(es[0].state, JournalState::Done);
        assert_eq!(es[1].state, JournalState::Failed);
        assert_eq!(j.runs().len(), 1);
        assert_eq!(j.replay_stats().ignored, 0);
        let run = es[0].run_id;
        assert_eq!(j.entries_of(run).len(), 2);
        assert!(j.next_run_id(nov).0 > run.0);
        // Reservations continue after the replayed ids.
        assert_eq!(j.reserve_entry_ids(run, 1).unwrap().0, es[1].id.0 + 1);
    }

    #[test]
    fn torn_final_line_is_ignored_and_repaired() {
        let t = TempDir::new("journal-torn").unwrap();
        let dir = t.mkdir("journal").unwrap();
        let at = Timestamp::from_ymd_hms(2026, 11, 13, 0, 0, 0);
        let good = serde_json::to_string(&entry(7, 0, JournalState::Done, at)).unwrap();
        let torn = &serde_json::to_string(&entry(8, 0, JournalState::Done, at)).unwrap()[..40];
        std::fs::write(dir.join("entries-2026-11.jsonl"), format!("{good}\nnot json\n{torn}")).unwrap();
        {
            let j = Journal::open(&dir).unwrap();
            assert_eq!(j.all_entries().len(), 1);
            let s = j.replay_stats();
            assert_eq!((s.lines, s.ignored, s.repaired), (1, 2, 1));
            j.append(&entry(9, 0, JournalState::Intent, at)).unwrap();
        }
        let j = Journal::open(&dir).unwrap();
        let ids: Vec<u64> = j.all_entries().iter().map(|e| e.id.0).collect();
        assert_eq!(ids, [7, 9]);
        assert_eq!(j.pending_intents().len(), 1);
        assert_eq!(j.replay_stats().repaired, 0);
        // Audit data is never deleted: the torn bytes are still in the file.
        let raw = std::fs::read_to_string(dir.join("entries-2026-11.jsonl")).unwrap();
        assert!(raw.contains(torn));
    }

    #[test]
    fn month_file_names() {
        assert_eq!(month_of_file("entries-2026-11.jsonl").as_deref(), Some("2026-11"));
        assert_eq!(month_of_file("entries-2026-1.jsonl"), None);
        assert_eq!(month_of_file("runs.jsonl"), None);
    }
}
