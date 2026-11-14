// @ts-check
/**
 * Timed simulations for the mock backend: the scan (progress and live
 * detector events), the apply queue, dry runs and undo. Delays are multiplied
 * by `speed` (0 in tests), and every timer is tracked so `whenIdle()` can wait
 * for the simulation to finish.
 */

const TICK = 140;

/**
 * @param {any} m model
 * @param {(event: string, payload: any) => void} emit
 * @param {(ms: number, fn: () => void) => void} later
 */
export function createSim(m, emit, later) {
  const progress = (p) => ({ generation: m.scan.generation, ...p });

  // ------------------------------------------------------------------ scan
  let scanTick = 0;
  const SCAN_TICKS = 24;
  const PHASES = [['walking', 0], ['hashing', 8], ['detecting', 14]];

  function scanStep() {
    if (m.scan.state === 'paused') return;
    if (m.scan.state === 'stopping') return finishScan('cancelled');
    scanTick += 1;
    const phase = [...PHASES].reverse().find(([, t]) => scanTick >= t)?.[0] ?? 'walking';
    const files = Math.round((scanTick / SCAN_TICKS) * 184233);
    m.scan.progress = progress({ phase, files, dirs: Math.round(files / 9), bytes: Math.round((scanTick / SCAN_TICKS) * 412e9), done: Math.min(18, Math.max(0, scanTick - 6)), total: 18, current: `C:\\Users\\Michel\\Projects\\app-${scanTick}\\node_modules`, elapsed_ms: scanTick * TICK });
    emit('scan://progress', structuredClone(m.scan.progress));
    const idx = scanTick - 7;
    if (idx >= 0 && idx < m.opps.length) {
      const o = m.opps[idx];
      emit('scan://detector', { generation: m.scan.generation, opportunity_id: o.id, title: o.title, saving: o.saving, count: o.count, finished: true });
    }
    if (scanTick >= SCAN_TICKS) return finishScan('done');
    later(TICK, scanStep);
  }

  /** @param {'done'|'cancelled'} state */
  function finishScan(state) {
    const first = m.scannedAt === null;
    m.scan = { ...m.scan, state, finished: m.nowNs(), cached: false };
    if (state === 'done') {
      m.scannedAt = m.nowNs();
      m.scan.stats = { files: 184233, dirs: 20117, links: 12, bytes: 412e9, allocated: 409e9, online_only_files: 310, online_only_bytes: 5.2e9, unreadable: 3, unreadable_sample: [], permission_denied: 3, excluded: 2, cache_hits: 0, cache_misses: 184233, elapsed_ms: m.scan.progress.elapsed_ms, used_fast_index: false };
    } else if (first) {
      m.scannedAt = null;
    }
    emit('scan://done', structuredClone(m.scan));
    emit('opportunities://changed', { generation: m.scan.generation });
  }

  m.startScan = () => {
    m.scan = { state: 'scanning', generation: m.scan.generation + 1, started: m.nowNs(), progress: progress({ phase: 'walking', files: 0, dirs: 0, bytes: 0, done: 0, elapsed_ms: 0 }), cached: false };
    scanTick = 0;
    later(TICK, scanStep);
  };
  m.resumeScan = () => {
    if (m.scan.state === 'paused') { m.scan.state = 'scanning'; later(TICK, scanStep); }
  };

  // ------------------------------------------------------------------ jobs
  const emitState = (job) => emit('job://state', m.jobJson(job));
  const isDone = (job) => ['completed', 'completed_with_errors', 'cancelled', 'failed'].includes(job.state);
  const okState = (j) => j.state === 'completed' || j.state === 'completed_with_errors';

  /** Queue runner: one job at a time, in order. */
  let running = false;
  function pump() {
    if (running) return;
    const next = m.jobs.find((j) => j.state === 'queued' && j.runner);
    if (!next) return;
    running = true;
    next.state = 'running';
    next.started = m.nowNs();
    emitState(next);
    later(TICK, () => jobStep(next, 0));
  }

  const STEPS = 8;
  /** @param {any} job @param {number} i */
  function jobStep(job, i) {
    if (job.state === 'paused') { job.resume = () => jobStep(job, i); return; }
    if (job.state === 'cancelling') return endJob(job, 'cancelled');
    const k = (i + 1) / STEPS;
    const t = job.totals;
    job.progress = { done: Math.round(t.total * k), total: t.total, bytes_done: Math.round(t.bytes * k), bytes_total: t.bytes, saved: Math.round(t.saved * k) };
    emit('job://progress', { job_id: job.id, run_id: job.run_id, progress: { ...job.progress }, current: job.currentPath(i) });
    if (i + 1 >= STEPS) return endJob(job, 'completed');
    later(TICK, () => jobStep(job, i + 1));
  }

  /** @param {any} job @param {string} state */
  function endJob(job, state) {
    job.finished = m.nowNs();
    job.outcome = job.makeOutcome && state === 'completed' ? job.makeOutcome(job) : { succeeded: job.progress.done, skipped: 0, failed: 0, bytes_saved: job.progress.saved, errors: [] };
    job.state = state === 'completed' && (job.outcome.failed > 0 || job.outcome.skipped > 0) ? 'completed_with_errors' : state;
    emitState(job);
    emit('job://done', { job: m.jobJson(job), report_available: job.kind === 'apply' });
    running = false;
    job.onEnd?.(job);
    pump();
  }

  /** @param {Record<string, any>} fields */
  function newJob(fields) {
    const job = { id: m.nextJobId++, created: m.nowNs(), state: 'queued', progress: { done: 0, total: fields.totals.total, bytes_done: 0, bytes_total: fields.totals.bytes, saved: 0 }, runner: true, ...fields };
    m.jobs.push(job);
    return job;
  }

  /** Strip the internals before a job goes over the wire. @param {any} j */
  m.jobJson = (j) => {
    const { runner, totals, currentPath, onEnd, resume, appliedStep, makeOutcome, ...wire } = j;
    void [runner, totals, currentPath, onEnd, resume, appliedStep, makeOutcome];
    return structuredClone(wire);
  };

  /** Queue one job per plan step; confirmations were validated by the caller. */
  m.startApply = () => {
    const runId = Math.round(m.nowNs() / 1e6);
    const steps = [...m.plan.steps];
    const created = steps.map((st) => newJob({
      kind: 'apply', run_id: runId, step_id: st.id, title: st.title,
      totals: { total: st.estimate.files, bytes: st.estimate.bytes_before, saved: st.estimate.bytes_before - st.estimate.bytes_after },
      currentPath: (i) => m.rows.get(st.opportunity)?.[i]?.path ?? '',
      makeOutcome: (job) => faultOutcome(job, st),
      onEnd: () => {
        if (m.jobs.filter((x) => x.run_id === runId && x.kind === 'apply').every(isDone)) finishRun(runId, steps);
      },
    }));
    const queued = created.map(m.jobJson); // snapshot while every job is still queued
    for (const j of created) emitState(j);
    pump();
    return { run_id: runId, jobs: queued };
  };

  /** Outcome of an apply job; the `rich` scenario injects skipped and failed items. */
  function faultOutcome(job, st) {
    const f = m.faults?.[st.opportunity];
    const base = { succeeded: job.progress.done, skipped: 0, failed: 0, bytes_saved: job.progress.saved, errors: [] };
    if (!f) return base;
    const rows = m.rows.get(st.opportunity) ?? [];
    const mkErr = (r, status, code, msg, hint) => ({ path: r.path, status, bytes_before: r.size, bytes_after: r.size, error: { code, message: `${msg}: ${r.path}`, hint, path: r.path } });
    const errors = [
      ...rows.slice(0, f.failed).map((r) => mkErr(r, 'failed', 'permission_denied', 'Permission denied', 'MyJanitor has no access to this path. Skip it, or run with access to it.')),
      ...rows.slice(f.failed, f.failed + f.skipped).map((r) => mkErr(r, 'skipped', 'changed', 'Changed since the scan', 'The file changed after the scan, so it was skipped. Scan again.')),
    ];
    return { succeeded: Math.max(0, job.progress.done - f.failed - f.skipped), skipped: f.skipped, failed: f.failed, bytes_saved: job.progress.saved, errors };
  }

  /** All jobs of a run ended: log history, remove applied steps. */
  function finishRun(runId, steps) {
    const jobs = m.jobs.filter((j) => j.run_id === runId && j.kind === 'apply');
    const doneJobs = jobs.filter(okState);
    const doneIds = new Set(doneJobs.map((j) => j.step_id));
    const saved = doneJobs.reduce((a, j) => a + (j.outcome?.bytes_saved ?? 0), 0);
    const files = doneJobs.reduce((a, j) => a + (j.outcome?.succeeded ?? 0), 0);
    const entries = [];
    for (const st of steps.filter((s) => doneIds.has(s.id))) {
      const picked = (m.rows.get(st.opportunity) ?? []).filter((x) => m.selection.get(st.opportunity)?.has(x.id)).slice(0, 40);
      for (const r of picked) {
        entries.push({ id: runId * 1000 + entries.length + 1, run_id: runId, job_id: jobs[0].id, step_id: st.id, at: m.nowNs(), state: 'done', action: st.action.kind, target: r.target, original: r.path, original_stamp: r.stamp, bytes_before: r.size, bytes_after: r.size_after, reversible: st.safety !== 'permanent' });
      }
    }
    const now = m.nowNs();
    m.runs.unshift({ run_id: runId, kind: 'apply', title: `Plan · ${steps.length} ${steps.length === 1 ? 'step' : 'steps'}`, started: jobs[0]?.started ?? now, finished: now, steps: steps.map((s) => s.title), items: files + jobs.reduce((a, j) => a + (j.outcome?.failed ?? 0) + (j.outcome?.skipped ?? 0), 0), done: files, failed: jobs.reduce((a, j) => a + (j.outcome?.failed ?? 0), 0), undone: 0, bytes_saved: saved, undoable: doneJobs.length > 0, undo_until: now + m.settings.general.undo_days * 86_400_000_000_000 });
    m.journals.set(runId, entries);
    m.freed += saved;
    m.plan.steps = m.plan.steps.filter((s) => !doneIds.has(s.id));
    m.plan.updated = now;
    emit('plan://changed', m.planJson());
    emit('history://changed', { run_id: runId });
    emit('opportunities://changed', { generation: m.scan.generation });
  }

  m.startDryRun = () => {
    const job = newJob({ kind: 'dry_run', title: 'Dry run', totals: { total: m.plan.steps.length, bytes: 0, saved: 0 }, currentPath: () => '', onEnd: (j) => {
      if (j.state !== 'completed') return;
      m.exact = true;
      m.plan.steps = m.plan.steps.map((s) => ({ ...s, estimate: { ...s.estimate, confidence: 'exact' } }));
      const plan = m.planJson();
      emit('plan://changed', plan);
      emit('plan://dry_run', { plan_id: plan.id, job_id: j.id, steps: plan.steps.map((s) => ({ step_id: s.id, title: s.title, estimate: s.estimate, skipped: [], skipped_count: 0, warnings: [] })), total: plan.estimate });
    } });
    emitState(job);
    pump();
    return m.jobJson(job);
  };

  /** @param {any} run */
  m.startUndo = (run) => {
    const entries = (m.journals.get(run.run_id) ?? []).filter((e) => e.reversible && ['done', 'undo_failed'].includes(e.state));
    const job = newJob({ kind: 'undo', run_id: run.run_id, title: `Undo ${run.title}`, totals: { total: run.items, bytes: 0, saved: 0 }, currentPath: (i) => entries[i]?.original ?? '', makeOutcome: (j) => settleRestore(j, entries), onEnd: (j) => {
      if (!okState(j)) return;
      run.undone = run.done - (m.journals.get(run.run_id) ?? []).filter((e) => e.state === 'undo_failed').length;
      run.undoable = (m.journals.get(run.run_id) ?? []).some((e) => e.state === 'undo_failed');
      emit('history://changed', { run_id: run.run_id });
      emit('opportunities://changed', { generation: m.scan.generation });
    } });
    emitState(job);
    pump();
    return m.jobJson(job);
  };

  /** Restore entries: an entry whose place is occupied is refused with the reason. @param {any} job @param {any[]} entries */
  function settleRestore(job, entries) {
    const errors = [];
    let ok = 0;
    for (const e of entries) {
      if (m.occupied?.has(e.id)) {
        e.state = 'undo_failed';
        e.error = { code: 'conflict', message: `The original location is occupied: ${e.original}`, hint: 'Move or rename the file that is there, then restore again.' };
        errors.push({ path: e.original, status: 'failed', bytes_before: e.bytes_before, bytes_after: e.bytes_after, error: { ...e.error, path: e.original } });
      } else { e.state = 'undone'; delete e.error; ok += 1; }
    }
    return { succeeded: ok, skipped: 0, failed: errors.length, bytes_saved: 0, errors };
  }

  /** @param {number[]} ids */
  m.startRestore = (ids) => {
    const all = [...m.journals.values()].flat();
    const entries = ids.map((id) => all.find((e) => e.id === id)).filter(Boolean);
    if (!entries.length) throw { code: 'invalid_input', message: 'None of these items can be restored', hint: 'Reload History and try again.' };
    const runId = entries[0].run_id;
    const job = newJob({ kind: 'undo', run_id: runId, title: `Restore ${entries.length} ${entries.length === 1 ? 'item' : 'items'}`, totals: { total: entries.length, bytes: 0, saved: 0 }, currentPath: (i) => entries[i]?.original ?? '', makeOutcome: (j) => settleRestore(j, entries), onEnd: (j) => {
      const run = m.runs.find((r) => r.run_id === runId);
      if (run && okState(j)) { const list = m.journals.get(runId) ?? []; run.undone = Math.min(run.done, (run.undone ?? 0) + j.outcome.succeeded); run.undoable = list.some((e) => e.reversible && ['done', 'undo_failed'].includes(e.state)); }
      emit('history://changed', { run_id: runId });
    } });
    emitState(job);
    pump();
    return m.jobJson(job);
  };

  m.startPurge = () => {
    const job = newJob({ kind: 'purge_quarantine', title: 'Purge quarantine', totals: { total: 38, bytes: 7408715776, saved: 0 }, currentPath: () => '', onEnd: () => { emit('history://changed', {}); } });
    emitState(job);
    pump();
    return m.jobJson(job);
  };

  /** @param {any} job */
  m.pauseJob = (job) => {
    if (job.state === 'running') { job.state = 'paused'; emitState(job); }
    return m.jobJson(job);
  };
  /** @param {any} job */
  m.resumeJob = (job) => {
    if (job.state === 'paused') { job.state = 'running'; emitState(job); const r = job.resume; job.resume = null; r?.(); }
    return m.jobJson(job);
  };
  /** @param {any} job */
  m.cancelJob = (job) => {
    if (job.state === 'queued') {
      job.state = 'cancelled';
      job.finished = m.nowNs();
      job.runner = false;
      emitState(job);
      emit('job://done', { job: m.jobJson(job), report_available: false });
      job.onEnd?.(job);
    } else if (job.state === 'running' || job.state === 'paused') {
      const wasPaused = job.state === 'paused';
      job.state = 'cancelling';
      emitState(job);
      if (wasPaused) { const r = job.resume; job.resume = null; r?.(); }
    }
    return m.jobJson(job);
  };
  return m;
}
