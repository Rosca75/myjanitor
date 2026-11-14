// @ts-check
/**
 * The control for one SettingDef of an opportunity (docs/IPC.md Opportunity
 * settings): toggle, number, choice, text or folder.
 */
import { h } from '../dom.js';
import { button, numberInput, select, toggle } from './controls.js';

/**
 * @param {any} st the setting
 * @param {Record<string, any>} params current values
 * @param {{ onChange: (key: string, value: any) => void, onPickFolder?: (key: string) => void }} h2
 */
export function settingControl(st, params, h2) {
  const value = params[st.key] ?? st.kind.default;
  const locked = !!st.disabled_reason;
  switch (st.kind.type) {
    case 'toggle':
      return toggle({ checked: !!value, label: st.label, disabled: locked, title: st.disabled_reason, fk: `p-${st.key}`, onChange: (on) => h2.onChange(st.key, on) });
    case 'choice':
      return select({ label: st.label, value: String(value), disabled: locked, fk: `p-${st.key}`, options: st.kind.options.map((o) => ({ value: o.value, label: o.label ?? o.value })), onChange: (v) => h2.onChange(st.key, v) });
    case 'text':
      return h('input', { type: 'text', class: 'input', value: String(value ?? ''), placeholder: st.kind.placeholder, 'aria-label': st.label, disabled: locked, 'data-fk': `p-${st.key}`, on: { change: (e) => h2.onChange(st.key, String(e.target.value)) } });
    case 'folder':
      return h('span', { class: 'folder-setting' }, h('span', { class: 'path', title: String(value ?? '') }, String(value ?? '') || '-'), button({ label: '…', size: 'sm', ariaLabel: st.label, disabled: locked || !h2.onPickFolder, fk: `p-${st.key}`, onClick: () => h2.onPickFolder?.(st.key) }));
    default:
      return numberInput({ value: Number(value), min: st.kind.min, max: st.kind.max, step: st.kind.step, unit: st.kind.unit, label: st.label, fk: `p-${st.key}`, onChange: (n) => h2.onChange(st.key, n) });
  }
}
