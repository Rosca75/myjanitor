// @ts-check
/** Folder tree sorted by size (SCAN-06) with allocated versus actual size (SCAN-08). */
import { h, icon, fk } from '../../dom.js';
import { copy } from '../../copy.js';
import { formatBytes, formatCount, formatPercent, compressionRate, displayPath } from '../../format.js';
import { pathText } from '../../components/path.js';
import { flatten } from '../../lib/disktree.js';

/** Share bar: width set through the CSSOM. @param {number} share 0..1 */
function shareBar(share) {
  const fill = h('span', { class: 'sbar__fill' });
  fill.style.setProperty('width', `${(Math.max(0, Math.min(1, share)) * 100).toFixed(1)}%`);
  return h('span', { class: 'sbar', 'aria-hidden': 'true' }, fill);
}

/** @param {any} r a flattened row @param {any} s @param {any} actions */
function row(r, s, actions) {
  const n = r.node;
  const rate = compressionRate(n.size, n.allocated);
  const label = r.other ? copy.disk.smaller(n.files) : displayPath(n.name || n.path);
  const toggle = r.expandable
    ? h('button', { type: 'button', class: 'tree__toggle', 'aria-label': r.open ? copy.disk.collapse(label) : copy.disk.expand(label), 'data-fk': `dt-${fk(n.path)}`, on: { click: () => actions.toggleDiskNode(n.path) } }, icon(r.open ? 'chevronDown' : 'chevronRight', 12))
    : h('span', { class: 'tree__toggle tree__toggle--none' });
  const indent = h('span', { class: 'tree__indent' });
  indent.style.setProperty('width', `${r.depth * 18}px`);
  return h('tr', { class: ['row', 'trow', r.other && 'trow--other'], role: 'row', 'aria-level': String(r.depth + 1), 'aria-expanded': r.expandable ? String(r.open) : undefined, 'data-path': n.path },
    h('td', { class: 'cell' }, h('div', { class: 'cell--tree' }, indent, toggle, r.other ? h('span', { class: 'tree__name tree__name--other' }, label)
      : h('button', { type: 'button', class: 'tree__name', title: displayPath(n.path), 'data-fk': `dn-${fk(n.path)}`, on: { click: () => actions.focusDisk(n.path) } }, icon(n.is_dir ? 'folder' : 'file', 14), pathText(label, { max: 44, class: 'tree__text' })),
      s.disk.loadingPaths[n.path] && h('span', { class: 'note' }, ` ${copy.disk.loadingMore}`))),
    h('td', { class: 'cell cell--share' }, shareBar(r.share), h('span', { class: 'share__text' }, formatPercent(r.share))),
    h('td', { class: 'cell cell--num' }, formatBytes(n.size)),
    h('td', { class: 'cell cell--num' }, formatBytes(n.allocated)),
    h('td', { class: 'cell cell--num' }, rate === null ? copy.disk.noneCompressed : copy.disk.saved(formatPercent(rate))),
    h('td', { class: 'cell cell--num' }, formatCount(n.files)));
}

/** @param {any} s @param {any} actions */
export function renderTree(s, actions) {
  const d = s.disk;
  const rows = flatten(d.root, d.expanded, d.sort);
  const sortBtn = (key, label, text) => h('button', { type: 'button', class: ['th-sort', d.sort === key && 'th-sort--on'], 'aria-label': text, 'data-fk': `dsort-${key}`, on: { click: () => actions.setDiskSort(key) } }, label);
  return h('table', { class: 'table table--tree', role: 'treegrid', 'aria-label': copy.disk.treeLabel },
    h('thead', {}, h('tr', {},
      h('th', { scope: 'col', class: 'cell', 'aria-sort': d.sort === 'name' ? 'ascending' : 'none' }, sortBtn('name', copy.disk.colName, copy.disk.sortName)),
      h('th', { scope: 'col', class: 'cell' }, copy.disk.colShare),
      h('th', { scope: 'col', class: 'cell cell--num', 'aria-sort': d.sort === 'size' ? 'descending' : 'none' }, sortBtn('size', copy.disk.colSize, copy.disk.sortSize)),
      h('th', { scope: 'col', class: 'cell cell--num' }, copy.disk.colAllocated),
      h('th', { scope: 'col', class: 'cell cell--num' }, copy.disk.colCompression),
      h('th', { scope: 'col', class: 'cell cell--num' }, copy.disk.colFiles))),
    h('tbody', {}, rows.map((r) => row(r, s, actions))));
}
