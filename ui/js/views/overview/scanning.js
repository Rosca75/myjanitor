// @ts-check
/** B1 while scanning: progress header with Pause/Stop and live-filling cards. */
import { h } from '../../dom.js';
import { copy } from '../../copy.js';
import { formatCount } from '../../format.js';
import { button } from '../../components/controls.js';
import { progressBar, setProgress } from '../../components/bars.js';
import { liveCard, placeholderCard } from '../../components/card.js';

/** Fraction of the whole scan: walking, hashing, then detectors done / total. @param {any} p */
export function scanFraction(p) {
  if (!p) return 0;
  if (p.phase === 'walking') return 0.15;
  if (p.phase === 'hashing') return 0.4;
  return 0.5 + (p.total ? 0.5 * Math.min(1, p.done / p.total) : 0);
}

const SLOTS = 9;

/** @param {any} p @param {number} found */
const metaText = (p, found) => (p ? `${copy.scanning.files(formatCount(p.files))}  ·  ${copy.scanning.found(found)}` : copy.scanning.placeholder);

/**
 * Update the progress text and bar in place (scan://progress arrives every
 * 50 ms; re-rendering the buttons that often would swallow clicks).
 * @param {any} root @param {any} s
 */
export function patchScanning(root, s) {
  const p = s.scanProgress;
  const meta = root.querySelector('[data-role="scan-meta"]');
  if (!meta) return;
  meta.textContent = metaText(p, Object.keys(s.detectors).length);
  const path = root.querySelector('[data-role="scan-path"]');
  if (path) { path.textContent = p?.current ?? ''; path.setAttribute('title', p?.current ?? ''); }
  const bar = root.querySelector('[role="progressbar"]');
  if (bar) setProgress(bar, scanFraction(p));
}

/** @param {any} s @param {any} actions */
export function renderScanning(s, actions) {
  const p = s.scanProgress;
  const paused = s.scan.state === 'paused';
  const stopping = s.scan.state === 'stopping';
  const found = Object.values(s.detectors).sort((a, b) => b.saving - a.saving);
  const phase = copy.scanning.phase[p?.phase ?? 'idle'] ?? copy.scanning.phase.idle;
  const slots = Math.max(0, SLOTS - found.length);
  return h('div', { class: 'view view--scanning' },
    h('section', { class: 'scanhead', 'aria-label': copy.scanning.title },
      h('div', { class: 'scanhead__text' },
        h('h2', { class: 'scanhead__title' }, stopping ? copy.scanning.stopping : paused ? copy.scanning.paused : `${copy.scanning.title}: ${phase}`),
        h('p', { class: 'scanhead__meta', 'aria-live': 'polite', 'data-role': 'scan-meta' }, metaText(p, found.length)),
        h('p', { class: 'scanhead__path', 'data-role': 'scan-path', title: p?.current ?? '' }, p?.current ?? '')),
      h('div', { class: 'scanhead__buttons' },
        paused
          ? button({ label: copy.scanning.resume, variant: 'secondary', fk: 'scan-resume', onClick: () => actions.resumeScan() })
          : button({ label: copy.scanning.pause, variant: 'secondary', disabled: stopping, fk: 'scan-pause', onClick: () => actions.pauseScan() }),
        button({ label: copy.scanning.stop, variant: 'quiet', disabled: stopping, fk: 'scan-stop', onClick: () => actions.stopScan() }))),
    progressBar({ value: scanFraction(p), label: copy.scanning.title, tone: 'accent' }),
    h('div', { class: 'grid grid--live', 'aria-label': copy.tabs.label }, found.map(liveCard), Array.from({ length: slots }, placeholderCard)));
}
