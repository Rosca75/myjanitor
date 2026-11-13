// @ts-check
/** Badges: safety classes on cards, row flags in the table. */
import { h } from '../dom.js';
import { copy } from '../copy.js';

const SAFETY_TONE = { reversible: 'success', rebuildable: 'success', light_copy: 'success', lossy_opt_in: 'warning', review_only: 'neutral', permanent: 'danger' };
const ROW_TONE = { duplicate: 'info', keeper: 'success', confidential: 'neutral', encrypted: 'neutral', signed: 'neutral', labelled: 'neutral', synced: 'warning', online_only: 'neutral', protected: 'neutral', hardlinked: 'neutral', rebuildable: 'success', in_use: 'warning', lossy: 'warning' };

/**
 * @param {string} label already-translated text (rendered uppercase by CSS)
 * @param {string} tone success | warning | danger | info | select | neutral
 * @param {{ title?: string, extra?: string }} [opts]
 */
export function badge(label, tone, opts = {}) {
  return h('span', { class: ['badge', `badge--${tone}`, opts.extra], title: opts.title }, label);
}

/** Safety badge of a card or plan step. @param {string} safety */
export function safetyBadge(safety) {
  return badge(copy.badge[safety] ?? copy.badge.unknownSafety, SAFETY_TONE[safety] ?? 'neutral', { extra: 'badge--safety' });
}

/** "In plan" badge. */
export const inPlanBadge = () => badge(copy.badge.in_plan, 'select');

/** A row flag from the engine's Badges array. @param {string} name */
export function rowBadge(name) {
  return badge(copy.badge[name] ?? name, ROW_TONE[name] ?? 'neutral', { title: copy.badgeHelp[name], extra: 'badge--row' });
}
