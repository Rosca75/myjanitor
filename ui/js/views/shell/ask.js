// @ts-check
/** Generic yes/no dialog: undo a whole run, empty the quarantine. */
import { h, icon } from '../../dom.js';
import { copy } from '../../copy.js';
import { formatBytes } from '../../format.js';
import { button } from '../../components/controls.js';
import { modal } from '../../components/modal.js';
import { pathText } from '../../components/path.js';
import { refused } from '../../lib/history.js';

/** @param {any} s @param {any} actions */
export function renderAsk(s, actions) {
  const a = s.ask;
  const undo = a.kind === 'undo_run';
  const run = undo ? s.history.runs.find((r) => r.run_id === a.runId) : null;
  const known = undo ? refused(s.historyUi.details[a.runId]?.entries ?? []) : [];
  const title = undo ? copy.historyView.askUndoTitle(run?.title ?? '') : copy.historyView.askPurgeTitle;
  const children = [
    h('div', { class: 'modal__head' },
      h('span', { class: ['alert-icon', !undo && 'alert-icon--danger'], 'aria-hidden': 'true' }, icon('warning', 22)),
      h('div', { class: 'modal__intro' },
        h('h2', { class: 'modal__title', id: 'ask-title' }, title),
        h('p', { class: 'modal__body' }, undo ? copy.historyView.askUndoBody : copy.historyView.askPurgeBody(a.days)),
        !undo && s.quarantine && h('p', { class: 'modal__body' }, copy.historyView.quarantineLine(s.quarantine.items, formatBytes(s.quarantine.bytes), '')))),
    known.length > 0 && h('div', { class: 'callout' },
      h('p', { class: 'callout__title' }, copy.historyView.askUndoRefused(known.length)),
      h('ul', { class: 'refusals' }, known.slice(0, 5).map((e) => h('li', {}, pathText(e.original, { max: 50 }), ' ', h('span', { class: 'callout__body' }, [e.error.message, e.error.hint].filter(Boolean).join(' ')))))),
    h('div', { class: 'modal__foot' }, h('span', { class: 'modal__spacer' }),
      button({ label: copy.ask.cancel, variant: 'secondary', fk: 'ask-cancel', onClick: () => actions.cancelAsk() }),
      button({ label: undo ? copy.historyView.askUndoAction : copy.historyView.askPurgeAction, variant: undo ? 'primary' : 'danger', fk: 'ask-confirm', onClick: () => actions.acceptAsk() })),
  ];
  return modal({ labelId: 'ask-title', title, children, onClose: () => actions.cancelAsk() });
}
