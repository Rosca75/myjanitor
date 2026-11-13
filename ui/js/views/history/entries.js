// @ts-check
/** The journal entries of one expanded run, with per-entry Restore. */
import { h, fk } from '../../dom.js';
import { copy } from '../../copy.js';
import { basename, formatBytes, formatDateTime, formatCount, displayPath } from '../../format.js';
import { button, checkbox } from '../../components/controls.js';
import { badge } from '../../components/badge.js';
import { pathText } from '../../components/path.js';
import { loadingBlock, emptyBlock, errorBlock } from '../../components/states.js';
import { disabledHint } from '../../components/states.js';
import { canRestore } from '../../lib/history.js';

const TONE = { done: 'success', undone: 'success', failed: 'danger', undo_failed: 'danger', skipped: 'neutral', purged: 'neutral', intent: 'warning' };

/** @param {any} e @param {any} s @param {any} actions */
function entryRow(e, s, actions) {
  const ok = canRestore(e);
  const name = displayPath(basename(e.original));
  return h('tr', { class: 'row entry', 'data-entry': String(e.id) },
    h('td', { class: 'cell cell--check' }, checkbox({ checked: !!s.historyUi.selected[e.id], label: copy.historyView.selectEntry(name), disabled: !ok, fk: `entry-${fk(e.id)}`, onChange: (on) => actions.selectEntry(e.id, on) })),
    h('td', { class: 'cell cell--name' },
      pathText(e.original, { max: 54 }),
      e.retained_at && h('div', { class: 'entry__sub' }, `${copy.historyView.retained} `, pathText(e.retained_at, { max: 48 })),
      e.error && h('div', { class: 'entry__error', role: 'note', 'data-role': 'entry-error' }, [e.error.message, e.error.hint].filter(Boolean).join(' '))),
    h('td', { class: 'cell' }, e.action),
    h('td', { class: 'cell' }, badge(copy.historyView.entryStates[e.state] ?? e.state, TONE[e.state] ?? 'neutral')),
    h('td', { class: 'cell cell--num' }, e.bytes_after === 0 ? formatBytes(e.bytes_before) : `${formatBytes(e.bytes_before)} → ${formatBytes(e.bytes_after)}`),
    h('td', { class: 'cell cell--when' }, formatDateTime(e.at)),
    h('td', { class: 'cell' }, button({ label: copy.historyView.restoreEntry, size: 'sm', variant: 'secondary', disabled: !ok || !!s.unavail.restore_entries, title: s.unavail.restore_entries ? disabledHint(s.unavail.restore_entries) : undefined, ariaLabel: copy.historyView.restoreEntryLabel(name), fk: `restore-entry-${fk(e.id)}`, onClick: () => actions.restoreEntries([e.id]) })));
}

/** @param {any} run @param {any} s @param {any} actions */
export function renderEntries(run, s, actions) {
  const d = s.historyUi.details[run.run_id];
  if (!d || (d.loading && !d.entries)) return h('div', { class: 'run__body' }, loadingBlock());
  if (d.error) return h('div', { class: 'run__body' }, errorBlock(d.error, { onRetry: () => actions.setRunPage(run.run_id, d.offset ?? 0), fk: `retry-${run.run_id}` }));
  const entries = d.entries ?? [];
  if (!entries.length) return h('div', { class: 'run__body' }, emptyBlock({ title: copy.historyView.entriesEmpty }));
  const restorable = entries.filter((e) => canRestore(e));
  const picked = entries.filter((e) => s.historyUi.selected[e.id] && canRestore(e)).map((e) => e.id);
  const allOn = restorable.length > 0 && restorable.every((e) => s.historyUi.selected[e.id]);
  const first = d.offset + 1;
  const last = d.offset + entries.length;
  return h('div', { class: 'run__body' },
    h('table', { class: 'table table--entries', 'aria-label': run.title },
      h('thead', {}, h('tr', {},
        h('th', { class: 'cell cell--check', scope: 'col' }, checkbox({ checked: allOn, label: copy.historyView.selectAllEntries, disabled: !restorable.length, fk: `entries-all-${run.run_id}`, onChange: (on) => actions.selectAllEntries(run.run_id, on, canRestore) })),
        [copy.historyView.colItem, copy.historyView.colAction, copy.historyView.colState].map((t) => h('th', { scope: 'col', class: 'cell' }, t)),
        h('th', { scope: 'col', class: 'cell cell--num' }, copy.historyView.colSizes),
        h('th', { scope: 'col', class: 'cell' }, copy.historyView.colWhen),
        h('th', { scope: 'col', class: 'cell' }, ''))),
      h('tbody', {}, entries.map((e) => entryRow(e, s, actions)))),
    h('div', { class: 'run__foot' },
      button({ label: copy.historyView.restoreSelected(picked.length), variant: 'secondary', size: 'sm', disabled: !picked.length || !!s.unavail.restore_entries, fk: `restore-selected-${run.run_id}`, onClick: () => actions.restoreEntries(picked) }),
      d.total > 100 && h('span', { class: 'pager' },
        h('span', { class: 'pager__range' }, copy.historyView.entriesRange(formatCount(first), formatCount(last), formatCount(d.total))),
        button({ label: copy.step.prev, size: 'sm', disabled: d.offset === 0, fk: `entries-prev-${run.run_id}`, onClick: () => actions.setRunPage(run.run_id, d.offset - 100) }),
        button({ label: copy.step.next, size: 'sm', disabled: last >= d.total, fk: `entries-next-${run.run_id}`, onClick: () => actions.setRunPage(run.run_id, d.offset + 100) }))));
}
