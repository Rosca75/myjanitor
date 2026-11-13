// @ts-check
/** B1 with results: hero, tabs, sort, card grid, smaller-opportunities expander. */
import { h } from '../../dom.js';
import { copy } from '../../copy.js';
import { formatBytes } from '../../format.js';
import { button } from '../../components/controls.js';
import { select } from '../../components/controls.js';
import { tabs } from '../../components/tabs.js';
import { oppCard } from '../../components/card.js';
import { splitCards } from '../../state.js';
import { renderHero } from './hero.js';

/** Banner after an apply finished (state 5). @param {any} s @param {any} actions */
function completedBanner(s, actions) {
  const c = s.completed;
  return h('section', { class: 'notice notice--success', role: 'status', 'data-role': 'completed' },
    h('div', { class: 'notice__text' },
      h('strong', {}, copy.done.title), h('span', {}, ` ${copy.done.summary(formatBytes(c.saved), c.files)}`),
      c.failed > 0 && h('span', {}, ` ${copy.done.errors(c.failed)}`),
      h('span', { class: 'notice__sub' }, ` ${copy.done.undoHint}`)),
    h('div', { class: 'notice__actions' },
      button({ label: copy.done.undo, variant: 'secondary', fk: 'done-undo', onClick: () => actions.undoLast() }),
      button({ label: copy.done.report, variant: 'link', fk: 'done-report', onClick: () => actions.viewReport(c.run_id) }),
      button({ label: copy.done.dismiss, variant: 'quiet', fk: 'done-dismiss', onClick: () => actions.dismissCompleted() })));
}

/** Tab items with counts from the list response. @param {any} counts */
const tabItems = (counts) => ['all', 'clean', 'optimise', 'review_only'].map((k) => ({ key: k, label: copy.tabs[k], count: counts[k] }));

/** @param {any} s @param {any} actions */
export function renderResults(s, actions) {
  const { main, small } = splitCards(s.opps.items);
  const handlers = { onReview: (id) => actions.openStep(id), onAdd: (id) => actions.addToPlan(id) };
  const smallSize = small.reduce((a, o) => a + o.size_before, 0);
  const grid = (list) => h('div', { class: 'grid', 'aria-label': copy.tabs.label }, list.map((o) => oppCard(o, handlers)));
  return h('div', { class: 'view view--overview' },
    s.completed && completedBanner(s, actions),
    renderHero(s),
    h('div', { class: 'toolbar' },
      tabs({ items: tabItems(s.opps.counts), active: s.tab, onSelect: (k) => actions.setTab(k), label: copy.tabs.label, fk: 'tab' }),
      h('label', { class: 'sort' }, h('span', { class: 'sort__label' }, copy.sort.label),
        select({ label: copy.sort.label, value: s.sort, fk: 'sort', options: ['saving', 'count', 'safety', 'name'].map((k) => ({ value: k, label: copy.sort[k] })), onChange: (v) => actions.setSort(v) }))),
    s.opps.items.length ? grid(main) : h('p', { class: 'empty-note' }, copy.card.none),
    small.length > 0 && button({ label: s.showSmall ? copy.card.hideSmaller : copy.card.showSmaller(small.length, formatBytes(smallSize)), variant: 'link', fk: 'show-small', pressed: undefined, onClick: () => actions.toggleSmall(), extra: 'expander' }),
    s.showSmall && grid(small));
}
