// @ts-check
/** Report of one run (SAF-04): totals, errors and skipped items with reasons, exports. */
import { h } from '../../dom.js';
import { copy } from '../../copy.js';
import { formatBytes, formatCount, formatDateTime, formatDate } from '../../format.js';
import { button } from '../../components/controls.js';
import { badge } from '../../components/badge.js';
import { modal } from '../../components/modal.js';
import { pathText } from '../../components/path.js';
import { disabledHint } from '../../components/states.js';

const TONE = { done: 'success', skipped: 'neutral', failed: 'danger' };
const SHOWN = 100;

/** Why an item was not changed: friendly code label plus the engine's message and hint. @param {any} item */
function reason(item) {
  const e = item.error;
  if (!e) return '';
  return h('span', { class: 'reason' }, h('strong', { class: 'reason__code' }, copy.report.codes[e.code] ?? e.code), ' ', [e.message, e.hint].filter(Boolean).join(' '));
}

/** @param {any} item */
const itemRow = (item) => h('tr', { class: 'row', 'data-status': item.status },
  h('td', { class: 'cell cell--name' }, pathText(item.path, { max: 56 })),
  h('td', { class: 'cell' }, badge(copy.report.status[item.status] ?? item.status, TONE[item.status] ?? 'neutral')),
  h('td', { class: 'cell cell--num' }, item.bytes_after === item.bytes_before ? formatBytes(item.bytes_before) : `${formatBytes(item.bytes_before)} → ${formatBytes(item.bytes_after)}`),
  h('td', { class: 'cell' }, reason(item)));

/** @param {any} sec @param {number} i @param {any} s @param {any} actions */
function section(sec, i, s, actions) {
  const filter = s.reportUi.filter;
  const all = (sec.items ?? []).filter((it) => filter === 'all' || it.status === filter);
  const limit = s.reportUi.shown[i] ?? SHOWN;
  const list = all.slice(0, limit);
  return h('section', { class: 'rsec', 'data-section': String(i) },
    h('h3', { class: 'rsec__title' }, sec.title, h('span', { class: 'rsec__sub' }, ` ${copy.report.sectionLine(sec.action ?? '', sec.totals?.files ?? (sec.items ?? []).length)}`)),
    list.length
      ? h('table', { class: 'table table--report', 'aria-label': sec.title },
        h('thead', {}, h('tr', {}, h('th', { scope: 'col', class: 'cell' }, copy.report.colItem), h('th', { scope: 'col', class: 'cell' }, copy.report.colStatus), h('th', { scope: 'col', class: 'cell cell--num' }, copy.report.colSize), h('th', { scope: 'col', class: 'cell' }, copy.report.colReason))),
        h('tbody', {}, list.map(itemRow)))
      : h('p', { class: 'note' }, copy.report.noItems),
    all.length > list.length && button({ label: copy.report.showMore(formatCount(Math.min(200, all.length - list.length))), variant: 'link', size: 'sm', fk: `more-${i}`, onClick: () => actions.showMoreReport(i) }));
}

/** @param {any} s @param {any} actions */
export function renderReport(s, actions) {
  const r = s.report;
  const t = r.totals;
  const stat = (label, value, tone) => h('div', { class: ['stat', tone && `stat--${tone}`], 'data-stat': label }, h('span', { class: 'stat__label' }, label), h('strong', { class: 'stat__value' }, value));
  const off = s.unavail.export;
  const exp = (label, job, fk) => button({ label, size: 'sm', disabled: !!off, title: disabledHint(off), fk, onClick: () => actions.startExport({ ...job, run_id: r.run_id }) });
  const counts = { all: t.files, done: t.succeeded, skipped: t.skipped, failed: t.failed };
  const children = [
    h('h2', { class: 'modal__title', id: 'report-title' }, r.title),
    h('p', { class: 'modal__body' }, copy.report.started(formatDateTime(r.started)), r.undo_until && `  ·  ${copy.report.undoUntil(formatDate(r.undo_until))}`),
    h('div', { class: 'stats stats--5' }, stat(copy.report.files, formatCount(t.files)), stat(copy.report.done, formatCount(t.succeeded)), stat(copy.report.skipped, formatCount(t.skipped)), stat(copy.report.failed, formatCount(t.failed), t.failed ? 'bad' : undefined), stat(copy.report.saving, formatBytes(t.bytes_saved), 'good')),
    h('p', { class: 'note' }, copy.report.bytes(formatBytes(t.bytes_before), formatBytes(t.bytes_after))),
    h('div', { class: 'rfilter', role: 'group', 'aria-label': copy.report.filterLabel }, Object.keys(copy.report.filters).map((k) => button({ label: `${copy.report.filters[k]} ${formatCount(counts[k])}`, size: 'sm', pressed: s.reportUi.filter === k, fk: `rfilter-${k}`, onClick: () => actions.setReportFilter(k) }))),
    h('div', { class: 'rbody' }, (r.sections ?? []).map((sec, i) => section(sec, i, s, actions))),
    off && h('p', { class: 'note' }, copy.report.exportsOff([off.message, off.hint].filter(Boolean).join(' '))),
    h('div', { class: 'modal__foot' },
      exp(copy.report.exportCsv, { type: 'report', format: 'csv' }, 'report-csv'),
      exp(copy.report.exportHtml, { type: 'report', format: 'html' }, 'report-html'),
      exp(copy.report.exportAudit, { type: 'audit', format: 'csv' }, 'report-audit'),
      h('span', { class: 'modal__spacer' }), button({ label: copy.report.close, variant: 'secondary', fk: 'report-close', onClick: () => actions.closeReport() })),
  ];
  return modal({ labelId: 'report-title', title: r.title, children, extra: 'modal--wide', onClose: () => actions.closeReport() });
}
