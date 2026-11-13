// @ts-check
/** B1 Overview: picks the state (empty, scanning, results). */
import { isScanning } from '../../state.js';
import { renderEmpty } from './empty.js';
import { renderScanning } from './scanning.js';
import { renderResults } from './results.js';

/** @param {any} s @param {any} actions */
export function renderOverview(s, actions) {
  if (isScanning(s)) return renderScanning(s, actions);
  if (!s.opps.scanned_at && s.opps.items.length === 0) return renderEmpty(s, actions);
  return renderResults(s, actions);
}
