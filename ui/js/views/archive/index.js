// @ts-check
import { renderCategory } from '../category.js';

/** Archive view. @param {any} s @param {any} actions */
export const renderArchive = (s, actions) => renderCategory(s, actions, 'archive');
