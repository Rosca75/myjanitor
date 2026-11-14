// @ts-check
/** Rule builder (CLN-ENG-01): nested AND/OR groups of typed condition rows. */
import { h } from '../../dom.js';
import { copy } from '../../copy.js';
import { button, checkbox, iconButton, select } from '../../components/controls.js';
import { FIELDS, FLAG_FIELDS, CMPS, TEXT_MODES, DATE_FIELDS, SPAN_UNITS, ENTRY_KINDS, ATTRIBUTES, TYPE_GROUPS, UNITS, EXTENSION_SETS } from '../../lib/rulebuilder.js';

const opts = (values, labels) => values.map((v) => ({ value: v, label: labels[v] ?? v }));
const key = (path, name) => `b-${path.join('_') || 'root'}-${name}`;
const same = (a, b) => a.length === b.length && a.every((x, i) => x === b[i]);

/** Text or number input bound to a node field. */
function input(type, node, path, name, label, actions, extra = {}) {
  return h('input', { type, class: 'input', value: String(node[name] ?? ''), 'aria-label': label, 'data-fk': key(path, name), ...extra, on: { input: (e) => actions.builderSet(path, { [name]: type === 'number' ? e.target.value : e.target.value }) } });
}

/** The value controls of one condition row, chosen by its field. @param {any} n @param {number[]} path @param {any} s @param {any} actions */
function valueEditor(n, path, s, actions) {
  const set = (patch) => actions.builderSet(path, patch);
  const cmpSel = (labels, name = 'cmp') => select({ label: copy.search.valueLabel, value: n[name], options: opts(CMPS, labels), fk: key(path, name), onChange: (v) => set({ [name]: v }) });
  switch (n.field) {
    case 'name': case 'path':
      return [select({ label: copy.search.fieldLabel, value: n.mode, options: opts(TEXT_MODES, copy.search.modes), fk: key(path, 'mode'), onChange: (v) => set({ mode: v }) }),
        input('text', n, path, 'pattern', copy.search.pattern, actions, { placeholder: n.mode === 'wildcard' ? '*.tmp' : n.mode === 'regex' ? '^IMG_\\d+' : '' }),
        h('label', { class: 'check-inline' }, checkbox({ checked: !!n.case_sensitive, label: copy.search.caseSensitive, fk: key(path, 'case'), onChange: (on) => set({ case_sensitive: on }) }), copy.search.caseSensitive)];
    case 'extension':
      return [input('text', n, path, 'values', copy.search.extensionsLabel, actions, { placeholder: 'mp4, mkv' }),
        h('span', { class: 'ext-sets', role: 'group', 'aria-label': copy.search.extensionsLabel }, Object.keys(EXTENSION_SETS).map((k) => button({ label: copy.search.extensionSets[k], size: 'sm', variant: 'quiet', fk: key(path, `set-${k}`), onClick: () => set({ values: EXTENSION_SETS[k] }) })))];
    case 'type_group': {
      const groups = [...new Set([...TYPE_GROUPS, ...Object.keys(s.settings?.type_groups ?? {})])];
      return select({ label: copy.search.groupSelect, value: n.group, options: groups.map((g) => ({ value: g, label: g })), fk: key(path, 'group'), onChange: (v) => set({ group: v }) });
    }
    case 'size':
      return [cmpSel(copy.search.cmps), input('number', n, path, 'amount', copy.search.amount, actions, { min: '0', step: 'any' }),
        select({ label: copy.search.unit, value: n.unit, options: Object.keys(UNITS).map((u) => ({ value: u, label: u })), fk: key(path, 'unit'), onChange: (v) => set({ unit: v }) })];
    case 'date':
      return [select({ label: copy.search.fieldLabel, value: n.which, options: opts(DATE_FIELDS, copy.search.whichDate), fk: key(path, 'which'), onChange: (v) => set({ which: v }) }),
        select({ label: copy.search.dateMode.ago, value: n.mode, options: opts(['ago', 'at'], copy.search.dateMode), fk: key(path, 'mode'), onChange: (v) => set({ mode: v }) }),
        cmpSel(n.mode === 'ago' ? copy.search.agoCmps : copy.search.dateCmps),
        n.mode === 'ago'
          ? [input('number', n, path, 'amount', copy.search.amount, actions, { min: '0', step: '1' }), select({ label: copy.search.unit, value: n.unit, options: opts(SPAN_UNITS, copy.search.spanUnits), fk: key(path, 'unit'), onChange: (v) => set({ unit: v }) })]
          : input('text', n, path, 'date', copy.search.dateLabel, actions, { placeholder: 'YYYY-MM-DD' })];
    case 'attribute':
      return h('span', { class: 'attrs', role: 'group', 'aria-label': copy.search.attrHelp }, ATTRIBUTES.map((a) => {
        const st = n.attrs?.[a] ?? 'any';
        const next = st === 'any' ? 'set' : st === 'set' ? 'clear' : 'any';
        return h('button', { type: 'button', class: ['attr', `attr--${st}`], 'data-state': st, 'aria-label': `${copy.search.attrs[a]}: ${copy.search.attrStates[st]}`, title: copy.search.attrHelp, 'data-fk': key(path, `attr-${a}`), on: { click: () => set({ attrs: { ...n.attrs, [a]: next === 'any' ? undefined : next } }) } }, `${copy.search.attrs[a]} `, h('span', { class: 'attr__state' }, copy.search.attrStates[st]));
      }));
    case 'owner': return input('text', n, path, 'value', copy.search.ownerLabel, actions, { placeholder: 'uid:1000' });
    case 'depth': case 'path_length': return [cmpSel(copy.search.numCmps), input('number', n, path, 'value', copy.search.valueLabel, actions, { min: '0', step: '1' })];
    case 'kind': return select({ label: copy.search.valueLabel, value: n.kind_value, options: opts(ENTRY_KINDS, copy.search.kinds), fk: key(path, 'kind'), onChange: (v) => set({ kind_value: v }) });
    case 'content_contains':
      return [input('text', n, path, 'text', copy.search.pattern, actions), h('label', { class: 'check-inline' }, checkbox({ checked: !!n.case_sensitive, label: copy.search.caseSensitive, fk: key(path, 'case'), onChange: (on) => set({ case_sensitive: on }) }), copy.search.caseSensitive), h('span', { class: 'note' }, copy.search.contentSlow)];
    case 'largest_n': return [input('number', n, path, 'n', copy.search.valueLabel, actions, { min: '1', step: '1' }), h('span', { class: 'note' }, copy.search.largestNote)];
    default: return FLAG_FIELDS.includes(n.field) ? null : h('span', { class: 'note' }, copy.search.fields.raw);
  }
}

/** @param {any} n @param {number[]} path @param {any} s @param {any} actions */
function condRow(n, path, s, actions) {
  const problems = s.custom.builderErrors.filter((e) => same(e.path, path));
  return h('li', { class: 'brow', 'data-path': path.join('.') },
    h('div', { class: 'brow__main' },
      h('label', { class: 'check-inline brow__not' }, checkbox({ checked: n.not, label: copy.search.not, fk: key(path, 'not'), onChange: (on) => actions.builderSet(path, { not: on }) }), copy.search.not),
      select({ label: copy.search.fieldLabel, value: n.field === 'raw' ? 'raw' : n.field, options: [...FIELDS, ...(n.field === 'raw' ? ['raw'] : [])].map((f) => ({ value: f, label: copy.search.fields[f] })), fk: key(path, 'field'), onChange: (v) => actions.builderField(path, v) }),
      h('span', { class: 'brow__value' }, valueEditor(n, path, s, actions)),
      iconButton({ icon: 'close', label: copy.search.removeLabel, fk: key(path, 'rm'), onClick: () => actions.builderRemove(path) })),
    problems.map((p) => h('p', { class: 'brow__error', role: 'alert' }, copy.search.problems[p.code] ?? p.code)));
}

/** @param {any} g @param {number[]} path @param {any} s @param {any} actions */
function group(g, path, s, actions) {
  const root = path.length === 0;
  const problems = s.custom.builderErrors.filter((e) => same(e.path, path));
  return h('div', { class: ['bgroup', !root && 'bgroup--nested'], role: 'group', 'aria-label': copy.search.groupLabel },
    h('div', { class: 'bgroup__head' },
      !root && h('label', { class: 'check-inline' }, checkbox({ checked: g.not, label: copy.search.not, fk: key(path, 'not'), onChange: (on) => actions.builderSet(path, { not: on }) }), copy.search.not),
      select({ label: copy.search.groupLabel, value: g.op, options: [{ value: 'all', label: copy.search.matchAll }, { value: 'any', label: copy.search.matchAny }], fk: key(path, 'op'), onChange: (v) => actions.builderSet(path, { op: v }) }),
      h('span', { class: 'bgroup__add' },
        button({ label: copy.search.addCond, size: 'sm', fk: key(path, 'add'), onClick: () => actions.builderAdd(path, 'cond', 'name') }),
        button({ label: copy.search.addGroup, size: 'sm', fk: key(path, 'addg'), onClick: () => actions.builderAdd(path, 'group') }),
        !root && iconButton({ icon: 'close', label: copy.search.removeLabel, fk: key(path, 'rm'), onClick: () => actions.builderRemove(path) }))),
    problems.map((p) => h('p', { class: 'brow__error', role: 'alert' }, copy.search.problems[p.code] ?? p.code)),
    h('ul', { class: 'bgroup__list' }, g.children.map((c, i) => (c.kind === 'group' ? h('li', {}, group(c, [...path, i], s, actions)) : condRow(c, [...path, i], s, actions)))));
}

/** @param {any} s @param {any} actions */
export const renderBuilder = (s, actions) => h('div', { class: 'builder' }, group(s.custom.builder, [], s, actions));
