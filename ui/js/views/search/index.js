// @ts-check
/** Custom search panel of the Clean view: query or builder, options, presets, results. */
import { h } from '../../dom.js';
import { copy } from '../../copy.js';
import { button, toggle } from '../../components/controls.js';
import { tabs } from '../../components/tabs.js';
import { renderQueryBox } from './query-box.js';
import { renderBuilder } from './builder.js';
import { renderPresets } from './presets.js';
import { renderResults } from './results.js';
import { select } from '../../components/controls.js';


/** @param {any} s @param {any} actions */
function options(s, actions) {
  const c = s.custom;
  return h('div', { class: 'coptions' },
    h('label', { class: 'field' }, h('span', { class: 'field__label' }, copy.search.titleLabel),
      h('input', { type: 'text', class: 'input', value: c.title, placeholder: copy.search.titlePlaceholder, 'aria-label': copy.search.titleLabel, 'data-fk': 'custom-title', on: { input: (e) => actions.setSearchOption({ title: e.target.value }) } })),
    h('label', { class: 'field' }, h('span', { class: 'field__label' }, copy.search.actionLabel),
      select({ label: copy.search.actionLabel, value: c.action, options: Object.entries(copy.search.actions).filter(([k]) => k !== 'permanent_delete' || c.advancedOpen || c.action === 'permanent_delete').map(([value, label]) => ({ value, label })), fk: 'custom-action-top', onChange: (v) => actions.setSearchOption({ action: v }) })),
    h('label', { class: 'field field--row' }, toggle({ checked: c.topmost, label: copy.search.topmost, fk: 'custom-topmost', onChange: (on) => actions.setSearchOption({ topmost: on }) }),
      h('span', { class: 'field__text' }, h('span', { class: 'field__label' }, copy.search.topmost), h('span', { class: 'note' }, copy.search.topmostHelp))),
    h('button', { type: 'button', class: 'disclosure', 'aria-expanded': String(c.advancedOpen), 'data-fk': 'custom-advanced', on: { click: () => actions.toggleSearchAdvanced() } }, c.advancedOpen ? copy.search.advancedHide : copy.search.advanced));
}

/** @param {any} s @param {any} actions */
export function renderSearchPanel(s, actions) {
  const c = s.custom;
  return h('section', { class: 'custom', 'aria-labelledby': 'custom-title' },
    h('h3', { class: 'custom__title', id: 'custom-title' }, copy.category.custom),
    h('p', { class: 'custom__intro' }, copy.category.customIntro),
    tabs({ items: [{ key: 'query', label: copy.search.modeQuery }, { key: 'builder', label: copy.search.modeBuilder }], active: c.mode, onSelect: (k) => actions.setSearchMode(k), label: copy.search.modeLabel, fk: 'smode' }),
    c.mode === 'query' ? renderQueryBox(s, actions)
      : h('div', { class: 'builderwrap' }, renderBuilder(s, actions),
        h('div', { class: 'custom__form' }, button({ label: c.running ? copy.search.searching : copy.search.search, variant: 'secondary', disabled: c.running, fk: 'custom-run', onClick: () => actions.runSearch() }))),
    options(s, actions),
    renderPresets(s, actions),
    renderResults(s, actions));
}
