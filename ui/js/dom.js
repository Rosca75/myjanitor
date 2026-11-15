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
      for (const [type, fn] of Object.entries(value)) if (fn) bindOn(el, type, fn);
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
 * One listener per event type that calls the element's current handler, so a re-render
 * that keeps a live element (see `replaceKeepingFocus`) can hand it the new handlers.
 * @param {any} el @param {string} type @param {(e: any) => void} fn
 */
function bindOn(el, type, fn) {
  const map = el.__mjOn ?? (el.__mjOn = {});
  if (!(type in map)) el.addEventListener(type, (e) => map[type]?.(e));
  map[type] = fn;
}

/**
 * Handlers for a text box typed in: `input` events while an IME composes are skipped and
 * the composed text arrives with `compositionend` (UI-9: a half-composed string is never
 * sent). @param {(e: any) => void} fn
 */
export const typing = (fn) => ({
  compositionstart: (e) => { e.target.__mjComposing = true; },
  input: (e) => { if (!e.isComposing && !e.target.__mjComposing) fn(e); },
  compositionend: (e) => { e.target.__mjComposing = false; fn(e); },
});

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
  image: 'M4 5h16v14H4zM4 16l5-5 4 4 3-3 4 4M15 9h.01',
  video: 'M3 6h13v12H3zM16 10l5-3v10l-5-3',
  audio: 'M9 18V6l10-2v12M9 18a2 2 0 1 1-4 0 2 2 0 0 1 4 0zM19 16a2 2 0 1 1-4 0 2 2 0 0 1 4 0z',
  doc: 'M7 3h7l4 4v14H7zM10 12h6M10 16h6',
  play: 'M8 5l11 7-11 7z',
  plus: 'M12 5v14M5 12h14',
  up: 'M6 14l6-6 6 6',
  down: 'M6 10l6 6 6-6',
  pause: 'M8 5v14M16 5v14',
  stop: 'M6 6h12v12H6z',
  lock: 'M7 11V8a5 5 0 0 1 10 0v3M6 11h12v9H6z',
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
 * The element of `root` (itself included) whose `data-fk` is `key`. No CSS selector is
 * built from the key: keys may hold paths with backslashes or quotes (UI-11).
 * @param {any} root @param {string} key
 */
function byFocusKey(root, key) {
  if (root?.getAttribute?.('data-fk') === key) return root;
  for (const el of root?.querySelectorAll?.('[data-fk]') ?? []) if (el.getAttribute('data-fk') === key) return el;
  return null;
}

const TEXT_TYPES = ['text', 'search', 'url', 'email', 'tel', 'password', null];
/** A text box someone types in (an IME may be composing in it). @param {any} el */
const isTextField = (el) => el?.tagName === 'TEXTAREA' || (el?.tagName === 'INPUT' && TEXT_TYPES.includes(el.getAttribute('type')));

/** `to` takes the attributes and handlers of `from`. @param {any} to @param {any} from */
function adopt(to, from) {
  for (const name of to.getAttributeNames()) if (!from.hasAttribute(name)) to.removeAttribute(name);
  for (const name of from.getAttributeNames()) if (to.getAttribute(name) !== from.getAttribute(name)) to.setAttribute(name, from.getAttribute(name));
  for (const type of Object.keys(to.__mjOn ?? {})) if (!(type in (from.__mjOn ?? {}))) to.__mjOn[type] = null;
  for (const [type, fn] of Object.entries(from.__mjOn ?? {})) bindOn(to, type, fn);
}

/**
 * UI-9: put the new tree in place while the focused text box itself stays in the document
 * (never detached), so an IME composition goes on and a screen reader does not announce
 * the field again. Each ancestor of the box adopts its counterpart in the new tree and
 * takes that counterpart's other children. False (nothing changed) when the two trees
 * differ on the way to the box. @param {any} container @param {any} node @param {any} active @param {any} again
 */
function keepTextField(container, node, active, again) {
  const path = (top, el) => { const out = []; for (let p = el; p; p = p.parentNode) { out.unshift(p); if (p === top) return out; } return null; };
  const olds = path(container.firstChild, active);
  const news = path(node, again);
  if (container.childNodes.length !== 1 || !olds || !news || olds.length !== news.length) return false;
  if (olds.some((o, i) => o.tagName !== news[i].tagName || o.namespaceURI !== news[i].namespaceURI)) return false;
  for (let i = 0; i < olds.length - 1; i++) {
    const o = olds[i];
    const n = news[i];
    adopt(o, n);
    for (const c of [...o.childNodes]) if (c !== olds[i + 1]) o.removeChild(c);
    const kids = [...n.childNodes];
    const at = kids.indexOf(news[i + 1]);
    for (const c of kids.slice(0, at)) o.insertBefore(c, olds[i + 1]);
    for (const c of kids.slice(at + 1)) o.appendChild(c);
  }
  adopt(active, again);
  // The value from the store, unless an IME is composing (the box holds the truth then).
  if (active.value !== again.value && !active.__mjComposing) {
    const from = active.selectionStart;
    const to = active.selectionEnd;
    active.value = again.value;
    try { active.setSelectionRange(from, to); } catch { /* not a text input */ }
  }
  return true;
}

/**
 * Replace the children of `container` with `node`, keeping keyboard focus on
 * the element with the same `data-fk` key when there is one. A text box being typed in
 * is kept itself (UI-9), anything else is focused again in the new tree.
 * @param {any} container
 * @param {any} node
 */
export function replaceKeepingFocus(container, node) {
  const active = document.activeElement;
  const key = active && container.contains?.(active) ? active.getAttribute?.('data-fk') : null;
  if (key && isTextField(active)) {
    const twin = byFocusKey(node, key);
    if (twin && twin.tagName === active.tagName && keepTextField(container, node, active, twin)) return;
  }
  const from = key ? active.selectionStart : undefined; // keep the caret of a text box being typed in
  const to = key ? active.selectionEnd : undefined;
  container.replaceChildren(node);
  if (key) {
    const again = byFocusKey(container, key);
    if (again && typeof again.focus === 'function') again.focus();
    if (again && typeof again.setSelectionRange === 'function' && typeof from === 'number' && typeof to === 'number') {
      try { again.setSelectionRange(from, to); } catch { /* not a text input */ }
    }
  }
}

/** Stable key for `data-fk` from arbitrary ids (keeps the selector safe). @param {any} v */
export const fk = (v) => String(v).replace(/[^A-Za-z0-9_:-]/g, '_');
