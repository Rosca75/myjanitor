// @ts-check
/**
 * Settings drawer. Sections: General, Scanning, Protected paths, Exclusions,
 * Safety, Performance, Engines, Automation. Settings values come from the shell
 * store (`s.settings`, `s.cache`, `s.tools`, `s.info`); the drawer's own state
 * and every write go through `controller.js`.
 */
import { h } from '../../dom.js';
import { sc } from './copy.js';
import { settingsController } from './controller.js';
import { general, scanning, protectedPaths, exclusions } from './sections-basic.js';
import { safety, performance } from './sections-safety.js';
import { engines, automation } from './sections-auto.js';

/** @param {any} s store state @param {any} ctl */
function build(s, ctl, actions) {
  return h('div', { class: 'view__body' },
    h('section', { class: 'intro' }, h('h2', { class: 'intro__title' }, sc.title), h('p', { class: 'intro__text' }, sc.intro)),
    general(s, ctl), scanning(s, ctl, actions), protectedPaths(s, ctl), exclusions(s, ctl),
    safety(s, ctl), performance(s, ctl), engines(s, ctl), automation(s, ctl));
}

/** @param {any} s @param {any} actions */
export function renderSettings(s, actions) {
  if (!s.settings) return h('div', { class: 'view' }, h('p', { class: 'empty-note' }, sc.loading));
  const ctl = settingsController(actions);
  const host = h('div', { class: 'view view--settings' });
  const draw = (state) => build({ ...state, tools: state.tools ?? [] }, ctl, actions);
  ctl.bind(host, s, draw);
  host.appendChild(draw(s));
  ctl.ensureLoaded();
  return host;
}
