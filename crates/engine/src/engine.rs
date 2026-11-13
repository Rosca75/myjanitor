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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use mj_actions::Journal;
use mj_core::{Archiver, Detector, ExternalTool, Optimiser, RulePreset, ScanScope, Schedule, Settings};
use mj_platform::{NativePlatform, Platform};
use mj_scan::ScanCache;

use crate::api::{Event, EventSink};
use crate::{jobs, plan, results, scan_ops};

/// Construction parameters.
#[derive(Clone)]
pub struct EngineConfig {
    /// Settings, journal, quarantine, schedules (`paths::default_dirs`).
    pub data_dir: PathBuf,
    /// Scan cache.
    pub cache_dir: PathBuf,
    pub portable: bool,
    pub events: Arc<dyn EventSink>,
    pub platform: Arc<dyn Platform>,
    /// Load and save settings/presets/schedules/journal files. False in unit tests.
    pub persist: bool,
}

impl EngineConfig {
    /// Config for the given directories with the native platform and no events.
    pub fn new(data_dir: PathBuf, cache_dir: PathBuf) -> Self {
        EngineConfig {
            data_dir,
            cache_dir,
            portable: false,
            events: Arc::new(crate::api::NullSink),
            platform: Arc::new(NativePlatform::new()),
            persist: true,
        }
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
    next_id: AtomicU64,
}

impl Engine {
    /// Build the engine. Never fails because of a missing or corrupt state file: those
    /// fall back to defaults (and the journal to memory, reported by `app_info`).
    pub fn new(cfg: EngineConfig) -> mj_core::Result<Arc<Engine>> {
        if cfg.persist {
            for d in [&cfg.data_dir, &cfg.cache_dir] {
                std::fs::create_dir_all(d).map_err(|e| mj_core::Error::io(d, e))?;
            }
        }
        let journal = if cfg.persist {
            Journal::open(&cfg.data_dir.join("journal")).unwrap_or_else(|_| Journal::in_memory())
        } else {
            Journal::in_memory()
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
            next_id: AtomicU64::new(1),
            cfg,
        };
        engine.load_state();
        Ok(Arc::new(engine))
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
    }
}
