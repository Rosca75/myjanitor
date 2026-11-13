// @ts-check
/** Report of one run ("View report" / "Last report"). */
import { h } from '../../dom.js';
import { copy } from '../../copy.js';
import { formatBytes, formatCount, formatDate } from '../../format.js';
import { button } from '../../components/controls.js';
import { modal } from '../../components/modal.js';

/** @param {any} s @param {any} actions */
export function renderReport(s, actions) {
  const r = s.report;
  const t = r.totals;
  const stat = (label, value) => h('div', { class: 'stat' }, h('span', { class: 'stat__label' }, label), h('strong', { class: 'stat__value' }, value));
  const children = [
    h('h2', { class: 'modal__title', id: 'report-title' }, r.title),
    h('p', { class: 'modal__body' }, formatDate(r.started)),
    h('div', { class: 'stats' }, stat('Files', formatCount(t.files)), stat('Done', formatCount(t.succeeded)), stat('Skipped', formatCount(t.skipped)), stat('Saved', formatBytes(t.bytes_saved))),
    h('ul', { class: 'report__sections' }, r.sections.map((sec) => h('li', {}, sec.title))),
    h('div', { class: 'modal__foot' }, h('span', { class: 'modal__spacer' }), button({ label: copy.common.close, variant: 'secondary', fk: 'report-close', onClick: () => actions.closeReport() })),
  ];
  return modal({ labelId: 'report-title', title: r.title, children, onClose: () => actions.closeReport() });
}
