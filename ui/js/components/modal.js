// @ts-check
/**
 * Modal shell: scrim, dialog with role=dialog and aria-modal, a Tab focus
 * trap and Escape to close. The host (views/shell/overlays.js) moves focus in
 * on open and back to the opener on close.
 */
import { h } from '../dom.js';

const FOCUSABLE = 'button, input, select, textarea, [tabindex]';

/** @param {any} root */
export function focusables(root) {
  return root.querySelectorAll(FOCUSABLE).filter((el) => !el.hasAttribute('disabled') && el.getAttribute('tabindex') !== '-1' && el.getAttribute('type') !== 'hidden');
}

/**
 * @param {{ labelId: string, title: any, onClose: () => void, children: any[], extra?: string, footer?: any }} o
 */
export function modal(o) {
  /** @param {any} e */
  const onKey = (e) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      o.onClose();
      return;
    }
    if (e.key !== 'Tab') return;
    const list = focusables(dialog);
    if (!list.length) { e.preventDefault(); return; }
    const first = list[0];
    const last = list[list.length - 1];
    const active = document.activeElement;
    if (e.shiftKey && (active === first || active === dialog)) { e.preventDefault(); last.focus(); }
    else if (!e.shiftKey && active === last) { e.preventDefault(); first.focus(); }
  };
  const dialog = h('div', { class: ['modal', o.extra], role: 'dialog', 'aria-modal': 'true', 'aria-labelledby': o.labelId, tabindex: '-1', 'data-modal': '', on: { keydown: onKey } }, o.children);
  return h('div', { class: 'scrim' }, dialog);
}
