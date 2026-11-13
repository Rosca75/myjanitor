// @ts-check
/** Bottom status strip, expanding into the job queue while work runs. */
import { h } from '../../dom.js';
import { copy } from '../../copy.js';
import { formatBytes } from '../../format.js';
import { button } from '../../components/controls.js';
import { progressBar, setProgress } from '../../components/bars.js';
import { activeJobs, isScanning, planSaving } from '../../state.js';

/** @param {any} job @param {any} s */
const progressOf = (job, s) => s.jobProgress[job.id] ?? job.progress ?? { done: 0, total: 0 };

/** @param {any} job @param {any} s @param {any} actions */
function jobRow(job, s, actions) {
  const p = progressOf(job, s);
  const frac = p.total ? p.done / p.total : 0;
  const state = copy.queue.states[job.state] ?? job.state;
  const live = ['queued', 'running', 'paused', 'cancelling'].includes(job.state);
  return h('li', { class: ['job', `job--${job.state}`], 'data-job': String(job.id) },
    h('span', { class: 'job__title', title: job.title }, job.title),
    h('span', { class: 'job__state' }, state),
    progressBar({ value: frac, label: job.title }),
    h('span', { class: 'job__count', 'data-role': 'job-count' }, copy.queue.progress(p.done, p.total)),
    h('span', { class: 'job__actions' },
      job.state === 'running' && button({ label: copy.queue.pause, size: 'sm', ariaLabel: copy.queue.pauseLabel(job.title), fk: `pause-${job.id}`, onClick: () => actions.pauseJob(job.id) }),
      job.state === 'paused' && button({ label: copy.queue.resume, size: 'sm', ariaLabel: copy.queue.resumeLabel(job.title), fk: `resume-${job.id}`, onClick: () => actions.resumeJob(job.id) }),
      live && job.state !== 'cancelling' && button({ label: copy.queue.cancel, size: 'sm', variant: 'quiet', ariaLabel: copy.queue.cancelLabel(job.title), fk: `cancel-${job.id}`, onClick: () => actions.cancelJob(job.id) })));
}

/** @param {any} s @param {any} actions */
export function renderStrip(s, actions) {
  const active = activeJobs(s);
  const jobs = s.jobs.slice(-6);
  const reclaimable = s.opps.summary.total;
  const total = reclaimable + s.reclaimed;
  const steps = s.plan.steps.length;
  const status = s.dialog ? copy.strip.waiting : isScanning(s) ? copy.strip.scanning : active.length ? copy.strip.running(active.length) : copy.strip.idle;
  const expanded = active.length > 0;
  return h('div', { class: ['strip', expanded && 'strip--expanded'] },
    expanded && h('section', { class: 'queue', 'aria-label': copy.strip.queue },
      h('h2', { class: 'queue__title' }, copy.strip.queue),
      h('ul', { class: 'queue__list' }, jobs.map((j) => jobRow(j, s, actions)))),
    h('div', { class: 'strip__bar' },
      h('div', { class: 'strip__group', 'aria-live': 'polite' },
        h('span', { class: 'strip__label' }, copy.strip.reclaimable), h('strong', { class: 'strip__value', 'data-role': 'reclaimable' }, formatBytes(reclaimable)),
        progressBar({ value: total ? s.reclaimed / total : 0, label: copy.strip.reclaimed }),
        h('span', { class: 'strip__label' }, copy.strip.reclaimed), h('strong', { class: 'strip__value', 'data-role': 'reclaimed' }, formatBytes(s.reclaimed))),
      h('div', { class: 'strip__group strip__sep', 'aria-live': 'polite', 'data-role': 'plan-summary' }, steps ? copy.strip.plan(steps, formatBytes(planSaving(s))) : copy.strip.planNone),
      h('div', { class: 'strip__group strip__sep', 'aria-live': 'polite' }, h('span', { class: ['dot', (active.length || isScanning(s)) && 'dot--busy'], 'aria-hidden': 'true' }), h('span', { 'data-role': 'status' }, status)),
      h('div', { class: 'strip__spacer' }),
      button({ label: copy.strip.undo, variant: 'quiet', disabled: !s.undoable || active.length > 0, title: s.undoable ? undefined : copy.strip.undoNone, fk: 'undo-last', onClick: () => actions.undoLast() }),
      button({ label: copy.strip.report, variant: 'link', disabled: !s.lastReport, title: s.lastReport ? undefined : copy.strip.reportNone, fk: 'last-report', onClick: () => actions.viewReport() })));
}

/**
 * Update job progress in place (job://progress is frequent; re-rendering the
 * Pause and Cancel buttons that often would swallow clicks).
 * @param {any} root @param {any} s
 */
export function patchStrip(root, s) {
  for (const li of root.querySelectorAll('.job')) {
    const job = s.jobs.find((j) => String(j.id) === li.getAttribute('data-job'));
    if (!job) continue;
    const p = progressOf(job, s);
    const count = li.querySelector('[data-role="job-count"]');
    if (count) count.textContent = copy.queue.progress(p.done, p.total);
    const bar = li.querySelector('[role="progressbar"]');
    if (bar) setProgress(bar, p.total ? p.done / p.total : 0);
  }
}
