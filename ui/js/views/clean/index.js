// @ts-check
import { renderCategory } from '../category.js';
import { renderCustomSearch } from './custom-search.js';

/** Clean view: its opportunities plus Custom search. @param {any} s @param {any} actions */
export const renderClean = (s, actions) => renderCategory(s, actions, 'clean', [renderCustomSearch(s, actions)]);
