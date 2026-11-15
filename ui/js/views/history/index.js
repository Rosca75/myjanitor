// @ts-check
/** History (SAF-03/04/12): the journal grouped by run, with Restore, Undo and exports. */
import { h, icon, fk } from '../../dom.js';
import { copy } from '../../copy.js';
import { formatBytes, formatCount, formatDateTime, formatDate } from '../../format.js';
import { button } from '../../components/controls.js';
import { badge } from '../../components/badge.js';
import { loadingBlock, emptyBlock, errorBlock } from '../../components/states.js';
import { disabledHint } from '../../components/states.js';
import { runStatus } from '../../lib/history.js';
import { renderEntries } from './entries.js';

const TONE = { undoable: 'select', restored: 'success', partly_restored: 'warning', with_errors: 'warning', failed: 'danger', expired: 'neutral', undo: 'neutral' };

/** @param {any} run @param {any} s @param {any} actions */
function runRow(run, s, actions) {
  const status = runStatus(run);
  const open = !!s.historyUi.expanded[run.run_id];
  return h('li', { class: ['run', open && 'run--open'], 'data-run': String(run.run_id) },
    h('div', { class: 'run__head' },
      h('button', { type: 'button', class: 'run__toggle', 'aria-expanded': String(open), 'aria-label': open ? copy.historyView.collapse(run.title) : copy.historyView.expand(run.title), 'data-fk': `toggle-${run.run_id}`, on: { click: () => actions.toggleRun(run.run_id) } },
        icon(open ? 'chevronDown' : 'chevronRight', 14),
        h('span', { class: 'run__main' },
          h('strong', { class: 'run__title' }, run.title),
          h('span', { class: 'run__meta' }, `${formatDateTime(run.started)}  ·  ${copy.historyView.steps(run.steps.length)}  ·  ${copy.historyView.files(formatCount(run.items))}  ·  ${copy.history.saved(formatBytes(run.bytes_saved))}`, run.failed > 0 && `  ·  ${copy.history.failed(run.failed)}`),
          h('span', { class: 'run__steps', title: run.steps.join(', ') }, run.steps.join(', ')))),
      h('div', { class: 'run__side' },
        badge(copy.historyView.status[status], TONE[status]),
        run.undoable && run.undo_until && h('span', { class: 'run__until' }, copy.history.until(formatDate(run.undo_until))),
        button({ label: copy.history.restore, variant: 'secondary', size: 'sm', disabled: !run.undoable, ariaLabel: copy.historyView.undoRunLabel(run.title), fk: `restore-${run.run_id}`, onClick: () => actions.restoreRun(run.run_id) }),
        button({ label: copy.done.report, variant: 'link', size: 'sm', fk: `report-${run.run_id}`, onClick: () => actions.viewReport(run.run_id) }),
        button({ label: copy.historyView.exportRunAudit, variant: 'link', size: 'sm', disabled: !!s.unavail.export, title: disabledHint(s.unavail.export), fk: `audit-${run.run_id}`, onClick: () => actions.startExport({ type: 'audit', format: 'csv', run_id: run.run_id }) }))),
    open && renderEntries(run, s, actions));
}

/** Outcome of the last restore job: counts and every refusal with its reason. @param {any} s @param {any} actions */
function resultPanel(s, actions) {
  const job = s.historyUi.result?.job;
  if (!job) return null;
  const o = job.outcome ?? { succeeded: 0, skipped: 0, failed: 0, errors: [] };
  return h('section', { class: ['notice', o.failed || o.skipped ? 'notice--warning' : 'notice--success'], role: 'status', 'data-role': 'restore-result' },
    h('div', { class: 'notice__text' }, h('strong', {}, copy.historyView.resultTitle), ' ', copy.historyView.resultLine(o.succeeded, o.skipped, o.failed),
      o.errors.length > 0 && h('div', {}, h('span', { class: 'notice__sub' }, copy.historyView.resultRefused),
        h('ul', { class: 'refusals' }, o.errors.map((e) => h('li', {}, h('span', { class: 'path', title: e.path }, e.path), ' ', e.error && h('span', { class: 'refusals__why' }, [e.error.message, e.error.hint].filter(Boolean).join(' ')))))),
    ),
    h('div', { class: 'notice__actions' }, button({ label: copy.historyView.dismiss, variant: 'quiet', fk: 'result-dismiss', onClick: () => actions.dismissResult() })));
}

/** @param {any} s @param {any} actions */
export function renderHistory(s, actions) {
  const q = s.quarantine;
  const ui = s.historyUi;
  const runs = s.history.runs;
  const undoDays = ui.undoDays;
  return h('div', { class: 'view view--history' },
    h('section', { class: 'intro' }, h('h2', { class: 'intro__title' }, copy.history.title), h('p', { class: 'intro__text' }, copy.history.intro)),
    h('div', { class: 'history-tools' },
      h('p', { class: 'note', 'data-role': 'undo-window' }, undoDays ? copy.historyView.undoWindow(undoDays) : copy.historyView.undoWindowUnknown),
      button({ label: copy.historyView.exportAudit, size: 'sm', disabled: !!s.unavail.export || !runs.length, title: disabledHint(s.unavail.export), fk: 'export-audit', onClick: () => actions.startExport({ type: 'audit', format: 'csv' }) })),
    q && h('div', { class: 'quarantine' },
      h('span', {}, copy.historyView.quarantineLine(formatCount(q.items), formatBytes(q.bytes), q.oldest ? formatDate(q.oldest) : '')),
      button({ label: copy.historyView.purge, size: 'sm', variant: 'secondary', disabled: !!s.unavail.quarantine_purge || q.items === 0, title: s.unavail.quarantine_purge ? disabledHint(s.unavail.quarantine_purge) : undefined, fk: 'purge', onClick: () => actions.askPurge() })),
    resultPanel(s, actions),
    ui.error ? errorBlock(ui.error, { onRetry: () => actions.loadHistory(), fk: 'history-retry' })
      : ui.loading && !runs.length ? loadingBlock()
        : runs.length ? h('ul', { class: 'runs', 'aria-label': copy.history.title }, runs.map((r) => runRow(r, s, actions)))
          : emptyBlock({ title: copy.historyView.emptyTitle, body: copy.historyView.emptyBody, action: button({ label: copy.states.goOverview, size: 'sm', fk: 'to-overview', onClick: () => actions.navigate('overview') }) }),
    s.history.total > runs.length && button({ label: copy.historyView.loadMore(formatCount(s.history.total - runs.length)), variant: 'link', disabled: ui.loadingMore, fk: 'more-runs', onClick: () => actions.loadMoreRuns() }));
}
void fk;
