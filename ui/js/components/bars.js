// @ts-check
/** Bars drawn with widths set through the CSSOM (no inline style attributes). */
import { h } from '../dom.js';
import { pct } from '../format.js';

/** @param {any} el @param {number} percent */
const width = (el, percent) => { el.style.setProperty('width', `${percent.toFixed(2)}%`); return el; };

/**
 * Stacked bar of segments { key, value, label, tone }. Min visible width 1.5%.
 * @param {{ segments: {key: string, value: number, tone: string}[], label: string }} o
 */
export function stackedBar(o) {
  const total = o.segments.reduce((a, s) => a + s.value, 0);
  return h('div', { class: 'stack', role: 'img', 'aria-label': o.label },
    o.segments.filter((s) => s.value > 0).map((s) => width(h('span', { class: ['stack__seg', `stack__seg--${s.tone}`], 'data-seg': s.key }), Math.max(1.5, pct(s.value, total)))));
}

/**
 * Drive bar: used after the plan, freed by the plan, free (the track).
 * @param {{ total: number, free: number, freeAfter: number, label: string }} o
 */
export function driveBar(o) {
  const usedAfter = pct(o.total - o.freeAfter, o.total);
  const freed = pct(Math.max(0, o.freeAfter - o.free), o.total);
  return h('div', { class: 'drive', role: 'img', 'aria-label': o.label },
    width(h('span', { class: 'drive__used', 'data-seg': 'used' }), usedAfter),
    freed > 0 && width(h('span', { class: 'drive__freed', 'data-seg': 'freed' }), Math.max(1, freed)));
}

/** Thin progress bar 0..1. @param {{ value: number, label: string, tone?: string }} o */
export function progressBar(o) {
  const v = Math.min(1, Math.max(0, o.value || 0));
  return h('div', { class: ['progress', o.tone && `progress--${o.tone}`], role: 'progressbar', 'aria-label': o.label, 'aria-valuemin': '0', 'aria-valuemax': '100', 'aria-valuenow': String(Math.round(v * 100)) },
    width(h('span', { class: 'progress__fill' }), v * 100));
}

/** Update a progress bar in place. @param {any} bar @param {number} value 0..1 */
export function setProgress(bar, value) {
  const v = Math.min(1, Math.max(0, value || 0));
  bar.setAttribute('aria-valuenow', String(Math.round(v * 100)));
  const fill = bar.querySelector('.progress__fill');
  if (fill) fill.style.setProperty('width', `${(v * 100).toFixed(2)}%`);
}
