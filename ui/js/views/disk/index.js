// @ts-check
/** Disk usage view (SCAN-05/06/07/08): tree, treemap and breakdown tabs. */
import { h } from '../../dom.js';
import { copy } from '../../copy.js';
import { formatBytes, formatCount } from '../../format.js';
import { button } from '../../components/controls.js';
import { tabs } from '../../components/tabs.js';
import { loadingBlock, errorBlock, emptyBlock, unavailableBlock } from '../../components/states.js';
import { isUnavailable } from '../../errors.js';
import { renderTree } from './tree.js';
import { renderTreemap } from './treemap.js';
import { renderBreakdown } from './breakdown.js';

/** @param {any} s @param {any} actions */
export function renderDisk(s, actions) {
  const d = s.disk;
  let body;
  if (d.loading && !d.root) body = loadingBlock();
  else if (d.error) {
    body = isUnavailable(d.error) ? unavailableBlock(d.error)
      : d.error.code === 'invalid_input' ? emptyBlock({ title: d.error.message, body: copy.disk.needScan, action: button({ label: copy.states.goOverview, size: 'sm', fk: 'disk-overview', onClick: () => actions.navigate('overview') }) })
        : errorBlock(d.error, { onRetry: () => actions.loadDisk(), fk: 'disk-retry' });
  } else if (!d.root || (!d.root.children.length && !d.root.other)) body = emptyBlock({ title: copy.disk.empty });
  else if (d.tab === 'folders') body = renderTree(s, actions);
  else if (d.tab === 'treemap') body = renderTreemap(s, actions);
  else body = renderBreakdown(s, actions, d.tab);
  return h('div', { class: 'view view--disk' },
    h('section', { class: 'intro' }, h('h2', { class: 'intro__title' }, copy.disk.title), h('p', { class: 'intro__text' }, copy.disk.intro),
      d.root && h('p', { class: 'note', 'data-role': 'disk-total' }, copy.disk.total(formatBytes(d.root.size), formatCount(d.root.files)))),
    tabs({ items: Object.entries(copy.disk.tabs).map(([key, label]) => ({ key, label })), active: d.tab, onSelect: (k) => actions.setDiskTab(k), label: copy.disk.tabsLabel, fk: 'dtab' }),
    h('div', { class: 'disk__body' }, body));
}
