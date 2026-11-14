// @ts-check
/** Opportunity card (B1 grid). Names and descriptions are set as text only. */
import { h, icon } from '../dom.js';
import { copy } from '../copy.js';
import { formatBytes, formatCount } from '../format.js';
import { button, } from './controls.js';
import { safetyBadge, inPlanBadge, badge } from './badge.js';

const CATEGORY_ICON = { clean: 'clean', optimise: 'optimise', archive: 'archive' };

/**
 * @param {any} opp an Opportunity
 * @param {{ onReview: (id: string) => void, onAdd: (id: string) => void }} handlers
 */
export function oppCard(opp, handlers) {
  const note = handlers.platformNote?.(opp) ?? null;
  const disabled = opp.availability?.state === 'disabled';
  const reviewOnly = opp.safety === 'review_only';
  const inPlan = !!opp.in_plan;
  const unit = copy.card.units[opp.unit] ?? opp.unit;
  const reason = disabled ? opp.availability.reason || note || copy.card.disabled : '';
  return h('article', { class: ['card', inPlan && 'card--in-plan', disabled && 'card--disabled'], 'data-opp': opp.id, 'aria-label': opp.title, 'aria-disabled': disabled ? 'true' : undefined },
    h('div', { class: 'card__head' },
      h('span', { class: 'card__icon', 'aria-hidden': 'true' }, icon(CATEGORY_ICON[opp.category] ?? 'clean', 18)),
      h('div', { class: 'card__titles' },
        h('h3', { class: 'card__title', title: opp.title }, opp.title),
        h('p', { class: 'card__desc', title: opp.description }, disabled ? opp.description : opp.description)),
      inPlan && inPlanBadge()),
    h('div', { class: 'card__figures' },
      h('span', { class: 'card__size', title: copy.card.saves(formatBytes(opp.saving)) }, formatBytes(opp.size_before)),
      h('span', { class: 'card__count' }, `${formatCount(opp.count)} ${opp.count === 1 ? unit.replace(/s$/, '') : unit}`)),
    disabled && h('p', { class: 'card__reason', 'data-role': 'card-reason' }, icon('info', 12), h('span', { class: 'card__reason-text' }, reason), note && note !== reason && badge(note, 'neutral', { extra: 'badge--row' })),
    h('div', { class: 'card__foot' },
      safetyBadge(opp.safety),
      h('div', { class: 'card__actions' },
        button({ label: copy.card.review, variant: inPlan ? 'secondary' : 'link', size: 'sm', disabled, title: disabled ? reason : undefined, ariaLabel: copy.card.reviewLabel(opp.title), fk: `review-${opp.id}`, onClick: () => handlers.onReview(opp.id) }),
        !inPlan && !reviewOnly && button({ label: copy.card.add, variant: 'secondary', size: 'sm', disabled: disabled, title: disabled ? reason : undefined, ariaLabel: copy.card.addLabel(opp.title), fk: `add-${opp.id}`, onClick: () => handlers.onAdd(opp.id) }))));
}

/** A card that fills in while the scan runs (DetectorTotalsEvent). @param {any} ev */
export function liveCard(ev) {
  return h('article', { class: 'card card--live', 'data-opp': ev.opportunity_id, 'aria-label': ev.title },
    h('div', { class: 'card__head' },
      h('span', { class: 'card__icon', 'aria-hidden': 'true' }, icon('overview', 18)),
      h('div', { class: 'card__titles' }, h('h3', { class: 'card__title', title: ev.title }, ev.title))),
    h('div', { class: 'card__figures' },
      h('span', { class: 'card__size' }, formatBytes(ev.saving)),
      h('span', { class: 'card__count' }, copy.scanning.items(formatCount(ev.count)))));
}

/** Empty slot while the scan has not found this opportunity yet. */
export function placeholderCard() {
  return h('div', { class: 'card card--placeholder', 'aria-hidden': 'true' },
    h('span', { class: 'sk sk--icon' }), h('span', { class: 'sk sk--line' }), h('span', { class: 'sk sk--big' }));
}
