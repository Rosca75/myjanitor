// @ts-check
/** Saved searches: built-in and user presets, save the current search, delete. */
import { h } from '../../dom.js';
import { copy } from '../../copy.js';
import { button } from '../../components/controls.js';
import { badge } from '../../components/badge.js';
import { disabledHint, loadingBlock } from '../../components/states.js';

/** @param {any} p @param {any} s @param {any} actions */
function row(p, s, actions) {
  const off = s.unavail.presets;
  return h('li', { class: 'preset', 'data-preset': p.id },
    h('div', { class: 'preset__text' }, h('strong', { class: 'preset__name', title: p.name }, p.name), p.description && h('span', { class: 'preset__desc' }, p.description)),
    badge(p.builtin ? copy.search.builtin : copy.search.yours, p.builtin ? 'neutral' : 'select'),
    button({ label: copy.search.usePreset, size: 'sm', variant: 'secondary', ariaLabel: copy.search.usePresetLabel(p.name), fk: `use-${p.id}`, onClick: () => actions.applyPreset(p.id) }),
    !p.builtin && button({ label: copy.search.deletePreset, size: 'sm', variant: 'quiet', ariaLabel: copy.search.deletePresetLabel(p.name), disabled: !!off, title: disabledHint(off), fk: `del-${p.id}`, onClick: () => actions.deletePreset(p.id) }));
}

/** @param {any} s @param {any} actions */
export function renderPresets(s, actions) {
  const c = s.custom;
  const off = s.unavail.presets;
  return h('section', { class: 'presets', 'aria-labelledby': 'presets-title' },
    h('h4', { class: 'eyebrow', id: 'presets-title' }, copy.search.presets),
    c.presetsLoading && !c.presets.length ? loadingBlock()
      : c.presets.length ? h('ul', { class: 'preset-list' }, c.presets.map((p) => row(p, s, actions))) : h('p', { class: 'note' }, copy.search.presetsEmpty),
    h('div', { class: 'preset-save' },
      h('input', { type: 'text', class: 'input', value: c.presetName, placeholder: copy.search.presetName, 'aria-label': copy.search.presetName, 'data-fk': 'preset-name', on: { input: (e) => actions.setSearchOption({ presetName: e.target.value }) } }),
      h('input', { type: 'text', class: 'input', value: c.presetDesc, placeholder: copy.search.presetDesc, 'aria-label': copy.search.presetDesc, 'data-fk': 'preset-desc', on: { input: (e) => actions.setSearchOption({ presetDesc: e.target.value }) } }),
      button({ label: copy.search.savePreset, size: 'sm', disabled: !!off || !c.presetName.trim(), title: disabledHint(off), fk: 'preset-save', onClick: () => actions.savePreset() })),
    off && h('p', { class: 'note' }, copy.search.presetsOff([off.message, off.hint].filter(Boolean).join(' '))));
}
