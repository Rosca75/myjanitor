// @ts-check
/**
 * Tiny DOM builder. Every view builds elements through `h()`, which only ever
 * uses `createElement`, `setAttribute`, `textContent` and text nodes, so
 * untrusted strings (file names!) can never become markup. No HTML-string
 * API is used anywhere in the UI, and `h()` refuses `onclick`-style attributes:
 * handlers go through the `on` map (addEventListener), which keeps the CSP free
 * of 'unsafe-inline'.
 */

const SVG_NS = 'http://www.w3.org/2000/svg';

/**
 * @param {any} el
 * @param {string} key
 * @param {any} value
 */
function applyProp(el, key, value) {
  if (value === undefined || value === null || value === false) return;
  if (/^on.+/i.test(key)) throw new Error(`Inline handler "${key}" is not allowed; use { on: { click } }`);
  switch (key) {
    case 'class':
      el.setAttribute('class', Array.isArray(value) ? value.flat(Infinity).filter(Boolean).join(' ') : String(value));
      return;
    case 'on':
      for (const [type, fn] of Object.entries(value)) if (fn) el.addEventListener(type, fn);
      return;
    case 'style':
      for (const [name, v] of Object.entries(value)) el.style.setProperty(name, String(v));
      return;
    case 'dataset':
      for (const [name, v] of Object.entries(value)) el.setAttribute(`data-${name}`, String(v));
      return;
    case 'text':
      el.textContent = String(value);
      return;
    case 'value':
    case 'checked':
    case 'selected':
      el[key] = value;
      return;
    case 'ref':
      value(el);
      return;
    default:
      el.setAttribute(key, value === true ? '' : String(value));
  }
}

/**
 * Append children: strings and numbers become text nodes, arrays are
 * flattened, null/undefined/false are skipped.
 * @param {any} el
 * @param {any[]} children
 */
function append(el, children) {
  for (const child of children) {
    if (child === null || child === undefined || child === false || child === true) continue;
    if (Array.isArray(child)) append(el, child);
    else if (typeof child === 'object' && child.nodeType) el.appendChild(child);
    else el.appendChild(document.createTextNode(String(child)));
  }
}

/**
 * Create an element. Tag `svg:path` creates an SVG element.
 * @param {string} tag
 * @param {Record<string, any> | null} [props]
 * @param {...any} children
 * @returns {any}
 */
export function h(tag, props, ...children) {
  const el = tag.startsWith('svg:') ? document.createElementNS(SVG_NS, tag.slice(4)) : document.createElement(tag);
  for (const [k, v] of Object.entries(props ?? {})) applyProp(el, k, v);
  append(el, children);
  return el;
}

/** Icon paths (24x24 viewBox, stroke icons). Static strings only. */
const ICONS = {
  logo: 'M5 17 12 6l7 11',
  overview: 'M4 5h7v7H4zM13 5h7v4h-7zM13 12h7v7h-7zM4 15h7v4H4z',
  clean: 'M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13',
  optimise: 'M4 8l8-4 8 4-8 4zM4 12l8 4 8-4M4 16l8 4 8-4',
  archive: 'M4 5h16v4H4zM6 9v10h12V9M10 13h4',
  history: 'M4 12a8 8 0 1 0 3-6.2M4 4v4h4M12 8v5l3 2',
  settings: 'M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6zM12 3v3M12 18v3M3 12h3M18 12h3M5.6 5.6l2.1 2.1M16.3 16.3l2.1 2.1M18.4 5.6l-2.1 2.1M7.7 16.3l-2.1 2.1',
  folder: 'M3 7a1 1 0 0 1 1-1h5l2 2h8a1 1 0 0 1 1 1v8a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1z',
  chevronDown: 'M6 9l6 6 6-6',
  chevronRight: 'M9 6l6 6-6 6',
  chevronLeft: 'M15 6l-6 6 6 6',
  chevronUp: 'M6 15l6-6 6 6',
  arrowRight: 'M5 12h14M13 6l6 6-6 6',
  check: 'M5 12l5 5 9-10',
  close: 'M6 6l12 12M18 6L6 18',
  grip: 'M5 8h14M5 12h14M5 16h14',
  warning: 'M12 8v5M12 16.5v.5',
  info: 'M12 8v.5M12 11v5',
  file: 'M7 3h7l4 4v14H7z',
  play: 'M8 5l11 7-11 7z',
  plus: 'M12 5v14M5 12h14',
  up: 'M6 14l6-6 6 6',
  down: 'M6 10l6 6 6-6',
  pause: 'M8 5v14M16 5v14',
  stop: 'M6 6h12v12H6z',
  undo: 'M9 7L4 12l5 5M4 12h11a5 5 0 0 1 0 10h-3',
};

/**
 * Decorative stroke icon (aria-hidden).
 * @param {keyof typeof ICONS} name
 * @param {number} [size]
 */
export function icon(name, size = 16) {
  const svg = h('svg:svg', { viewBox: '0 0 24 24', width: size, height: size, 'aria-hidden': 'true', focusable: 'false', class: `icon icon-${name}` });
  svg.appendChild(h('svg:path', { d: ICONS[name] ?? ICONS.info }));
  return svg;
}

/**
 * Replace the children of `container` with `node`, keeping keyboard focus on
 * the element with the same `data-fk` key when there is one.
 * @param {any} container
 * @param {any} node
 */
export function replaceKeepingFocus(container, node) {
  const active = document.activeElement;
  const key = active && container.contains?.(active) ? active.getAttribute?.('data-fk') : null;
  const from = key ? active.selectionStart : undefined; // keep the caret of a text box being typed in
  const to = key ? active.selectionEnd : undefined;
  container.replaceChildren(node);
  if (key) {
    const again = container.querySelector(`[data-fk="${key}"]`);
    if (again && typeof again.focus === 'function') again.focus();
    if (again && typeof again.setSelectionRange === 'function' && typeof from === 'number' && typeof to === 'number') {
      try { again.setSelectionRange(from, to); } catch { /* not a text input */ }
    }
  }
}

/** Stable key for `data-fk` from arbitrary ids (keeps the selector safe). @param {any} v */
export const fk = (v) => String(v).replace(/[^A-Za-z0-9_:-]/g, '_');
