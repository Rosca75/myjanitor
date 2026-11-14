// @ts-check
/** General, Scanning, Protected paths and Exclusions. */
import { h } from '../../dom.js';
import { button, toggle } from '../../components/controls.js';
import { badge } from '../../components/badge.js';
import { formatBytes, formatCount, displayPath } from '../../format.js';
import { sc } from './copy.js';
import { advanced, removableList, row, rows, section, textInput, errorLine } from './ui.js';
import { renderPicker } from './picker.js';

/** @param {any} s @param {any} ctl */
export function general(s, ctl) {
  const info = s.info;
  return section('general', sc.general.title,
    h('p', { class: 'note', 'data-role': 'offline' }, sc.general.offline),
    info && h('p', { class: 'note', 'data-role': 'mode' }, info.portable ? sc.general.portable : sc.general.installed),
    info && rows(row(ctl, 'general', sc.general.dataFolder, h('span', { class: 'tile__sub', title: info.data_dir }, displayPath(info.data_dir)))),
    info && !info.journal_persistent && h('p', { class: 'notice notice--warning', role: 'note' }, sc.general.journalWarning),
    info && h('p', { class: 'note' }, sc.general.version(info.version, info.platform)));
}

/** @param {any} s @param {any} ctl */
export function scanning(s, ctl, actions) {
  const o = s.settings.scan;
  const set = (k) => (on) => ctl.commit(k, (n) => { n.scan[k] = on; });
  return section('scan', sc.scan.title, rows(
    row(ctl, 'use_fast_index', sc.scan.fastIndex, toggle({ label: sc.scan.fastIndex, checked: o.use_fast_index, onChange: set('use_fast_index'), fk: 'set-fast-index' }), { help: sc.scan.fastIndexHelp }),
    row(ctl, 'use_cache', sc.scan.cache, toggle({ label: sc.scan.cache, checked: o.use_cache, onChange: set('use_cache'), fk: 'set-use-cache' }), { help: sc.scan.cacheHelp }),
    row(ctl, 'cache', s.cache ? sc.scan.cacheLine(formatCount(s.cache.entries), formatBytes(s.cache.bytes)) : sc.loading, button({ label: sc.scan.clearCache, variant: 'secondary', size: 'sm', fk: 'clear-cache', onClick: () => actions.clearCache() }))),
    advanced(ctl, 'scan',
      row(ctl, 'include_hidden', sc.scan.includeHidden, toggle({ label: sc.scan.includeHidden, checked: o.include_hidden, onChange: set('include_hidden'), fk: 'set-hidden' })),
      row(ctl, 'one_file_system', sc.scan.oneFileSystem, toggle({ label: sc.scan.oneFileSystem, checked: o.one_file_system, onChange: set('one_file_system'), fk: 'set-one-fs' }))));
}

/** @param {any} s @param {any} ctl */
export function protectedPaths(s, ctl) {
  const p = ctl.st.prot;
  const user = p?.user ?? s.settings.safety.protected_paths;
  const all = p ? [...p.builtin, ...p.builtin_patterns] : [];
  const showAll = !!ctl.st.open.builtin;
  return section('protected', sc.protectedPaths.title,
    h('p', { class: 'tile__sub' }, sc.protectedPaths.intro),
    p && h('div', { class: 'kvs', 'data-role': 'builtin' },
      h('div', { class: 'kv' }, h('span', { class: 'kv__k' }, sc.protectedPaths.builtin), badge(sc.locked, 'neutral'), h('span', { class: 'tile__sub' }, sc.protectedPaths.builtinCount(p.builtin.length, p.builtin_patterns.length))),
      h('ul', { class: 'tools' }, all.slice(0, showAll ? undefined : 6).map((v) => h('li', { class: 'tool' }, badge(sc.locked, 'neutral'), h('span', { class: 'kv__k', title: v }, v)))),
      all.length > 6 && button({ label: showAll ? sc.less : sc.more, variant: 'link', size: 'sm', fk: 'builtin-more', onClick: () => ctl.toggleOpen('builtin') })),
    h('div', { class: 'kv' }, h('span', { class: 'kv__k' }, sc.protectedPaths.yours)),
    removableList(user, 'rm-protected', `${sc.remove} {}`, (v) => ctl.setProtected(user.filter((x) => x !== v)), sc.protectedPaths.none),
    ctl.st.picker?.purpose === 'protected' ? renderPicker(ctl) : button({ label: sc.protectedPaths.addFolder, variant: 'secondary', size: 'sm', icon: 'plus', fk: 'add-protected', onClick: () => ctl.openPicker('protected') }),
    errorLine(ctl, 'protected'), ctl.st.picker?.purpose === 'protected' && errorLine(ctl, 'picker'));
}

/** @param {any} s @param {any} ctl */
export function exclusions(s, ctl) {
  const list = s.settings.safety.exclusions;
  let input = null;
  const add = () => ctl.addExclusion(input?.value ?? '');
  input = h('input', { type: 'text', class: ['input', ctl.st.errors.exclusions && 'input--invalid'], placeholder: sc.exclusions.placeholder, 'aria-label': sc.exclusions.label, 'data-fk': 'exclusion-input', on: { keydown: (e) => { if (e.key === 'Enter') add(); } } });
  return section('exclusions', sc.exclusions.title,
    h('p', { class: 'tile__sub' }, sc.exclusions.intro),
    removableList(list, 'rm-excl', `${sc.remove} {}`, (v) => ctl.removeExclusion(v), sc.exclusions.none),
    h('div', { class: 'custom__form' }, input, button({ label: sc.add, variant: 'secondary', size: 'sm', fk: 'add-exclusion', onClick: add })),
    errorLine(ctl, 'exclusions'));
}

export { textInput };
