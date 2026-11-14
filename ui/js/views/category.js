// @ts-check
/** Shared list for the Clean, Optimise and Archive views: that category's cards. */
import { h } from '../dom.js';
import { copy } from '../copy.js';
import { oppCard } from '../components/card.js';
import { loadingBlock, errorBlock, emptyBlock } from '../components/states.js';
import { missingRequirement } from '../lib/platform.js';

/** Short label for what this PC lacks, from app_info, drives and tools. @param {any} s */
export const platformNote = (s) => (opp) => missingRequirement(opp.id, s.info, s.drives, s.tools, copy.platform);

/**
 * @param {any} s @param {any} actions @param {'clean'|'optimise'|'archive'} name
 * @param {any[]} [extra] extra content above the cards (Clean: Custom search, Archive: parameters)
 */
export function renderCategory(s, actions, name, extra = []) {
  const cat = s.category;
  const items = cat.name === name ? cat.items : [];
  const handlers = { onReview: (id) => actions.openStep(id), onAdd: (id) => actions.addToPlan(id), platformNote: platformNote(s) };
  const body = cat.name === name && cat.error ? errorBlock(cat.error, { onRetry: () => actions.loadCategory(name), fk: 'category-retry' })
    : cat.loading && !items.length ? loadingBlock()
      : items.length ? h('div', { class: 'grid', 'aria-label': copy.category[name].title }, items.map((o) => oppCard(o, handlers)))
        : emptyBlock({ title: copy.category2.emptyTitle, body: copy.category2.emptyBody });
  return h('div', { class: ['view', `view--${name}`] },
    h('section', { class: 'intro' }, h('h2', { class: 'intro__title' }, copy.category[name].title), h('p', { class: 'intro__text' }, copy.category[name].intro),
      items.length > 0 && h('p', { class: 'note', 'data-role': 'category-count' }, copy.category2.count(items.length))),
    extra,
    body);
}
