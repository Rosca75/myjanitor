// @ts-check
/** B2 step review: summary, sub-group tabs, table (center) and the settings panel (right). */
import { h } from '../../dom.js';
import { copy } from '../../copy.js';
import { formatBytes, formatAbout, formatCount } from '../../format.js';
import { renderTable } from './table.js';
import { renderShrinkPanel } from './shrink-panel.js';
import { renderCleanPanel } from './clean-panel.js';

/** Noun for the summary line. @param {any} opp */
export const nounFor = (opp) => (opp.id === 'opt:audio' ? 'recordings' : copy.step.noun[opp.unit] ?? opp.unit);

/** @param {any} s @param {any} actions */
export function renderStep(s, actions) {
  const st = s.step;
  if (!st || (!st.detail && !st.loading)) return h('div', { class: 'view' }, h('p', { class: 'empty-note' }, copy.step.notFound));
  if (!st.detail) return h('div', { class: 'view' }, h('p', { class: 'empty-note' }, copy.step.loading));
  const opp = st.detail.opportunity;
  const sum = st.detail.summary;
  const changes = opp.category === 'optimise' || opp.category === 'archive';
  const line = changes
    ? copy.step.summary(formatCount(sum.selected), nounFor(opp), formatBytes(sum.size_before), `about ${formatAbout(sum.size_after)}`)
    : copy.step.summaryRemove(formatCount(sum.selected), nounFor(opp), formatBytes(sum.size_before));
  return h('div', { class: 'view view--step' },
    h('div', { class: 'step-head' },
      h('p', { class: 'step-head__summary', 'aria-live': 'polite', 'data-role': 'step-summary' }, line),
      h('p', { class: 'step-head__sub' }, st.detail.diagnostics.notes.join(' ') || opp.description)),
    renderTable(s, actions));
}

/** Right panel for the step route. @param {any} s @param {any} actions */
export function renderStepPanel(s, actions) {
  if (!s.step?.detail) return h('div', {});
  const opp = s.step.detail.opportunity;
  return opp.category === 'optimise' && (opp.presets.length || opp.settings.length) ? renderShrinkPanel(s, actions) : renderCleanPanel(s, actions);
}
