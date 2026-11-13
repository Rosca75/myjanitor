// @ts-check
/** Right panel "Your plan" (B1): reorderable steps, total, checklist, actions. */
import { h, icon, fk } from '../../dom.js';
import { copy } from '../../copy.js';
import { formatBytes } from '../../format.js';
import { button, iconButton } from '../../components/controls.js';
import { badge } from '../../components/badge.js';
import { activeJobs, isScanning, planSaving } from '../../state.js';

const LEVEL_ICON = { ok: 'check', warning: 'warning', info: 'info' };

/** @param {any} step @param {number} i @param {number} n @param {any} actions */
function stepRow(step, i, n, actions) {
  const saving = Math.max(0, step.estimate.bytes_before - step.estimate.bytes_after);
  /** @param {any} e */
  const onKey = (e) => {
    if (!e.altKey) return;
    if (e.key === 'ArrowUp') { e.preventDefault(); actions.moveStep(step.id, -1); }
    if (e.key === 'ArrowDown') { e.preventDefault(); actions.moveStep(step.id, 1); }
  };
  return h('li', { class: 'step', tabindex: '0', 'data-step': String(step.id), 'data-fk': `step-${step.id}`, 'aria-label': `${step.title}, ${formatBytes(saving)}`, on: { keydown: onKey } },
    h('span', { class: 'step__grip', 'aria-hidden': 'true' }, icon('grip', 12)),
    h('div', { class: 'step__main' },
      h('div', { class: 'step__top' }, h('span', { class: 'step__title', title: step.title }, step.title), step.lossy && badge(copy.plan.lossy, 'warning', { extra: 'badge--row' })),
      h('span', { class: 'step__sub', title: step.summary }, step.summary)),
    h('span', { class: ['step__size', step.estimate.confidence === 'exact' && 'step__size--exact'], title: step.estimate.confidence === 'exact' ? copy.plan.exact : copy.plan.estimatedTag }, formatBytes(saving)),
    h('div', { class: 'step__tools' },
      iconButton({ icon: 'up', label: copy.plan.up(step.title), disabled: i === 0, fk: `up-${step.id}`, onClick: () => actions.moveStep(step.id, -1) }),
      iconButton({ icon: 'down', label: copy.plan.down(step.title), disabled: i === n - 1, fk: `down-${step.id}`, onClick: () => actions.moveStep(step.id, 1) }),
      iconButton({ icon: 'close', label: copy.plan.remove(step.title), fk: `rm-${fk(step.id)}`, onClick: () => actions.removeStep(step.id) })));
}

/** @param {any} s @param {any} actions */
export function renderPlanPanel(s, actions) {
  const steps = s.plan.steps;
  const saving = planSaving(s);
  const busy = activeJobs(s).some((j) => j.kind === 'apply') || isScanning(s);
  const lossyOrPermanent = steps.some((x) => x.safety === 'lossy_opt_in' || x.safety === 'permanent');
  return h('div', { class: 'plan' },
    h('div', { class: 'plan__head' }, h('h2', { class: 'panel__title' }, copy.plan.title), h('span', { class: 'plan__count', 'aria-live': 'polite' }, copy.plan.steps(steps.length))),
    h('div', { class: 'plan__body' },
      steps.length
        ? h('ol', { class: 'steps', 'aria-label': copy.plan.stepsLabel }, steps.map((st, i) => stepRow(st, i, steps.length, actions)))
        : h('p', { class: 'plan__empty' }, copy.plan.empty),
      steps.length > 1 && h('p', { class: 'plan__hint' }, copy.plan.reorderHint),
      h('div', { class: 'plan__total' }, h('span', { class: 'plan__total-label' }, copy.plan.estimated), h('strong', { class: 'plan__total-value', 'aria-live': 'polite', 'data-role': 'plan-total' }, formatBytes(saving))),
      s.plan.checklist.length > 0 && h('section', { class: 'check', 'aria-label': copy.plan.before },
        h('h3', { class: 'eyebrow' }, copy.plan.before),
        h('ul', { class: 'check__list' }, s.plan.checklist.map((c) => h('li', { class: ['check__item', `check__item--${c.level}`] },
          h('span', { class: 'check__icon', 'aria-hidden': 'true' }, icon(LEVEL_ICON[c.level] ?? 'info', 12)), h('span', {}, c.text)))))),
    h('div', { class: 'plan__actions' },
      button({ label: s.dryRunning ? copy.plan.estimating : copy.plan.estimate, variant: 'secondary', block: true, disabled: !steps.length || s.dryRunning, fk: 'dry-run', onClick: () => actions.dryRun() }),
      button({ label: copy.plan.apply(formatBytes(saving)), variant: 'primary', size: 'lg', block: true, disabled: !steps.length || busy, title: steps.length ? undefined : copy.plan.applyDisabled, fk: 'apply', onClick: () => actions.startApply() }),
      button({ label: copy.plan.schedule, variant: 'link', block: true, disabled: !steps.length || lossyOrPermanent, title: lossyOrPermanent ? copy.plan.scheduleLossy : undefined, fk: 'schedule', onClick: () => actions.saveSchedule() })));
}
