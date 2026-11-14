// @ts-check
/**
 * Results table of a step, shared by the step review and the Custom search
 * results. One page of rows at a time (50), sortable columns, duplicate group
 * separators, a path filter, keyboard movement and paging. It reads a step
 * state (`st`) and a set of operations (`ops`), so any slot can drive it.
 */
import { h, icon, fk } from '../../dom.js';
import { copy } from '../../copy.js';
import { basename, dirname, formatBytes, displayPath, middleEllipsis, formatCount } from '../../format.js';
import { button, checkbox } from '../../components/controls.js';
import { badge, rowBadge } from '../../components/badge.js';
import { tabs } from '../../components/tabs.js';
import { loadingBlock, emptyBlock } from '../../components/states.js';
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
    row.badges.filter((b) => b !== 'duplicate').map(rowBadge),
    row.blocked_reason && h('span', { class: 'cell__blocked', title: copy.step2.blocked(row.blocked_reason) }, row.blocked_reason));
}

/** @param {any} row @param {boolean} audio */
function afterCell(row, audio) {
  if (!row.selected) return copy.step.unchecked;
  if (row.size_after === 0) return copy.step.removed;
  return `${audio ? '~' : ''}${formatBytes(row.size_after)}`;
}

/** @param {any} row @param {boolean} audio @param {any} ops */
function tableRow(row, audio, ops) {
  const name = basename(row.path);
  const folder = displayPath(dirname(row.path));
  return h('tr', { class: ['row', row.selected && 'row--checked', row.badges.includes('keeper') && 'row--keeper'], 'data-row': String(row.id) },
    h('td', { class: 'cell cell--check' }, checkbox({ checked: row.selected, label: copy.step.selectRow(name), disabled: !row.actionable, fk: `row-${fk(row.id)}`, onChange: (on) => ops.toggleRow(row.id, on) })),
    h('td', { class: 'cell cell--name', title: displayPath(row.path) }, h('button', { type: 'button', class: 'cell__file', 'data-fk': `name-${fk(row.id)}`, 'data-id': String(row.id), 'aria-label': `${copy.step.preview}: ${displayPath(name)}`, on: { click: () => ops.previewSample(row.id), focus: () => ops.focusRow?.(row.id) } }, icon('file', 14), h('span', { class: 'cell__text' }, displayPath(name)))),
    audio
      ? [h('td', { class: 'cell' }, row.columns?.source ?? ''), h('td', { class: 'cell cell--num' }, row.columns?.length ?? '')]
      : h('td', { class: 'cell cell--folder', title: folder }, middleEllipsis(folder, 40)),
    h('td', { class: 'cell cell--num' }, formatBytes(row.size)),
    h('td', { class: 'cell cell--num' }, afterCell(row, audio)),
    notesCell(row));
}

/** Separator before the first row of a duplicate group. @param {any} group @param {number} cols */
const groupHead = (group, cols) => h('tr', { class: 'group-head' }, h('td', { class: 'cell', colspan: String(cols) }, copy.step2.groupRow(displayPath(group.label), `${Math.round(group.confidence)} %`)));

/** Arrow keys move between rows, Page Up and Page Down change page. @param {any} e @param {any} st @param {any} ops @param {number} total */
function onTableKey(e, st, ops, total) {
  const buttons = e.currentTarget.querySelectorAll('.cell__file');
  const i = buttons.indexOf(document.activeElement);
  const go = (j) => { if (buttons[j]) { e.preventDefault(); buttons[j].focus(); } };
  if (e.key === 'ArrowDown' && i >= 0) go(i + 1);
  else if (e.key === 'ArrowUp' && i >= 0) go(i - 1);
  else if (e.key === 'Home' && i >= 0) go(0);
  else if (e.key === 'End' && i >= 0) go(buttons.length - 1);
  else if (e.key === 'PageDown' && st.offset + PAGE_SIZE < total) { e.preventDefault(); ops.setPage(st.offset + PAGE_SIZE); }
  else if (e.key === 'PageUp' && st.offset > 0) { e.preventDefault(); ops.setPage(st.offset - PAGE_SIZE); }
}

/** A sortable column header; the text stays the plain label (the state is in aria-sort). @param {string} label @param {string|null} key @param {any} st @param {any} ops @param {string} [cls] */
function th(label, key, st, ops, cls = '') {
  if (!key) return h('th', { scope: 'col', class: ['cell', cls] }, label);
  const on = st.sort === key;
  return h('th', { scope: 'col', class: ['cell', cls], 'aria-sort': on ? (st.descending ? 'descending' : 'ascending') : 'none' },
    h('button', { type: 'button', class: ['th-sort', on && 'th-sort--on'], 'aria-label': copy.step2.sortBy(label), 'data-fk': `sort-${key}`, 'data-dir': on ? (st.descending ? 'desc' : 'asc') : undefined, on: { click: () => ops.setStepSort(key) } }, label));
}

/**
 * @param {any} st the step state
 * @param {any} ops table operations (toggleRow, selectAll, setSubGroup, setPage, setStepSort, setFilter, previewSample, focusRow)
 * @param {{ exports?: (format: 'csv'|'html') => void, filterId?: string }} [o]
 */
export function renderTable(st, ops, o = {}) {
  const opp = st.detail.opportunity;
  const audio = opp.source?.optimiser === 'audio';
  const rows = st.page?.rows ?? [];
  const total = st.page?.total ?? 0;
  const selectable = rows.filter((r) => r.actionable);
  const allOn = selectable.length > 0 && selectable.every((r) => r.selected);
  const first = total ? st.offset + 1 : 0;
  const last = st.offset + rows.length;
  const pages = Math.max(1, Math.ceil(total / PAGE_SIZE));
  const groupTabs = [{ key: null, label: copy.step.all, count: st.detail.summary.candidates }, ...opp.sub_groups.map((g) => ({ key: g.key, label: g.label, count: g.count }))];
  const heads = audio ? [[copy.step.colRecording, 'name'], [copy.step.colSource, null], [copy.step.colLength, null]] : [[copy.step.colName, 'name'], [copy.step.colFolder, 'path']];
  const groups = new Map((st.page?.groups ?? []).map((g) => [g.id, g]));
  const body = [];
  let prev;
  for (const r of rows) {
    if (st.sort === 'group' && r.group !== undefined && r.group !== prev && groups.has(r.group)) body.push(groupHead(groups.get(r.group), heads.length + 4));
    prev = r.group;
    body.push(tableRow(r, audio, ops));
  }
  const empty = st.loading ? loadingBlock() : st.filter ? emptyBlock({ title: copy.step2.filteredEmpty }) : emptyBlock({ title: copy.step.empty });
  return h('div', { class: 'tablewrap' },
    h('div', { class: 'table-tools' },
      h('label', { class: 'filter' }, h('span', { class: 'filter__label' }, copy.search.filterLabel),
        h('input', { type: 'search', class: 'input', value: st.filter ?? '', placeholder: copy.search.filterPlaceholder, 'aria-describedby': `${o.filterId ?? 'filter'}-help`, 'data-fk': `${o.filterId ?? 'filter'}-input`, on: { input: (e) => ops.setFilter(e.target.value) } })),
      o.exports && h('span', { class: 'table-tools__export' },
        button({ label: copy.step2.exportCsv, size: 'sm', fk: 'export-results-csv', onClick: () => o.exports('csv') }),
        button({ label: copy.step2.exportHtml, size: 'sm', fk: 'export-results-html', onClick: () => o.exports('html') })),
      h('p', { class: 'note filter__help', id: `${o.filterId ?? 'filter'}-help` }, copy.search.filterHelp, ' ', copy.step2.keyboard)),
    opp.sub_groups.length > 0 && tabs({ items: groupTabs, active: st.subGroup, onSelect: (k) => ops.setSubGroup(k), label: 'Groups', fk: 'sub' }),
    h('table', { class: 'table', on: { keydown: (e) => onTableKey(e, st, ops, total) } },
      h('thead', {}, h('tr', {},
        h('th', { class: 'cell cell--check', scope: 'col' }, checkbox({ checked: allOn, label: copy.step.selectAll, disabled: !selectable.length, fk: 'select-all', onChange: (on) => ops.selectAll(on) })),
        heads.map(([label, key]) => th(label, key, st, ops)),
        th(copy.step.colSize, 'size', st, ops, 'cell--num'),
        th(copy.step.colAfter, 'saving', st, ops, 'cell--num'),
        th(copy.step.colNotes, null, st, ops))),
      h('tbody', {}, body)),
    rows.length === 0 && empty,
    total > PAGE_SIZE && h('nav', { class: 'pager', 'aria-label': 'Pages' },
      h('span', { class: 'pager__range', 'aria-live': 'polite' }, copy.step.range(formatCount(first), formatCount(last), formatCount(total)), '  ·  ', copy.step2.page(Math.floor(st.offset / PAGE_SIZE) + 1, pages)),
      button({ label: copy.step.prev, size: 'sm', icon: 'chevronLeft', disabled: st.offset === 0, fk: 'page-prev', onClick: () => ops.setPage(st.offset - PAGE_SIZE) }),
      button({ label: copy.step.next, size: 'sm', disabled: last >= total, fk: 'page-next', onClick: () => ops.setPage(st.offset + PAGE_SIZE) })));
}
