// A minimal DOM stand-in for node --test: elements, attributes, children,
// textContent, events with bubbling, focus, and a small querySelector. Only
// what the UI uses; it is not a spec-complete DOM.

/** @param {any} n */
const isEl = (n) => n && n.nodeType === 1;

class FakeText {
  constructor(data) { this.nodeType = 3; this.data = data; this.parentNode = null; }
  get textContent() { return this.data; }
  set textContent(v) { this.data = String(v); }
}

class FakeStyle {
  constructor() { this.props = new Map(); }
  setProperty(k, v) { this.props.set(k, v); }
  getPropertyValue(k) { return this.props.get(k) ?? ''; }
  removeProperty(k) { this.props.delete(k); }
}

/** Parse one compound selector (no combinators) into predicates. @param {string} s */
function parseCompound(s) {
  const out = { tag: null, id: null, classes: [], attrs: [] };
  const re = /^([a-zA-Z][\w-]*|\*)|#([\w-]+)|\.([\w-]+)|\[([\w:-]+)(?:(\^|\$|\*)?=("([^"]*)"|'([^']*)'|[^\]]*))?\]/g;
  let m;
  let last = 0;
  while ((m = re.exec(s))) {
    if (m.index !== last) throw new Error(`Unsupported selector: ${s}`);
    last = re.lastIndex;
    if (m[1]) out.tag = m[1] === '*' ? null : m[1].toLowerCase();
    else if (m[2]) out.id = m[2];
    else if (m[3]) out.classes.push(m[3]);
    else if (m[4]) out.attrs.push({ name: m[4], op: m[5] ?? (m[6] !== undefined ? '=' : null), value: m[7] ?? m[8] ?? m[6] });
  }
  if (last !== s.length) throw new Error(`Unsupported selector: ${s}`);
  return out;
}

/** @param {any} el @param {ReturnType<typeof parseCompound>} c */
function matchCompound(el, c) {
  if (!isEl(el)) return false;
  if (c.tag && el.tagName.toLowerCase() !== c.tag) return false;
  if (c.id && el.getAttribute('id') !== c.id) return false;
  const cls = (el.getAttribute('class') ?? '').split(/\s+/);
  if (!c.classes.every((x) => cls.includes(x))) return false;
  for (const a of c.attrs) {
    const v = el.getAttribute(a.name);
    if (v === null) return false;
    if (a.op === '=' && v !== a.value) return false;
    if (a.op === '^' && !v.startsWith(a.value)) return false;
    if (a.op === '$' && !v.endsWith(a.value)) return false;
    if (a.op === '*' && !v.includes(a.value)) return false;
  }
  return true;
}

/** Split on a separator outside [...] and quotes. @param {string} sel @param {(ch: string) => boolean} isSep */
function splitTop(sel, isSep) {
  const out = [];
  let cur = '';
  let depth = 0;
  let quote = '';
  for (const ch of sel) {
    if (quote) { if (ch === quote) quote = ''; cur += ch; continue; }
    if (ch === '"' || ch === "'") { quote = ch; cur += ch; continue; }
    if (ch === '[') depth += 1;
    if (ch === ']') depth -= 1;
    if (depth === 0 && isSep(ch)) { out.push(cur); cur = ''; continue; }
    cur += ch;
  }
  out.push(cur);
  return out.filter((x) => x.length);
}

/** Split a selector into compounds joined by ' ' or '>'. @param {string} sel */
function parseComplex(sel) {
  const parts = [];
  let comb = ' ';
  for (const t of splitTop(sel.trim().replace(/\s*>\s*/g, (m, off, str) => (str.slice(0, off).split('[').length === str.slice(0, off).split(']').length ? ' > ' : m)), (ch) => /\s/.test(ch))) {
    if (t === '>') comb = '>';
    else { parts.push({ comb, c: parseCompound(t) }); comb = ' '; }
  }
  return parts;
}

/** @param {any} el @param {{comb:string,c:any}[]} parts @param {number} i */
function matchParts(el, parts, i) {
  if (!matchCompound(el, parts[i].c)) return false;
  if (i === 0) return true;
  const comb = parts[i].comb;
  let p = el.parentNode;
  while (p && isEl(p)) {
    if (matchParts(p, parts, i - 1)) return true;
    if (comb === '>') return false;
    p = p.parentNode;
  }
  return false;
}

export class FakeElement {
  /** @param {string} tag @param {any} doc @param {string|null} [ns] */
  constructor(tag, doc, ns = null) {
    this.nodeType = 1;
    this.tagName = ns ? tag : tag.toUpperCase();
    this.namespaceURI = ns;
    this.ownerDocument = doc;
    this.attributes = new Map();
    this.childNodes = [];
    this.parentNode = null;
    this.listeners = new Map();
    this.style = new FakeStyle();
    this.value = '';
    this.checked = false;
  }
  get children() { return this.childNodes.filter(isEl); }
  get firstChild() { return this.childNodes[0] ?? null; }
  get id() { return this.getAttribute('id') ?? ''; }
  get className() { return this.getAttribute('class') ?? ''; }
  get disabled() { return this.attributes.has('disabled'); }
  set disabled(v) { if (v) this.setAttribute('disabled', ''); else this.removeAttribute('disabled'); }
  get dataset() {
    const out = {};
    for (const [k, v] of this.attributes) if (k.startsWith('data-')) out[k.slice(5).replace(/-([a-z])/g, (_, c) => c.toUpperCase())] = v;
    return out;
  }
  get classList() {
    const el = this;
    const list = () => (el.getAttribute('class') ?? '').split(/\s+/).filter(Boolean);
    const api = {
      contains: (c) => list().includes(c),
      add: (c) => { const l = list(); if (!l.includes(c)) el.setAttribute('class', [...l, c].join(' ')); },
      remove: (c) => el.setAttribute('class', list().filter((x) => x !== c).join(' ')),
      toggle: (c, on) => { const has = list().includes(c); if (on ?? !has) api.add(c); else api.remove(c); },
    };
    return api;
  }
  setAttribute(k, v) { this.attributes.set(k, String(v)); }
  getAttribute(k) { return this.attributes.has(k) ? this.attributes.get(k) : null; }
  hasAttribute(k) { return this.attributes.has(k); }
  removeAttribute(k) { this.attributes.delete(k); }
  appendChild(n) {
    if (n.parentNode) n.parentNode.removeChild(n);
    n.parentNode = this;
    this.childNodes.push(n);
    return n;
  }
  append(...nodes) { for (const n of nodes) this.appendChild(typeof n === 'string' ? new FakeText(n) : n); }
  removeChild(n) { this.childNodes = this.childNodes.filter((c) => c !== n); n.parentNode = null; return n; }
  remove() { this.parentNode?.removeChild(this); }
  replaceChildren(...nodes) {
    for (const c of this.childNodes) c.parentNode = null;
    this.childNodes = [];
    this.append(...nodes);
  }
  contains(n) { for (let p = n; p; p = p.parentNode) if (p === this) return true; return false; }
  get textContent() { return this.childNodes.map((c) => c.textContent).join(''); }
  set textContent(v) { this.replaceChildren(new FakeText(String(v))); }
  matches(sel) { return splitTop(sel, (ch) => ch === ',').some((s) => { const p = parseComplex(s); return matchParts(this, p, p.length - 1); }); }
  closest(sel) { for (let p = this; p && isEl(p); p = p.parentNode) if (p.matches(sel)) return p; return null; }
  querySelectorAll(sel) {
    const out = [];
    const walk = (n) => { for (const c of n.childNodes) if (isEl(c)) { if (c.matches(sel)) out.push(c); walk(c); } };
    walk(this);
    return out;
  }
  querySelector(sel) { return this.querySelectorAll(sel)[0] ?? null; }
  addEventListener(type, fn) { if (!this.listeners.has(type)) this.listeners.set(type, []); this.listeners.get(type).push(fn); }
  removeEventListener(type, fn) { this.listeners.set(type, (this.listeners.get(type) ?? []).filter((f) => f !== fn)); }
  /** Dispatch with bubbling to the document. @param {any} ev */
  dispatchEvent(ev) {
    ev.target ??= this;
    ev.bubbles ??= true;
    ev.defaultPrevented = false;
    ev.preventDefault ??= () => { ev.defaultPrevented = true; };
    ev.stopPropagation ??= () => { ev.stopped = true; };
    for (let n = this; n && !ev.stopped; n = n.parentNode) {
      ev.currentTarget = n;
      for (const fn of n.listeners?.get(ev.type) ?? []) fn(ev);
      if (!ev.bubbles) break;
    }
    return !ev.defaultPrevented;
  }
  click() {
    if (this.disabled) return;
    if (this.tagName === 'INPUT' && this.getAttribute('type') === 'checkbox') {
      this.checked = !this.checked;
      this.dispatchEvent({ type: 'click' });
      this.dispatchEvent({ type: 'change' });
      return;
    }
    this.dispatchEvent({ type: 'click' });
  }
  focus() { this.ownerDocument.activeElement = this; }
  blur() { if (this.ownerDocument.activeElement === this) this.ownerDocument.activeElement = this.ownerDocument.body; }
  /** Test helper: send a keydown. @param {string} key @param {object} [extra] */
  key(key, extra = {}) { return this.dispatchEvent({ type: 'keydown', key, ...extra }); }
  /** Test helper: set a form value and fire change. @param {any} v */
  setValue(v) { this.value = String(v); this.dispatchEvent({ type: 'change' }); this.dispatchEvent({ type: 'input' }); }
  /** Test helper: set a checkbox and fire change. @param {boolean} on */
  setChecked(on) { this.checked = on; this.dispatchEvent({ type: 'change' }); }
}

export function createFakeDocument() {
  const doc = /** @type {any} */ ({ listeners: new Map(), nodeType: 9 });
  doc.createElement = (tag) => new FakeElement(tag, doc);
  doc.createElementNS = (ns, tag) => new FakeElement(tag, doc, ns);
  doc.createTextNode = (t) => new FakeText(String(t));
  doc.addEventListener = FakeElement.prototype.addEventListener;
  doc.removeEventListener = FakeElement.prototype.removeEventListener;
  doc.getElementById = (id) => doc.body.querySelector(`#${id}`);
  doc.querySelector = (s) => doc.body.querySelector(s);
  doc.querySelectorAll = (s) => doc.body.querySelectorAll(s);
  doc.body = new FakeElement('body', doc);
  doc.body.parentNode = doc;
  doc.parentNode = null;
  doc.activeElement = doc.body;
  return doc;
}

/** Install a fresh fake document as the global `document`; returns it. */
export function installFakeDom() {
  const doc = createFakeDocument();
  /** @type {any} */ (globalThis).document = doc;
  return doc;
}
