// @ts-check
/**
 * Mounts the shell: nav rail, header, main view, right panel, status strip,
 * overlays and toast. Each region subscribes to the store keys it reads and
 * redraws only when one of them changes. Progress numbers live in their own
 * keys and are patched in place, so controls are never replaced mid-click.
 */
import { h, replaceKeepingFocus } from './dom.js';
import { copy } from './copy.js';
import { formatBytes } from './format.js';
import { planSaving } from './state.js';
import { renderRail } from './views/shell/rail.js';
import { renderHeader } from './views/shell/header.js';
import { renderStrip, patchStrip } from './views/shell/strip.js';
import { renderPanel, hasPanel } from './views/shell/panel.js';
import { renderOverlay, renderToast, createFocusManager, overlayKind } from './views/shell/overlays.js';
import { renderOverview } from './views/overview/index.js';
import { patchScanning } from './views/overview/scanning.js';
import { renderStep } from './views/step/index.js';
import { renderClean } from './views/clean/index.js';
import { renderOptimise } from './views/optimise/index.js';
import { renderArchive } from './views/archive/index.js';
import { renderHistory } from './views/history/index.js';
import { renderSettings } from './views/settings/index.js';

/** Route name to view function. */
export const VIEWS = { overview: renderOverview, step: renderStep, clean: renderClean, optimise: renderOptimise, archive: renderArchive, history: renderHistory, settings: renderSettings };

const MAIN_KEYS = ['route', 'scan', 'detectors', 'opps', 'tab', 'sort', 'showSmall', 'plan', 'drives', 'completed', 'step', 'category', 'history', 'quarantine', 'settings', 'cache', 'tools', 'custom', 'info', 'scope', 'historyUi', 'disk', 'unavail', 'drives', 'tools'];

/**
 * @param {any} root element to mount into
 * @param {any} store
 * @param {any} actions
 * @param {{ now?: () => number }} [opts]
 */
export function mountApp(root, store, actions, opts = {}) {
  const now = opts.now ?? (() => Date.now());
  /** Every store subscription, so teardown can drop them all. */
  const offs = [];
  const sub = (fn, keys) => { offs.push(store.subscribe(fn, keys)); };
  const rail = h('div', { class: 'app__rail' });
  const header = h('div', { class: 'app__header' });
  const main = h('main', { class: 'app__main', id: 'main', 'aria-label': 'Content' });
  const panel = h('aside', { class: 'app__panel', 'aria-label': 'Side panel' });
  const strip = h('footer', { class: 'app__strip', 'aria-label': 'Status' });
  const layout = h('div', { class: 'app' }, rail, header, main, panel, strip);
  const overlay = h('div', { class: 'overlay' });
  const toast = h('div', { class: 'toasts' });
  const live = h('div', { class: 'sr-only', id: 'live', 'aria-live': 'polite', 'aria-atomic': 'true' });
  root.classList.remove('booting');
  root.replaceChildren(layout, overlay, toast, live);

  /** Draw a region now and whenever one of `keys` changes. */
  const bind = (el, keys, render) => {
    const draw = (s) => replaceKeepingFocus(el, render(s));
    draw(store.get());
    sub(draw, keys);
  };

  bind(rail, ['route', 'step', 'opps'], (s) => renderRail(s, actions));
  bind(header, ['route', 'scope', 'scan', 'opps', 'step', 'plan'], (s) => renderHeader(s, actions, now()));
  bind(main, MAIN_KEYS, (s) => (VIEWS[s.route.name] ?? renderOverview)(s, actions));
  bind(panel, ['route', 'plan', 'step', 'tools', 'dryRunning', 'jobs', 'scan'], (s) => renderPanel(s, actions));
  bind(strip, ['jobs', 'plan', 'opps', 'reclaimed', 'undoable', 'lastReport', 'scan', 'dialog', 'completed', 'queueOpen'], (s) => renderStrip(s, actions));

  // In-place progress (no re-render of the controls).
  sub((s) => patchScanning(main, s), ['scanProgress']);
  sub((s) => patchStrip(strip, s), ['jobProgress']);

  // Panel visibility and scroll reset follow the route.
  const onRoute = (s) => {
    layout.classList.toggle('app--no-panel', !hasPanel(s));
    main.scrollTop = 0;
  };
  onRoute(store.get());
  sub(onRoute, ['route']);

  // Overlays: focus goes in on open and back to the opener on close.
  const focus = createFocusManager();
  const drawOverlay = (s) => {
    const kind = overlayKind(s);
    focus.before(kind);
    replaceKeepingFocus(overlay, renderOverlay(s, actions));
    focus.after(overlay, kind);
  };
  drawOverlay(store.get());
  sub(drawOverlay, ['dialog', 'browser', 'report', 'ask', 'compare', 'reportUi']);

  bind(toast, ['toast'], (s) => renderToast(s, actions));

  // One polite live region for the totals screen readers should hear.
  const announce = (s) => {
    const n = s.plan.steps.length;
    live.textContent = `${copy.plan.steps(n)}, ${formatBytes(planSaving(s))}. ${copy.strip.reclaimable} ${formatBytes(s.opps.summary.total)}.`;
  };
  announce(store.get());
  sub(announce, ['plan', 'opps']);

  /** Unsubscribe every region (view teardown). */
  const dispose = () => offs.splice(0).forEach((off) => off());
  return { layout, main, panel, strip, header, rail, overlay, toast, live, dispose };
}
