// @ts-check
/** Breakdown by type or age (SCAN-07): bars with the numbers written out. */
import { h } from '../../dom.js';
import { copy } from '../../copy.js';
import { formatBytes, formatCount, formatPercent } from '../../format.js';
import { loadingBlock, errorBlock, emptyBlock, unavailableBlock } from '../../components/states.js';
import { isUnavailable } from '../../errors.js';

/** @param {any} s @param {any} actions @param {'type'|'age'} by */
export function renderBreakdown(s, actions, by) {
  const b = s.disk.breakdown[by];
  if (!b || b.loading) return loadingBlock();
  if (b.error) return isUnavailable(b.error) ? unavailableBlock(b.error) : errorBlock(b.error, { onRetry: () => actions.loadBreakdown(by), fk: `bd-retry-${by}` });
  const rows = b.data?.rows ?? [];
  if (!rows.length) return emptyBlock({ title: copy.disk.empty });
  const total = rows.reduce((a, r) => a + r.bytes, 0);
  const max = Math.max(1, ...rows.map((r) => r.bytes));
  return h('section', { class: 'bars', 'aria-label': copy.disk.breakdownTitle[by] },
    h('h3', { class: 'eyebrow' }, copy.disk.breakdownTitle[by]),
    h('ul', { class: 'bars__list' }, rows.map((r) => {
      const fill = h('span', { class: 'sbar__fill' });
      fill.style.setProperty('width', `${((r.bytes / max) * 100).toFixed(1)}%`);
      return h('li', { class: 'bars__row', 'data-key': r.key, 'aria-label': copy.disk.breakdownRow(r.label, formatBytes(r.bytes), formatCount(r.files), formatPercent(total ? r.bytes / total : 0)) },
        h('span', { class: 'bars__label', title: r.label }, r.label),
        h('span', { class: 'sbar sbar--wide', 'aria-hidden': 'true' }, fill),
        h('span', { class: 'bars__size' }, formatBytes(r.bytes)),
        h('span', { class: 'bars__meta' }, `${formatPercent(total ? r.bytes / total : 0)}  ·  ${copy.disk.files(formatCount(r.files))}`));
    })));
}
