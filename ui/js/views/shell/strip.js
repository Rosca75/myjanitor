// @ts-check
/** Bottom status strip, expanding into the job queue while work runs. */
import { h } from '../../dom.js';
import { copy } from '../../copy.js';
import { formatBytes } from '../../format.js';
import { button } from '../../components/controls.js';
import { progressBar } from '../../components/bars.js';
import { renderQueue, patchQueue, queueVisible } from './queue.js';
import { activeJobs, isScanning, planSaving } from '../../state.js';

/** @param {any} s @param {any} actions */
export function renderStrip(s, actions) {
  const active = activeJobs(s);
  const reclaimable = s.opps.summary.total;
  const total = reclaimable + s.reclaimed;
  const steps = s.plan.steps.length;
  const status = s.dialog ? copy.strip.waiting : isScanning(s) ? copy.strip.scanning : active.length ? copy.strip.running(active.length) : copy.strip.idle;
  const show = queueVisible(s, active.length > 0);
  return h('div', { class: ['strip', show && 'strip--expanded'] },
    show && renderQueue(s, actions),
    h('div', { class: 'strip__bar' },
      h('div', { class: 'strip__group', 'aria-live': 'polite' },
        h('span', { class: 'strip__label' }, copy.strip.reclaimable), h('strong', { class: 'strip__value', 'data-role': 'reclaimable' }, formatBytes(reclaimable)),
        progressBar({ value: total ? s.reclaimed / total : 0, label: copy.strip.reclaimed }),
        h('span', { class: 'strip__label' }, copy.strip.reclaimed), h('strong', { class: 'strip__value', 'data-role': 'reclaimed' }, formatBytes(s.reclaimed))),
      h('div', { class: 'strip__group strip__sep', 'aria-live': 'polite', 'data-role': 'plan-summary' }, steps ? copy.strip.plan(steps, formatBytes(planSaving(s))) : copy.strip.planNone),
      h('div', { class: 'strip__group strip__sep', 'aria-live': 'polite' }, h('span', { class: ['dot', (active.length || isScanning(s)) && 'dot--busy'], 'aria-hidden': 'true' }), h('span', { 'data-role': 'status' }, status)),
      h('div', { class: 'strip__spacer' }),
      s.jobs.length > 0 && !active.length && !s.completed && button({ label: s.queueOpen ? copy.jobs.hide : copy.jobs.open(s.jobs.length), variant: 'quiet', fk: 'queue-toggle', pressed: s.queueOpen, onClick: () => actions.toggleQueue() }),
      button({ label: copy.strip.undo, variant: 'quiet', disabled: !s.undoable || active.length > 0, title: s.undoable ? undefined : copy.strip.undoNone, fk: 'undo-last', onClick: () => actions.undoLast() }),
      button({ label: copy.strip.report, variant: 'link', disabled: !s.lastReport, title: s.lastReport ? undefined : copy.strip.reportNone, fk: 'last-report', onClick: () => actions.viewReport() })));
}

/** In-place progress, see queue.js. @param {any} root @param {any} s */
export const patchStrip = patchQueue;
