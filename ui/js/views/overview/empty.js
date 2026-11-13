// @ts-check
/** B1 empty state: scope picker and one primary Scan button. */
import { h, icon } from '../../dom.js';
import { copy } from '../../copy.js';
import { button, iconButton } from '../../components/controls.js';

/** @param {any} s @param {any} actions */
export function renderEmpty(s, actions) {
  const roots = s.scope.roots;
  return h('div', { class: 'view view--empty' },
    h('section', { class: 'welcome' },
      h('h2', { class: 'welcome__title' }, copy.empty.title),
      h('p', { class: 'welcome__body' }, copy.empty.body),
      h('div', { class: 'picker' },
        h('h3', { class: 'picker__title' }, copy.empty.scopeHeading),
        roots.length
          ? h('ul', { class: 'chips chips--wrap', 'aria-label': copy.header.scopeLabel }, roots.map((r) => h('li', { class: 'chip' }, icon('folder', 14), h('span', { class: 'chip__text', title: r }, r), iconButton({ icon: 'close', label: copy.header.removeLocation(r), extra: 'chip__x', fk: `rm-${r}`, onClick: () => actions.removeRoot(r) }))))
          : h('p', { class: 'picker__none' }, copy.empty.noLocations),
        button({ label: copy.empty.add, variant: 'secondary', icon: 'plus', fk: 'add-folder', onClick: () => actions.openBrowser() })),
      h('div', { class: 'welcome__go' },
        button({ label: copy.empty.scan, variant: 'primary', size: 'lg', disabled: !roots.length, title: roots.length ? undefined : copy.empty.scanDisabled, fk: 'scan', onClick: () => actions.startScan() }),
        h('p', { class: 'welcome__safe' }, copy.empty.safe))));
}
