// @ts-check
/** Small builders shared by the Settings sections. */
import { h } from '../../dom.js';
import { button } from '../../components/controls.js';
import { sc } from './copy.js';

/** Inline error under a control (role=alert), or nothing. @param {any} ctl @param {string} key */
export const errorLine = (ctl, key) => ctl.st.errors[key] && h('span', { class: 'custom__help custom__help--error', role: 'alert', 'data-error': key }, ctl.st.errors[key]);

/**
 * A labelled row: label, optional help, optional inline error, control on the right.
 * @param {any} ctl @param {string} key error key @param {string} label @param {any} control @param {{ help?: string }} [o]
 */
export function row(ctl, key, label, control, o = {}) {
  return h('div', { class: 'kv' },
    h('div', { class: 'tile__text' }, h('span', { class: 'kv__k' }, label), o.help && h('span', { class: 'tile__sub' }, o.help), errorLine(ctl, key)),
    control);
}

/** A card section with a heading. @param {string} id @param {string} title @param {...any} children */
export function section(id, title, ...children) {
  return h('section', { class: 'drawer', 'aria-labelledby': `set-${id}`, 'data-section': id }, h('h3', { id: `set-${id}`, class: 'drawer__title' }, title), ...children);
}

/** Rows container (full width). @param {...any} rows */
export const rows = (...rows) => h('div', { class: 'kvs' }, ...rows);

/** Collapsible "Advanced" block. @param {any} ctl @param {string} key @param {...any} children */
export function advanced(ctl, key, ...children) {
  const open = !!ctl.st.open[key];
  return h('div', { class: 'kvs' },
    h('button', { type: 'button', class: 'disclosure', 'aria-expanded': String(open), 'data-fk': `adv-${key}`, on: { click: () => ctl.toggleOpen(key) } }, sc.advanced),
    open && rows(...children));
}

/** Text input committing on change. @param {{ value: string, label: string, placeholder?: string, fk: string, onCommit: (v: string) => void, invalid?: boolean }} o */
export function textInput(o) {
  return h('input', { type: 'text', class: ['input', o.invalid && 'input--invalid'], value: o.value, placeholder: o.placeholder, 'aria-label': o.label, 'aria-invalid': o.invalid ? 'true' : undefined, 'data-fk': o.fk, on: { change: (e) => o.onCommit(String(e.target.value)) } });
}

/** A list of removable entries. @param {string[]} items @param {string} fkPrefix @param {string} removeLabel @param {(v: string) => void} onRemove @param {string} empty */
export function removableList(items, fkPrefix, removeLabel, onRemove, empty) {
  if (!items.length) return h('p', { class: 'tile__sub' }, empty);
  return h('ul', { class: 'tools' }, items.map((v) => h('li', { class: 'tool' },
    h('span', { class: 'kv__k', title: v }, v),
    button({ label: sc.remove, ariaLabel: removeLabel.replace('{}', v), variant: 'quiet', size: 'sm', fk: `${fkPrefix}-${v}`, onClick: () => onRemove(v) }))));
}
