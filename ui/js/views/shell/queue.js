// @ts-check
/** The expanded bottom strip: the job queue (OPT-ENG-11) and the completion summary. */
import { h } from '../../dom.js';
import { copy } from '../../copy.js';
import { formatBytes, formatCount, middleEllipsis, displayPath } from '../../format.js';
import { button, iconButton } from '../../components/controls.js';
import { progressBar, setProgress } from '../../components/bars.js';
import { badge } from '../../components/badge.js';

const LIVE = ['queued', 'running', 'paused', 'cancelling'];
const TONE = { queued: 'neutral', running: 'select', paused: 'warning', cancelling: 'warning', cancelled: 'neutral', completed: 'success', completed_with_errors: 'warning', failed: 'danger' };

/** @param {any} job @param {any} s */
export const progressOf = (job, s) => s.jobProgress[job.id] ?? job.progress ?? { done: 0, total: 0, bytes_done: 0, bytes_total: 0 };

/** @param {any} job */
const errorCount = (job) => (job.outcome?.failed ?? 0) + (job.state === 'failed' ? 1 : 0);

/** @param {any} job @param {any} s @param {any} actions @param {string[]} queuedIds */
function jobRow(job, s, actions, queuedIds) {
  const p = progressOf(job, s);
  const frac = p.total ? p.done / p.total : job.state === 'completed' ? 1 : 0;
  const live = LIVE.includes(job.state);
  const errs = errorCount(job);
  const skipped = job.outcome?.skipped ?? 0;
  const qi = queuedIds.indexOf(job.id);
  const current = s.jobCurrent[job.id];
  return h('li', { class: ['job', `job--${job.state}`], 'data-job': String(job.id) },
    h('span', { class: 'job__title', title: job.title }, job.title, h('span', { class: 'job__kind' }, ` ${copy.jobs.kinds[job.kind] ?? job.kind}`)),
    badge(copy.queue.states[job.state] ?? job.state, TONE[job.state] ?? 'neutral', { extra: 'job__state' }),
    progressBar({ value: frac, label: job.title }),
    h('span', { class: 'job__count', 'data-role': 'job-count' }, copy.queue.progress(formatCount(p.done), formatCount(p.total))),
    h('span', { class: 'job__current', 'data-role': 'job-current', title: current ? displayPath(current) : undefined }, job.state === 'running' && current ? middleEllipsis(displayPath(current), 60) : ''),
    h('span', { class: 'job__errors', 'data-role': 'job-errors' }, errs > 0 && badge(copy.jobs.errors(errs), 'danger'), skipped > 0 && badge(copy.jobs.skippedN(skipped), 'neutral'), job.error && h('span', { class: 'job__fail', title: job.error.hint }, copy.jobs.failedJob(job.error.message))),
    h('span', { class: 'job__actions' },
      job.state === 'queued' && queuedIds.length > 1 && [
        iconButton({ icon: 'up', label: copy.jobs.up(job.title), disabled: qi === 0, fk: `jup-${job.id}`, onClick: () => actions.moveJob(job.id, -1) }),
        iconButton({ icon: 'down', label: copy.jobs.down(job.title), disabled: qi === queuedIds.length - 1, fk: `jdown-${job.id}`, onClick: () => actions.moveJob(job.id, 1) })],
      job.state === 'running' && button({ label: copy.queue.pause, size: 'sm', ariaLabel: copy.queue.pauseLabel(job.title), fk: `pause-${job.id}`, onClick: () => actions.pauseJob(job.id) }),
      job.state === 'paused' && button({ label: copy.queue.resume, size: 'sm', ariaLabel: copy.queue.resumeLabel(job.title), fk: `resume-${job.id}`, onClick: () => actions.resumeJob(job.id) }),
      live && job.state !== 'cancelling' && button({ label: copy.queue.cancel, size: 'sm', variant: 'quiet', ariaLabel: copy.queue.cancelLabel(job.title), fk: `cancel-${job.id}`, onClick: () => actions.cancelJob(job.id) }),
      !live && job.run_id && job.kind === 'apply' && button({ label: copy.jobs.report, size: 'sm', variant: 'link', fk: `jreport-${job.id}`, onClick: () => actions.viewReport(job.run_id) })));
}

/** Summary after a run finished, with Undo and View report. @param {any} s @param {any} actions */
function summary(s, actions) {
  const c = s.completed;
  return h('div', { class: ['qsummary', c.failed ? 'qsummary--warn' : 'qsummary--ok'], role: 'status', 'data-role': 'queue-summary' },
    h('strong', {}, copy.jobs.summaryTitle), ' ', h('span', {}, copy.jobs.summary(formatBytes(c.saved), formatCount(c.files), c.failed)),
    h('span', { class: 'qsummary__actions' },
      button({ label: copy.jobs.undo, size: 'sm', variant: 'secondary', fk: 'queue-undo', onClick: () => actions.undoLast() }),
      button({ label: copy.jobs.report, size: 'sm', variant: 'link', fk: 'queue-report', onClick: () => actions.viewReport(c.run_id) }),
      button({ label: copy.jobs.dismiss, size: 'sm', variant: 'quiet', fk: 'queue-dismiss', onClick: () => actions.dismissCompleted() })));
}

/** Is the queue section shown? While work runs, after a run finished, or when the user opened it. @param {any} s @param {boolean} active */
export const queueVisible = (s, active) => active || !!s.completed || (s.queueOpen && s.jobs.length > 0);

/** @param {any} s @param {any} actions */
export function renderQueue(s, actions) {
  const queuedIds = s.jobs.filter((j) => j.state === 'queued').map((j) => j.id);
  const jobs = s.jobs.slice(-8);
  return h('section', { class: 'queue', 'aria-label': copy.strip.queue },
    h('h2', { class: 'queue__title' }, copy.strip.queue),
    s.completed && summary(s, actions),
    jobs.length ? h('ul', { class: 'queue__list' }, jobs.map((j) => jobRow(j, s, actions, queuedIds))) : h('p', { class: 'note' }, copy.jobs.empty));
}

/** Update counts, bars and the current file in place (job://progress is frequent). @param {any} root @param {any} s */
export function patchQueue(root, s) {
  for (const li of root.querySelectorAll('.job')) {
    const job = s.jobs.find((j) => String(j.id) === li.getAttribute('data-job'));
    if (!job) continue;
    const p = progressOf(job, s);
    const count = li.querySelector('[data-role="job-count"]');
    if (count) count.textContent = copy.queue.progress(formatCount(p.done), formatCount(p.total));
    const bar = li.querySelector('[role="progressbar"]');
    if (bar) setProgress(bar, p.total ? p.done / p.total : 0);
    const cur = li.querySelector('[data-role="job-current"]');
    const path = s.jobCurrent[job.id];
    if (cur && job.state === 'running') { cur.textContent = path ? middleEllipsis(displayPath(path), 60) : ''; cur.setAttribute('title', path ? displayPath(path) : ''); }
  }
}
