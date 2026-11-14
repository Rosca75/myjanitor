// @ts-check
/**
 * Entry point: builds the store and controller on top of api.js (real Tauri
 * backend, or the mock when running in a plain browser / ?mock=1), mounts the
 * shell, loads the start-up data and bridges the IPC events.
 */
import api, { backendKind } from './api.js';
import { createStore, initialState } from './state.js';
import { createController } from './controller.js';
import { bridgeEvents } from './events.js';
import { mountApp } from './app.js';
import { copy } from './copy.js';

async function start() {
  const root = document.getElementById('app');
  if (!root) return;
  // api.js creates its backend lazily and concurrent first calls would each create one,
  // so resolve it once here before anything else talks to it.
  await backendKind();
  const store = createStore(initialState());
  const actions = createController({ api, store });
  const app = mountApp(root, store, actions);
  const unlisten = await bridgeEvents({ api, store, actions });
  // Tear everything down when the page goes away: no listener outlives its view.
  globalThis.addEventListener?.('pagehide', () => { unlisten(); app.dispose(); });
  // Dev aid for the mock backend only: lets browser tests read the state.
  if ((await backendKind()) === 'mock') /** @type {any} */ (globalThis).__mj = { store, actions };
  await actions.init();
}

start().catch((e) => {
  const root = document.getElementById('app');
  if (root) root.textContent = copy.boot.failed(e?.message ?? e);
});
