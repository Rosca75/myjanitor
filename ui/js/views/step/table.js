// @ts-check
/**
 * Results table of a step. One page of rows at a time (50), with paging
 * buttons: a large list never builds more than a window of rows, and the
 * table lives in the view's single scroll container.
 */
import { h, icon, fk } from '../../dom.js';
import { copy } from '../../copy.js';
import { basename, dirname, formatBytes, displayPath } from '../../format.js';
import { button, checkbox } from '../../components/controls.js';
import { badge, rowBadge } from '../../components/badge.js';
import { tabs } from '../../components/tabs.js';
import { PAGE_SIZE } from '../../ctl-step.js';

/** Notes chip text from the engine's reason; silences read as a warning. @param {string} reason */
const reasonTone = (reason) => (/silence/i.test(reason) ? 'warning' : /same meeting|screen/i.test(reason) ? 'info' : 'neutral');

/** Notes: duplicate flag, the reason as a chip (unless generic), then the other flags. @param {any} row */
function notesCell(row) {
  const reason = String(row.reason ?? '');
  const generic = /^(same content|kept:)/i.test(reason) || reason.toLowerCase() === 'duplicate';
  return h('td', { class: 'cell cell--notes' },
    row.badges.includes('duplicate') && rowBadge('duplicate'),
    reason && !generic && badge(displayPath(reason), reasonTone(reason), { extra: 'badge--row', title: reason }),
    row.badges.filter((b) => b !== 'duplicate').map(rowBadge));
}

/** @param {any} row @param {boolean} audio */
function afterCell(row, audio) {
  if (!row.selected) return copy.step.unchecked;
  if (row.size_after === 0) return copy.step.removed;
  return `${audio ? '~' : ''}${formatBytes(row.size_after)}`;
}

/** @param {any} row @param {boolean} audio @param {any} actions */
function tableRow(row, audio, actions) {
  const name = basename(row.path);
  return h('tr', { class: ['row', row.selected && 'row--checked'], 'data-row': String(row.id) },
    h('td', { class: 'cell cell--check' }, checkbox({ checked: row.selected, label: copy.step.selectRow(name), disabled: !row.actionable, fk: `row-${fk(row.id)}`, onChange: (on) => actions.toggleRow(row.id, on) })),
    h('td', { class: 'cell cell--name', title: displayPath(row.path) }, h('button', { type: 'button', class: 'cell__file', 'data-fk': `name-${fk(row.id)}`, 'aria-label': `${copy.step.preview}: ${displayPath(name)}`, on: { click: () => actions.previewSample(row.id) } }, icon('file', 14), h('span', { class: 'cell__text' }, displayPath(name)))),
    audio
      ? [h('td', { class: 'cell' }, row.columns?.source ?? ''), h('td', { class: 'cell cell--num' }, row.columns?.length ?? '')]
      : h('td', { class: 'cell cell--folder', title: displayPath(dirname(row.path)) }, displayPath(dirname(row.path))),
    h('td', { class: 'cell cell--num' }, formatBytes(row.size)),
    h('td', { class: 'cell cell--num' }, afterCell(row, audio)),
    notesCell(row));
}

/** @param {any} s @param {any} actions */
export function renderTable(s, actions) {
  const st = s.step;
  const opp = st.detail.opportunity;
  const audio = opp.source?.optimiser === 'audio';
  const rows = st.page?.rows ?? [];
  const total = st.page?.total ?? 0;
  const selectable = rows.filter((r) => r.actionable);
  const allOn = selectable.length > 0 && selectable.every((r) => r.selected);
  const first = total ? st.offset + 1 : 0;
  const last = st.offset + rows.length;
  const groupTabs = [{ key: null, label: copy.step.all, count: st.detail.summary.candidates }, ...opp.sub_groups.map((g) => ({ key: g.key, label: g.label, count: g.count }))];
  const head = audio ? [copy.step.colRecording, copy.step.colSource, copy.step.colLength] : [copy.step.colName, copy.step.colFolder];
  return h('div', { class: 'tablewrap' },
    opp.sub_groups.length > 0 && tabs({ items: groupTabs, active: st.subGroup, onSelect: (k) => actions.setSubGroup(k), label: 'Groups', fk: 'sub' }),
    h('table', { class: 'table' },
      h('thead', {}, h('tr', {},
        h('th', { class: 'cell cell--check', scope: 'col' }, checkbox({ checked: allOn, label: copy.step.selectAll, disabled: !selectable.length, fk: 'select-all', onChange: (on) => actions.selectAll(on) })),
        head.map((t) => h('th', { scope: 'col', class: 'cell' }, t)),
        h('th', { scope: 'col', class: 'cell cell--num' }, copy.step.colSize),
        h('th', { scope: 'col', class: 'cell cell--num' }, copy.step.colAfter),
        h('th', { scope: 'col', class: 'cell' }, copy.step.colNotes))),
      h('tbody', {}, rows.map((r) => tableRow(r, audio, actions)))),
    rows.length === 0 && h('p', { class: 'empty-note' }, st.loading ? copy.step.loading : copy.step.empty),
    total > PAGE_SIZE && h('nav', { class: 'pager', 'aria-label': 'Pages' },
      h('span', { class: 'pager__range', 'aria-live': 'polite' }, copy.step.range(first, last, total)),
      button({ label: copy.step.prev, size: 'sm', icon: 'chevronLeft', disabled: st.offset === 0, fk: 'page-prev', onClick: () => actions.setPage(st.offset - PAGE_SIZE) }),
      button({ label: copy.step.next, size: 'sm', disabled: last >= total, fk: 'page-next', onClick: () => actions.setPage(st.offset + PAGE_SIZE) })));
}
