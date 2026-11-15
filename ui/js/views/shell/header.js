// @ts-check
/** Top header: title and scope chips (Overview), breadcrumb (step review). */
import { h, icon } from '../../dom.js';
import { copy } from '../../copy.js';
import { formatAgo } from '../../format.js';
import { button, iconButton } from '../../components/controls.js';
import { safetyBadge, inPlanBadge } from '../../components/badge.js';
import { isScanning, planSaving, stepFor } from '../../state.js';
import { formatBytes } from '../../format.js';

/** @param {any} s @param {any} actions @param {{ removable?: boolean }} [o] */
function scopeChips(s, actions, o = {}) {
  return h('ul', { class: 'chips', 'aria-label': copy.header.scopeLabel },
    s.scope.roots.map((root) => h('li', { class: 'chip' }, icon('folder', 14), h('span', { class: 'chip__text', title: root }, root),
      o.removable && iconButton({ icon: 'close', label: copy.header.removeLocation(root), fk: `rm-${root}`, extra: 'chip__x', onClick: () => actions.removeRoot(root) }))));
}

/** @param {any} s @param {any} actions */
function stepHeader(s, actions) {
  const opp = s.step?.detail?.opportunity;
  const inPlan = opp ? !!stepFor(s, opp.id) : false;
  const lossy = s.step?.detail?.lossy;
  return h('header', { class: 'header header--step' },
    h('nav', { class: 'crumbs', 'aria-label': 'Breadcrumb' },
      h('button', { type: 'button', class: 'crumbs__back', 'aria-label': copy.header.backLabel, 'data-fk': 'back', on: { click: () => actions.back() } }, icon('chevronLeft', 14), copy.header.back),
      h('span', { class: 'crumbs__sep', 'aria-hidden': 'true' }, '/'),
      h('h1', { class: 'header__title' }, opp?.title ?? copy.common.loading)),
    h('div', { class: 'header__badges' }, inPlan && inPlanBadge(), opp && safetyBadge(lossy ? 'lossy_opt_in' : s.step.detail.safety)),
    h('div', { class: 'header__right' }, opp && opp.safety !== 'review_only' && (inPlan
      ? button({ label: copy.step.applyPlan(formatBytes(planSaving(s))), variant: 'primary', fk: 'step-apply', onClick: () => actions.startApply() })
      : button({ label: copy.step.addToPlan, variant: 'primary', fk: 'step-add', onClick: () => actions.addStepToPlan() }))));
}

/** @param {any} s @param {any} actions @param {number} [nowMs] */
export function renderHeader(s, actions, nowMs = Date.now()) {
  const r = s.route.name;
  if (r === 'step') return stepHeader(s, actions);
  if (r === 'disk') {
    return h('header', { class: 'header header--step' },
      h('nav', { class: 'crumbs', 'aria-label': 'Breadcrumb' },
        h('button', { type: 'button', class: 'crumbs__back', 'aria-label': copy.header.backLabel, 'data-fk': 'back', on: { click: () => actions.navigate('overview') } }, icon('chevronLeft', 14), copy.disk.back),
        h('span', { class: 'crumbs__sep', 'aria-hidden': 'true' }, '/'),
        h('h1', { class: 'header__title' }, copy.disk.title)));
  }
  if (r !== 'overview') return h('header', { class: 'header' }, h('h1', { class: 'header__title' }, copy.nav[r] ?? copy.appName));
  const scanning = isScanning(s);
  const scanned = s.opps.scanned_at && !scanning;
  return h('header', { class: 'header' },
    h('h1', { class: 'header__title' }, copy.nav.overview),
    scanned && scopeChips(s, actions),
    scanned && button({ label: copy.header.addLocation, variant: 'link', fk: 'add-location', onClick: () => actions.openBrowser() }),
    h('div', { class: 'header__right' },
      scanned && h('span', { class: 'header__scanned' }, copy.header.scanned(formatAgo(s.opps.scanned_at, nowMs), s.opps.cached)),
      scanned && button({ label: copy.header.scanAgain, variant: 'secondary', fk: 'scan-again', onClick: () => actions.startScan() })));
}
