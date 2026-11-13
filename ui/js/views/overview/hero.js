// @ts-check
/** Savings summary card: total, stacked bar by safety, per-drive bars. */
import { h, icon } from '../../dom.js';
import { copy } from '../../copy.js';
import { formatBytes } from '../../format.js';
import { stackedBar, driveBar } from '../../components/bars.js';

/** @param {string} tone @param {string} label @param {string} value */
const legend = (tone, label, value) => h('li', { class: 'legend__item' }, h('span', { class: ['swatch', `swatch--${tone}`], 'aria-hidden': 'true' }), value ? `${label} ${value}` : label);

/** @param {any} d a Drive */
function driveRow(d) {
  const changed = d.free_after_plan > d.free;
  const name = `${d.id}  ${d.label}`;
  return h('div', { class: 'drive-row', 'data-drive': d.id },
    h('div', { class: 'drive-row__head' },
      h('strong', { class: 'drive-row__name' }, name),
      h('span', { class: 'drive-row__free' }, copy.hero.freeNow(formatBytes(d.free)), changed && [h('span', { class: 'drive-row__arrow', 'aria-hidden': 'true' }, icon('arrowRight', 12)), copy.hero.afterPlan(formatBytes(d.free_after_plan))])),
    driveBar({ total: d.total, free: d.free, freeAfter: d.free_after_plan, label: `${name}: ${copy.hero.freeNow(formatBytes(d.free))}${changed ? `, ${copy.hero.afterPlan(formatBytes(d.free_after_plan))}` : ''}` }));
}

/** @param {any} s */
export function renderHero(s) {
  const sum = s.opps.summary;
  return h('section', { class: 'hero', 'aria-label': copy.hero.freeUp },
    h('div', { class: 'hero__left' },
      h('p', { class: 'hero__eyebrow' }, copy.hero.freeUp),
      h('p', { class: 'hero__total' }, h('strong', { class: 'hero__number', 'aria-live': 'polite', 'data-role': 'hero-total' }, formatBytes(sum.total)), h('span', { class: 'hero__across' }, copy.hero.across(sum.opportunities))),
      stackedBar({ label: copy.hero.barLabel, segments: [{ key: 'reversible', value: sum.reversible, tone: 'reversible' }, { key: 'opt_in', value: sum.opt_in, tone: 'opt-in' }, { key: 'review_only', value: sum.review_only, tone: 'review' }] }),
      h('ul', { class: 'legend' }, legend('reversible', copy.hero.reversible, formatBytes(sum.reversible)), legend('opt-in', copy.hero.optIn, formatBytes(sum.opt_in)), legend('review', copy.hero.reviewOnly, formatBytes(sum.review_only)))),
    h('div', { class: 'hero__right', role: 'group', 'aria-label': copy.hero.drivesLabel },
      s.drives.map(driveRow),
      h('ul', { class: 'legend legend--drives' }, legend('used', copy.hero.used, ''), legend('freed', copy.hero.freed, ''), legend('track', copy.hero.free, ''))));
}
