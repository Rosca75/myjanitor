// @ts-check
/**
 * The single state store: one plain object, replaced (never mutated) on every
 * `set`, with subscriptions. Views read from it and never keep shared state of
 * their own; the controller (controller.js) is the only writer.
 */
import { initialModel } from './lib/rulebuilder.js';

/**
 * @template T
 * @param {T} initial
 */
export function createStore(initial) {
  let state = initial;
  /** @type {Set<{fn: (s: T, changed: string[]) => void, keys: string[] | null}>} */
  const subs = new Set();
  return {
    get: () => state,
    /**
     * Shallow-merge a patch (or a function of the current state returning one).
     * Subscribers with `keys` are called only when one of those keys changed.
     * @param {Partial<T> | ((s: T) => Partial<T>)} patch
     */
    set(patch) {
      const p = typeof patch === 'function' ? patch(state) : patch;
      const changed = Object.keys(p).filter((k) => !Object.is(/** @type {any} */ (state)[k], /** @type {any} */ (p)[k]));
      if (!changed.length) return;
      state = { ...state, ...p };
      for (const s of [...subs]) if (!s.keys || s.keys.some((k) => changed.includes(k))) s.fn(state, changed);
    },
    /**
     * @param {(s: T, changed: string[]) => void} fn
     * @param {string[] | null} [keys]
     * @returns {() => void} unsubscribe
     */
    subscribe(fn, keys = null) {
      const entry = { fn, keys };
      subs.add(entry);
      return () => subs.delete(entry);
    },
  };
}

export const emptySummary = { total: 0, reversible: 0, opt_in: 0, review_only: 0, opportunities: 0 };
export const emptyCounts = { all: 0, clean: 0, optimise: 0, archive: 0, review_only: 0 };

/** Custom search (Clean view): quick query, rule builder, presets and results. */
export function initialCustom() {
  return {
    mode: 'query', // 'query' | 'builder'
    query: '', caret: 0, errors: [], parsedRule: null, suggestions: [], suggestIndex: -1,
    builder: initialModel(), builderErrors: [],
    title: '', action: 'trash', topmost: false, advancedOpen: false,
    presets: [], presetsLoading: false, presetName: '', presetDesc: '',
    running: false, result: null, step: null,
  };
}

/** Initial application state. */
export function initialState() {
  return {
    ready: false,
    info: null,
    route: { name: 'overview', params: {}, from: null },

    scope: { roots: [], include: [], exclude: [] },
    scan: { state: 'idle', generation: 0, cached: false },
    scanProgress: null, // kept apart from `scan` so 50 ms progress never re-renders the controls
    jobProgress: {}, // job id -> progress, for the same reason
    detectors: {}, // opportunity_id -> DetectorTotalsEvent (live cards while scanning)
    drives: [],
    tools: [],

    /** Result of opportunities_list for the active tab. */
    opps: { items: [], summary: emptySummary, counts: emptyCounts, generation: 0 },
    tab: 'all',
    sort: 'saving',
    showSmall: false,

    plan: { id: 0, name: 'Your plan', steps: [], estimate: { files: 0, bytes_before: 0, bytes_after: 0, confidence: 'estimated', skipped: 0 }, checklist: [] },
    dryRunning: false,
    jobs: [],
    reclaimed: 0,
    completed: null, // { run_id, saved, files, failed } after an apply finished
    lastReport: null,
    undoable: false,
    history: { total: 0, runs: [] },
    quarantine: null,
    settings: null,
    cache: null,

    category: { name: null, items: [], loading: false, error: null }, // Clean / Optimise / Archive lists
    report: null,
    reportUi: { filter: 'all', shown: {} },
    /** Features the backend answered with not_implemented / unsupported: key -> ErrorInfo. */
    unavail: {},
    /** History: expanded runs, their entries (per run), selection, results of the last restore. */
    historyUi: { loading: false, error: null, expanded: {}, details: {}, selected: {}, result: null, undoDays: null, loadingMore: false },
    /** Generic yes/no dialog (undo a run, purge quarantine), or null. */
    ask: null,
    /** Disk usage view. */
    disk: { loading: false, error: null, root: null, expanded: {}, tab: 'folders', sort: 'size', focusPath: null, breakdown: {}, loadingPaths: {} },
    /** Compare before / after dialog, or null. */
    compare: null,
    /** Latest file being processed per job id (job://progress `current`). */
    jobCurrent: {},
    /** Job queue forced open by the user even when nothing runs. */
    queueOpen: false,
    /** Step review. */
    step: null,
    /** Folder browser modal, or null. */
    browser: null,
    /** Confirmation dialog sequence, or null. */
    dialog: null,
    /** Custom search state (Clean view). */
    custom: initialCustom(),
    toast: null,
  };
}

/** @param {any} s */
export const planSaving = (s) => Math.max(0, (s.plan.estimate?.bytes_before ?? 0) - (s.plan.estimate?.bytes_after ?? 0));

/** @param {any} s opportunity id -> card */
export const findOpp = (s, id) => s.opps.items.find((o) => o.id === id) ?? null;

/** @param {any} s */
export const hasResults = (s) => s.scan.state !== 'scanning' && (s.opps.generation > 0 || s.opps.items.length > 0 || !!s.opps.scanned_at);

/** @param {any} s */
export const isScanning = (s) => ['scanning', 'paused', 'stopping'].includes(s.scan.state);

/** @param {any} s */
export const activeJobs = (s) => s.jobs.filter((j) => ['queued', 'running', 'paused', 'cancelling'].includes(j.state));

/** The plan step for an opportunity id, or null. @param {any} s @param {string} id */
export const stepFor = (s, id) => s.plan.steps.find((x) => x.opportunity === id) ?? null;

/** Top cards and the "smaller opportunities" remainder. @param {any[]} items @param {number} [top] */
export function splitCards(items, top = 9) {
  return { main: items.slice(0, top), small: items.slice(top) };
}
