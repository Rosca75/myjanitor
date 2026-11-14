// @ts-check
/** Inline folder picker (D-104: the in-app browser through `fs_list_dir`, no native dialog). */
import { h, icon } from '../../dom.js';
import { button } from '../../components/controls.js';
import { displayPath } from '../../format.js';
import { sc } from './copy.js';

const SHOWN = 12;

/** @param {any} ctl */
export function renderPicker(ctl) {
  const p = ctl.st.picker;
  if (!p) return null;
  const shown = p.entries.slice(0, p.showAll ? undefined : SHOWN);
  return h('div', { class: 'browser', role: 'group', 'aria-label': sc.picker.title, 'data-picker': p.purpose },
    h('div', { class: 'browser__places', role: 'group', 'aria-label': sc.picker.places },
      p.places.map((pl) => button({ label: pl.label, variant: 'quiet', size: 'sm', fk: `pick-place-${pl.label}`, extra: 'browser__place', onClick: () => ctl.pickerTo(pl.path) }))),
    h('div', { class: 'browser__main' },
      h('div', { class: 'browser__path', 'aria-label': sc.picker.current },
        h('span', { class: 'browser__current', title: p.path }, displayPath(p.path) || sc.picker.loading),
        button({ label: sc.picker.up, variant: 'secondary', size: 'sm', icon: 'up', disabled: !p.parent, fk: 'pick-up', onClick: () => ctl.pickerTo(p.parent) })),
      shown.length
        ? h('ul', { class: 'browser__list' }, shown.map((e) => h('li', {}, h('button', { type: 'button', class: 'browser__entry', 'data-fk': `pick-dir-${e.name}`, on: { click: () => ctl.pickerTo(e.path) } }, icon('folder', 14), h('span', {}, displayPath(e.name))))))
        : h('p', { class: 'browser__empty' }, p.loading ? sc.picker.loading : sc.picker.empty),
      p.entries.length > SHOWN && button({ label: p.showAll ? sc.less : sc.more, variant: 'link', size: 'sm', fk: 'pick-more', onClick: () => { p.showAll = !p.showAll; ctl.rerender(); } })),
    h('div', { class: 'modal__foot' }, h('span', { class: 'modal__spacer' }),
      button({ label: sc.cancel, variant: 'secondary', fk: 'pick-cancel', onClick: () => ctl.closePicker() }),
      button({ label: sc.picker.choose, variant: 'primary', disabled: !p.path, fk: 'pick-choose', onClick: () => ctl.pickerChoose() })));
}
