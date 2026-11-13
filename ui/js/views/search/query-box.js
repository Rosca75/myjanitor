// @ts-check
/** Quick-query box (CLN-ENG-05): autocomplete from search_suggest, parse errors with position. */
import { h } from '../../dom.js';
import { copy } from '../../copy.js';
import { segments, columnOf } from '../../lib/query.js';
import { button } from '../../components/controls.js';

/** The query with flawed parts underlined, so the position is visible. @param {any} c */
function marks(c) {
  return h('div', { class: 'query__marks', 'aria-hidden': 'true', 'data-role': 'query-marks' },
    segments(c.query, c.errors).map((seg) => (seg.error ? h('span', { class: 'query__bad', title: seg.error }, seg.text) : h('span', {}, seg.text))));
}

/** @param {any} s @param {any} actions */
export function renderQueryBox(s, actions) {
  const c = s.custom;
  const bad = c.errors.length > 0;
  const open = c.suggestions.length > 0;
  /** @param {any} e */
  const onKey = (e) => {
    if (open && e.key === 'ArrowDown') { e.preventDefault(); actions.moveSuggestion(1); }
    else if (open && e.key === 'ArrowUp') { e.preventDefault(); actions.moveSuggestion(-1); }
    else if (open && (e.key === 'Enter' || e.key === 'Tab')) { e.preventDefault(); actions.acceptSuggestion(); }
    else if (open && e.key === 'Escape') { e.preventDefault(); e.stopPropagation?.(); actions.closeSuggestions(); }
  };
  return h('div', { class: 'query' },
    h('form', { class: 'custom__form', on: { submit: (e) => { e.preventDefault(); actions.runSearch(); } } },
      h('div', { class: 'query__field' },
        h('input', {
          type: 'search', class: ['input', bad && 'input--invalid'], value: c.query, placeholder: copy.category.customPlaceholder, autocomplete: 'off', spellcheck: 'false',
          role: 'combobox', 'aria-expanded': String(open), 'aria-controls': 'query-suggestions', 'aria-autocomplete': 'list', 'aria-activedescendant': open && c.suggestIndex >= 0 ? `sugg-${c.suggestIndex}` : undefined,
          'aria-label': copy.category.customLabel, 'aria-invalid': bad ? 'true' : undefined, 'aria-describedby': 'custom-help', 'data-fk': 'custom-query',
          on: { input: (e) => actions.setQuery(e.target.value, e.target.selectionStart ?? undefined), keydown: onKey },
        }),
        open && h('ul', { class: 'suggest', id: 'query-suggestions', role: 'listbox', 'aria-label': copy.search.suggestions },
          c.suggestions.map((sg, i) => h('li', { class: ['suggest__item', i === c.suggestIndex && 'suggest__item--on'], role: 'option', id: `sugg-${i}`, 'aria-selected': String(i === c.suggestIndex), 'data-fk': `sugg-${i}`, on: { mousedown: (e) => { e.preventDefault(); actions.acceptSuggestion(i); } } }, sg.label)))),
      button({ label: c.running ? copy.search.searching : copy.search.search, variant: 'secondary', disabled: c.running, fk: 'custom-run', onClick: (e) => { e?.preventDefault?.(); actions.runSearch(); } })),
    bad && marks(c),
    h('p', { class: ['custom__help', bad && 'custom__help--error'], id: 'custom-help' }, bad ? `${copy.category.customError} ${c.errors.map((x) => x.message).join('. ')}` : copy.search.queryHelp),
    bad && h('ul', { class: 'query__errors', 'aria-label': copy.search.errorsLabel }, c.errors.map((e) => h('li', {}, copy.search.errorAt(columnOf(c.query, e), e.message)))),
    c.parsedRule && !bad && button({ label: copy.search.editInBuilder, variant: 'link', size: 'sm', fk: 'to-builder', onClick: () => actions.importQueryToBuilder() }));
}
