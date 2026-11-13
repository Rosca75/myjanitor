// @ts-check
/** Loading, empty, error and "not available" blocks shared by every view. */
import { h, icon } from '../dom.js';
import { copy } from '../copy.js';
import { toErrorInfo } from '../errors.js';
import { button } from './controls.js';

/** @param {string} [text] */
export const loadingBlock = (text = copy.states.loading) => h('div', { class: 'state state--loading', role: 'status', 'aria-live': 'polite' }, h('span', { class: 'state__spinner', 'aria-hidden': 'true' }), h('span', {}, text));

/** @param {{ title: string, body?: string, action?: any }} o */
export const emptyBlock = (o) => h('div', { class: 'state state--empty' }, h('strong', { class: 'state__title' }, o.title), o.body && h('p', { class: 'state__body' }, o.body), o.action);

/**
 * A failed load: message and hint from the backend, with a retry.
 * @param {any} err @param {{ onRetry?: () => void, fk?: string }} [o]
 */
export function errorBlock(err, o = {}) {
  const e = toErrorInfo(err);
  return h('div', { class: 'state state--error', role: 'alert' },
    h('span', { class: 'state__icon', 'aria-hidden': 'true' }, icon('warning', 16)),
    h('div', { class: 'state__text' }, h('strong', { class: 'state__title' }, e.message), e.hint && h('p', { class: 'state__body' }, e.hint)),
    o.onRetry && button({ label: copy.states.retry, size: 'sm', fk: o.fk ?? 'retry', onClick: o.onRetry }));
}

/** A feature the backend does not offer yet: shown as information, never as a failure. @param {any} err */
export function unavailableBlock(err) {
  const e = toErrorInfo(err);
  return h('div', { class: 'state state--unavailable', role: 'note' },
    h('span', { class: 'state__icon', 'aria-hidden': 'true' }, icon('info', 16)),
    h('div', { class: 'state__text' }, h('strong', { class: 'state__title' }, copy.states.unavailableTitle), h('p', { class: 'state__body' }, [e.message, e.hint].filter(Boolean).join(' '))));
}

/** Text for the `title` of a disabled control. @param {any} err */
export const disabledHint = (err) => (err ? copy.states.disabledBecause([toErrorInfo(err).message, toErrorInfo(err).hint].filter(Boolean).join(' ')) : undefined);
