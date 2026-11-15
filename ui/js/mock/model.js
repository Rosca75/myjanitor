// @ts-check
/**
 * In-memory model behind the mock backend: opportunities, candidates,
 * selection, plan, jobs and history. All derived numbers (card sizes, step
 * summaries, plan estimate, checklist) are computed here from the rows so the
 * UI behaves like it would against the real engine.
 */
import { buildOpportunities, buildCandidates, PLAN_BASE, DRIVES, GB, SETTINGS, TOOLS, HERO_SUMMARY } from './fixtures-all.js';

const DAY_NS = 86_400_000_000_000;

/** @param {{ now?: () => number, scenario?: string }} opts */
export function createModel(opts) {
  const nowMs = opts.now ?? (() => Date.now());
  const nowNs = () => nowMs() * 1e6;
  const opps = buildOpportunities();
  const { rows, groups } = buildCandidates(opps);
  /** @type {Map<string, Set<number>>} */
  const selection = new Map();
  for (const [id, list] of rows) selection.set(id, new Set(list.filter((r) => r.default_selected).map((r) => r.id)));
  const baseBefore = new Map(opps.map((o) => [o.id, o.size_before]));

  const m = {
    nowNs,
    opps,
    rows,
    groups,
    selection,
    lossy: new Map(opps.map((o) => [o.id, o.safety === 'lossy_opt_in'])),
    actions: new Map(), // oppId -> Action chosen in the step panel before it is in the plan
    scope: { roots: [], include: [], exclude: [], options: SETTINGS.scan },
    scan: { state: 'idle', generation: 0, progress: { generation: 0, phase: null, files: 0, dirs: 0, bytes: 0, done: 0, elapsed_ms: 0 }, cached: false },
    scannedAt: null,
    plan: { id: 1, steps: [], updated: nowNs() },
    nextStepId: 12,
    nextJobId: 31,
    nextCustom: 1,
    jobs: [],
    runs: [],
    journals: new Map(),
    settings: structuredClone(SETTINGS),
    tools: structuredClone(TOOLS),
    schedules: [],
    presets: [],
    exact: false,
    freed: 0, // bytes freed by applied runs, shown in the drive bars
    cache: { entries: 184233, bytes: 19_300_000, path: 'C:\\Users\\Michel\\AppData\\Local\\MyJanitor\\cache\\scan-cache.bin' },
  };

  /** @param {string} id */
  m.opp = (id) => opps.find((o) => o.id === id);
  /** @param {string} id */
  m.step = (id) => m.plan.steps.find((s) => s.opportunity === id);

  /** StepSummary of the current selection. @param {string} id */
  m.summary = (id) => {
    const list = rows.get(id) ?? [];
    const sel = selection.get(id) ?? new Set();
    let before = 0;
    let after = 0;
    let n = 0;
    for (const r of list) if (sel.has(r.id)) { before += r.size; after += r.size_after; n += 1; }
    return { candidates: list.length, selected: n, size_before: before, size_after: after, saving: before - after, confidence: m.opp(id)?.confidence ?? 'estimated' };
  };

  /** Push the selection numbers back into the opportunity card. @param {string} id */
  m.refresh = (id) => {
    const o = m.opp(id);
    if (!o || o.safety === 'review_only') return;
    const s = m.summary(id);
    o.size_before = s.size_before;
    o.saving = s.saving;
    o.selected = s.selected;
    const st = m.step(id);
    if (st) Object.assign(st, m.buildStep(o, st));
  };

  /** Keep-rule labels used in plan step summaries. */
  const KEEP = { newest: 'newest', oldest: 'oldest', shortest_path: 'shortest path', highest_resolution: 'highest resolution' };
  const AUDIO_TEXT = { speech: 'Opus 24 kbps mono', transcription_safe: 'Opus 48 kbps', lossless: 'FLAC' };

  /** @param {any} o @param {any} action @param {any} params */
  function summaryFor(o, action, params) {
    if (o.id === 'opt:audio') return `${AUDIO_TEXT[params.preset ?? 'speech']} · originals to Recycle Bin`;
    switch (action.kind) {
      case 'trash':
        if (o.id === 'det:exact_duplicates') return `Keep ${KEEP[params.keep_rule ?? 'newest']} · others to Recycle Bin`;
        return o.id === 'det:dev_artefacts' ? 'To Recycle Bin · rebuild with npm / pip' : 'To Recycle Bin';
      case 'quarantine':
        return o.id === 'det:app_caches' ? `Apps recreate these · quarantine ${action.purge_after_days ?? 7} days` : `Quarantine ${action.purge_after_days ?? 7} days`;
      case 'permanent_delete':
        return 'Deleted for good · cannot be undone';
      case 'move':
        return 'Moved to the folder you chose';
      default:
        return o.description;
    }
  }

  /**
   * A PlanStep for an opportunity from its base numbers and the current selection.
   * @param {any} o @param {any} [prev]
   */
  m.buildStep = (o, prev) => {
    const base = PLAN_BASE[o.id];
    const s = m.summary(o.id);
    const factor = (baseBefore.get(o.id) ?? 1) ? s.size_before / (baseBefore.get(o.id) ?? 1) : 1;
    const before = base ? Math.round(base.before * factor) : s.size_before;
    const afterRatio = s.size_before ? s.size_after / s.size_before : 0;
    const action = prev?.action ?? m.actions.get(o.id) ?? base?.action ?? o.default_action;
    const params = { ...(base?.params ?? o.params), ...(prev?.params ?? {}), ...(o.params.preset ? { preset: o.params.preset } : {}) };
    const permanent = action.kind === 'permanent_delete';
    const lossy = !permanent && !!m.lossy.get(o.id);
    const removal = ['trash', 'quarantine', 'permanent_delete', 'move'].includes(action.kind);
    return {
      id: prev?.id ?? m.nextStepId++,
      opportunity: o.id,
      title: o.title,
      summary: summaryFor(o, action, { ...o.params, ...params }),
      action,
      params,
      selection: { include: [], exclude: [] },
      estimate: { files: s.selected, bytes_before: before, bytes_after: removal ? 0 : Math.round(before * afterRatio), confidence: m.exact ? 'exact' : o.confidence, skipped: 0 },
      safety: permanent ? 'permanent' : lossy ? 'lossy_opt_in' : o.safety === 'lossy_opt_in' ? 'reversible' : o.safety,
      lossy,
    };
  };

  /** The Plan response. */
  m.planJson = () => {
    const steps = m.plan.steps;
    const est = steps.reduce((a, s) => ({ files: a.files + s.estimate.files, bytes_before: a.bytes_before + s.estimate.bytes_before, bytes_after: a.bytes_after + s.estimate.bytes_after, confidence: a.confidence, skipped: 0 }), { files: 0, bytes_before: 0, bytes_after: 0, confidence: m.exact ? 'exact' : 'estimated', skipped: 0 });
    const confirm = steps.filter((s) => s.safety === 'lossy_opt_in' || s.safety === 'permanent');
    const lossyN = steps.filter((s) => s.safety === 'lossy_opt_in').length;
    const permN = steps.filter((s) => s.safety === 'permanent').length;
    const checklist = steps.length ? [{ level: 'ok', code: 'undo_window', text: `Every ${permN ? 'reversible ' : ''}step can be undone from History for ${m.settings.general.undo_days} days` }] : [];
    if (lossyN) checklist.push({ level: 'warning', code: 'confirmation_steps', text: `${lossyN} lossy ${lossyN === 1 ? 'step' : 'steps'} will ask for confirmation` });
    if (permN) checklist.push({ level: 'warning', code: 'permanent_steps', text: `${permN} permanent ${permN === 1 ? 'step' : 'steps'} cannot be undone and will ask for confirmation` });
    if (steps.length) {
      checklist.push({ level: 'ok', code: 'skipped_confidential', text: '14 signed or sensitivity-labelled files are skipped' });
      checklist.push({ level: 'info', code: 'synced_files', text: 'Synced OneDrive files are made online-only, never deleted in the cloud' });
    }
    void confirm;
    return { id: m.plan.id, name: 'Your plan', steps: structuredClone(steps), estimate: est, checklist, updated: m.plan.updated };
  };

  /** Hero drive bars: the wireframe plan frees 29.3 GB, other plans scale. */
  m.drives = () => {
    const est = m.planJson().estimate;
    const k = (est.bytes_before - est.bytes_after) / (29.3 * GB);
    const base = DRIVES.reduce((a, d) => a + d.plan_saving, 0);
    return DRIVES.map((d) => {
      const freed = Math.round(m.freed * (d.plan_saving / base));
      return { ...structuredClone(d), free: d.free + freed, available: d.free + freed, plan_saving: Math.round(d.plan_saving * k), free_after_plan: d.free + freed + Math.round(d.plan_saving * k) };
    });
  };

  /** Opportunity list response for a tab and sort. @param {{tab?: string, sort?: string}} req */
  m.list = (req) => {
    if (m.scannedAt === null) return { generation: m.scan.generation, items: [], summary: { total: 0, reversible: 0, opt_in: 0, review_only: 0, opportunities: 0 }, counts: { all: 0, clean: 0, optimise: 0, archive: 0, review_only: 0 }, cached: false };
    const tab = req.tab ?? 'all';
    const review = (o) => o.safety === 'review_only';
    const inTab = (o) => (tab === 'all' ? true : tab === 'review_only' ? review(o) : o.category === tab && !review(o));
    const safetyOrder = ['reversible', 'rebuildable', 'light_copy', 'lossy_opt_in', 'permanent', 'review_only'];
    const cmp = {
      saving: (a, b) => b.rank - a.rank,
      count: (a, b) => b.count - a.count,
      safety: (a, b) => safetyOrder.indexOf(a.safety) - safetyOrder.indexOf(b.safety) || b.rank - a.rank,
      name: (a, b) => a.title.localeCompare(b.title),
    }[req.sort ?? 'saving'];
    const count = (f) => opps.filter(f).length;
    const items = opps.filter(inTab).sort(cmp).map((o) => ({ ...structuredClone(o), in_plan: !!m.step(o.id) }));
    return {
      generation: m.scan.generation,
      items,
      summary: { ...HERO_SUMMARY, opportunities: opps.length },
      counts: { all: opps.length, clean: count((o) => o.category === 'clean' && !review(o)), optimise: count((o) => o.category === 'optimise' && !review(o)), archive: count((o) => o.category === 'archive' && !review(o)), review_only: count(review) },
      scanned_at: m.scannedAt,
      cached: m.scan.cached,
    };
  };

  /** Seed one of the named scenarios. @param {string} scenario */
  m.seed = (scenario) => {
    if (scenario === 'empty') return;
    m.scope.roots = ['C:\\Users\\Michel', 'D:\\Projects'];
    m.scannedAt = nowNs() - 2 * 60 * 1e9;
    m.scan = { state: 'done', generation: 3, started: m.scannedAt - 60e9, finished: m.scannedAt, progress: { generation: 3, phase: 'detecting', files: 184233, dirs: 20117, bytes: 412e9, done: 18, total: 18, elapsed_ms: 60012 }, cached: true };
    if (scenario === 'fresh') return;
    for (const o of opps.filter((x) => x.in_plan)) m.plan.steps.push(m.buildStep(o));
    const t = nowNs() - 3 * DAY_NS;
    m.runs.push({ run_id: Math.round(t / 1e6), kind: 'apply', title: 'Plan · 2 steps', started: t, finished: t + 90e9, steps: ['Temporary files', 'Old log files'], items: 299, done: 299, failed: 0, undone: 299, bytes_saved: Math.round(0.6 * GB), undoable: false, undo_until: t + 30 * DAY_NS });
  };

  return m;
}
