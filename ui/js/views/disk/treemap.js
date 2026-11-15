// @ts-check
/** Treemap (SCAN-05): squarified tiles as positioned buttons, each with a text label and size. */
import { h } from '../../dom.js';
import { copy } from '../../copy.js';
import { formatBytes, formatPercent, displayPath, middleEllipsis } from '../../format.js';
import { squarify } from '../../lib/treemap.js';
import { findNode, treemapItems } from '../../lib/disktree.js';
import { button } from '../../components/controls.js';
import { pathText } from '../../components/path.js';

const W = 1000;
const H = 520;
const TONES = ['a', 'b', 'c', 'd', 'e'];

/** The folder the treemap shows. @param {any} d */
export const treemapNode = (d) => (d.focusPath ? findNode(d.root, d.focusPath) : null) ?? d.root;

/** @param {any} s @param {any} actions */
export function renderTreemap(s, actions) {
  const d = s.disk;
  const node = treemapNode(d);
  if (!node) return null;
  const tiles = squarify(treemapItems(node), { x: 0, y: 0, w: W, h: H });
  const byPath = new Map(node.children.map((c) => [c.path, c]));
  const box = h('div', { class: 'tmap', role: 'group', 'aria-label': copy.disk.treemapLabel });
  tiles.forEach((t, i) => {
    const child = byPath.get(t.key);
    const name = child ? displayPath(child.name || child.path) : copy.disk.smaller(node.other?.count ?? 0);
    const share = node.size > 0 ? t.value / node.size : 0;
    const big = t.w >= 110 && t.h >= 44;
    const tile = h(child?.is_dir ? 'button' : 'div', {
      type: child?.is_dir ? 'button' : undefined, class: ['tile2', `tile2--${child ? TONES[i % TONES.length] : 'other'}`], 'data-path': t.key, title: displayPath(child?.path ?? name),
      'aria-label': copy.disk.tile(name, formatBytes(t.value), formatPercent(share)),
      on: child?.is_dir ? { click: () => actions.focusDisk(child.path) } : undefined,
    }, big && h('span', { class: 'tile2__name' }, middleEllipsis(name, Math.max(8, Math.floor(t.w / 9)))), big && h('span', { class: 'tile2__size' }, formatBytes(t.value)));
    for (const [k, v] of [['left', t.x / W], ['top', t.y / H], ['width', t.w / W], ['height', t.h / H]]) tile.style.setProperty(k, `${(v * 100).toFixed(3)}%`);
    box.appendChild(tile);
  });
  const top = [...node.children].slice(0, 10);
  return h('div', { class: 'treemap' },
    h('div', { class: 'treemap__bar' },
      button({ label: copy.disk.treemapUp, size: 'sm', icon: 'up', disabled: !d.focusPath, fk: 'treemap-up', onClick: () => actions.focusDisk(parentOf(d.root, d.focusPath)) }),
      h('span', { class: 'treemap__where' }, copy.disk.focusLabel, ' ', d.focusPath ? pathText(d.focusPath, { max: 60 }) : copy.disk.focusAll)),
    box,
    h('section', { class: 'largest', 'aria-label': copy.disk.largest },
      h('h3', { class: 'eyebrow' }, copy.disk.largest),
      h('ol', { class: 'largest__list' }, top.map((c) => h('li', {}, pathText(c.name || c.path, { max: 40, class: 'largest__name' }), h('span', { class: 'largest__size' }, `${formatBytes(c.size)}  ·  ${formatPercent(node.size ? c.size / node.size : 0)}`))))));
}

/** Path of the folder above `path` in the loaded tree, or null for the top. @param {any} root @param {string|null} path */
function parentOf(root, path) {
  if (!path) return null;
  const walk = (n) => { for (const c of n.children ?? []) { if (c.path === path) return n.path || null; const f = walk(c); if (f !== undefined) return f; } return undefined; };
  const r = walk(root);
  return r === undefined ? null : r === root.path ? null : r;
}
