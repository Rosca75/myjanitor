// @ts-check
/**
 * Mock backend implementing the docs/IPC.md shapes in memory. Owner: W2-F
 * (extend with fixtures for every screen; keep shapes identical to IPC.md).
 * Wave 1 implements the start-up commands; everything else rejects with
 * `not_implemented` exactly like the real engine does for unfinished commands.
 */

/** @param {string} what */
const notImplemented = (what) => ({ code: 'not_implemented', message: `Not implemented yet: ${what}`, hint: 'This feature is not available in this build yet.' });

export function createMockBackend() {
  /** @type {Map<string, Set<(p: any) => void>>} */
  const listeners = new Map();
  const state = {
    scope: { roots: [], include: [], exclude: [], options: { include_hidden: true, use_fast_index: true, use_cache: true, one_file_system: true, max_depth: null, threads: 0, collect_owner: false } },
    plan: { id: 0, name: 'Your plan', steps: [], estimate: { files: 0, bytes_before: 0, bytes_after: 0, confidence: 'estimated', skipped: 0 }, checklist: [], updated: 0 },
  };
  const idleScan = { state: 'idle', generation: 0, progress: { generation: 0, phase: null, files: 0, dirs: 0, bytes: 0, done: 0, elapsed_ms: 0 }, cached: false };

  /** @type {Record<string, (req: any) => any>} */
  const handlers = {
    app_info: () => ({ name: 'MyJanitor', version: '0.1.0', platform: 'linux', bundle_id: 'com.myjanitor.app', portable: false, data_dir: '/mock/data', cache_dir: '/mock/cache', elevated: false, journal_persistent: false }),
    scope_get: () => state.scope,
    scope_set: (req) => (state.scope = req.scope),
    scope_add_root: (req) => {
      if (!state.scope.roots.includes(req.path)) state.scope.roots.push(req.path);
      return state.scope;
    },
    scope_remove_root: (req) => {
      state.scope.roots = state.scope.roots.filter((r) => r !== req.path);
      return state.scope;
    },
    scan_status: () => idleScan,
    opportunities_list: () => ({ generation: 0, items: [], summary: { total: 0, reversible: 0, opt_in: 0, review_only: 0, opportunities: 0 }, counts: { all: 0, clean: 0, optimise: 0, archive: 0, review_only: 0 }, cached: false }),
    plan_get: () => state.plan,
    jobs_list: () => [],
    history_list: () => ({ total: 0, runs: [] }),
    report_last: () => null,
    tools_list: () => [],
  };

  return {
    kind: 'mock',
    /** @param {string} cmd @param {any} [req] */
    async call(cmd, req) {
      const h = handlers[cmd];
      if (!h) throw notImplemented(cmd);
      return structuredClone(h(req));
    },
    /** @param {string} event @param {(p: any) => void} cb */
    async listen(event, cb) {
      if (!listeners.has(event)) listeners.set(event, new Set());
      listeners.get(event)?.add(cb);
      return () => listeners.get(event)?.delete(cb);
    },
    /** Test helper: push an event to subscribers. @param {string} event @param {any} payload */
    emit(event, payload) {
      for (const cb of listeners.get(event) ?? []) cb(payload);
    },
  };
}
