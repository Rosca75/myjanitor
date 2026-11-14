// @ts-check
/**
 * W3-D2 additions to the mock backend: richer search (parse, suggest, presets),
 * previews, disk usage, history with journal entries, reports and exports, and
 * the `rich` scenario (disabled cards, faults, a long history).
 */
import { GB } from './fixtures.js';
import { buildCandidates } from './candidates.js';
import { PNG_B64 } from './preview-data.js';
import { diskTree, breakdown } from './disk.js';
import { fail } from './handlers.js';
import { byteOffset } from '../lib/query.js';

const DAY = 86_400_000_000_000;
const enc = new TextEncoder();
const UNIT = { k: 1024, m: 1024 ** 2, g: GB, t: 1024 ** 4 };
const CMP = { '<': 'lt', '<=': 'le', '=': 'eq', '>=': 'ge', '>': 'gt' };
const SPAN = { d: 'days', w: 'weeks', m: 'months', y: 'years' };
const KEYS = ['ext', 'name', 'path', 'size', 'modified', 'created', 'accessed', 'type', 'attr', 'owner', 'depth', 'pathlen', 'kind', 'content', 'is', 'top'];
const VALUES = {
  size: [['>1GB', 'bigger than 1 GB'], ['>100MB', 'bigger than 100 MB'], ['<1MB', 'smaller than 1 MB'], ['1MB..10MB', 'between 1 and 10 MB']],
  modified: [['>1y', 'older than 1 year'], ['>6m', 'older than 6 months'], ['<2024', 'before 2024']],
  type: [['video', 'videos'], ['images', 'pictures'], ['audio', 'sound'], ['installers', 'setup programs'], ['archives', 'zip and 7z']],
  is: [['empty', 'zero bytes'], ['hidden', 'hidden files'], ['invalid', 'names Windows rejects'], ['mismatch', 'extension does not match content']],
  kind: [['file', 'files'], ['dir', 'folders'], ['link', 'links']],
  attr: [['hidden', 'hidden'], ['system', 'system'], ['readonly', 'read-only'], ['compressed', 'compressed'], ['encrypted', 'encrypted']],
};

/** One `key:value` condition, or an error message. @param {string} key @param {string} val */
function condFor(key, val) {
  const cmpNum = /^(<=|>=|<|>|=)?(\d+)$/;
  switch (key) {
    case 'ext': return { cond: { field: 'extension', values: val.split(',').map((x) => x.toLowerCase()) } };
    case 'name': case 'path': return { cond: { field: key, mode: /^\/.*\/$/.test(val) ? 'regex' : 'wildcard', pattern: val.replace(/^\/|\/$/g, ''), case_sensitive: false } };
    case 'size': {
      const mm = /^(<=|>=|<|>|=)?(\d+(?:\.\d+)?)([kmgt])?b?$/i.exec(val);
      if (!mm) return 'Expected a size such as >1GB';
      const bytes = Math.round(Number(mm[2]) * (UNIT[(mm[3] ?? '').toLowerCase()] ?? 1));
      return mm[1] === '=' && bytes === 0 ? { cond: { field: 'zero_byte' } } : { cond: { field: 'size', cmp: CMP[mm[1] ?? '='], bytes } };
    }
    case 'modified': case 'created': case 'accessed': {
      const rel = /^(<=|>=|<|>)?(\d+)([dwmy])$/.exec(val);
      if (rel) return { cond: { field: 'date', which: key, cmp: { '<': 'gt', '<=': 'ge', '>': 'lt', '>=': 'le', undefined: 'eq' }[rel[1]], value: { ago: { amount: Number(rel[2]), unit: SPAN[rel[3]] } } } };
      const abs = /^(<=|>=|<|>)?(\d{4})(?:-(\d{2}))?(?:-(\d{2}))?$/.exec(val);
      if (!abs) return 'Expected a date such as <2024 or an age such as >2y';
      return { cond: { field: 'date', which: key, cmp: CMP[abs[1] ?? '='], value: { at: Date.UTC(Number(abs[2]), Number(abs[3] ?? 1) - 1, Number(abs[4] ?? 1)) * 1e6 } } };
    }
    case 'type': return { cond: { field: 'type_group', group: val } };
    case 'attr': return { cond: { field: 'attribute', attr: val.split(','), set: true } };
    case 'owner': return { cond: { field: 'owner', value: val } };
    case 'depth': case 'pathlen': {
      const mm = cmpNum.exec(val);
      return mm ? { cond: { field: key === 'depth' ? 'depth' : 'path_length', cmp: CMP[mm[1] ?? '='], value: Number(mm[2]) } } : 'Expected a number such as >5';
    }
    case 'kind': return ['file', 'dir', 'link'].includes(val) ? { cond: { field: 'kind', kind: val } } : 'Expected file, dir or link';
    case 'content': return { cond: { field: 'content_contains', text: val, case_sensitive: false } };
    case 'is': {
      const f = { empty: { field: 'zero_byte' }, hidden: { field: 'attribute', attr: ['hidden'], set: true }, invalid: { field: 'invalid_name' }, mismatch: { field: 'extension_mismatch' } }[val];
      return f ? { cond: f } : 'Expected empty, hidden, invalid or mismatch';
    }
    case 'top': return /^\d+$/.test(val) ? { cond: { field: 'largest_n', n: Number(val) } } : 'Expected a number such as 50';
    default: return null;
  }
}

/** Quick query to { rule?, errors } with byte offsets. @param {string} query */
export function parseQuery(query) {
  const conds = [];
  const errors = [];
  for (const m of query.matchAll(/\S+/g)) {
    const tok = m[0];
    const at = byteOffset(query, m.index);
    const i = tok.indexOf(':');
    if (i < 0) { conds.push({ cond: { field: 'name', mode: 'contains', pattern: tok, case_sensitive: false } }); continue; }
    const key = tok.slice(0, i);
    const val = tok.slice(i + 1);
    if (!KEYS.includes(key)) { errors.push({ offset: at, len: enc.encode(tok).length, message: `Unknown key "${key}"` }); continue; }
    if (!val) { errors.push({ offset: at + enc.encode(`${key}:`).length, len: 1, message: `Missing value after "${key}:"` }); continue; }
    const c = condFor(key, val);
    if (typeof c === 'string') errors.push({ offset: at + enc.encode(`${key}:`).length, len: enc.encode(val).length, message: c });
    else if (c) conds.push(c);
  }
  return errors.length ? { errors } : { rule: { all: conds }, errors: [] };
}

/** @param {string} query @param {number} cursor bytes */
function suggest(query, cursor) {
  const head = enc.encode(query).slice(0, cursor);
  const text = new TextDecoder().decode(head);
  const word = text.split(/\s+/).pop() ?? '';
  if (!word) return [];
  const len = enc.encode(word).length;
  const i = word.indexOf(':');
  if (i < 0) return KEYS.filter((k) => k.startsWith(word.toLowerCase())).map((k) => ({ insert: `${k}:`, replace_len: len, label: `${k}:` }));
  const key = word.slice(0, i);
  const prefix = word.slice(i + 1).toLowerCase();
  return (VALUES[key] ?? []).filter(([v]) => v.toLowerCase().startsWith(prefix)).map(([v, d]) => ({ insert: `${key}:${v}`, replace_len: len, label: `${key}:${v}  ${d}` }));
}

const SAMPLE_TEXT = 'Setup log\n2026-09-14 08:12:01 INFO  Starting installer\n2026-09-14 08:12:03 INFO  Extracting 412 files\n2026-09-14 08:12:41 WARN  Disk space is low on C:\n2026-09-14 08:12:58 INFO  Finished with exit code 0\n';

/** Build the overrides. @param {any} m @param {any} sim @param {(e: string, p: any) => void} emit @param {Record<string, (r: any) => any>} base */
export function createExtras(m, sim, emit, base) {
  const builtin = [
    { id: 'downloads_older_90d', name: 'Downloads older than 90 days', description: 'Anything in a Downloads folder not modified for 90 days', rule: { all: [{ cond: { field: 'path', mode: 'wildcard', pattern: '**/Downloads/**', case_sensitive: false } }, { cond: { field: 'date', which: 'modified', cmp: 'lt', value: { ago: { amount: 90, unit: 'days' } } } }] }, topmost_only: false, builtin: true },
    { id: 'dev_junk', name: 'Dev junk', description: 'node_modules, target and build output folders', rule: { any: [{ cond: { field: 'name', mode: 'exact', pattern: 'node_modules', case_sensitive: true } }, { cond: { field: 'name', mode: 'exact', pattern: 'target', case_sensitive: true } }] }, topmost_only: true, builtin: true },
    { id: 'big_videos', name: 'Big old videos', description: 'Videos over 1 GB not touched for a year', rule: { all: [{ cond: { field: 'type_group', group: 'video' } }, { cond: { field: 'size', cmp: 'gt', bytes: GB } }] }, topmost_only: false, builtin: true },
  ];
  m.presets ??= [];
  m.exported = new Set();
  const exportTo = (req, rows) => {
    if (!/^([A-Za-z]:\\|\/)/.test(String(req.dest))) throw fail('invalid_input', `Not an absolute path: ${req.dest}`, 'Choose a folder in the list.');
    let path = String(req.dest);
    if (m.exported.has(path)) path = path.replace(/(\.\w+)$/, ' (1)$1');
    m.exported.add(path);
    return { path, bytes: 1200 + rows * 140, rows };
  };

  /** Report of one run from its journal entries. @param {any} run */
  const reportOf = (run) => {
    const entries = m.journals.get(run.run_id) ?? [];
    const by = new Map();
    for (const e of entries) {
      const title = e.step_title ?? run.steps[0] ?? 'Step';
      if (!by.has(title)) by.set(title, []);
      by.get(title).push({ path: e.original, status: e.state === 'skipped' ? 'skipped' : e.state === 'failed' ? 'failed' : 'done', bytes_before: e.bytes_before, bytes_after: e.state === 'done' || e.state === 'undone' ? e.bytes_after : e.bytes_before, ...(e.error ? { error: e.error } : {}) });
    }
    const tot = (items) => ({ files: items.length, succeeded: items.filter((i) => i.status === 'done').length, skipped: items.filter((i) => i.status === 'skipped').length, failed: items.filter((i) => i.status === 'failed').length, bytes_before: items.reduce((a, i) => a + i.bytes_before, 0), bytes_after: items.reduce((a, i) => a + i.bytes_after, 0), bytes_saved: items.filter((i) => i.status === 'done').reduce((a, i) => a + i.bytes_before - i.bytes_after, 0) });
    const sections = [...by].map(([title, items]) => ({ title, action: 'To Recycle Bin', totals: tot(items), items }));
    const all = sections.flatMap((s) => s.items);
    return { run_id: run.run_id, title: run.title, started: run.started, finished: run.finished, totals: tot(all), sections, undo_until: run.undo_until };
  };

  return {
    app_info: () => ({ ...base.app_info(), platform: m.platform ?? 'windows' }),
    search_parse: (req) => parseQuery(String(req.query ?? '')),
    search_suggest: (req) => suggest(String(req.query ?? ''), req.cursor ?? enc.encode(String(req.query ?? '')).length),
    search_run: (req) => {
      if (m.scannedAt === null) throw fail('invalid_input', 'Scan before searching', 'Run a scan from Overview first.');
      const title = req.title || 'Custom search';
      const none = /nomatch|zzz/i.test(JSON.stringify(req.rule));
      const n = none ? 0 : 12 + (JSON.stringify(req.rule).length % 40);
      let o = m.opps.find((x) => x.title === title && x.id.startsWith('custom:'));
      const action = req.action ?? { kind: 'trash' };
      if (!o) {
        const id = m.nextCustom++;
        o = { id: `custom:${id}`, source: { custom: String(id) }, category: 'clean', title, description: 'Your own search', safety: action.kind === 'permanent_delete' ? 'permanent' : 'reversible', size_before: 0, saving: 0, confidence: 'exact', count: 0, unit: 'files', candidates: 0, selected: 0, badges: [], default_action: action, available_actions: ['trash', 'quarantine', 'permanent_delete'], presets: [], settings: [], params: {}, sub_groups: [], availability: { state: 'available' }, in_plan: false, rank: 1e8, feature_ids: ['CLN-ENG-05'] };
        m.opps.push(o);
      }
      o.count = n; o.candidates = n; o.selected = n; o.default_action = action;
      o.size_before = n * 8 * 1024 * 1024; o.saving = o.size_before;
      const built = n ? buildCandidates([{ ...o, count: n }]) : { rows: new Map([[o.id, []]]) };
      m.rows.set(o.id, built.rows.get(o.id));
      m.selection.set(o.id, new Set(built.rows.get(o.id).map((r) => r.id)));
      emit('opportunities://changed', { generation: m.scan.generation });
      return structuredClone(o);
    },
    presets_list: () => structuredClone([...builtin, ...m.presets]),
    preset_save: (req) => {
      const p = req.preset;
      if (!p?.name?.trim()) throw fail('invalid_input', 'A preset needs a name', 'Type a name for it.');
      if (builtin.some((b) => b.id === p.id)) throw fail('invalid_input', 'Built-in presets cannot be changed', 'Save it under a new name.');
      const id = p.id || `user_${m.presets.length + 1}_${m.presets.length + 40}`;
      m.presets = [...m.presets.filter((x) => x.id !== id), { ...p, id, builtin: false }];
      return structuredClone([...builtin, ...m.presets]);
    },
    preset_delete: (req) => {
      if (builtin.some((b) => b.id === req.id)) throw fail('invalid_input', 'Built-in presets cannot be deleted', 'Only your own presets can be deleted.');
      m.presets = m.presets.filter((x) => x.id !== req.id);
      return structuredClone([...builtin, ...m.presets]);
    },
    candidate_preview: (req) => {
      const row = [...m.rows.values()].flat().find((x) => x.id === req.candidate_id);
      if (!row) throw fail('invalid_input', `Unknown candidate: ${req.candidate_id}`, 'Scan again to refresh the list.');
      if (row.source?.optimiser === 'audio') return base.candidate_preview(req);
      if (row.target === 'dir') return { kind: 'none', reason: 'Folders have no preview' };
      if (/\.(png|jpe?g|gif)$/i.test(row.path)) return { kind: 'image', mime: 'image/png', data: PNG_B64 };
      if (/\.(txt|log|tmp|md|csv)$/i.test(row.path)) return { kind: 'text', text: SAMPLE_TEXT };
      return { kind: 'none', reason: 'No preview for this file type' };
    },
    disk_usage_tree: (req) => {
      if (m.scannedAt === null) throw fail('invalid_input', 'Scan before exploring disk usage', 'Run a scan from Overview first.');
      const t = diskTree(req ?? {});
      if (!t) throw fail('not_found', `Not in the scan: ${req?.path}`, 'Scan again to refresh the tree.');
      return t;
    },
    breakdown: (req) => {
      if (m.scannedAt === null) throw fail('invalid_input', 'Scan before exploring disk usage', 'Run a scan from Overview first.');
      return breakdown(req);
    },
    restore_entries: (req) => sim.startRestore(req.entry_ids ?? []),
    quarantine_purge: () => sim.startPurge(),
    report_get: (req) => reportOf(m.runs.find((r) => r.run_id === req.run_id) ?? (() => { throw fail('not_found', `Unknown run: ${req.run_id}`, 'It may have been removed.'); })()),
    report_last: () => (m.runs.length ? reportOf(m.runs[0]) : null),
    report_export: (req) => {
      const run = req.run_id ? m.runs.find((r) => r.run_id === req.run_id) : null;
      if (req.run_id && !run) throw fail('not_found', `Unknown run: ${req.run_id}`);
      return exportTo(req, req.kind === 'audit' ? (run ? (m.journals.get(run.run_id) ?? []).length : [...m.journals.values()].reduce((a, l) => a + l.length, 0)) : run ? reportOf(run).totals.files : 0);
    },
    results_export: (req) => exportTo(req, (req.opportunity_id ? (m.rows.get(req.opportunity_id) ?? []).length : 80)),
  };
}

/** Seed the `rich` scenario on top of `results`. @param {any} m */
export function applyRich(m) {
  // ffmpeg missing: the video and audio optimisers are shown disabled with the reason.
  const ff = m.tools.find((t) => t.id === 'ffmpeg');
  if (ff) { ff.found = false; delete ff.path; delete ff.version; }
  for (const id of ['opt:video', 'opt:audio']) m.opp(id).availability = { state: 'disabled', reason: 'Requires ffmpeg on PATH' };
  m.plan.steps = m.plan.steps.filter((s) => s.opportunity !== 'opt:audio');
  const btrfs = { id: 'opt:btrfs_compress', source: { optimiser: 'btrfs_compress' }, category: 'optimise', title: 'Btrfs compression', description: 'Transparent zstd on cold folders', safety: 'reversible', size_before: 6 * GB, saving: 2.4 * GB, confidence: 'estimated', count: 4, unit: 'folders', candidates: 4, selected: 4, badges: [], default_action: { kind: 'fs_compress', algorithm: 'zstd' }, available_actions: ['fs_compress'], presets: [], settings: [], params: {}, sub_groups: [], availability: { state: 'disabled', reason: 'Needs a Btrfs volume' }, in_plan: false, rank: 1e9, feature_ids: ['OPT-FSC-06'] };
  m.opps.push(btrfs);
  const built = buildCandidates([btrfs]);
  m.rows.set(btrfs.id, built.rows.get(btrfs.id));
  m.selection.set(btrfs.id, new Set(built.rows.get(btrfs.id).map((r) => r.id)));
  const arch = m.opp('det:inactive_engagements');
  arch.settings = [{ key: 'idle_months', label: 'Idle for at least', kind: { type: 'number', default: 18, min: 1, max: 120, step: 1, unit: 'months' } }];
  arch.params = { idle_months: 18 };
  arch.default_action = { ...arch.default_action, format: 'seven_zip' };
  m.faults = { 'det:app_caches': { failed: 2, skipped: 1 } };

  // History: a run with refusals and errors, a clean one, an expired one.
  const now = m.nowNs();
  const mk = (daysAgo, title, specs, undoDays) => {
    const run_id = Math.round((now - daysAgo * DAY) / 1e6);
    const entries = [];
    for (const sp of specs) {
      const rows = (m.rows.get(sp.opp) ?? []).slice(0, sp.n);
      rows.forEach((r, i) => {
        const state = i < sp.failed ? 'failed' : i < sp.failed + sp.skipped ? 'skipped' : i < sp.failed + sp.skipped + (sp.refused ?? 0) ? 'undo_failed' : 'done';
        const e = { id: run_id * 1000 + entries.length + 1, run_id, job_id: 1, step_id: 1, step_title: sp.title, at: now - daysAgo * DAY + i * 1e9, state, action: sp.action ?? 'trash', target: r.target === 'dir' ? 'folder' : 'file', original: r.path, original_stamp: r.stamp, bytes_before: r.size, bytes_after: 0, reversible: state === 'done' || state === 'undo_failed', undo_deadline: now - daysAgo * DAY + undoDays * DAY };
        if (state === 'failed') e.error = { code: 'permission_denied', message: `Permission denied: ${r.path}`, hint: 'MyJanitor has no access to this path. Skip it, or run with access to it.' };
        if (state === 'skipped') e.error = { code: i % 2 ? 'protected' : 'changed', message: i % 2 ? `Protected path: ${r.path}` : `Changed since the scan: ${r.path}`, hint: i % 2 ? 'Protected paths are never touched. Edit them in Settings.' : 'The file changed after the scan, so it was skipped. Scan again.' };
        if (state === 'undo_failed') { e.error = { code: 'conflict', message: `The original location is occupied: ${r.path}`, hint: 'Move or rename the file that is there, then restore again.' }; m.occupied.add(e.id); }
        if (sp.action === 'quarantine') e.retained_at = `C:\\Users\\Michel\\AppData\\Roaming\\MyJanitor\\quarantine\\${run_id}\\${e.id}`;
        entries.push(e);
      });
    }
    m.journals.set(run_id, entries);
    const done = entries.filter((e) => e.state === 'done' || e.state === 'undo_failed').length;
    const undoable = undoDays > daysAgo && entries.some((e) => e.reversible);
    m.runs.push({ run_id, kind: 'apply', title, started: now - daysAgo * DAY, finished: now - daysAgo * DAY + 120e9, steps: specs.map((s) => s.title), items: entries.length, done, failed: entries.filter((e) => e.state === 'failed').length, undone: 0, bytes_saved: entries.filter((e) => e.state === 'done').reduce((a, e) => a + e.bytes_before, 0), undoable, undo_until: now - daysAgo * DAY + undoDays * DAY });
  };
  m.occupied = new Set();
  m.runs = [];
  mk(2, 'Plan · 3 steps', [{ opp: 'det:temp_files', title: 'Temporary files', n: 40, failed: 2, skipped: 3, refused: 2 }, { opp: 'det:old_logs', title: 'Old log files', n: 30, failed: 0, skipped: 0 }, { opp: 'det:app_caches', title: 'App caches & junk', n: 12, failed: 0, skipped: 1, action: 'quarantine' }], 30);
  mk(9, 'Plan · 1 step', [{ opp: 'det:exact_duplicates', title: 'Exact duplicates', n: 45, failed: 0, skipped: 0 }], 30);
  mk(41, 'Plan · 1 step', [{ opp: 'det:old_installers', title: 'Old installers', n: 20, failed: 0, skipped: 0 }], 30);
  m.runs.sort((a, b) => b.started - a.started);
}
