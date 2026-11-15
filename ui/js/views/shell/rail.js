// @ts-check
/** Left navigation rail (84 px). */
import { h, icon } from '../../dom.js';
import { copy } from '../../copy.js';
import { findOpp } from '../../state.js';

const ITEMS = ['overview', 'clean', 'optimise', 'archive', 'history'];

/** The rail item to highlight for the current route. @param {any} s */
export function activeNav(s) {
  if (s.route.name === 'disk') return 'overview';
  if (s.route.name !== 'step') return s.route.name;
  const opp = findOpp(s, s.step?.id) ?? s.step?.detail?.opportunity;
  return opp?.category && ['clean', 'optimise', 'archive'].includes(opp.category) ? opp.category : 'overview';
}

/** @param {any} s @param {any} actions */
export function renderRail(s, actions) {
  const active = activeNav(s);
  const item = (name) => h('button', { type: 'button', class: ['rail__item', name === active && 'rail__item--active'], 'aria-current': name === active ? 'page' : undefined, 'data-fk': `nav-${name}`, 'data-nav': name, on: { click: () => actions.navigate(name) } },
    h('span', { class: 'rail__icon' }, icon(name, 20)), h('span', { class: 'rail__label' }, copy.nav[name]));
  return h('nav', { class: 'rail', 'aria-label': copy.navLabel },
    h('div', { class: 'rail__logo', 'aria-hidden': 'true' }, icon('logo', 16)),
    h('div', { class: 'rail__items' }, ITEMS.map(item)),
    h('div', { class: 'rail__bottom' }, item('settings')));
}
