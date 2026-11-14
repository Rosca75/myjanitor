// @ts-check
/** Custom search results: the shared step table, with "Add to plan" and the action choice. */
import { h } from '../../dom.js';
import { copy } from '../../copy.js';
import { formatBytes, formatCount } from '../../format.js';
import { button, select } from '../../components/controls.js';
import { loadingBlock, emptyBlock } from '../../components/states.js';
import { renderTable } from '../step/table.js';
import { stepFor } from '../../state.js';

/** The table operations of the results slot: the `custom_` copies of the step actions. @param {any} actions */
export const customOps = (actions) => ({
  toggleRow: actions.custom_toggleRow, selectAll: actions.custom_selectAll, setSubGroup: actions.custom_setSubGroup, setPage: actions.custom_setPage,
  setStepSort: actions.custom_setStepSort, setFilter: actions.custom_setFilter, previewSample: actions.custom_previewSample, focusRow: actions.custom_focusRow,
});

/** @param {any} s @param {any} actions */
export function renderResults(s, actions) {
  const c = s.custom;
  const r = c.result;
  if (!r) return null;
  const inPlan = !!stepFor(s, r.id);
  const st = c.step;
  const sum = st?.detail?.summary;
  return h('section', { class: 'cresults', 'aria-label': copy.search.resultsTitle },
    h('p', { class: 'custom__result', role: 'status' }, copy.category.customResult(r.title, r.count), ' ',
      button({ label: copy.search.review, variant: 'link', size: 'sm', fk: 'custom-review', ariaLabel: copy.card.reviewLabel(r.title), onClick: () => actions.openStep(r.id) }),
      button({ label: copy.search.clear, variant: 'quiet', size: 'sm', fk: 'custom-clear', onClick: () => actions.clearSearchResult() })),
    r.count === 0 ? emptyBlock({ title: copy.search.noMatches })
      : !st?.detail ? loadingBlock()
        : [h('p', { class: 'note', 'data-role': 'custom-summary' }, `${formatCount(sum.selected)} / ${formatCount(sum.candidates)}  ·  ${formatBytes(sum.size_before)}`),
          renderTable(st, customOps(actions), { filterId: 'cfilter', exports: (format) => actions.startExport({ type: 'results', format, opportunity_id: r.id, selected_only: false }) }),
          h('div', { class: 'cresults__foot' },
            select({ label: copy.search.actionLabel, value: c.action, options: Object.entries(copy.search.actions).map(([value, label]) => ({ value, label })), fk: 'custom-action', onChange: (v) => actions.setSearchOption({ action: v }) }),
            button({ label: inPlan ? copy.search.inPlan : copy.search.addToPlan, variant: 'primary', disabled: inPlan, fk: 'custom-add', onClick: () => actions.addSearchToPlan() }))]);
}
