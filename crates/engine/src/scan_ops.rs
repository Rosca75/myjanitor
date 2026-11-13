//! Scan orchestration (SCAN-10/11). Owner: W3-D.
//!
//! Wave 1 wiring (real, minimal): walk with `mj_scan`, run every detector and
//! available optimiser analysis on the index (each isolated with
//! `catch_unwind`; `not_implemented`/`unsupported` outputs are skipped), build
//! opportunity records, publish them if the generation is still current, emit
//! `scan://progress` (throttled), `scan://detector`, `scan://done`.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;
use std::time::Instant;

use mj_core::{
    lock, Badges, CancelToken, DetectContext, Error, Progress, ProgressSink, Result, ScanScope, ThrottledSink,
    Timestamp,
};
use mj_scan::ScanRequest;

use crate::api::{
    DetectorTotalsEvent, Event, OpportunitiesChanged, ScanProgressEvent, ScanRunState, ScanStartRequest, ScanStarted,
    ScanStatus, ScopeSetRequest,
};
use crate::opportunities::{build_record, CardInfo};
use crate::results::OpportunityRecord;
use crate::Engine;

/// One analysis (detector run or optimiser analysis).
type AnalysisFn<'a> = dyn Fn(&DetectContext<'_>) -> Result<mj_core::DetectorOutput> + 'a;

/// Scan state of the session.
#[derive(Debug)]
pub struct ScanState {
    pub status: ScanStatus,
    pub cancel: Option<CancelToken>,
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
        }
    }
}

fn running(s: ScanRunState) -> bool {
    matches!(s, ScanRunState::Scanning | ScanRunState::Paused | ScanRunState::Stopping)
}

fn skippable(e: &Error) -> bool {
    matches!(e, Error::NotImplemented(_) | Error::Unsupported { .. })
}

impl Engine {
    /// Start scanning the scope.
    pub fn scan_start(self: &Arc<Self>, req: ScanStartRequest) -> Result<ScanStarted> {
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
            if running(st.status.state) {
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
            generation
        };
        let me = Arc::clone(self);
        mj_core::pool::spawn_large("mj-scan", move || me.run_scan(generation, scope, cancel))
            .map_err(|e| Error::Internal(format!("cannot start the scan thread: {e}")))?;
        Ok(ScanStarted { generation })
    }

    fn run_scan(&self, generation: u64, scope: ScanScope, cancel: CancelToken) {
        let started = Instant::now();
        let settings = lock(&self.settings).clone();
        let emit_progress = |p: &Progress| {
            let ev = ScanProgressEvent {
                generation,
                phase: Some(p.phase),
                files: 0,
                dirs: 0,
                bytes: p.bytes,
                done: p.done,
                total: p.total,
                current: p.current.clone(),
                elapsed_ms: started.elapsed().as_millis() as u64,
            };
            lock(&self.scan).status.progress = ev.clone();
            self.emit(Event::ScanProgress(ev));
        };
        let sink = ThrottledSink::new(emit_progress);
        let outcome =
            catch_unwind(AssertUnwindSafe(|| -> Result<(Arc<mj_core::FileIndex>, Vec<OpportunityRecord>)> {
                let req = ScanRequest {
                    scope: &scope,
                    exclusions: &settings.safety.exclusions,
                    platform: self.platform(),
                    cancel: &cancel,
                    progress: &sink,
                };
                let index = Arc::new(mj_scan::scan(&req)?);
                let records = self.run_analyses(generation, &index, &cancel, &sink)?;
                Ok((index, records))
            }));
        let (state, error, stats) = match outcome {
            Ok(Ok((index, records))) => {
                let stats = index.stats.clone();
                let mut results = self.results.write().unwrap_or_else(|e| e.into_inner());
                if lock(&self.scan).status.generation == generation {
                    results.generation = generation;
                    results.index = Some(index);
                    results.opportunities = records;
                    results.cached = stats.cache_hits > 0;
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
            st.status.stats = stats;
            st.cancel = None;
            st.status.clone()
        };
        self.emit(Event::ScanDone(status));
        if state == ScanRunState::Done {
            self.emit(Event::OpportunitiesChanged(OpportunitiesChanged { generation }));
        }
    }

    fn run_analyses(
        &self,
        generation: u64,
        index: &Arc<mj_core::FileIndex>,
        cancel: &CancelToken,
        sink: &dyn ProgressSink,
    ) -> Result<Vec<OpportunityRecord>> {
        let settings = lock(&self.settings).clone();
        let tools = lock(&self.tools).clone();
        let inspector = mj_actions::SafetyInspector::new(
            self.platform(),
            mj_actions::ProtectedPaths::new(&settings.safety.protected_paths),
        );
        let badges = |c: &mj_core::Candidate| -> Badges {
            index.get(&c.path).map(|e| inspector.path_badges(e)).unwrap_or_default()
        };
        let mut out = Vec::new();
        let mut jobs: Vec<(CardInfo, Box<AnalysisFn<'_>>)> = Vec::new();
        for d in &self.detectors {
            jobs.push((CardInfo::detector(d.info()), Box::new(move |ctx| d.run(ctx))));
        }
        for o in &self.optimisers {
            jobs.push((CardInfo::optimiser(o.info(), o.availability(&tools)), Box::new(move |ctx| o.analyse(ctx))));
        }
        let now = Timestamp::now();
        for (card, run) in jobs {
            cancel.checkpoint()?;
            let params = mj_core::params::defaults(&card.settings);
            let ctx = DetectContext {
                index,
                params: &params,
                now,
                fingerprints: &self.cache,
                cancel,
                progress: sink,
                threads: settings.performance.threads as usize,
            };
            let result = catch_unwind(AssertUnwindSafe(|| run(&ctx)));
            let output = match result {
                Ok(Ok(o)) => o,
                Ok(Err(Error::Cancelled)) => return Err(Error::Cancelled),
                Ok(Err(e)) if skippable(&e) => continue,
                Ok(Err(_)) | Err(_) => continue,
            };
            let rec = build_record(self, card, output, &badges);
            self.emit(Event::ScanDetector(DetectorTotalsEvent {
                generation,
                opportunity_id: rec.opportunity.id.clone(),
                title: rec.opportunity.title.clone(),
                saving: rec.opportunity.saving,
                count: rec.opportunity.count,
                finished: true,
            }));
            out.push(rec);
        }
        Ok(out)
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

    /// Stop the scan.
    pub fn scan_stop(&self) -> Result<ScanStatus> {
        let mut st = lock(&self.scan);
        if running(st.status.state) {
            if let Some(c) = &st.cancel {
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
