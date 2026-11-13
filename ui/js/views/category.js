// @ts-check
/** Shared list for the Clean, Optimise and Archive views: that category's cards. */
import { h } from '../dom.js';
import { copy } from '../copy.js';
import { oppCard } from '../components/card.js';

/**
 * @param {any} s @param {any} actions @param {'clean'|'optimise'|'archive'} name
 * @param {any[]} [extra] extra content above the cards (Clean: Custom search)
 */
export function renderCategory(s, actions, name, extra = []) {
  const items = s.category.name === name ? s.category.items : [];
  const handlers = { onReview: (id) => actions.openStep(id), onAdd: (id) => actions.addToPlan(id) };
  return h('div', { class: ['view', `view--${name}`] },
    h('section', { class: 'intro' }, h('h2', { class: 'intro__title' }, copy.category[name].title), h('p', { class: 'intro__text' }, copy.category[name].intro)),
    extra,
    items.length ? h('div', { class: 'grid', 'aria-label': copy.category[name].title }, items.map((o) => oppCard(o, handlers))) : h('p', { class: 'empty-note' }, copy.category.empty));
}
