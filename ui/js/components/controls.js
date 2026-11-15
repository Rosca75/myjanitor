// @ts-check
/** Small controls: buttons, checkbox, switch, select, radio tile. No inline handlers. */
import { h, icon } from '../dom.js';

/**
 * @param {{ label: string, variant?: 'primary'|'secondary'|'link'|'warning'|'danger'|'quiet', onClick?: (e: any) => void, disabled?: boolean, icon?: string, fk?: string, ariaLabel?: string, title?: string, size?: 'sm'|'md'|'lg', block?: boolean, pressed?: boolean, extra?: string, describedBy?: string }} o
 */
export function button(o) {
  return h('button', {
    type: 'button',
    class: ['btn', `btn--${o.variant ?? 'secondary'}`, o.size && `btn--${o.size}`, o.block && 'btn--block', o.extra],
    disabled: o.disabled,
    'aria-label': o.ariaLabel,
    'aria-pressed': o.pressed === undefined ? undefined : String(o.pressed),
    'aria-describedby': o.describedBy,
    title: o.title,
    'data-fk': o.fk,
    on: { click: o.onClick },
  }, o.icon && icon(/** @type {any} */ (o.icon), 14), o.label);
}

/** Icon-only button; the accessible name is required. @param {{ icon: string, label: string, onClick?: (e:any)=>void, disabled?: boolean, fk?: string, extra?: string }} o */
export function iconButton(o) {
  return h('button', { type: 'button', class: ['icon-btn', o.extra], 'aria-label': o.label, title: o.label, disabled: o.disabled, 'data-fk': o.fk, on: { click: o.onClick } }, icon(/** @type {any} */ (o.icon), 14));
}

/**
 * Native checkbox with an accessible name.
 * @param {{ checked: boolean, label: string, onChange: (checked: boolean) => void, disabled?: boolean, fk?: string, extra?: string }} o
 */
export function checkbox(o) {
  return h('input', {
    type: 'checkbox', class: ['checkbox', o.extra], 'aria-label': o.label, checked: o.checked, disabled: o.disabled, 'data-fk': o.fk,
    on: { change: (e) => o.onChange(!!e.target.checked) },
  });
}

/** Switch (checkbox with role=switch). @param {{ checked: boolean, label: string, onChange: (checked: boolean) => void, disabled?: boolean, fk?: string, title?: string }} o */
export function toggle(o) {
  return h('input', {
    type: 'checkbox', role: 'switch', class: 'switch', 'aria-label': o.label, 'aria-checked': String(o.checked), checked: o.checked, disabled: o.disabled, title: o.title, 'data-fk': o.fk,
    on: { change: (e) => o.onChange(!!e.target.checked) },
  });
}

/**
 * Dropdown. `options` are { value, label }; the selected value is a string.
 * @param {{ options: {value: string, label: string}[], value: string, label: string, onChange: (value: string) => void, fk?: string, extra?: string, disabled?: boolean }} o
 */
export function select(o) {
  return h('select', { class: ['select', o.extra], 'aria-label': o.label, 'data-fk': o.fk, disabled: o.disabled, on: { change: (e) => o.onChange(String(e.target.value)) } },
    o.options.map((opt) => h('option', { value: opt.value, selected: opt.value === o.value }, opt.label)));
}

/** Number input with a suffix. @param {{ value: number, min?: number, max?: number, step?: number, label: string, unit?: string, onChange: (n: number) => void, fk?: string }} o */
export function numberInput(o) {
  const input = h('input', { type: 'number', class: 'number', min: o.min, max: o.max, step: o.step, value: String(o.value), 'aria-label': o.label, 'data-fk': o.fk, on: { change: (e) => { const n = Number(e.target.value); if (Number.isFinite(n)) o.onChange(n); } } });
  input.value = String(o.value);
  return h('span', { class: 'number-wrap' }, input, o.unit && h('span', { class: 'number-unit' }, o.unit));
}

/**
 * Radio tile (preset or action choice) as a role=radio button.
 * @param {{ title: string, sub?: string, selected: boolean, onSelect: () => void, fk?: string, disabled?: boolean }} o
 */
export function radioTile(o) {
  return h('button', { type: 'button', role: 'radio', class: ['tile', o.selected && 'tile--selected'], 'aria-checked': String(o.selected), disabled: o.disabled, 'data-fk': o.fk, on: { click: () => !o.selected && o.onSelect() } },
    h('span', { class: 'radio', 'aria-hidden': 'true' }),
    h('span', { class: 'tile__text' }, h('span', { class: 'tile__title' }, o.title), o.sub && h('span', { class: 'tile__sub' }, o.sub)));
}
