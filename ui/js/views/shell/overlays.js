// @ts-check
/**
 * Overlay host: confirmation dialog, folder browser and report, then the
 * toast. Focus goes into a dialog when it opens and back to the control that
 * opened it when it closes.
 */
import { h } from '../../dom.js';
import { copy } from '../../copy.js';
import { button } from '../../components/controls.js';
import { renderConfirmDialog } from '../step/confirm-dialog.js';
import { renderBrowser } from './browser.js';
import { renderReport } from './report.js';
import { renderAsk } from './ask.js';
import { renderCompare } from './compare.js';

/** Which overlay is open, if any. @param {any} s */
export const overlayKind = (s) => (s.dialog ? 'dialog' : s.ask ? 'ask' : s.browser ? 'browser' : s.compare ? 'compare' : s.report ? 'report' : null);

/** @param {any} s @param {any} actions */
export function renderOverlay(s, actions) {
  const kind = overlayKind(s);
  if (kind === 'dialog') return renderConfirmDialog(s, actions);
  if (kind === 'browser') return renderBrowser(s, actions);
  if (kind === 'ask') return renderAsk(s, actions);
  if (kind === 'compare') return renderCompare(s, actions);
  if (kind === 'report') return renderReport(s, actions);
  return h('span', {});
}

/** Remember the opener and place focus around an overlay change. */
export function createFocusManager() {
  /** @type {any} */
  let opener = null;
  let was = /** @type {string|null} */ (null);
  return {
    /** Call before the overlay DOM is replaced. @param {string|null} kind */
    before(kind) {
      if (kind && !was) opener = document.activeElement;
    },
    /** Call after the overlay DOM was replaced. @param {HTMLElement} root @param {string|null} kind @param {string|null} [prevKind] */
    after(root, kind) {
      if (kind && !was) {
        const dlg = root.querySelector('[data-modal]');
        dlg?.focus?.();
      } else if (!kind && was && opener?.focus) {
        opener.focus();
        opener = null;
      }
      was = kind;
    },
  };
}

/** @param {any} s @param {any} actions */
export function renderToast(s, actions) {
  if (!s.toast) return h('span', {});
  const t = s.toast;
  return h('div', { class: ['toast', `toast--${t.level}`], role: t.level === 'error' ? 'alert' : 'status' },
    h('div', { class: 'toast__text' }, h('strong', {}, t.message), t.hint && h('span', { class: 'toast__hint' }, ` ${t.hint}`)),
    button({ label: copy.toast.dismiss, variant: 'quiet', size: 'sm', fk: 'toast-dismiss', onClick: () => actions.dismissToast() }));
}
