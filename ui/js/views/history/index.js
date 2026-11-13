// @ts-check
/** History: the journal of runs, each with Restore while its undo window is open. */
import { h } from '../../dom.js';
import { copy } from '../../copy.js';
import { formatBytes, formatDate } from '../../format.js';
import { button } from '../../components/controls.js';
import { badge } from '../../components/badge.js';

/** @param {any} run @param {any} actions */
function runRow(run, actions) {
  const restored = run.undone > 0 && !run.undoable;
  return h('li', { class: 'run', 'data-run': String(run.run_id) },
    h('div', { class: 'run__main' },
      h('strong', { class: 'run__title' }, run.title),
      h('span', { class: 'run__meta' }, `${formatDate(run.started)}  ·  ${copy.history.items(run.items)}  ·  ${copy.history.saved(formatBytes(run.bytes_saved))}`, run.failed > 0 && `  ·  ${copy.history.failed(run.failed)}`),
      h('span', { class: 'run__steps' }, run.steps.join(', '))),
    h('div', { class: 'run__side' },
      restored ? badge(copy.history.restored, 'success') : !run.undoable && badge(copy.history.expired, 'neutral'),
      run.undoable && run.undo_until && h('span', { class: 'run__until' }, copy.history.until(formatDate(run.undo_until))),
      button({ label: copy.history.restore, variant: 'secondary', size: 'sm', disabled: !run.undoable, ariaLabel: copy.history.restoreLabel(run.title), fk: `restore-${run.run_id}`, onClick: () => actions.restoreRun(run.run_id) }),
      button({ label: copy.done.report, variant: 'link', size: 'sm', fk: `report-${run.run_id}`, onClick: () => actions.viewReport(run.run_id) })));
}

/** @param {any} s @param {any} actions */
export function renderHistory(s, actions) {
  const q = s.quarantine;
  return h('div', { class: 'view view--history' },
    h('section', { class: 'intro' }, h('h2', { class: 'intro__title' }, copy.history.title), h('p', { class: 'intro__text' }, copy.history.intro)),
    q && h('p', { class: 'note' }, copy.history.quarantine(q.items, formatBytes(q.bytes))),
    s.history.runs.length ? h('ul', { class: 'runs' }, s.history.runs.map((r) => runRow(r, actions))) : h('p', { class: 'empty-note' }, copy.history.empty));
}
