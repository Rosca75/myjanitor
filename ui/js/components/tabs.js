// @ts-check
/** Tab strip with counts: role=tablist, arrow keys move the selection. */
import { h } from '../dom.js';

/**
 * @param {{ items: {key: string|null, label: string, count?: number}[], active: string|null, onSelect: (key: string|null) => void, label: string, fk: string }} o
 */
export function tabs(o) {
  const keys = o.items.map((i) => i.key);
  /** @param {any} e @param {number} i */
  const onKey = (e, i) => {
    const j = e.key === 'ArrowRight' ? i + 1 : e.key === 'ArrowLeft' ? i - 1 : e.key === 'Home' ? 0 : e.key === 'End' ? keys.length - 1 : -1;
    if (j < 0 || j >= keys.length) return;
    e.preventDefault();
    o.onSelect(keys[j]);
  };
  return h('div', { class: 'tabs', role: 'tablist', 'aria-label': o.label },
    o.items.map((it, i) => h('button', {
      type: 'button', role: 'tab', class: ['tab', it.key === o.active && 'tab--active'], 'aria-selected': String(it.key === o.active), tabindex: it.key === o.active ? '0' : '-1', 'data-fk': `${o.fk}-${it.key ?? 'all'}`,
      on: { click: () => o.onSelect(it.key), keydown: (e) => onKey(e, i) },
    }, it.label, it.count !== undefined && h('span', { class: 'tab__count' }, ` ${it.count}`))));
}
