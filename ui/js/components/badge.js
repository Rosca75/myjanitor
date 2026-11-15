// @ts-check
/** Badges: safety classes on cards, row flags in the table. */
import { h, icon } from '../dom.js';
import { copy } from '../copy.js';

const SAFETY_TONE = { reversible: 'success', rebuildable: 'success', light_copy: 'success', lossy_opt_in: 'warning', review_only: 'neutral', permanent: 'danger' };
const ROW_TONE = { duplicate: 'info', keeper: 'success', confidential: 'neutral', encrypted: 'neutral', signed: 'neutral', labelled: 'neutral', synced: 'warning', online_only: 'neutral', protected: 'neutral', hardlinked: 'neutral', rebuildable: 'success', in_use: 'warning', lossy: 'warning' };

/** Row flags that mean the file is held back from the selection (shown with a lock). */
const LOCKED = ['encrypted', 'signed', 'labelled', 'protected', 'confidential'];

/**
 * @param {string} label already-translated text (rendered uppercase by CSS)
 * @param {string} tone success | warning | danger | info | select | neutral
 * @param {{ title?: string, extra?: any, lead?: any }} [opts]
 */
export function badge(label, tone, opts = {}) {
  return h('span', { class: ['badge', `badge--${tone}`, opts.extra], title: opts.title }, opts.lead, label);
}

/** Safety badge of a card or plan step. @param {string} safety */
export function safetyBadge(safety) {
  return badge(copy.badge[safety] ?? copy.badge.unknownSafety, SAFETY_TONE[safety] ?? 'neutral', { extra: 'badge--safety' });
}

/** "In plan" badge. */
export const inPlanBadge = () => badge(copy.badge.in_plan, 'select');

/** A row flag from the engine's Badges array. @param {string} name */
export function rowBadge(name) {
  const locked = LOCKED.includes(name);
  return badge(copy.badge[name] ?? name, ROW_TONE[name] ?? 'neutral', { title: copy.badgeHelp[name], extra: ['badge--row', locked && 'badge--lock'], lead: locked ? h('span', { class: 'badge__icon', 'aria-hidden': 'true' }, icon('lock', 10)) : undefined });
}
