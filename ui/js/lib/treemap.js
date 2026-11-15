// @ts-check
/**
 * Squarified treemap layout (Bruls, Huizing and van Wijk): turns a list of
 * sizes into rectangles that fill a box exactly, keeping the tiles as close to
 * squares as possible so labels stay readable. Pure function, no DOM.
 */

/**
 * @typedef {{ x: number, y: number, w: number, h: number }} Rect
 * @typedef {{ key: string, value: number }} Item
 * @typedef {{ key: string, value: number, x: number, y: number, w: number, h: number }} Tile
 */

/** Worst aspect ratio of a row laid along the short side `side`. @param {number[]} areas @param {number} side */
function worst(areas, side) {
  const sum = areas.reduce((a, b) => a + b, 0);
  if (sum <= 0 || side <= 0) return Infinity;
  const max = Math.max(...areas);
  const min = Math.min(...areas);
  const s2 = side * side;
  return Math.max((s2 * max) / (sum * sum), (sum * sum) / (s2 * min));
}

/**
 * Lay `items` out inside `rect`. Items with a value of zero or less are
 * dropped (they have no area). The result is ordered largest first and the
 * tile areas add up to the area of `rect`.
 * @param {Item[]} items
 * @param {Rect} rect
 * @returns {Tile[]}
 */
export function squarify(items, rect) {
  const list = items.filter((i) => i.value > 0).sort((a, b) => b.value - a.value);
  const total = list.reduce((a, i) => a + i.value, 0);
  if (!list.length || total <= 0 || rect.w <= 0 || rect.h <= 0) return [];
  const scale = (rect.w * rect.h) / total;
  const nodes = list.map((i) => ({ key: i.key, value: i.value, area: i.value * scale }));
  /** @type {Tile[]} */
  const out = [];
  let { x, y, w, h } = rect;
  let i = 0;
  while (i < nodes.length) {
    const side = Math.min(w, h);
    let row = [nodes[i]];
    let j = i + 1;
    while (j < nodes.length && worst([...row, nodes[j]].map((n) => n.area), side) <= worst(row.map((n) => n.area), side)) {
      row = [...row, nodes[j]];
      j += 1;
    }
    const rowArea = row.reduce((a, n) => a + n.area, 0);
    if (w >= h) {
      // Row is a column on the left edge.
      const colW = rowArea / h;
      let cy = y;
      for (const n of row) {
        const th = (n.area / rowArea) * h;
        out.push({ key: n.key, value: n.value, x, y: cy, w: colW, h: th });
        cy += th;
      }
      x += colW;
      w -= colW;
    } else {
      const rowH = rowArea / w;
      let cx = x;
      for (const n of row) {
        const tw = (n.area / rowArea) * w;
        out.push({ key: n.key, value: n.value, x: cx, y, w: tw, h: rowH });
        cx += tw;
      }
      y += rowH;
      h -= rowH;
    }
    i = j;
  }
  return out;
}

/** Aspect ratio (>= 1) of a tile. @param {{ w: number, h: number }} t */
export const aspect = (t) => (t.w > 0 && t.h > 0 ? Math.max(t.w / t.h, t.h / t.w) : Infinity);
