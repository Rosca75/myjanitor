// @ts-check
/**
 * Mock backend implementing docs/IPC.md in memory. Owner: W2-F.
 *
 * Options (also read from the page URL: `?mock=1&scenario=results&speed=0.5`):
 *  - scenario: `empty` (default, nothing scanned), `fresh` (scanned, empty
 *    plan) or `results` (scanned, the four-step plan of the wireframe).
 *  - speed: multiplier for every simulated delay; 0 makes timers fire at once.
 *  - now: a clock function (ms) for deterministic tests.
 * The returned object adds `emit`, `whenIdle` and `model` for tests.
 */
import { createModel } from './model.js';
import { createSim } from './sim.js';
import { createHandlers, fail } from './handlers.js';
import { createExtras, applyRich } from './extras.js';

/** @param {string} key */
function fromLocation(key) {
  try {
    return new URLSearchParams(/** @type {any} */ (globalThis).location?.search ?? '').get(key);
  } catch {
    return null;
  }
}

/** @param {{ scenario?: string, speed?: number, now?: () => number }} [opts] */
export function createMockBackend(opts = {}) {
  const platform = opts.platform ?? fromLocation('platform') ?? 'windows';
  /** Commands answered with not_implemented (tests and `?ni=disk_usage_tree,breakdown`). */
  const disabled = new Set(opts.disabled ?? (fromLocation('ni') ?? '').split(',').filter(Boolean));
  /** One-shot failures: command -> ErrorInfo. */
  const failures = new Map();
  const scenario = opts.scenario ?? fromLocation('scenario') ?? 'empty';
  const speed = opts.speed ?? (fromLocation('speed') !== null ? Number(fromLocation('speed')) : 1);
  /** @type {Map<string, Set<(p: any) => void>>} */
  const listeners = new Map();
  let pending = 0;
  /** @type {Array<() => void>} */
  let idleWaiters = [];

  /** @param {string} event @param {any} payload */
  const emit = (event, payload) => {
    for (const cb of [...(listeners.get(event) ?? [])]) cb(structuredClone(payload));
  };
  /** @param {number} ms @param {() => void} fn */
  const later = (ms, fn) => {
    pending += 1;
    setTimeout(() => {
      try {
        fn();
      } finally {
        pending -= 1;
        if (pending === 0) { const w = idleWaiters; idleWaiters = []; w.forEach((r) => r()); }
      }
    }, ms * speed);
  };

  const model = createModel({ now: opts.now, scenario });
  model.platform = platform;
  model.occupied = new Set();
  model.seed(scenario);
  if (scenario === 'rich') applyRich(model);
  const sim = createSim(model, emit, later);
  const base = createHandlers(model, sim, emit);
  const handlers = { ...base, ...createExtras(model, sim, emit, base) };

  return {
    kind: 'mock',
    model,
    /** @param {string} cmd @param {any} [req] */
    async call(cmd, req) {
      const h = /** @type {Record<string, (r: any) => any>} */ (handlers)[cmd];
      if (failures.has(cmd)) { const e = failures.get(cmd); failures.delete(cmd); throw e; }
      if (disabled.has(cmd)) throw fail('not_implemented', `Not implemented yet: ${cmd}`, 'This feature is not available in this build yet.');
      if (!h) throw { code: 'invalid_input', message: `Unknown command: ${cmd}`, hint: 'This is a bug in the UI.' };
      return structuredClone(h(req ?? {}));
    },
    /** @param {string} event @param {(p: any) => void} cb */
    async listen(event, cb) {
      if (!listeners.has(event)) listeners.set(event, new Set());
      listeners.get(event)?.add(cb);
      return () => { listeners.get(event)?.delete(cb); };
    },
    /** Test helper: the next call of `cmd` rejects with `error`. @param {string} cmd @param {any} error */
    failNext: (cmd, error) => { failures.set(cmd, error); },
    /** Test helper: answer `cmd` with not_implemented from now on. @param {string} cmd */
    disable: (cmd) => { disabled.add(cmd); },
    /** Test helper: number of live event listeners. */
    listenerCount: () => [...listeners.values()].reduce((a, set) => a + set.size, 0),
    /** Test helper: push an event to subscribers. @param {string} event @param {any} payload */
    emit,
    /** Test helper: resolves when no simulated timer is pending. */
    whenIdle: () => (pending === 0 ? Promise.resolve() : new Promise((r) => idleWaiters.push(r))),
  };
}
