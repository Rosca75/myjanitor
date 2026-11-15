// @ts-check
import { renderCategory } from '../category.js';
import { renderArchiveParams } from './params.js';

/** Archive view: the cold-folder parameters, then the cards. @param {any} s @param {any} actions */
export function renderArchive(s, actions) {
  const opp = s.category.name === 'archive' ? s.category.items.find((o) => o.default_action?.kind === 'archive') : null;
  return renderCategory(s, actions, 'archive', opp ? [renderArchiveParams(opp, s, actions)] : []);
}
