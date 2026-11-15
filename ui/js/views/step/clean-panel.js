// @ts-check
/** Right panel "What happens to them" for clean and archive steps. */
import { h } from '../../dom.js';
import { copy } from '../../copy.js';
import { formatBytes } from '../../format.js';
import { button, radioTile, select, toggle } from '../../components/controls.js';
import { previewPane, compareButton } from './preview-pane.js';

const BASIC = ['trash', 'quarantine', 'move', 'copy'];
const TO_FOLDER = ['move', 'copy'];
/** CLN-ACT-06 link actions and the app_info capability each needs. */
const LINKS = [['hardlink', 'hardlinks'], ['reflink', 'reflinks']];

/** Which action is in force: the plan step's, else the one picked before adding. @param {any} s */
function currentAction(s) {
  const step = s.plan.steps.find((x) => x.opportunity === s.step.id);
  return step?.action ?? s.step.pendingAction ?? s.step.detail.opportunity.default_action;
}

/** The keep rule as a select value (the engine may send a bare name or the tagged object). @param {any} v */
const keepValue = (v) => (typeof v === 'object' && v ? v.rule : v) ?? 'newest';

/**
 * CLN-ACT-02: the label and help of a clean action tile; the default route is named after
 * where it really goes (`app_info.trash_route`). @param {string} kind @param {any} info
 */
export function actionLabel(kind, info) {
  return (kind === 'trash' && copy.trashRoute[info?.trash_route]) || copy.clean.actions[kind];
}

/** @param {any} s @param {any} actions */
export function renderCleanPanel(s, actions) {
  const st = s.step;
  const opp = st.detail.opportunity;
  const sum = st.detail.summary;
  const act = currentAction(s);
  const offered = opp.available_actions;
  const label = (kind) => actionLabel(kind, s.info);
  // CLN-ACT-05: Move and Copy pick their destination in the in-app folder browser.
  const tile = (kind) => radioTile({ title: label(kind).label, sub: TO_FOLDER.includes(kind) && act.kind === kind && act.dest ? act.dest : label(kind).help, selected: act.kind === kind, fk: `act-${kind}`, onSelect: () => (TO_FOLDER.includes(kind) ? actions.openBrowser({ mode: kind }) : actions.setStepAction(kind)) });
  const structure = TO_FOLDER.includes(act.kind) && act.dest && h('div', { class: 'setting' },
    h('label', { class: 'setting__text' }, h('span', { class: 'setting__label' }, copy.clean.keepStructure), h('span', { class: 'setting__help' }, copy.clean.keepStructureHelp)),
    toggle({ label: copy.clean.keepStructure, checked: act.keep_structure !== false, fk: 'keep-structure', onChange: (on) => actions.setStepAction(act.kind, act.dest, on) }));
  const dup = opp.id === 'det:exact_duplicates';
  return h('div', { class: 'shrink clean' },
    h('h2', { class: 'panel__title' }, copy.clean.title),
    BASIC.some((k) => offered.includes(k))
      ? h('div', { class: 'tiles', role: 'radiogroup', 'aria-label': copy.clean.action }, BASIC.filter((k) => offered.includes(k)).map(tile))
      : h('p', { class: 'note' }, opp.description),
    structure,
    LINKS.some(([k]) => offered.includes(k)) && h('div', { class: 'setting' },
      h('p', { class: 'kv__k', id: 'link-label' }, copy.clean.linkTitle),
      h('p', { class: 'setting__help' }, copy.clean.linkIntro),
      h('div', { class: 'tiles', role: 'radiogroup', 'aria-labelledby': 'link-label' },
        LINKS.filter(([k]) => offered.includes(k)).map(([k, cap]) => {
          const can = !!s.info?.capabilities?.includes(cap);
          return radioTile({ title: copy.clean.actions[k].label, sub: can ? copy.clean.actions[k].help : copy.clean.actions[k].unavailable, selected: act.kind === k, disabled: !can, fk: `act-${k}`, onSelect: () => actions.setStepAction(k) });
        }))),
    dup && h('div', { class: 'setting' },
      h('label', { class: 'setting__text' }, h('span', { class: 'setting__label' }, copy.clean.keep), h('span', { class: 'setting__help' }, copy.clean.keepHelp)),
      select({ label: copy.clean.keep, value: keepValue(opp.params.keep_rule), fk: 'keep-rule', options: [...Object.entries(copy.clean.keepRules).map(([value, label]) => ({ value, label })), { value: 'preferred_folder', label: copy.step2.keepFolder }], onChange: (v) => (v === 'preferred_folder' ? actions.openBrowser({ mode: 'keep' }) : actions.applyKeepRule(v)) })),
    previewPane(s, actions),
    opp.category === 'optimise' && compareButton(s, actions),
    h('div', { class: 'estimate', role: 'status', 'aria-live': 'polite' }, h('span', { class: 'estimate__label' }, copy.step.thisStep), h('strong', { class: 'estimate__value', 'data-role': 'step-estimate' }, sum.size_after > 0 ? copy.step.estimateLine(formatBytes(sum.size_before), formatBytes(sum.size_after), formatBytes(sum.saving)) : copy.step.estimateRemove(formatBytes(sum.saving)))),
    h('div', { class: 'advanced' },
      h('button', { type: 'button', class: 'disclosure', 'aria-expanded': String(st.advancedOpen), 'data-fk': 'adv-toggle', on: { click: () => actions.toggleAdvanced() } }, st.advancedOpen ? copy.shrink.hideAdvanced : copy.shrink.showAdvanced),
      st.advancedOpen && h('div', { class: 'advanced__body' },
        button({ label: copy.clean.reset, variant: 'secondary', block: true, fk: 'reset-selection', onClick: () => actions.resetSelection() }),
        offered.includes('permanent_delete') && h('div', { class: 'tiles tiles--danger', role: 'radiogroup', 'aria-label': copy.clean.advanced },
          radioTile({ title: copy.clean.actions.permanent_delete.label, sub: copy.clean.actions.permanent_delete.help, selected: act.kind === 'permanent_delete', fk: 'act-permanent_delete', onSelect: () => actions.setStepAction('permanent_delete') })))));
}
