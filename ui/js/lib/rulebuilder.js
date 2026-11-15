// @ts-check
/**
 * Rule builder model and its conversion to and from the engine's Rule JSON
 * (docs/IPC.md "Rule"). Pure functions, no DOM, so the round trip is unit
 * tested. The model is what the form edits; `toRule` produces exactly the wire
 * shape (`{"all":[{"cond":{...}}]}`) and `fromRule` reads any Rule back.
 *
 *   group: { kind: 'group', op: 'all' | 'any', not: boolean, children: Node[] }
 *   cond:  { kind: 'cond', field: string, not: boolean, ...field values }
 *
 * A path is a list of child indexes from the root group: [] is the root,
 * [0, 2] is the third child of the first child.
 */

export const UNITS = Object.freeze({ B: 1, KB: 1024, MB: 1024 ** 2, GB: 1024 ** 3, TB: 1024 ** 4 });
export const CMPS = Object.freeze(['lt', 'le', 'eq', 'ge', 'gt']);
export const TEXT_MODES = Object.freeze(['wildcard', 'contains', 'exact', 'regex']);
export const DATE_FIELDS = Object.freeze(['created', 'modified', 'accessed']);
export const SPAN_UNITS = Object.freeze(['days', 'weeks', 'months', 'years']);
export const ENTRY_KINDS = Object.freeze(['file', 'dir', 'link']);
export const ATTRIBUTES = Object.freeze(['hidden', 'system', 'readonly', 'archive', 'compressed', 'sparse', 'encrypted', 'offline', 'temporary', 'online_only', 'pinned']);
/** Built-in type groups; the user's own come from Settings.type_groups. */
export const TYPE_GROUPS = Object.freeze(['images', 'video', 'audio', 'documents', 'office', 'archives', 'installers', 'code', 'system']);
/** One-click extension lists for the extension field. */
export const EXTENSION_SETS = Object.freeze({
  images: 'jpg, jpeg, png, gif, webp, heic, bmp, tiff',
  video: 'mp4, mkv, mov, avi, wmv, webm',
  audio: 'mp3, m4a, wav, flac, ogg, aac',
  documents: 'pdf, doc, docx, txt, rtf, odt',
  archives: 'zip, 7z, rar, tar, gz, iso',
  installers: 'msi, exe, dmg, pkg, deb',
});

/** Field ids in the order of the picker. */
export const FIELDS = Object.freeze(['name', 'path', 'extension', 'type_group', 'size', 'date', 'attribute', 'owner', 'depth', 'path_length', 'zero_byte', 'invalid_name', 'kind', 'extension_mismatch', 'content_contains', 'largest_n']);
/** Fields that take no value. */
export const FLAG_FIELDS = Object.freeze(['zero_byte', 'invalid_name', 'extension_mismatch']);

/** @returns {any} an empty `all` group */
export const newGroup = (op = 'all') => ({ kind: 'group', op, not: false, children: [] });

/** A condition with the defaults of its field. @param {string} field @returns {any} */
export function newCond(field = 'name') {
  const base = { kind: 'cond', field, not: false };
  switch (field) {
    case 'name': case 'path': return { ...base, mode: 'wildcard', pattern: '', case_sensitive: false };
    case 'extension': return { ...base, values: '' };
    case 'type_group': return { ...base, group: 'images' };
    case 'size': return { ...base, cmp: 'gt', amount: 100, unit: 'MB' };
    case 'date': return { ...base, which: 'modified', cmp: 'lt', mode: 'ago', amount: 6, unit: 'months', date: '2024-01-01' };
    case 'attribute': return { ...base, attrs: {} };
    case 'owner': return { ...base, value: '' };
    case 'depth': case 'path_length': return { ...base, cmp: 'gt', value: field === 'depth' ? 5 : 200 };
    case 'kind': return { ...base, kind_value: 'file' };
    case 'content_contains': return { ...base, text: '', case_sensitive: false };
    case 'largest_n': return { ...base, n: 50 };
    default: return base;
  }
}

/** The group a new builder starts with: one empty name condition. */
export const initialModel = () => ({ ...newGroup('all'), children: [newCond('name')] });

// ------------------------------------------------------------------ paths

/** @param {any} model @param {number[]} path */
export function getAt(model, path) {
  let node = model;
  for (const i of path) node = node?.children?.[i];
  return node;
}

/**
 * Immutable update of the node at `path`.
 * @param {any} model @param {number[]} path @param {(node: any) => any} fn
 */
export function updateAt(model, path, fn) {
  if (path.length === 0) return fn(model);
  const [i, ...rest] = path;
  return { ...model, children: model.children.map((c, k) => (k === i ? updateAt(c, rest, fn) : c)) };
}

/** Remove the node at `path` (the root cannot be removed). @param {any} model @param {number[]} path */
export function removeAt(model, path) {
  if (path.length === 0) return model;
  const parent = path.slice(0, -1);
  const idx = path[path.length - 1];
  return updateAt(model, parent, (g) => ({ ...g, children: g.children.filter((_, k) => k !== idx) }));
}

/** Append `node` to the group at `groupPath`. @param {any} model @param {number[]} groupPath @param {any} node */
export function addTo(model, groupPath, node) {
  return updateAt(model, groupPath, (g) => ({ ...g, children: [...g.children, node] }));
}

// -------------------------------------------------------------- to a Rule

/** @param {string} text comma or space separated extensions @returns {string[]} */
export function parseExtensions(text) {
  return String(text ?? '').split(/[\s,;]+/).map((x) => x.replace(/^\*?\./, '').toLowerCase()).filter(Boolean);
}

/** Bytes of a size field. @param {number} amount @param {string} unit */
export const toBytes = (amount, unit) => Math.round(Number(amount) * (UNITS[/** @type {keyof typeof UNITS} */ (unit)] ?? 1));

/** Midnight UTC of `YYYY-MM-DD` in nanoseconds, or NaN. @param {string} date */
export function dateToNs(date) {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(String(date))) return NaN;
  const ms = Date.parse(`${date}T00:00:00Z`);
  return Number.isNaN(ms) ? NaN : ms * 1e6;
}

/** @param {number} ns */
export const nsToDate = (ns) => new Date(ns / 1e6).toISOString().slice(0, 10);

/** The leaf conditions (without the `cond` wrapper) one condition row stands for. @param {any} c @returns {any[]} */
function leaves(c) {
  switch (c.field) {
    case 'name': case 'path': return [{ field: c.field, mode: c.mode, pattern: c.pattern, case_sensitive: !!c.case_sensitive }];
    case 'extension': return [{ field: 'extension', values: parseExtensions(c.values) }];
    case 'type_group': return [{ field: 'type_group', group: c.group }];
    case 'size': return [{ field: 'size', cmp: c.cmp, bytes: toBytes(c.amount, c.unit) }];
    case 'date': return [{ field: 'date', which: c.which, cmp: c.cmp, value: c.mode === 'at' ? { at: dateToNs(c.date) } : { ago: { amount: Number(c.amount), unit: c.unit } } }];
    case 'attribute': {
      const set = ATTRIBUTES.filter((a) => c.attrs?.[a] === 'set');
      const clear = ATTRIBUTES.filter((a) => c.attrs?.[a] === 'clear');
      return [...(set.length ? [{ field: 'attribute', attr: set, set: true }] : []), ...(clear.length ? [{ field: 'attribute', attr: clear, set: false }] : [])];
    }
    case 'owner': return [{ field: 'owner', value: c.value }];
    case 'depth': case 'path_length': return [{ field: c.field, cmp: c.cmp, value: Number(c.value) }];
    case 'kind': return [{ field: 'kind', kind: c.kind_value }];
    case 'content_contains': return [{ field: 'content_contains', text: c.text, case_sensitive: !!c.case_sensitive }];
    case 'largest_n': return [{ field: 'largest_n', n: Number(c.n) }];
    case 'raw': return [c.raw];
    default: return [{ field: c.field }];
  }
}

/** Convert the builder model to the engine's Rule JSON. @param {any} node @returns {any} */
export function toRule(node) {
  if (node.kind === 'group') {
    const rule = { [node.op]: node.children.map(toRule) };
    return node.not ? { not: rule } : rule;
  }
  const ls = leaves(node).map((l) => ({ cond: l }));
  const rule = ls.length === 1 ? ls[0] : { all: ls };
  return node.not ? { not: rule } : rule;
}

// ------------------------------------------------------------ from a Rule

/** Largest unit that expresses `bytes` without losing precision. @param {number} bytes */
export function splitBytes(bytes) {
  for (const unit of /** @type {const} */ (['TB', 'GB', 'MB', 'KB'])) {
    const v = bytes / UNITS[unit];
    if (v >= 1 && toBytes(Math.round(v * 100) / 100, unit) === bytes) return { amount: Math.round(v * 100) / 100, unit };
  }
  return { amount: bytes, unit: 'B' };
}

/** @param {any} leaf a condition body @returns {any} */
function condFromLeaf(leaf) {
  const base = newCond(leaf.field);
  switch (leaf.field) {
    case 'name': case 'path': return { ...base, mode: leaf.mode, pattern: leaf.pattern, case_sensitive: !!leaf.case_sensitive };
    case 'extension': return { ...base, values: (leaf.values ?? []).join(', ') };
    case 'type_group': return { ...base, group: leaf.group };
    case 'size': return { ...base, cmp: leaf.cmp, ...splitBytes(leaf.bytes) };
    case 'date': return leaf.value && 'at' in leaf.value
      ? { ...base, which: leaf.which, cmp: leaf.cmp, mode: 'at', date: nsToDate(leaf.value.at) }
      : { ...base, which: leaf.which, cmp: leaf.cmp, mode: 'ago', amount: leaf.value?.ago?.amount ?? 1, unit: leaf.value?.ago?.unit ?? 'days' };
    case 'attribute': return { ...base, attrs: Object.fromEntries((leaf.attr ?? []).map((a) => [a, leaf.set ? 'set' : 'clear'])) };
    case 'owner': return { ...base, value: leaf.value };
    case 'depth': case 'path_length': return { ...base, cmp: leaf.cmp, value: leaf.value };
    case 'kind': return { ...base, kind_value: leaf.kind };
    case 'content_contains': return { ...base, text: leaf.text, case_sensitive: !!leaf.case_sensitive };
    case 'largest_n': return { ...base, n: leaf.n };
    case 'zero_byte': case 'invalid_name': case 'extension_mismatch': return base;
    default: return { kind: 'cond', field: 'raw', not: false, raw: leaf };
  }
}

/**
 * Read any Rule into the builder model. A root that is not a group is wrapped
 * in an `all` group so the builder always has a root to add to.
 * @param {any} rule @returns {any}
 */
export function fromRule(rule) {
  const node = nodeFromRule(rule);
  return node.kind === 'group' && !node.not ? node : { ...newGroup('all'), children: [node] };
}

/** @param {any} rule @returns {any} */
function nodeFromRule(rule) {
  if (!rule || typeof rule !== 'object') return newGroup('all');
  if ('all' in rule || 'any' in rule) {
    const op = 'all' in rule ? 'all' : 'any';
    return { kind: 'group', op, not: false, children: rule[op].map(nodeFromRule) };
  }
  if ('not' in rule) {
    const inner = nodeFromRule(rule.not);
    if (inner.kind === 'cond') return { ...inner, not: !inner.not };
    return inner.not ? { kind: 'group', op: 'all', not: true, children: [inner] } : { ...inner, not: true };
  }
  if ('cond' in rule) return condFromLeaf(rule.cond);
  return { kind: 'cond', field: 'raw', not: false, raw: rule };
}

// -------------------------------------------------------------- validation

/**
 * Problems that would make the engine refuse the rule, as { path, code }.
 * The view turns each code into a sentence from copy.js.
 * @param {any} model
 * @returns {{ path: number[], code: string }[]}
 */
export function validate(model) {
  /** @type {{ path: number[], code: string }[]} */
  const out = [];
  const walk = (node, path) => {
    if (node.kind === 'group') {
      if (path.length === 0 && node.children.length === 0) out.push({ path, code: 'empty_group' });
      node.children.forEach((c, i) => walk(c, [...path, i]));
      return;
    }
    const add = (code) => out.push({ path, code });
    const int = (v) => Number.isInteger(Number(v)) && Number(v) >= 0 && String(v).trim() !== '';
    switch (node.field) {
      case 'name': case 'path':
        if (!String(node.pattern).trim()) add('pattern_empty');
        else if (node.mode === 'regex') { try { new RegExp(node.pattern); } catch { add('regex_invalid'); } }
        break;
      case 'extension': if (!parseExtensions(node.values).length) add('extension_empty'); break;
      case 'type_group': if (!String(node.group).trim()) add('group_empty'); break;
      case 'size': if (!(Number(node.amount) >= 0) || String(node.amount).trim() === '') add('number_invalid'); break;
      case 'date':
        if (node.mode === 'at' ? Number.isNaN(dateToNs(node.date)) : !int(node.amount)) add(node.mode === 'at' ? 'date_invalid' : 'number_invalid');
        break;
      case 'attribute': if (!Object.values(node.attrs ?? {}).some((v) => v === 'set' || v === 'clear')) add('attribute_none'); break;
      case 'owner': if (!String(node.value).trim()) add('owner_empty'); break;
      case 'depth': case 'path_length': if (!int(node.value)) add('number_invalid'); break;
      case 'content_contains': if (!String(node.text).trim()) add('text_empty'); break;
      case 'largest_n':
        if (!(Number.isInteger(Number(node.n)) && Number(node.n) >= 1)) add('number_invalid');
        if (path.length !== 1 || model.op !== 'all' || model.not) add('largest_n_placement');
        break;
      default: break;
    }
  };
  walk(model, []);
  return out;
}

/** Count the leaf conditions of a model. @param {any} node @returns {number} */
export const countConds = (node) => (node.kind === 'group' ? node.children.reduce((a, c) => a + countConds(c), 0) : 1);
