// @ts-check
/** Settings: a drawer skeleton fed by the engine's settings (completed in Wave 3). */
import { h } from '../../dom.js';
import { copy } from '../../copy.js';
import { formatCount, formatBytes } from '../../format.js';
import { button } from '../../components/controls.js';
import { badge } from '../../components/badge.js';

/** @param {string} label @param {any} value */
const row = (label, value) => h('div', { class: 'kv' }, h('dt', { class: 'kv__k' }, label), h('dd', { class: 'kv__v' }, value));

/** @param {any} s @param {any} actions */
export function renderSettings(s, actions) {
  const st = s.settings;
  if (!st) return h('div', { class: 'view' }, h('p', { class: 'empty-note' }, copy.common.loading));
  return h('div', { class: 'view view--settings' },
    h('section', { class: 'intro' }, h('h2', { class: 'intro__title' }, copy.settings.title), h('p', { class: 'intro__text' }, copy.settings.intro)),
    s.info && !s.info.journal_persistent && h('p', { class: 'notice notice--warning', role: 'note' }, copy.settings.journalWarning),
    h('section', { class: 'drawer', 'aria-labelledby': 'set-general' }, h('h3', { id: 'set-general', class: 'drawer__title' }, copy.settings.general),
      h('dl', { class: 'kvs' }, row(copy.settings.undoDays, copy.settings.days(st.general.undo_days)), row(copy.settings.quarantineDays, copy.settings.days(st.general.quarantine_days)), row(copy.settings.defaultOriginals, copy.settings.originals[st.general.default_originals.policy] ?? st.general.default_originals.policy))),
    h('section', { class: 'drawer', 'aria-labelledby': 'set-cache' }, h('h3', { id: 'set-cache', class: 'drawer__title' }, copy.settings.cache),
      h('p', {}, s.cache ? copy.settings.cacheLine(formatCount(s.cache.entries), formatBytes(s.cache.bytes)) : ''),
      button({ label: copy.settings.clearCache, variant: 'secondary', size: 'sm', fk: 'clear-cache', onClick: () => actions.clearCache() })),
    h('section', { class: 'drawer', 'aria-labelledby': 'set-tools' }, h('h3', { id: 'set-tools', class: 'drawer__title' }, copy.settings.tools),
      h('ul', { class: 'tools' }, s.tools.map((t) => h('li', { class: 'tool' }, h('strong', {}, t.id), t.found ? badge(copy.settings.found, 'success') : badge(copy.settings.missing, 'neutral'), t.version && h('span', { class: 'tool__v' }, t.version))))),
    s.info && h('p', { class: 'note' }, copy.settings.version(s.info.version, s.info.platform)));
}
