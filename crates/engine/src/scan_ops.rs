//! Scan orchestration (SCAN-04/09/10/11). Owner: W3-D.
//!
//! One scan = one generation, on one large-stack thread:
//!
//! 1. walk with `mj_scan::scan_with_cache` (cache hits/misses in `ScanStats`);
//! 2. run every detector and every *available* optimiser analysis on worker threads
//!    (`parallel_map`, each item isolated with `catch_unwind`), emitting a
//!    `scan://detector` event as each finishes (live per-card totals, SCAN-10);
//! 3. badge candidates: path badges (protected, synced, online-only, encrypted
//!    attribute) and content badges (signed, labelled, encrypted) for document files;
//! 4. if the generation is still current: prune and atomically save the cache,
//!    publish the results, refresh the plan, emit `scan://done` and
//!    `opportunities://changed`. A stale or stopped scan publishes nothing.
//!
//! Progress goes through `ThrottledSink` (50 ms; final values and phase changes pass).
//! Detectors' own progress is not forwarded (several run at once and their phases
//! would interleave); the engine reports `detecting` as analyses done / total.

use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use mj_core::detector::env;
use mj_core::{
    lock, Availability, Badges, CancelToken, DetectContext, Detector, DetectorOutput, Error, FileIndex, NullProgress,
    Optimiser, ParamValue, ParamValues, Phase, Progress, ProgressSink, Result, ScanScope, ThrottledSink, Timestamp,
};
use mj_platform::SyncRoot;
use mj_scan::ScanRequest;

use crate::api::{
    DetectorTotalsEvent, Event, OpportunitiesChanged, ScanProgressEvent, ScanRunState, ScanStartRequest, ScanStarted,
    ScanStatus, ScopeSetRequest,
};
use crate::opportunities::{build_record, disabled_record, CardInfo};
use crate::results::OpportunityRecord;
use crate::Engine;

/// Scan state of the session.
#[derive(Debug)]
pub struct ScanState {
    pub status: ScanStatus,
    pub cancel: Option<CancelToken>,
    /// The scan thread is alive (also while `stopping`).
    pub thread_alive: bool,
}

impl Default for ScanState {
    fn default() -> Self {
        ScanState {
            status: ScanStatus {
                state: ScanRunState::Idle,
                generation: 0,
                started: None,
                finished: None,
                progress: ScanProgressEvent::default(),
                stats: None,
                cached: false,
                error: None,
            },
            cancel: None,
            thread_alive: false,
        }
    }
}

fn running(s: ScanRunState) -> bool {
    matches!(s, ScanRunState::Scanning | ScanRunState::Paused | ScanRunState::Stopping)
}

/// One analysis to run.
#[derive(Clone, Copy)]
enum Analysis<'a> {
    Det(&'a dyn Detector),
    Opt(&'a dyn Optimiser),
}

/// Extensions whose content is inspected for signatures, labels and encryption.
const CONTENT_EXTS: &[&str] = &[
    "pdf", "docx", "docm", "dotx", "xlsx", "xlsm", "xltx", "pptx", "pptm", "potx", "doc", "xls", "ppt", "odt", "ods",
    "odp", "msg", "vsdx",
];

/// What the engine knows about the machine for detectors (`mj_core::detector::env`).
pub(crate) struct ScanEnv {
    pub sync_roots: Option<Vec<SyncRoot>>,
    pub downloads: Vec<PathBuf>,
    pub fonts: Vec<PathBuf>,
    pub protected: Vec<String>,
    /// `Settings::general::kept_languages`, comma-separated (empty = system language).
    pub kept_languages: String,
}

impl ScanEnv {
    pub(crate) fn detect(engine: &Engine) -> ScanEnv {
        let settings = lock(&engine.settings).clone();
        let home = crate::paths::home_dir();
        let mut downloads = Vec::new();
        let mut fonts = Vec::new();
        if let Some(h) = &home {
            downloads.push(h.join("Downloads"));
            if cfg!(windows) {
                if let Some(l) = std::env::var_os("LOCALAPPDATA") {
                    fonts.push(PathBuf::from(l).join("Microsoft").join("Windows").join("Fonts"));
                }
            } else {
                fonts.push(h.join(".fonts"));
                fonts.push(h.join(".local/share/fonts"));
            }
        }
        if cfg!(windows) {
            let windir = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| "C:\\Windows".into());
            fonts.push(windir.join("Fonts"));
        } else {
            fonts.push("/usr/share/fonts".into());
            fonts.push("/usr/local/share/fonts".into());
        }
        let mut protected: Vec<String> = mj_actions::protected::builtin_paths();
        protected.extend(mj_actions::protected::builtin_patterns());
        protected.extend(settings.safety.protected_paths.iter().map(|p| mj_core::pathutil::encode_path(p)));
        protected.extend(engine.cfg.app_dirs().iter().map(|p| mj_core::pathutil::encode_path(p)));
        let kept_languages = settings.general.kept_languages.join(", ");
        ScanEnv { sync_roots: engine.platform().sync_roots().ok(), downloads, fonts, protected, kept_languages }
    }

    /// Add the env keys, and fill W2-B's own folder parameters when the user left them empty.
    fn inject(&self, params: &mut ParamValues) {
        if let Some(roots) = &self.sync_roots {
            let paths: Vec<PathBuf> = roots.iter().map(|r| r.path.clone()).collect();
            params.set(env::SYNC_ROOTS, ParamValue::Text(env::encode_paths(&paths)));
            fill_text(params, "synced_folders", &paths);
        }
        params.set(env::DOWNLOADS, ParamValue::Text(env::encode_paths(&self.downloads)));
        params.set(env::FONT_FOLDERS, ParamValue::Text(env::encode_paths(&self.fonts)));
        params.set(env::PROTECTED, ParamValue::Text(env::encode(&self.protected)));
        fill_text(params, "downloads_folders", &self.downloads);
        fill_text(params, "font_folders", &self.fonts);
        if let Some(ParamValue::Text(t)) = params.0.get("kept_languages") {
            if t.trim().is_empty() && !self.kept_languages.is_empty() {
                params.set("kept_languages", ParamValue::Text(self.kept_languages.clone()));
            }
        }
    }
}

/// Set a text parameter (one path per line) when it exists and is empty.
fn fill_text(params: &mut ParamValues, key: &str, paths: &[PathBuf]) {
    if let Some(ParamValue::Text(t)) = params.0.get(key) {
        if t.trim().is_empty() && !paths.is_empty() {
            let v: Vec<String> = paths.iter().map(|p| p.to_string_lossy().into_owned()).collect();
            params.set(key, ParamValue::Text(v.join("\n")));
        }
    }
}

/// Parameters as the user sees them (env keys removed).
pub(crate) fn visible_params(p: &ParamValues) -> ParamValues {
    ParamValues(p.0.iter().filter(|(k, _)| !k.starts_with("env.")).map(|(k, v)| (k.clone(), v.clone())).collect())
}

impl Engine {
    /// Start scanning the scope.
    pub fn scan_start(self: &Arc<Self>, req: ScanStartRequest) -> Result<ScanStarted> {
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err(Error::Conflict("MyJanitor is shutting down".into()));
        }
        if let Some(scope) = req.scope {
            self.scope_set(ScopeSetRequest { scope })?;
        }
        let scope = self.scope_get()?;
        if scope.roots.is_empty() {
            return Err(Error::InvalidInput("add a location to scan first".into()));
        }
        let cancel = CancelToken::new();
        let generation = {
            let mut st = lock(&self.scan);
            if running(st.status.state) || st.thread_alive {
                return Err(Error::Conflict("a scan is already running".into()));
            }
            let generation = st.status.generation + 1;
            st.status = ScanStatus {
                state: ScanRunState::Scanning,
                generation,
                started: Some(Timestamp::now()),
                finished: None,
                progress: ScanProgressEvent { generation, ..Default::default() },
                stats: None,
                cached: false,
                error: None,
            };
            st.cancel = Some(cancel.clone());
            st.thread_alive = true;
            generation
        };
        let me = Arc::clone(self);
        let spawned = mj_core::pool::spawn_large("mj-scan", move || {
            me.run_scan(generation, scope, cancel);
            lock(&me.scan).thread_alive = false;
        });
        if let Err(e) = spawned {
            let mut st = lock(&self.scan);
            st.thread_alive = false;
            st.status.state = ScanRunState::Failed;
            return Err(Error::Internal(format!("cannot start the scan thread: {e}")));
        }
        Ok(ScanStarted { generation })
    }

    /// True while a scan thread is alive.
    pub(crate) fn scan_running(&self) -> bool {
        lock(&self.scan).thread_alive
    }

    /// True when `generation` is the newest scan.
    fn is_current(&self, generation: u64) -> bool {
        lock(&self.scan).status.generation == generation
    }

    fn run_scan(self: &Arc<Self>, generation: u64, mut scope: ScanScope, cancel: CancelToken) {
        let started = Instant::now();
        let settings = lock(&self.settings).clone();
        // AUT-05: the walk and hashing workers follow the settings when the scope does not
        // ask for a number itself (scan.threads, else performance.threads; 0 = every CPU).
        if scope.options.threads == 0 {
            scope.options.threads =
                if settings.scan.threads != 0 { settings.scan.threads } else { settings.performance.threads };
        }
        let emit_progress = |p: &Progress| {
            let walking = p.phase == Phase::Walking;
            let ev = ScanProgressEvent {
                generation,
                phase: Some(p.phase),
                files: if walking { p.done } else { 0 },
                dirs: 0,
                bytes: p.bytes,
                done: p.done,
                total: p.total,
                current: p.current.clone(),
                elapsed_ms: started.elapsed().as_millis() as u64,
            };
            // Later phases keep the file count of the walk: the event carries what the status
            // holds, so the header never falls back to 0 after the walk (IPC-4).
            let ev = {
                let mut st = lock(&self.scan);
                if st.status.generation != generation {
                    return;
                }
                let mut ev = ev;
                if !walking {
                    ev.files = st.status.progress.files;
                }
                st.status.progress = ev.clone();
                ev
            };
            self.emit(Event::ScanProgress(ev));
        };
        let sink = ThrottledSink::new(emit_progress);
        let outcome = catch_unwind(AssertUnwindSafe(|| -> Result<(Arc<FileIndex>, Vec<OpportunityRecord>)> {
            let req = ScanRequest {
                scope: &scope,
                exclusions: &settings.safety.exclusions,
                platform: self.platform(),
                cancel: &cancel,
                progress: &sink,
            };
            let index = Arc::new(mj_scan::scan_with_cache(&req, &self.cache)?);
            cancel.checkpoint()?;
            let mut records = self.run_analyses(generation, &index, &cancel, &sink)?;
            cancel.checkpoint()?;
            records.extend(self.custom_records(&index));
            Ok((index, records))
        }));
        let (state, error, stats) = match outcome {
            Ok(Ok((index, records))) => {
                let stats = index.stats.clone();
                if self.is_current(generation) {
                    if scope.options.use_cache && self.cfg.persist {
                        self.cache.prune(&index);
                        // Cache write failures are never fatal (ARCHITECTURE §6).
                        let _ = self.cache.save();
                    }
                    {
                        let mut results = self.results.write().unwrap_or_else(|e| e.into_inner());
                        let mut records = records;
                        crate::results::carry_selections(&results.opportunities, &mut records);
                        let skip = crate::results::skip_badges(&lock(&self.settings));
                        for rec in &mut records {
                            // ENG-7: carried selections keep the kept badge truthful.
                            crate::steps::sync_keepers(rec, skip);
                            crate::opportunities::recompute(rec, skip);
                        }
                        results.generation = generation;
                        results.index = Some(index);
                        results.removed.clear();
                        results.opportunities = records;
                        results.cached = stats.cache_hits > 0;
                    }
                    self.plan_after_rescan();
                }
                (ScanRunState::Done, None, Some(stats))
            }
            Ok(Err(Error::Cancelled)) => (ScanRunState::Cancelled, None, None),
            Ok(Err(e)) => (ScanRunState::Failed, Some(e.info()), None),
            Err(_) => {
                (ScanRunState::Failed, Some(Error::Internal("the scan stopped unexpectedly".into()).info()), None)
            }
        };
        let status = {
            let mut st = lock(&self.scan);
            if st.status.generation != generation {
                return;
            }
            st.status.state = state;
            st.status.finished = Some(Timestamp::now());
            st.status.error = error;
            st.status.cached = stats.as_ref().is_some_and(|s| s.cache_hits > 0);
            if let Some(s) = &stats {
                st.status.progress.files = s.files;
                st.status.progress.dirs = s.dirs;
            }
            st.status.stats = stats;
            st.cancel = None;
            st.status.clone()
        };
        self.emit(Event::ScanDone(status));
        if state == ScanRunState::Done {
            self.emit(Event::OpportunitiesChanged(OpportunitiesChanged { generation }));
        }
    }

    /// Effective parameters for an analysis: spec defaults, then the parameters the user
    /// chose in the review this session (DUP-2), then the plan step's parameters when the
    /// opportunity is in the plan.
    fn params_for(&self, card: &CardInfo) -> ParamValues {
        let mut params = mj_core::params::defaults(&card.settings);
        let plan = lock(&self.plan);
        if let Some(chosen) = plan.chosen.get(&card.id) {
            params = params.merged(chosen);
        }
        match plan.plan.steps.iter().find(|s| s.opportunity == card.id) {
            Some(step) => params.merged(&step.params),
            None => params,
        }
    }

    /// Run one analysis (detector or optimiser) with `params` on the index.
    pub(crate) fn run_one(
        &self,
        card: CardInfo,
        run: &(dyn Fn(&DetectContext<'_>) -> Result<DetectorOutput> + Sync),
        params: ParamValues,
        index: &FileIndex,
        env: &ScanEnv,
        cancel: &CancelToken,
    ) -> Result<OpportunityRecord> {
        let settings = lock(&self.settings).clone();
        if let Availability::Disabled { reason } = &card.availability {
            return Ok(disabled_record(self, card.clone(), reason.clone()));
        }
        let mut full = params.clone();
        env.inject(&mut full);
        let ctx = DetectContext {
            index,
            params: &full,
            now: Timestamp::now(),
            fingerprints: &self.cache,
            cancel,
            progress: &NullProgress,
            threads: settings.performance.threads as usize,
        };
        let output = match catch_unwind(AssertUnwindSafe(|| run(&ctx))) {
            Ok(Ok(o)) => o,
            Ok(Err(Error::Cancelled)) => return Err(Error::Cancelled),
            Ok(Err(e @ (Error::NotImplemented(_) | Error::Unsupported { .. }))) => {
                return Ok(disabled_record(self, card, e.to_string()))
            }
            Ok(Err(e)) => return Ok(disabled_record(self, card, format!("The analysis failed: {e}"))),
            Err(_) => return Ok(disabled_record(self, card, "The analysis failed (internal error)".into())),
        };
        let owned = card.safety == mj_core::SafetyClass::Rebuildable;
        let badges = self.badger(index, &output, owned, cancel);
        Ok(build_record(self, card, output, visible_params(&params), &|c| {
            badges.get(&c.path).copied().unwrap_or_default()
        }))
    }

    /// Path and content badges for every candidate of an output. A folder candidate is
    /// badged `protected` when the scanned tree below it holds a protected path (F-01);
    /// `owned` (a regenerable artefact card) exempts version-control metadata only.
    pub(crate) fn badger(
        &self,
        index: &FileIndex,
        out: &DetectorOutput,
        owned: bool,
        cancel: &CancelToken,
    ) -> HashMap<PathBuf, Badges> {
        let settings = lock(&self.settings).clone();
        let protected =
            mj_actions::ProtectedPaths::new(&settings.safety.protected_paths).with_app_dirs(&self.cfg.app_dirs());
        let inspector = mj_actions::SafetyInspector::new(self.platform(), protected.clone());
        let spellings = root_spellings(&index.roots);
        let mut map: HashMap<PathBuf, Badges> = HashMap::new();
        let mut to_read = Vec::new();
        for c in &out.candidates {
            let mut b = match index.get(&c.path) {
                Some(e) => inspector.path_badges(e),
                None => Badges::empty(),
            };
            if inspector.sync_root_of(&c.path).is_some() {
                b |= Badges::SYNCED;
            }
            // G-01: the candidate itself, in the canonical spelling of its scan root.
            if other_spelling(&spellings, &c.path).is_some_and(|q| protected.is_protected(&q)) {
                b |= Badges::PROTECTED;
            }
            if c.target == mj_core::TargetKind::Folder
                && folder_holds_protected(index, &protected, &c.path, owned, &spellings)
            {
                b |= Badges::PROTECTED;
            }
            let ext = mj_core::pathutil::extension_lower(&c.path);
            if c.target == mj_core::TargetKind::File
                && !b.contains(Badges::ONLINE_ONLY)
                && CONTENT_EXTS.contains(&ext.as_str())
            {
                to_read.push(c.path.clone());
            }
            map.insert(c.path.clone(), b);
        }
        let read = mj_core::pool::parallel_map(&to_read, 0, cancel, |_, p| inspector.content_badges(p).ok());
        for (p, b) in to_read.iter().zip(read) {
            if let Some(Some(b)) = b {
                *map.entry(p.clone()).or_default() |= b;
            }
        }
        map
    }

    fn run_analyses(
        &self,
        generation: u64,
        index: &Arc<FileIndex>,
        cancel: &CancelToken,
        sink: &dyn ProgressSink,
    ) -> Result<Vec<OpportunityRecord>> {
        let tools = lock(&self.tools).clone();
        let env = ScanEnv::detect(self);
        let mut list: Vec<(CardInfo, Analysis<'_>)> = Vec::new();
        for d in &self.detectors {
            list.push((CardInfo::detector(d.info()), Analysis::Det(d.as_ref())));
        }
        for o in &self.optimisers {
            list.push((CardInfo::optimiser(o.info(), o.availability(&tools)), Analysis::Opt(o.as_ref())));
        }
        let total = list.len() as u64;
        let done = AtomicU64::new(0);
        sink.report(&Progress::new(Phase::Detecting, 0, Some(total)));
        let workers = mj_core::pool::default_workers().clamp(2, 8);
        let results = mj_core::pool::parallel_map(&list, workers, cancel, |_, (card, a)| {
            let params = self.params_for(card);
            let rec = match a {
                Analysis::Det(d) => self.run_one(card.clone(), &|ctx| d.run(ctx), params, index, &env, cancel),
                Analysis::Opt(o) => self.run_one(card.clone(), &|ctx| o.analyse(ctx), params, index, &env, cancel),
            };
            if let Ok(r) = &rec {
                if self.is_current(generation) {
                    self.emit(Event::ScanDetector(DetectorTotalsEvent {
                        generation,
                        opportunity_id: r.opportunity.id.clone(),
                        title: r.opportunity.title.clone(),
                        saving: r.opportunity.saving,
                        count: r.opportunity.count,
                        finished: true,
                    }));
                }
            }
            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            sink.report(&Progress::new(Phase::Detecting, n, Some(total)));
            rec
        });
        cancel.checkpoint()?;
        let mut out = Vec::with_capacity(results.len());
        for (r, (card, _)) in results.into_iter().zip(list) {
            match r {
                Some(Ok(rec)) => out.push(rec),
                Some(Err(Error::Cancelled)) => return Err(Error::Cancelled),
                Some(Err(e)) => out.push(disabled_record(self, card, format!("The analysis failed: {e}"))),
                None => out.push(disabled_record(self, card, "The analysis failed (internal error)".into())),
            }
        }
        Ok(out)
    }

    /// ENG-12: the custom-search cards rebuilt on a new index from their stored rules. A
    /// rule that no longer runs (for example a type group that was deleted) loses its card.
    fn custom_records(self: &Arc<Self>, index: &FileIndex) -> Vec<OpportunityRecord> {
        let rules = self.results.read().unwrap_or_else(|e| e.into_inner()).custom_rules.clone();
        rules.iter().filter_map(|(id, req)| self.custom_record(index, req, id).ok()).collect()
    }

    /// Re-run one detector or optimiser with new parameters on the current index
    /// (step parameters changed). Returns the new record.
    pub(crate) fn rerun_analysis(&self, id: &mj_core::OpportunityId, params: ParamValues) -> Result<OpportunityRecord> {
        let (index, removed) = {
            let r = self.results.read().unwrap_or_else(|e| e.into_inner());
            (r.index.clone().ok_or_else(|| Error::InvalidInput("scan first".into()))?, r.removed.clone())
        };
        let tools = lock(&self.tools).clone();
        let env = ScanEnv::detect(self);
        let cancel = CancelToken::new();
        let mut rec = None;
        for d in &self.detectors {
            let card = CardInfo::detector(d.info());
            if &card.id == id {
                rec = Some(self.run_one(card, &|ctx| d.run(ctx), params.clone(), &index, &env, &cancel)?);
                break;
            }
        }
        if rec.is_none() {
            for o in &self.optimisers {
                let card = CardInfo::optimiser(o.info(), o.availability(&tools));
                if &card.id == id {
                    rec = Some(self.run_one(card, &|ctx| o.analyse(ctx), params.clone(), &index, &env, &cancel)?);
                    break;
                }
            }
        }
        let mut rec = rec.ok_or_else(|| crate::results::unknown(id))?;
        // ENG-11: the index still lists what was applied since it was built.
        let skip = crate::results::skip_badges(&lock(&self.settings));
        crate::results::drop_gone(&mut rec, &|p| crate::results::is_removed(&removed, p), skip);
        Ok(rec)
    }

    /// Pause the running scan.
    pub fn scan_pause(&self) -> Result<ScanStatus> {
        let mut st = lock(&self.scan);
        if st.status.state == ScanRunState::Scanning {
            if let Some(c) = &st.cancel {
                c.pause();
            }
            st.status.state = ScanRunState::Paused;
        }
        Ok(st.status.clone())
    }

    /// Resume a paused scan.
    pub fn scan_resume(&self) -> Result<ScanStatus> {
        let mut st = lock(&self.scan);
        if st.status.state == ScanRunState::Paused {
            if let Some(c) = &st.cancel {
                c.resume();
            }
            st.status.state = ScanRunState::Scanning;
        }
        Ok(st.status.clone())
    }

    /// Stop the scan (the previous results stay visible).
    pub fn scan_stop(&self) -> Result<ScanStatus> {
        let mut st = lock(&self.scan);
        if running(st.status.state) {
            if let Some(c) = &st.cancel {
                c.resume();
                c.cancel();
            }
            st.status.state = ScanRunState::Stopping;
        }
        Ok(st.status.clone())
    }

    /// Current scan status.
    pub fn scan_status(&self) -> Result<ScanStatus> {
        Ok(lock(&self.scan).status.clone())
    }
}

/// `(root, canonical root)` for every scan root whose canonical spelling differs (a
/// symlinked root, a junction, a mapped drive): G-01. Computed once per badging pass.
fn root_spellings(roots: &[PathBuf]) -> Vec<(PathBuf, PathBuf)> {
    roots.iter().filter_map(|r| mj_actions::protected::canonical_spelling(r).map(|c| (r.clone(), c))).collect()
}

/// `path` in the other spelling of its scan root, when it has one (G-01).
fn other_spelling(spellings: &[(PathBuf, PathBuf)], path: &std::path::Path) -> Option<PathBuf> {
    spellings.iter().find_map(|(r, c)| path.strip_prefix(r).ok().map(|rel| c.join(rel)))
}

/// True when the scanned tree below folder `path` holds a protected entry (F-01), in the
/// scan's spelling or in the canonical spelling of its root (G-01). The action engine
/// walks the real tree again before acting; this only badges the row.
fn folder_holds_protected(
    index: &FileIndex,
    protected: &mj_actions::ProtectedPaths,
    path: &std::path::Path,
    owned: bool,
    spellings: &[(PathBuf, PathBuf)],
) -> bool {
    let hit = |p: &std::path::Path| {
        let one = |q: &std::path::Path| {
            if owned {
                protected.is_protected_in_owned_tree(q)
            } else {
                protected.is_protected(q)
            }
        };
        one(p) || other_spelling(spellings, p).is_some_and(|q| one(&q))
    };
    if protected.has_protected_below(path)
        || other_spelling(spellings, path).is_some_and(|q| protected.has_protected_below(&q))
    {
        return true;
    }
    let Some(i) = index.position(path) else { return false };
    index.subtree(i).any(|j| hit(&index.entry(j).path))
}

#[cfg(test)]
mod tests {
    use super::folder_holds_protected;
    use mj_core::{EntryKind, FileEntry, FileIndex, ScanStats, Timestamp};
    use std::path::{Path, PathBuf};

    #[test]
    fn f01_folder_badge_sees_protected_entries_below() {
        let e = |p: &str, k| FileEntry::new(p, k, 1);
        let ix = FileIndex::new(
            vec![PathBuf::from("/r")],
            vec![
                e("/r", EntryKind::Dir),
                e("/r/proj", EntryKind::Dir),
                e("/r/proj/.git", EntryKind::Dir),
                e("/r/proj/.git/config", EntryKind::File),
                e("/r/proj/keep", EntryKind::Dir),
                e("/r/proj/keep/a.pdf", EntryKind::File),
                e("/r/plain", EntryKind::Dir),
                e("/r/plain/a.txt", EntryKind::File),
                e("/r/node_modules", EntryKind::Dir),
                e("/r/node_modules/x/.git/HEAD", EntryKind::File),
            ],
            ScanStats::default(),
            Timestamp(0),
        );
        let p = mj_actions::ProtectedPaths::new(&[]);
        assert!(folder_holds_protected(&ix, &p, Path::new("/r/proj"), false, &[]), ".git below");
        assert!(!folder_holds_protected(&ix, &p, Path::new("/r/plain"), false, &[]));
        assert!(folder_holds_protected(&ix, &p, Path::new("/r/node_modules"), false, &[]));
        assert!(!folder_holds_protected(&ix, &p, Path::new("/r/node_modules"), true, &[]), "owned artefact");
        let user = mj_actions::ProtectedPaths::empty().with_user_folders(&[PathBuf::from("/r/plain/sub")]);
        assert!(folder_holds_protected(&ix, &user, Path::new("/r/plain"), false, &[]), "lexical rule");
        // G-01: a folder protected under the real location of a symlinked root.
        let real = mj_actions::ProtectedPaths::empty().with_user_folders(&[PathBuf::from("/real/proj/keep")]);
        let sp = vec![(PathBuf::from("/r"), PathBuf::from("/real"))];
        assert!(!folder_holds_protected(&ix, &real, Path::new("/r/proj"), false, &[]));
        assert!(folder_holds_protected(&ix, &real, Path::new("/r/proj"), false, &sp), "other spelling");
        let real_file = mj_actions::ProtectedPaths::empty().with_patterns(&["/real/plain/a.txt".into()]);
        assert!(folder_holds_protected(&ix, &real_file, Path::new("/r/plain"), false, &sp), "entry below");
    }
}
