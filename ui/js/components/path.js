// @ts-check
/** A path (or any long text) shortened in the middle, with the full text in `title`. */
import { h } from '../dom.js';
import { displayPath, middleEllipsis } from '../format.js';

/**
 * @param {string} path
 * @param {{ max?: number, tag?: string, class?: string }} [o]
 */
export function pathText(path, o = {}) {
  const full = displayPath(path);
  return h(o.tag ?? 'span', { class: ['path', o.class], title: full }, middleEllipsis(full, o.max ?? 48));
}
