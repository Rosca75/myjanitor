// @ts-check
/** Right panel router: plan on Overview, step settings on the step review. */
import { h } from '../../dom.js';
import { renderPlanPanel } from '../overview/plan-panel.js';
import { renderStepPanel } from '../step/index.js';

/** Routes that show a right panel. @param {any} s */
export const hasPanel = (s) => s.route.name === 'overview' || s.route.name === 'step';

/** @param {any} s @param {any} actions */
export function renderPanel(s, actions) {
  if (s.route.name === 'step') return renderStepPanel(s, actions);
  if (s.route.name === 'overview') return renderPlanPanel(s, actions);
  return h('div', {});
}
