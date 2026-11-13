// @ts-check
/** Clean: "Custom search" entry with a quick-query box (CLN-ENG-05). */
import { h } from '../../dom.js';
import { copy } from '../../copy.js';
import { button } from '../../components/controls.js';

/** @param {any} s @param {any} actions */
export function renderCustomSearch(s, actions) {
  const c = s.custom;
  const bad = c.errors.length > 0;
  return h('section', { class: 'custom', 'aria-labelledby': 'custom-title' },
    h('h3', { class: 'custom__title', id: 'custom-title' }, copy.category.custom),
    h('p', { class: 'custom__intro' }, copy.category.customIntro),
    h('form', { class: 'custom__form', on: { submit: (e) => { e.preventDefault(); actions.runSearch(); } } },
      h('input', { type: 'search', class: ['input', bad && 'input--invalid'], value: c.query, placeholder: copy.category.customPlaceholder, 'aria-label': copy.category.customLabel, 'aria-invalid': bad ? 'true' : undefined, 'aria-describedby': 'custom-help', 'data-fk': 'custom-query', on: { input: (e) => actions.setQuery(e.target.value) } }),
      button({ label: copy.category.customRun, variant: 'secondary', fk: 'custom-run', onClick: (e) => { e?.preventDefault?.(); actions.runSearch(); } })),
    h('p', { class: ['custom__help', bad && 'custom__help--error'], id: 'custom-help' }, bad ? `${copy.category.customError} ${c.errors.map((x) => x.message).join('. ')}` : copy.category.customHelp),
    c.result && h('p', { class: 'custom__result', role: 'status' }, copy.category.customResult(c.result.title, c.result.count), ' ',
      button({ label: copy.card.review, variant: 'link', size: 'sm', fk: 'custom-review', ariaLabel: copy.card.reviewLabel(c.result.title), onClick: () => actions.openStep(c.result.id) })));
}
