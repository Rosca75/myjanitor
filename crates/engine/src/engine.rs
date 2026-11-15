//! The [`Engine`] facade: one instance per process, shared by every command.
//! **Frozen layout (W1).** Command methods are implemented in one module per
//! command group (`impl Engine` blocks in `scope.rs`, `plan.rs`, ...); each
//! module owns its own state type, held here behind a lock.
//!
//! Threading: command methods are synchronous and fast. Long work
//! (`spawns: true` in `api::COMMANDS`) takes `self: &Arc<Engine>`, starts a
//! large-stack worker (`mj_core::pool::spawn_large`), returns at once and
//! reports through events. Locks are never held across I/O or while emitting.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use mj_actions::{Journal, TrashRoute};
use mj_core::{Archiver, Detector, ExternalTool, Optimiser, RulePreset, ScanScope, Schedule, Settings};
use mj_platform::{NativePlatform, Platform};
use mj_scan::ScanCache;

use crate::api::{Event, EventSink};
use crate::{jobs, plan, results, scan_ops};

/// Construction parameters. Every folder the engine writes to is explicit, so tests and
/// the CLI can point everything at temporary directories. The engine never scans or acts
/// on anything by default: the scope starts empty.
#[derive(Clone)]
pub struct EngineConfig {
    /// Settings, presets, schedules, run metadata (`paths::default_dirs`).
    pub data_dir: PathBuf,
    /// Scan cache.
    pub cache_dir: PathBuf,
    /// Undo journal (default `<data>/journal`).
    pub journal_dir: PathBuf,
    /// Quarantine (default `<data>/quarantine`).
    pub quarantine_dir: PathBuf,
    /// Scratch folder for dry-run optimiser outputs (default `<cache>/tmp`).
    pub temp_dir: PathBuf,
    /// Where "to Recycle Bin / Trash" goes. `None` (default) follows the OS rule (D-2E01)
    /// and, on Windows, the `windows_recycle_bin` setting. `Some(route)` pins the route:
    /// no setting can change it. Tests pin the quarantine so nothing ever reaches the
    /// user's real Trash or Recycle Bin.
    pub trash_route: Option<TrashRoute>,
    pub portable: bool,
    pub events: Arc<dyn EventSink>,
    pub platform: Arc<dyn Platform>,
    /// Load and save settings/presets/schedules/journal/cache files. False in unit tests
    /// that need no files at all (the journal is then in memory).
    pub persist: bool,
    /// Context-menu launch (`--optimise` / `--duplicates <path>`): the scope starts as
    /// that path and `app_info` reports it. Never scans or applies by itself.
    pub launch: Option<crate::api::LaunchIntent>,
    /// CLN-ACT-04: purge expired quarantine items when the engine starts (the GUI). The
    /// CLI turns it off and purges at the start of its acting commands instead.
    pub purge_at_start: bool,
}

impl EngineConfig {
    /// Config for the given directories with the native platform and no events.
    pub fn new(data_dir: PathBuf, cache_dir: PathBuf) -> Self {
        EngineConfig {
            journal_dir: data_dir.join("journal"),
            quarantine_dir: data_dir.join("quarantine"),
            temp_dir: cache_dir.join("tmp"),
            data_dir,
            cache_dir,
            trash_route: None,
            portable: false,
            events: Arc::new(crate::api::NullSink),
            platform: Arc::new(NativePlatform::new()),
            persist: true,
            launch: None,
            purge_at_start: true,
        }
    }

    /// Config with every folder given explicitly (config/settings, cache, journal, quarantine).
    pub fn with_dirs(config_dir: PathBuf, cache_dir: PathBuf, journal_dir: PathBuf, quarantine_dir: PathBuf) -> Self {
        let mut c = EngineConfig::new(config_dir, cache_dir);
        c.journal_dir = journal_dir;
        c.quarantine_dir = quarantine_dir;
        c
    }

    /// MyJanitor's own folders: always protected from every action.
    pub fn app_dirs(&self) -> Vec<PathBuf> {
        vec![
            self.data_dir.clone(),
            self.cache_dir.clone(),
            self.journal_dir.clone(),
            self.quarantine_dir.clone(),
            self.temp_dir.clone(),
        ]
    }
}

/// The engine.
pub struct Engine {
    pub(crate) cfg: EngineConfig,
    pub(crate) detectors: Vec<Box<dyn Detector>>,
    pub(crate) optimisers: Vec<Box<dyn Optimiser>>,
    /// Used by `Action::Archive` once apply is wired (W3-D).
    #[allow(dead_code)]
    pub(crate) archiver: Box<dyn Archiver>,
    pub(crate) journal: Journal,
    pub(crate) cache: ScanCache,
    pub(crate) settings: Mutex<Settings>,
    pub(crate) scope: Mutex<ScanScope>,
    pub(crate) scan: Mutex<scan_ops::ScanState>,
    pub(crate) results: RwLock<results::Results>,
    pub(crate) plan: Mutex<plan::PlanState>,
    pub(crate) jobs: Mutex<jobs::JobsState>,
    pub(crate) presets: Mutex<Vec<RulePreset>>,
    pub(crate) schedules: Mutex<Vec<Schedule>>,
    pub(crate) tools: Mutex<Vec<ExternalTool>>,
    /// Run metadata the journal does not hold (scope roots, step titles), persisted.
    pub(crate) run_meta: Mutex<std::collections::BTreeMap<u64, crate::history::RunMeta>>,
    /// Bytes freed by runs applied this session.
    pub(crate) reclaimed: AtomicU64,
    pub(crate) shutting_down: AtomicBool,
    /// F-08: why the persistent journal could not be opened (`None` when it is open, or
    /// when `persist` is off and the in-memory journal is intended). While set, every
    /// command that changes files refuses ([`Engine::require_writable`]).
    pub(crate) journal_error: Option<String>,
    /// F-13: the data folder's instance lock, held for the life of the engine (persistent
    /// engines only), or why another process holds it.
    #[allow(dead_code)]
    instance: Option<crate::instance::InstanceLock>,
    pub(crate) instance_error: Option<String>,
    /// Last step-table order (W6-A: paging re-uses it).
    pub(crate) review_order: Mutex<crate::review_order::OrderCache>,
    /// Review thumbnails (W6-A): byte-capped LRU and the large-decode gate.
    pub(crate) thumbs: crate::thumbs::ThumbCache,
    next_id: AtomicU64,
}

impl Engine {
    /// Build the engine. Never fails because of a missing or corrupt state file: those
    /// fall back to defaults (and the journal to memory, reported by `app_info`).
    pub fn new(cfg: EngineConfig) -> mj_core::Result<Arc<Engine>> {
        if cfg.persist {
            for d in [&cfg.data_dir, &cfg.cache_dir, &cfg.journal_dir, &cfg.quarantine_dir] {
                std::fs::create_dir_all(d).map_err(|e| mj_core::Error::io(d, e))?;
            }
        }
        // F-13: one acting process per data folder (GUI, CLI and scheduled runs alike).
        let (instance, instance_error) = if cfg.persist {
            match crate::instance::acquire(&cfg.data_dir) {
                Ok(l) => (Some(l), None),
                Err(e) => (None, Some(e)),
            }
        } else {
            (None, None)
        };
        // F-08: an unreadable journal is never silently replaced by one in memory for
        // acting: reading (scan, review, History) keeps working, changing files does not.
        let (journal, journal_error) = if cfg.persist {
            match Journal::open(&cfg.journal_dir) {
                Ok(j) => (j, None),
                Err(e) => (Journal::in_memory(), Some(format!("{}: {e}", cfg.journal_dir.display()))),
            }
        } else {
            (Journal::in_memory(), None)
        };
        let cache =
            if cfg.persist { ScanCache::open(&cfg.cache_dir.join("scan-cache.bin")) } else { ScanCache::in_memory() };
        let mut detectors = mj_dupes::detectors();
        detectors.extend(mj_detect::detectors());
        let engine = Engine {
            detectors,
            optimisers: mj_optimise::optimisers(),
            archiver: mj_optimise::archiver(),
            journal,
            cache,
            settings: Mutex::new(Settings::default()),
            scope: Mutex::new(ScanScope::default()),
            scan: Mutex::new(scan_ops::ScanState::default()),
            results: RwLock::new(results::Results::default()),
            plan: Mutex::new(plan::PlanState::default()),
            jobs: Mutex::new(jobs::JobsState::default()),
            presets: Mutex::new(Vec::new()),
            schedules: Mutex::new(Vec::new()),
            tools: Mutex::new(Vec::new()),
            run_meta: Mutex::new(Default::default()),
            reclaimed: AtomicU64::new(0),
            shutting_down: AtomicBool::new(false),
            journal_error,
            instance,
            instance_error,
            review_order: Mutex::new(Default::default()),
            thumbs: Default::default(),
            next_id: AtomicU64::new(1),
            cfg,
        };
        engine.load_state();
        // Recovery settles dangling intents; only the process holding the lock may do it
        // (another live process's intents are in flight, not dangling).
        if engine.instance_error.is_none() {
            engine.recover_journal();
            // CLN-ACT-04: expired quarantine items go automatically, through the guarded
            // purge (only exact quarantine slots, each item's own retention).
            if engine.cfg.purge_at_start && engine.cfg.persist {
                engine.quarantine_auto_purge();
            }
        }
        if let Some(l) = engine.cfg.launch.clone() {
            // Preselect the scope only; an invalid path is dropped (the app opens normally).
            let scope = mj_core::ScanScope { roots: vec![l.path.clone()], ..mj_core::ScanScope::default() };
            let exists = std::fs::symlink_metadata(&l.path).is_ok();
            if !exists || engine.scope_set(crate::api::ScopeSetRequest { scope }).is_err() {
                let mut e = engine;
                e.cfg.launch = None;
                return Ok(Arc::new(e));
            }
        }
        Ok(Arc::new(engine))
    }

    /// Graceful shutdown: stop the scan and the jobs between items, wait up to `wait` for
    /// them to settle, then save the scan cache. The journal is synced after every record,
    /// so nothing else needs flushing. Safe to call more than once.
    pub fn shutdown(&self, wait: Duration) {
        self.shutting_down.store(true, Ordering::SeqCst);
        let _ = self.scan_stop();
        self.cancel_all_jobs();
        let deadline = Instant::now() + wait;
        while Instant::now() < deadline && (self.scan_running() || self.jobs_running()) {
            std::thread::sleep(Duration::from_millis(20));
        }
        if self.cfg.persist {
            let _ = self.cache.save();
        }
        crate::history::save_run_meta(self);
    }

    /// Refuse a command that changes files when the journal is not durable (F-08).
    pub(crate) fn require_writable(&self) -> mj_core::Result<()> {
        if let Some(why) = &self.instance_error {
            return Err(mj_core::Error::Conflict(format!(
                "{why}: close it or wait for it to finish, then try again (only one MyJanitor may change files \
                 at a time)"
            )));
        }
        if let Some(why) = &self.journal_error {
            return Err(mj_core::Error::Conflict(format!(
                "the undo journal could not be opened ({why}). MyJanitor changes no file without it: \
                 check that this folder is readable and writable (permissions, antivirus, a full disk), \
                 then restart MyJanitor"
            )));
        }
        Ok(())
    }

    /// Why this engine may not change files because another process holds the data
    /// folder (F-13), if so. The GUI refuses to open a second window on it.
    pub fn instance_conflict(&self) -> Option<&str> {
        self.instance_error.as_deref()
    }

    /// Emit an event (never while holding a lock).
    pub(crate) fn emit(&self, event: Event) {
        self.cfg.events.emit(&event);
    }

    /// A fresh session-unique id (< 2^53).
    pub(crate) fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// The platform.
    pub fn platform(&self) -> &dyn Platform {
        self.cfg.platform.as_ref()
    }

    /// Load persisted settings, presets, schedules and tools at start-up.
    fn load_state(&self) {
        crate::settings::load(self);
        crate::presets::load(self);
        crate::schedules::load(self);
        crate::tools::load(self);
        crate::history::load_run_meta(self);
    }
}
