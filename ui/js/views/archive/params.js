// @ts-check
/** Archive view: parameters of the cold-folder archive. The integrity test is always on. */
import { h } from '../../dom.js';
import { copy } from '../../copy.js';
import { numberInput, select, toggle } from '../../components/controls.js';

/** @param {any} opp the archive opportunity @param {any} s @param {any} actions */
export function renderArchiveParams(opp, s, actions) {
  const months = opp.params.idle_months ?? opp.settings.find((x) => x.key === 'idle_months')?.kind.default ?? 18;
  const step = s.plan.steps.find((x) => x.opportunity === opp.id);
  const action = step?.action ?? opp.default_action;
  const format = action.format ?? 'zip';
  return h('section', { class: 'aparams', 'aria-labelledby': 'aparams-title' },
    h('h3', { class: 'custom__title', id: 'aparams-title' }, copy.category2.archiveTitle),
    h('p', { class: 'custom__intro' }, copy.category2.archiveIntro),
    h('div', { class: 'aparams__row' },
      h('label', { class: 'field field--row' }, h('span', { class: 'field__label' }, copy.category2.months), numberInput({ value: Number(months), min: 1, max: 120, step: 1, unit: copy.category2.monthsUnit, label: copy.category2.months, fk: 'archive-months', onChange: (n) => actions.setOppParam(opp.id, 'idle_months', n) })),
      h('label', { class: 'field field--row' }, h('span', { class: 'field__label' }, copy.category2.format), select({ label: copy.category2.format, value: format, options: Object.entries(copy.category2.formats).map(([value, label]) => ({ value, label })), fk: 'archive-format', onChange: (v) => actions.setArchiveFormat(v) })),
      h('label', { class: 'field field--row' }, toggle({ checked: true, disabled: true, label: copy.category2.verify, title: copy.category2.verifyLocked, fk: 'archive-verify', onChange: () => {} }), h('span', { class: 'field__text' }, h('span', { class: 'field__label' }, copy.category2.verify), h('span', { class: 'note', id: 'verify-note' }, copy.category2.verifyLocked)))));
}
