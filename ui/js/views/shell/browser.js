// @ts-check
/** In-app folder browser (D-104: no native dialog plugin). */
import { h, icon } from '../../dom.js';
import { copy } from '../../copy.js';
import { displayPath } from '../../format.js';
import { button } from '../../components/controls.js';
import { modal } from '../../components/modal.js';

/** @param {any} s @param {any} actions */
export function renderBrowser(s, actions) {
  const b = s.browser;
  const children = [
    h('h2', { class: 'modal__title', id: 'browser-title' }, copy.browser.title),
    h('div', { class: 'browser' },
      h('div', { class: 'browser__places', role: 'group', 'aria-label': copy.browser.places },
        b.places.map((p) => button({ label: p.label, variant: 'quiet', size: 'sm', fk: `place-${p.kind}-${p.label}`, extra: 'browser__place', onClick: () => actions.browseTo(p.path) }))),
      h('div', { class: 'browser__main' },
        h('div', { class: 'browser__path', 'aria-label': copy.browser.current }, h('span', { class: 'browser__current', title: b.path }, displayPath(b.path) || copy.browser.loading),
          button({ label: copy.browser.up, variant: 'secondary', size: 'sm', icon: 'up', disabled: !b.parent, fk: 'browser-up', onClick: () => actions.browseTo(b.parent) })),
        b.entries.length
          ? h('ul', { class: 'browser__list' }, b.entries.filter((e) => e.is_dir).map((e) => h('li', {}, h('button', { type: 'button', class: 'browser__entry', 'data-fk': `dir-${e.name}`, on: { click: () => actions.browseTo(e.path) } }, icon('folder', 14), h('span', {}, displayPath(e.name))))))
          : h('p', { class: 'browser__empty' }, b.loading ? copy.browser.loading : copy.browser.empty))),
    b.mode === 'export' && h('label', { class: 'browser__name' }, h('span', {}, copy.browser2.fileName),
      h('input', { type: 'text', class: 'input', value: b.filename, 'aria-label': copy.browser2.fileName, 'data-fk': 'export-name', on: { change: (e) => actions.setExportName(e.target.value), input: (e) => actions.setExportName(e.target.value) } })),
    h('div', { class: 'modal__foot' }, h('span', { class: 'modal__spacer' }),
      button({ label: copy.browser.cancel, variant: 'secondary', fk: 'browser-cancel', onClick: () => actions.closeBrowser() }),
      button({ label: b.mode === 'export' ? copy.browser2.save : copy.browser.choose, variant: 'primary', disabled: !b.path || (b.mode === 'export' && !String(b.filename).trim()), fk: 'browser-choose', onClick: () => actions.chooseFolder() })),
  ];
  return modal({ labelId: 'browser-title', title: copy.browser.title, children, extra: 'modal--browser', onClose: () => actions.closeBrowser() });
}
