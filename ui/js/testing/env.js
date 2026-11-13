// Test environment: a fake DOM, a mock backend (no delays) and a recording API,
// wired exactly like main.js wires the real one.
import { installFakeDom } from './fakedom.js';
import { createMockBackend } from '../mock/backend.js';
import { createStore, initialState } from '../state.js';
import { createController } from '../controller.js';
import { bridgeEvents } from '../events.js';
import { mountApp } from '../app.js';

/** camelCase api method -> snake_case command, the inverse of api.js. @param {string} name */
const snake = (name) => name.replace(/[A-Z]/g, (c) => `_${c.toLowerCase()}`);

/**
 * @param {{ scenario?: 'empty'|'fresh'|'results', mount?: boolean, speed?: number, now?: () => number }} [opts]
 */
export async function makeEnv(opts = {}) {
  const doc = installFakeDom();
  const backend = createMockBackend({ scenario: opts.scenario ?? 'results', speed: opts.speed ?? 0, now: opts.now });
  /** @type {{ cmd: string, req: any }[]} */
  const calls = [];
  const api = new Proxy({}, {
    get: (_, name) => {
      if (name === 'on') return (event, cb) => backend.listen(event, cb);
      return (req) => { calls.push({ cmd: snake(String(name)), req }); return backend.call(snake(String(name)), req); };
    },
  });
  const store = createStore(initialState());
  const actions = createController({ api, store });
  const root = doc.createElement('div');
  root.setAttribute('id', 'app');
  doc.body.append(root);
  const env = {
    doc, backend, api, calls, store, actions, root,
    state: () => store.get(),
    /** Commands sent since the last mark, optionally one name. @param {string} [cmd] */
    sent: (cmd) => calls.filter((c) => !cmd || c.cmd === cmd),
    clearCalls: () => { calls.length = 0; },
    /** Wait for the simulation and every pending promise to settle. */
    async settle() {
      for (let i = 0; i < 4; i++) {
        await backend.whenIdle();
        await new Promise((r) => setTimeout(r, 0));
      }
    },
    $: (sel, from = root) => from.querySelector(sel),
    $$: (sel, from = root) => from.querySelectorAll(sel),
    /** First element whose own tag matches and whose text includes `text`. */
    byText(tag, text, from = root) {
      return from.querySelectorAll(tag).find((e) => e.textContent.includes(text)) ?? null;
    },
    /** Click an element and let the effects settle. @param {any} el */
    async click(el) {
      if (!el) throw new Error('click: element not found');
      el.click();
      await env.settle();
    },
  };
  if (opts.mount !== false) {
    mountApp(root, store, actions, { now: opts.now });
    await bridgeEvents({ api, store, actions });
    await actions.init();
    await env.settle();
    env.clearCalls();
  }
  return env;
}

/** A hostile "file name" for escaping tests. */
export const HOSTILE = '<img src=x onerror=alert(1)>"><script>alert(2)</script>&amp;.mp4';

/** A hostile file name without path separators (a real name cannot contain them). */
export const HOSTILE_FILE = '<img src=x onerror=alert(1)>"><b>x<b>&amp;<script>alert(2)<-script>.mp4';
