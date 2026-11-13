// @ts-check
/**
 * Entry point (Wave 1 placeholder). W2-F replaces the body with the Option B
 * shell; keep the import of `api` as the only bridge to the backend.
 */
import api, { backendKind } from './api.js';

async function start() {
  const root = document.getElementById('app');
  if (!root) return;
  try {
    const info = await api.appInfo();
    const kind = await backendKind();
    root.querySelector('[data-role="version"]')?.replaceChildren(`${info.name} ${info.version} · ${info.platform} · ${kind} backend`);
  } catch (e) {
    root.querySelector('[data-role="version"]')?.replaceChildren(`Backend unavailable: ${/** @type {any} */ (e).message ?? e}`);
  }
}

start();
