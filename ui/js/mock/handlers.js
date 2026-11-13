// @ts-check
/**
 * The command table of the mock backend. Every handler takes the request and
 * returns exactly the docs/IPC.md response type; failures throw ErrorInfo.
 */
import { FS, GB } from './fixtures.js';
import { buildCandidates } from './candidates.js';

/** @param {string} code @param {string} message @param {string} [hint] */
export const fail = (code, message, hint = 'Check the request and try again.') => ({ code, message, hint });

const lc = (s) => String(s).toLowerCase();
const sameOrInside = (child, parent) => lc(child) === lc(parent) || lc(child).startsWith(lc(parent).replace(/[\\/]+$/, '') + '\\');

/** Wire shape of a candidate row. @param {any} m @param {string} oppId @param {any} r */
function rowJson(m, oppId, r) {
  const { base_after, ...rest } = r;
  void base_after;
  const o = m.opp(oppId);
  const protectedRow = r.badges.includes('protected');
  return { ...structuredClone(rest), selected: !!m.selection.get(oppId)?.has(r.id), actionable: !protectedRow && o.safety !== 'review_only' };
}

/** Preset-dependent size after the action for audio rows. @param {any} m @param {any} o */
function reestimateAudio(m, o) {
  const preset = o.params.preset ?? 'speech';
  const ratio = { transcription_safe: 0.15, lossless: 0.55 }[preset];
  const trim = o.params.remove_silences === false ? 1.08 : 1;
  for (const r of m.rows.get(o.id)) {
    if (r.badges.includes('duplicate') || r.size_after === r.size) continue;
    r.size_after = preset === 'speech' ? Math.round(r.base_after * trim) : Math.round(r.size * ratio);
  }
}

/** Keeper index of a duplicate group for a rule. @param {any[]} members @param {any} rule */
function keeperOf(members, rule) {
  const by = (f, dir) => [...members].sort((a, b) => dir * (f(a) - f(b)))[0];
  switch (rule.rule) {
    case 'oldest': return by((r) => r.modified, 1);
    case 'shortest_path': return by((r) => r.path.length, 1);
    case 'highest_resolution': return by((r) => r.size, -1);
    case 'preferred_folder': return members.find((r) => lc(r.path).startsWith(lc(rule.folder))) ?? by((r) => r.modified, -1);
    default: return by((r) => r.modified, -1);
  }
}

/** Deterministic waveform for the preview. @param {any} row */
function waveform(row) {
  const peaks = [];
  let x = row.id * 9301 + 49297;
  for (let i = 0; i < 96; i++) {
    x = (x * 233280 + 1) % 2147483647;
    peaks.push(Math.round(40 + (x % 160)));
  }
  const dur = String(row.columns?.length ?? '0:30:00').split(':').reduce((a, v) => a * 60 + Number(v), 0) * 1000;
  const silences = [[Math.round(dur * 0.16), Math.round(dur * 0.22)], [Math.round(dur * 0.47), Math.round(dur * 0.52)], [Math.round(dur * 0.82), Math.round(dur * 0.9)]];
  return { kind: 'waveform', waveform: { peaks: peaks.map((p, i) => (silences.some(([a, b]) => (i / 96) * dur >= a && (i / 96) * dur <= b) ? 3 : p)), duration_ms: dur, silences } };
}

const QUERY_KEYS = ['ext', 'name', 'path', 'size', 'modified', 'created', 'accessed', 'type', 'attr', 'owner', 'depth', 'pathlen', 'kind', 'content', 'is', 'top'];
const UNIT = { k: 1024, m: 1024 ** 2, g: GB, t: 1024 ** 4 };

/** Quick query to a rule, with byte-offset errors. @param {string} query */
function parseQuery(query) {
  const conds = [];
  const errors = [];
  for (const m of query.matchAll(/\S+/g)) {
    const tok = m[0];
    const i = tok.indexOf(':');
    if (i < 0) { conds.push({ cond: { field: 'name', mode: 'contains', pattern: tok, case_sensitive: false } }); continue; }
    const key = tok.slice(0, i);
    const val = tok.slice(i + 1);
    if (!QUERY_KEYS.includes(key)) { errors.push({ offset: m.index, len: tok.length, message: `Unknown key "${key}"` }); continue; }
    if (key === 'ext') conds.push({ cond: { field: 'extension', values: val.split(',') } });
    else if (key === 'size') {
      const mm = /^(<=|>=|<|>|=)?(\d+(?:\.\d+)?)([kmgt])?b?$/i.exec(val);
      if (!mm) { errors.push({ offset: m.index + i + 1, len: val.length, message: 'Expected a size such as >1GB' }); continue; }
      const cmp = { '<': 'lt', '<=': 'le', '=': 'eq', '>=': 'ge', '>': 'gt', undefined: 'eq' }[mm[1]];
      conds.push({ cond: { field: 'size', cmp, bytes: Math.round(Number(mm[2]) * (UNIT[(mm[3] ?? '').toLowerCase()] ?? 1)) } });
    } else conds.push({ cond: { field: key === 'name' || key === 'path' ? key : 'name', mode: 'wildcard', pattern: val, case_sensitive: false } });
  }
  return { rule: { all: conds }, errors };
}

/**
 * @param {any} m the model
 * @param {any} sim the simulations (startScan, startApply, ...)
 * @param {(event: string, payload: any) => void} emit
 */
export function createHandlers(m, sim, emit) {
  /** @param {string} id */
  const needOpp = (id) => m.opp(id) ?? (() => { throw fail('invalid_input', `Unknown opportunity: ${id}`, 'Scan again to refresh the list.'); })();
  /** @param {number} id */
  const needStep = (id) => m.plan.steps.find((s) => s.id === id) ?? (() => { throw fail('invalid_input', `Unknown plan step: ${id}`, 'The plan changed. Reopen it.'); })();
  /** @param {number} id */
  const needJob = (id) => m.jobs.find((j) => j.id === id) ?? (() => { throw fail('invalid_input', `Unknown job: ${id}`, 'The queue changed.'); })();
  const planChanged = () => { m.plan.updated = m.nowNs(); const p = m.planJson(); emit('plan://changed', p); return p; };
  /** After a selection or parameter change keep cards and plan in step. @param {string} id */
  const touched = (id) => { m.refresh(id); emit('opportunities://changed', { generation: m.scan.generation }); if (m.step(id)) planChanged(); };
  const scanStatus = () => structuredClone(m.scan);
  const notYet = (what) => () => { throw fail('not_implemented', `Not implemented yet: ${what}`, 'This feature is not available in this build yet.'); };

  /** @param {any} run */
  const reportOf = (run) => {
    const entries = m.journals.get(run.run_id) ?? [];
    const sections = run.steps.map((title) => ({ title, action: 'Applied', totals: { files: 0, succeeded: 0, skipped: 0, failed: 0, bytes_before: 0, bytes_after: 0, bytes_saved: 0 }, items: entries.filter((e) => m.plan && true).slice(0, 0) }));
    return { run_id: run.run_id, title: run.title, started: run.started, finished: run.finished, totals: { files: run.items, succeeded: run.done, skipped: 0, failed: run.failed, bytes_before: run.bytes_saved, bytes_after: 0, bytes_saved: run.bytes_saved }, sections, undo_until: run.undo_until };
  };

  return {
    app_info: () => ({ name: 'MyJanitor', version: '0.1.0', platform: 'windows', bundle_id: 'com.myjanitor.app', portable: false, data_dir: 'C:\\Users\\Michel\\AppData\\Roaming\\MyJanitor', cache_dir: m.cache.path, elevated: false, journal_persistent: true }),

    scope_get: () => m.scope,
    scope_set: (req) => (m.scope = { ...m.scope, ...req.scope }),
    scope_add_root: (req) => {
      const p = String(req.path ?? '').trim();
      if (!/^([A-Za-z]:\\|\/)/.test(p)) throw fail('invalid_input', `Not an absolute folder path: ${p}`, 'Pick a folder from the list.');
      if (m.scope.roots.some((r) => sameOrInside(p, r))) return m.scope;
      m.scope.roots = [...m.scope.roots.filter((r) => !sameOrInside(r, p)), p];
      return m.scope;
    },
    scope_remove_root: (req) => { m.scope.roots = m.scope.roots.filter((r) => lc(r) !== lc(req.path)); return m.scope; },
    fs_list_dir: (req) => {
      if (!req.path) return { path: '', entries: [], places: FS.places };
      const p = String(req.path);
      const names = FS.dirs[p] ?? FS.dirs[p.replace(/[\\/]+$/, '')] ?? [];
      const parent = p.length <= 3 ? undefined : p.slice(0, Math.max(p.lastIndexOf('\\'), 2) + (p.lastIndexOf('\\') === 2 ? 1 : 0));
      const join = (n) => `${p.replace(/\\$/, '')}\\${n}`;
      return { path: p, ...(parent ? { parent } : {}), entries: names.map((n) => ({ name: n, path: join(n), is_dir: true, is_link: false, hidden: false })), places: FS.places };
    },

    scan_start: (req) => {
      if (m.scan.state === 'scanning' || m.scan.state === 'paused') throw fail('conflict', 'A scan is already running', 'Wait for it to finish or stop it.');
      if (req?.scope) m.scope = { ...m.scope, ...req.scope };
      if (!m.scope.roots.length) throw fail('invalid_input', 'The scan scope has no folder', 'Add at least one folder to scan.');
      sim.startScan();
      return { generation: m.scan.generation };
    },
    scan_pause: () => { if (m.scan.state === 'scanning') m.scan.state = 'paused'; return scanStatus(); },
    scan_resume: () => { sim.resumeScan(); return scanStatus(); },
    scan_stop: () => {
      if (m.scan.state === 'scanning' || m.scan.state === 'paused') { const was = m.scan.state; m.scan.state = 'stopping'; if (was === 'paused') sim.resumeScan(); }
      return scanStatus();
    },
    scan_status: scanStatus,

    opportunities_list: (req) => m.list(req ?? {}),
    step_detail: (req) => {
      const o = needOpp(req.opportunity_id);
      const lossy = !!m.lossy.get(o.id);
      return { opportunity: { ...structuredClone(o), in_plan: !!m.step(o.id) }, summary: m.summary(o.id), diagnostics: { unreadable: 0, unreadable_sample: [], skipped: {}, notes: o.id === 'opt:audio' ? ['Found in Teams, Zoom and local Recordings folders. 6 are the same meeting saved twice (audio fingerprint).'] : [] }, lossy, safety: o.safety };
    },
    candidates_page: (req) => {
      const o = needOpp(req.opportunity_id);
      let list = [...(m.rows.get(o.id) ?? [])];
      if (req.sub_group) list = list.filter((r) => r.sub_group === req.sub_group);
      if (req.filter) list = list.filter((r) => lc(r.path).includes(lc(req.filter)));
      const key = { saving: (r) => r.size - r.size_after, size: (r) => r.size, modified: (r) => r.modified, name: (r) => lc(r.path.split('\\').pop()), path: (r) => lc(r.path) }[req.sort ?? 'group'];
      if (key) list.sort((a, b) => (key(a) > key(b) ? 1 : key(a) < key(b) ? -1 : 0));
      if (req.descending) list.reverse();
      const limit = Math.min(500, Math.max(1, req.limit ?? 100));
      const page = list.slice(req.offset ?? 0, (req.offset ?? 0) + limit);
      const gs = new Set(page.map((r) => r.group).filter(Boolean));
      return { opportunity_id: o.id, total: list.length, offset: req.offset ?? 0, rows: page.map((r) => rowJson(m, o.id, r)), groups: (m.groups.get(o.id) ?? []).filter((g) => gs.has(g.id)) };
    },
    selection_set: (req) => {
      const o = needOpp(req.opportunity_id);
      const sel = m.selection.get(o.id) ?? new Set();
      let list = m.rows.get(o.id) ?? [];
      if (req.ids) { const ids = new Set(req.ids); list = list.filter((r) => ids.has(r.id)); } else if (req.sub_group) list = list.filter((r) => r.sub_group === req.sub_group);
      for (const r of list) {
        if (r.badges.includes('protected') || o.safety === 'review_only') continue;
        if (req.selected) sel.add(r.id); else sel.delete(r.id);
      }
      m.selection.set(o.id, sel);
      touched(o.id);
      return m.summary(o.id);
    },
    selection_apply_keep_rule: (req) => {
      const o = needOpp(req.opportunity_id);
      const sel = new Set();
      for (const g of m.groups.get(o.id) ?? []) {
        const members = (m.rows.get(o.id) ?? []).filter((r) => r.group === g.id);
        const keeper = keeperOf(members, req.rule);
        for (const r of members) {
          const isKeeper = r === keeper;
          r.badges = isKeeper ? ['duplicate', 'keeper'] : ['duplicate'];
          r.reason = isKeeper ? `Kept: ${req.rule.rule.replace('_', ' ')}` : `Same content as ${members.length - 1} other files`;
          if (!isKeeper) sel.add(r.id);
        }
        g.keeper = keeper.path;
      }
      m.selection.set(o.id, sel);
      o.params = { ...o.params, keep_rule: req.rule.rule };
      const st = m.step(o.id);
      if (st) st.params = { ...st.params, keep_rule: req.rule.rule };
      touched(o.id);
      return m.summary(o.id);
    },
    selection_reset: (req) => {
      const o = needOpp(req.opportunity_id);
      m.selection.set(o.id, new Set((m.rows.get(o.id) ?? []).filter((r) => r.default_selected).map((r) => r.id)));
      touched(o.id);
      return m.summary(o.id);
    },
    step_params_set: (req) => {
      const o = needOpp(req.opportunity_id);
      const preset = req.preset_id ? o.presets.find((p) => p.id === req.preset_id) : null;
      if (req.preset_id && !preset) throw fail('invalid_input', `Unknown preset: ${req.preset_id}`);
      if (preset) { o.params = { ...o.params, ...preset.values, preset: preset.id }; m.lossy.set(o.id, !!preset.lossy); }
      o.params = { ...o.params, ...(req.params ?? {}) };
      if (req.params && 'remove_silences' in req.params && o.params.remove_silences === false && !preset) m.lossy.set(o.id, o.params.preset ? m.lossy.get(o.id) : true);
      o.safety = m.lossy.get(o.id) ? 'lossy_opt_in' : 'reversible';
      if (o.id === 'opt:audio') reestimateAudio(m, o);
      const st = m.step(o.id);
      if (st) st.params = { ...st.params, ...o.params };
      touched(o.id);
      return { params: structuredClone(o.params), summary: m.summary(o.id), lossy: !!m.lossy.get(o.id), safety: o.safety };
    },
    candidate_preview: (req) => {
      for (const [, list] of m.rows) {
        const r = list.find((x) => x.id === req.candidate_id);
        if (r) return r.source.optimiser === 'audio' ? waveform(r) : { kind: 'none', reason: 'No preview for this file type' };
      }
      throw fail('invalid_input', `Unknown candidate: ${req.candidate_id}`, 'Scan again to refresh the list.');
    },

    search_parse: (req) => parseQuery(String(req.query ?? '')),
    search_suggest: (req) => {
      const word = String(req.query ?? '').slice(0, req.cursor ?? undefined).split(/\s+/).pop() ?? '';
      return QUERY_KEYS.filter((k) => k.startsWith(word) && word.length > 0 && !word.includes(':')).map((k) => ({ insert: `${k}:`, replace_len: word.length, label: `${k}:` }));
    },
    search_run: (req) => {
      if (m.scannedAt === null) throw fail('invalid_input', 'Scan before searching', 'Run a scan from Overview first.');
      const title = req.title || 'Custom search';
      let o = m.opps.find((x) => x.title === title && x.id.startsWith('custom:'));
      const n = 12 + (JSON.stringify(req.rule).length % 40);
      if (!o) {
        o = { id: `custom:${m.nextCustom++}`, source: { custom: m.nextCustom }, category: 'clean', title, description: 'Your own search', safety: 'reversible', size_before: n * 8 * 1024 * 1024, saving: n * 8 * 1024 * 1024, confidence: 'exact', count: n, unit: 'files', candidates: n, selected: n, badges: [], default_action: req.action ?? { kind: 'trash' }, available_actions: ['trash', 'quarantine', 'permanent_delete'], presets: [], settings: [], params: {}, sub_groups: [], availability: { state: 'available' }, in_plan: false, rank: 1e8, feature_ids: ['CLN-ENG-05'] };
        m.opps.push(o);
        const built = buildCandidates([o]);
        m.rows.set(o.id, built.rows.get(o.id));
        m.selection.set(o.id, new Set(built.rows.get(o.id).map((r) => r.id)));
      }
      emit('opportunities://changed', { generation: m.scan.generation });
      return structuredClone(o);
    },
    presets_list: () => [
      { id: 'downloads_older_90d', name: 'Downloads older than 90 days', description: 'Anything in a Downloads folder not modified for 90 days', rule: { all: [{ cond: { field: 'path', mode: 'wildcard', pattern: '**/Downloads/**', case_sensitive: false } }] }, topmost_only: false, builtin: true },
      ...m.presets,
    ],
    preset_save: notYet('preset_save'),
    preset_delete: notYet('preset_delete'),

    plan_get: () => m.planJson(),
    plan_add_step: (req) => {
      const o = needOpp(req.opportunity_id);
      if (m.step(o.id)) throw fail('conflict', `${o.title} is already in the plan`, 'Remove it first to change it.');
      if (o.safety === 'review_only' || o.availability.state === 'disabled') throw fail('unsupported', `${o.title} has no automatic action`, 'Review these files by hand.');
      const action = req.action ?? m.actions.get(o.id);
      m.plan.steps.push(m.buildStep(o, action ? { action, params: req.params ?? {} } : req.params ? { params: req.params } : undefined));
      return planChanged();
    },
    plan_update_step: (req) => {
      const st = needStep(req.step_id);
      const o = needOpp(st.opportunity);
      const next = m.buildStep(o, { ...st, action: req.action ?? st.action, params: { ...st.params, ...(req.params ?? {}) } });
      Object.assign(st, next);
      m.actions.set(o.id, st.action);
      return planChanged();
    },
    plan_remove_step: (req) => { needStep(req.step_id); m.plan.steps = m.plan.steps.filter((s) => s.id !== req.step_id); return planChanged(); },
    plan_reorder: (req) => {
      const ids = req.step_ids ?? [];
      if (ids.length !== m.plan.steps.length || !m.plan.steps.every((s) => ids.includes(s.id))) throw fail('invalid_input', 'The reorder list must name every step exactly once', 'Reload the plan and try again.');
      m.plan.steps = ids.map((id) => needStep(id));
      return planChanged();
    },
    plan_clear: () => { m.plan.steps = []; return planChanged(); },
    plan_dry_run: () => {
      if (!m.plan.steps.length) throw fail('invalid_input', 'The plan is empty', 'Add a step first.');
      return sim.startDryRun();
    },
    plan_confirmation_info: (req) => {
      const st = needStep(req.step_id);
      const o = needOpp(st.opportunity);
      const n = st.estimate.files;
      const permanent = st.safety === 'permanent';
      const audio = o.id === 'opt:audio';
      return {
        step_id: st.id,
        title: permanent ? `Delete ${n} ${o.unit} permanently?` : `Compress ${n} ${audio ? 'recordings' : o.unit}?`,
        body: permanent ? 'This cannot be undone. The files do not go to the Recycle Bin and History cannot bring them back.' : audio ? 'This is lossy. The new files sound fine for speech but cannot be turned back into the originals.' : 'This is lossy. The new files cannot be turned back into the originals.',
        files: n,
        bytes_now: st.estimate.bytes_before,
        bytes_after: st.estimate.bytes_after,
        saving: st.estimate.bytes_before - st.estimate.bytes_after,
        confidence: st.estimate.confidence,
        originals_options: permanent ? [] : [
          { policy: { policy: 'recycle_bin' }, label: 'Move to Recycle Bin (default)', reversible: true },
          { policy: { policy: 'quarantine' }, label: 'Keep in quarantine for 30 days', reversible: true },
          { policy: { policy: 'delete' }, label: 'Delete permanently', reversible: false },
        ],
        default_originals: { policy: 'recycle_bin' },
        ...(audio || o.id === 'det:old_installers' ? { synced: { count: audio ? 6 : 3, provider: 'OneDrive – Acme', can_online_only: true } } : {}),
        acknowledge_text: permanent ? 'I understand these files cannot be recovered' : 'I understand the new files are lower quality than the originals',
        undo_text: permanent ? 'This step is not undoable. Everything else in the plan stays in History.' : `Every change is logged; History → Restore brings the originals back for ${m.settings.general.undo_days} days.`,
        action_label: permanent ? `Delete ${n} files permanently` : `Compress ${n} files`,
        permanent,
      };
    },
    plan_apply: (req) => {
      if (!m.plan.steps.length) throw fail('not_implemented', 'Nothing to apply: the plan is empty', 'Add a step to the plan first.');
      if (m.jobs.some((j) => j.kind === 'apply' && ['queued', 'running', 'paused'].includes(j.state))) throw fail('conflict', 'A plan is already running', 'Wait for it to finish.');
      for (const st of m.plan.steps) {
        if (st.safety !== 'lossy_opt_in' && st.safety !== 'permanent') continue;
        const c = (req.confirmations ?? []).find((x) => x.step_id === st.id);
        if (!c?.acknowledged) throw fail('confirmation_required', `Confirmation required: step ${st.id} (${st.title}) needs confirmation`, 'Review the step and confirm it in the dialog.');
      }
      return sim.startApply();
    },

    jobs_list: () => m.jobs.map(m.jobJson),
    job_pause: (req) => sim.pauseJob(needJob(req.job_id)),
    job_resume: (req) => sim.resumeJob(needJob(req.job_id)),
    job_cancel: (req) => sim.cancelJob(needJob(req.job_id)),
    jobs_reorder: (req) => {
      const queued = m.jobs.filter((j) => j.state === 'queued');
      const order = (req.job_ids ?? []).map(needJob);
      const rest = m.jobs.filter((j) => !queued.includes(j));
      m.jobs = [...rest, ...order, ...queued.filter((j) => !order.includes(j))];
      return m.jobs.map(m.jobJson);
    },

    history_list: (req) => ({ total: m.runs.length, runs: structuredClone(m.runs.slice(req?.offset ?? 0, (req?.offset ?? 0) + (req?.limit ?? 50))) }),
    history_run_detail: (req) => {
      const run = m.runs.find((r) => r.run_id === req.run_id) ?? (() => { throw fail('not_found', `Unknown run: ${req.run_id}`); })();
      const all = m.journals.get(run.run_id) ?? [];
      return { run: structuredClone(run), total: all.length, entries: structuredClone(all.slice(req.offset ?? 0, (req.offset ?? 0) + (req.limit ?? 100))) };
    },
    undo_last: () => {
      const run = m.runs.find((r) => r.undoable);
      if (!run) throw fail('invalid_input', 'Nothing can be undone', 'History has no run inside its undo window.');
      return sim.startUndo(run);
    },
    undo_run: (req) => {
      const run = m.runs.find((r) => r.run_id === req.run_id && r.undoable) ?? (() => { throw fail('invalid_input', 'This run cannot be undone', 'Its undo window may be closed or it was already restored.'); })();
      return sim.startUndo(run);
    },
    restore_entries: notYet('restore_entries'),
    quarantine_info: () => ({ dir: 'C:\\Users\\Michel\\AppData\\Roaming\\MyJanitor\\quarantine', items: 38, bytes: 7408715776, oldest: m.nowNs() - 12 * 86_400_000_000_000, purge_after_days: m.settings.general.quarantine_days }),
    quarantine_purge: notYet('quarantine_purge'),

    report_get: (req) => reportOf(m.runs.find((r) => r.run_id === req.run_id) ?? (() => { throw fail('not_found', `Unknown run: ${req.run_id}`); })()),
    report_last: () => (m.runs.length ? reportOf(m.runs[0]) : null),
    report_export: () => ({ path: 'C:\\Users\\Michel\\Documents\\myjanitor-report.csv', bytes: 4096, rows: 40 }),
    results_export: () => ({ path: 'C:\\Users\\Michel\\Documents\\myjanitor-results.csv', bytes: 8192, rows: 80 }),

    settings_get: () => m.settings,
    settings_set: (req) => (m.settings = { ...m.settings, ...req.settings }),
    protected_paths_get: () => ({ builtin: ['C:\\Windows', 'C:\\Program Files'], builtin_patterns: ['**/.git', '**/.ssh/**'], user: [] }),
    protected_paths_set: (req) => ({ builtin: ['C:\\Windows', 'C:\\Program Files'], builtin_patterns: ['**/.git', '**/.ssh/**'], user: req.user ?? [] }),
    cache_info: () => m.cache,
    cache_clear: () => { m.cache = { ...m.cache, entries: 0, bytes: 0 }; return m.cache; },
    schedules_list: () => m.schedules,
    schedule_save: notYet('schedule_save'),
    schedule_delete: notYet('schedule_delete'),
    schedule_from_plan: (req) => {
      if (m.plan.steps.some((s) => s.safety === 'lossy_opt_in' || s.safety === 'permanent')) throw fail('invalid_input', 'Lossy and permanent steps never run unattended', 'Remove those steps from the plan to save a schedule.');
      if (!m.plan.steps.length) throw fail('invalid_input', 'The plan is empty', 'Add a step first.');
      const s = { id: `sched_${m.schedules.length + 1}`, name: req.name, enabled: true, cadence: req.cadence, scope: { roots: m.scope.roots }, steps: m.plan.steps.map((st) => ({ opportunity: st.opportunity, action: st.action, params: st.params })), last_run: null, next_run: m.nowNs() + 7 * 86_400_000_000_000 };
      m.schedules.push(s);
      return structuredClone(s);
    },
    schedule_run_now: notYet('schedule_run_now'),
    tools_list: () => m.tools,
    tools_refresh: () => m.tools,
    drives_list: () => m.drives(),
    disk_usage_tree: notYet('disk_usage_tree'),
    breakdown: notYet('breakdown'),
  };
}
