// @ts-check
/**
 * The controller: the only writer of the store and the only caller of the IPC
 * API. Views receive `actions` (this object) and never touch `api` directly,
 * which makes every control testable by checking the commands it sends.
 *
 * `api` is injected: the real one in the app, a recording wrapper in tests.
 */
import { stepActions } from './ctl-step.js';
import { applyActions } from './ctl-apply.js';
import { miscActions } from './ctl-misc.js';
import { historyActions } from './ctl-history.js';
import { reportActions } from './ctl-report.js';
import { searchActions } from './ctl-search.js';
import { diskActions } from './ctl-disk.js';
import { copy } from './copy.js';
import { createGens } from './lib/gens.js';
import { toErrorInfo, isUnavailable } from './errors.js';

/**
 * @param {{ api: any, store: ReturnType<typeof import('./state.js').createStore<any>> }} deps
 */
export function createController({ api, store }) {
  const ctx = { api, store, get: store.get, set: store.set, gens: createGens(), actions: /** @type {any} */ ({}) };

  /** Show an error toast from an ErrorInfo (or anything thrown): never swallowed. @param {any} e */
  ctx.fail = (e) => {
    const info = toErrorInfo(e);
    store.set({ toast: { level: 'error', message: info.message, hint: info.hint } });
  };
  /**
   * A feature the backend does not offer (`not_implemented`, `unsupported`):
   * remember it so its controls render disabled with the hint, and say so once.
   * @param {string} key @param {any} e
   */
  ctx.markUnavailable = (key, e) => {
    const info = toErrorInfo(e);
    store.set({ unavail: { ...store.get().unavail, [key]: info }, toast: { level: 'info', message: info.message, hint: info.hint } });
  };
  /**
   * Like `guard`, for a command that may be missing in this build: those errors
   * disable the feature `key`, every other error is a toast.
   * @param {string} key @param {(...a: any[]) => Promise<any>} fn
   */
  ctx.feature = (key, fn) => async (...a) => {
    try {
      return await fn(...a);
    } catch (e) {
      if (isUnavailable(e)) ctx.markUnavailable(key, e);
      else ctx.fail(e);
      return undefined;
    }
  };
  /**
   * Show a transient message.
   * @param {string} message @param {'info'|'success'|'error'} [level]
   */
  ctx.notify = (message, level = 'info') => {
    const toast = { level, message, hint: '' };
    store.set({ toast });
    const t = setTimeout(() => { if (store.get().toast === toast) store.set({ toast: null }); }, 6000);
    /** @type {any} */ (t).unref?.();
  };
  /** Wrap an async action so a rejected command becomes a toast. @param {(...a: any[]) => Promise<any>} fn */
  ctx.guard = (fn) => async (...a) => {
    try {
      return await fn(...a);
    } catch (e) {
      ctx.fail(e);
      return undefined;
    }
  };

  const a = ctx.actions;
  Object.assign(a, coreActions(ctx), stepActions(ctx), applyActions(ctx), miscActions(ctx), historyActions(ctx), reportActions(ctx), searchActions(ctx), diskActions(ctx), { notify: ctx.notify });
  return a;
}

/** Drop `progress` from a ScanStatus: it lives in `scanProgress`. @param {any} status */
export const withoutProgress = (status) => { const { progress, ...rest } = status; void progress; return rest; };

/** Start-up, navigation, scan, scope, folder browser, overview data and plan. @param {any} c */
function coreActions(c) {
  const { api, set, get, guard } = c;
  /** @template T @param {Promise<T>} p @param {any} fallback */
  const soft = (p, fallback) => p.catch(() => fallback);

  const refreshOpps = async () => {
    const { tab, sort } = get();
    const g = c.gens.next('opps');
    const [opps, drives] = await Promise.all([api.opportunitiesList({ tab, sort }), soft(api.drivesList(), [])]);
    if (c.gens.current('opps', g)) set({ opps, drives }); // a newer refresh is on its way: ignore this stale answer
  };

  const refreshHistory = async () => {
    const [history, lastReport] = await Promise.all([soft(api.historyList({ offset: 0, limit: 50 }), { total: 0, runs: [] }), soft(api.reportLast(), null)]);
    set({ history, lastReport, undoable: history.runs.some((r) => r.undoable) });
  };
  /** A start-up read that may fail without stopping the app: toast it, use the fallback. */
  const startup = (p, fallback) => p.catch((e) => { c.fail(e); return fallback; });

  return {
    /** Load everything the shell shows at start-up (docs/IPC.md §2). */
    async init() {
      const [info, scope, scan, plan, jobs, tools] = await Promise.all([
        soft(api.appInfo(), null), startup(api.scopeGet(), get().scope), startup(api.scanStatus(), get().scan), startup(api.planGet(), get().plan), soft(api.jobsList(), []), soft(api.toolsList(), []),
      ]);
      const { progress, ...scanState } = scan;
      set({ info, scope, scan: scanState, scanProgress: progress ?? null, plan, jobs, tools, jobProgress: Object.fromEntries(jobs.map((j) => [j.id, j.progress])) });
      await Promise.all([soft(refreshOpps(), null), refreshHistory()]);
      set({ ready: true });
    },
    refreshOpps: guard(refreshOpps),
    refreshHistory: guard(refreshHistory),

    /** Hash-free router: the route lives in the store. @param {string} name @param {object} [params] */
    navigate(name, params = {}) {
      const cur = get().route;
      set({ route: { name, params, from: name === 'step' ? cur : null } });
      if (['clean', 'optimise', 'archive'].includes(name)) c.actions.loadCategory(name);
      if (name === 'history') c.actions.loadHistory();
      if (name === 'disk') c.actions.loadDisk();
      if (name === 'clean') c.actions.loadPresets();
      if (name === 'settings') c.actions.loadSettings();
    },
    back() {
      const from = get().route.from;
      c.actions.navigate(from?.name ?? 'overview', from?.params ?? {});
    },

    // ---------------------------------------------------------------- scope
    removeRoot: guard(async (path) => set({ scope: await api.scopeRemoveRoot({ path }) })),
    /**
     * Open the in-app folder browser. Modes: scope (add a location), move, keep
     * (preferred folder of a keep rule) and export (choose where to save; also
     * carries the file name and what to export).
     * @param {{ mode?: 'scope'|'move'|'keep'|'export', exportJob?: any, filename?: string }} [opts]
     */
    async openBrowser(opts = {}) {
      const mode = opts.mode ?? 'scope';
      const extra = { exportJob: opts.exportJob ?? null, filename: opts.filename ?? '' };
      set({ browser: { mode, ...extra, path: '', parent: null, entries: [], places: [], loading: true } });
      try {
        const first = await api.fsListDir({});
        set({ browser: { mode, ...extra, path: '', parent: null, entries: [], places: first.places, loading: false } });
        const start = (mode === 'export' && first.places.find((p) => p.kind === 'documents')) || first.places[0];
        if (start) await c.actions.browseTo(start.path);
      } catch (e) {
        c.fail(e);
        set({ browser: null });
      }
    },
    /** @param {string} path */
    browseTo: guard(async (path) => {
      const prev = get().browser;
      set({ browser: { ...prev, loading: true } });
      const r = await api.fsListDir({ path, show_hidden: false, include_files: false });
      set({ browser: { ...prev, path: r.path, parent: r.parent ?? null, entries: r.entries, places: r.places, loading: false } });
    }),
    closeBrowser: () => set({ browser: null }),
    /** "Choose this folder": add it to the scan scope. */
    chooseFolder: guard(async () => {
      const b = get().browser;
      if (!b?.path) return;
      if (b.mode === 'move') {
        set({ browser: null });
        await c.actions.setStepAction('move', b.path);
        return;
      }
      if (b.mode === 'keep') {
        set({ browser: null });
        await c.actions.applyKeepRule('preferred_folder', b.path);
        return;
      }
      if (b.mode === 'export') {
        await c.actions.finishExport();
        return;
      }
      set({ scope: await api.scopeAddRoot({ path: b.path }), browser: null });
    }),

    // ----------------------------------------------------------------- scan
    startScan: guard(async () => {
      const r = await api.scanStart({});
      set({ scan: { ...get().scan, state: 'scanning', generation: r.generation }, scanProgress: null, detectors: {}, completed: null });
    }),
    pauseScan: guard(async () => set({ scan: withoutProgress(await api.scanPause()) })),
    resumeScan: guard(async () => set({ scan: withoutProgress(await api.scanResume()) })),
    stopScan: guard(async () => set({ scan: withoutProgress(await api.scanStop()) })),

    // ------------------------------------------------------------- overview
    /** @param {string} tab */
    setTab: guard(async (tab) => { set({ tab, showSmall: false }); await refreshOpps(); }),
    /** @param {string} sort */
    setSort: guard(async (sort) => { set({ sort }); await refreshOpps(); }),
    toggleSmall: () => set({ showSmall: !get().showSmall }),

    // ----------------------------------------------------------------- plan
    /** "+ Add" on a card. @param {string} id */
    addToPlan: guard(async (id) => { set({ plan: await api.planAddStep({ opportunity_id: id }) }); await refreshOpps(); }),
    /** @param {number} stepId */
    removeStep: guard(async (stepId) => { set({ plan: await api.planRemoveStep({ step_id: stepId }) }); await refreshOpps(); }),
    /** Move a step up (-1) or down (+1) with the arrow buttons or the keyboard. @param {number} stepId @param {number} delta */
    moveStep: guard(async (stepId, delta) => {
      const ids = get().plan.steps.map((s) => s.id);
      const i = ids.indexOf(stepId);
      const j = i + delta;
      if (i < 0 || j < 0 || j >= ids.length) return;
      [ids[i], ids[j]] = [ids[j], ids[i]];
      set({ plan: await api.planReorder({ step_ids: ids }) });
    }),
    /** "Estimate exactly (dry run)". */
    dryRun: guard(async () => {
      set({ dryRunning: true });
      try {
        await api.planDryRun();
      } catch (e) {
        set({ dryRunning: false });
        throw e;
      }
    }),
    /** "Save as weekly schedule". */
    saveSchedule: guard(async () => {
      await api.scheduleFromPlan({ name: 'Weekly cleanup', cadence: { every: 'weekly', weekday: 1, hour: 8, minute: 47 } });
      c.notify(copy.plan.scheduleSaved, 'success');
    }),
    dismissToast: () => set({ toast: null }),
  };
}
