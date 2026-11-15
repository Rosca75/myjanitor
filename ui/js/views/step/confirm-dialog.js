// @ts-check
/**
 * Confirmation dialog for lossy and permanent steps. The acting button is the
 * WARNING style for lossy steps (never the primary style) and the danger style
 * for permanent ones, and stays disabled until "I understand" is ticked.
 */
import { h, icon } from '../../dom.js';
import { copy } from '../../copy.js';
import { formatBytes, formatApprox, formatCount } from '../../format.js';
import { button, select } from '../../components/controls.js';
import { modal } from '../../components/modal.js';

/** @param {string} label @param {string} value @param {string} [tone] */
const stat = (label, value, tone) => h('div', { class: ['stat', tone && `stat--${tone}`], 'data-stat': label },
  h('span', { class: 'stat__label' }, label), h('strong', { class: 'stat__value' }, value));

/** @param {any} s @param {any} actions */
export function renderConfirmDialog(s, actions) {
  const d = s.dialog;
  const info = d.infos[d.index];
  const a = d.current;
  const permanent = !!info.permanent;
  const unit = (s.plan.steps.find((x) => x.id === info.step_id)?.title) ?? '';
  void unit;
  const originals = info.originals_options;
  const children = [
    h('div', { class: 'modal__head' },
      h('span', { class: ['alert-icon', permanent && 'alert-icon--danger'], 'aria-hidden': 'true' }, icon('warning', 22)),
      h('div', { class: 'modal__intro' },
        d.infos.length > 1 && h('p', { class: 'eyebrow' }, copy.confirm.stepOf(d.index + 1, d.infos.length)),
        h('h2', { class: 'modal__title', id: 'dlg-title' }, info.title),
        h('p', { class: 'modal__body' }, info.body))),
    h('div', { class: 'stats' },
      stat(copy.confirm.files, formatCount(info.files)),
      stat(copy.confirm.now, formatBytes(info.bytes_now)),
      stat(copy.confirm.after, info.bytes_after > 0 ? formatApprox(info.bytes_after) : '0 B'),
      stat(permanent ? copy.confirm.freed : copy.confirm.saving, formatBytes(info.saving), 'good')),
    originals.length > 0 && h('div', { class: 'modal__row' },
      h('label', { class: 'modal__label', for: 'dlg-originals' }, copy.confirm.originals),
      select({ label: copy.confirm.originals, value: String(a.originalsIndex), fk: 'dlg-originals', options: originals.map((o, i) => ({ value: String(i), label: o.label })), onChange: (v) => actions.dialogSet({ originalsIndex: Number(v) }) })),
    info.synced && h('div', { class: 'callout', role: 'group', 'aria-label': copy.confirm.syncedTitle(info.synced.count, info.synced.provider) },
      h('p', { class: 'callout__title' }, copy.confirm.syncedTitle(info.synced.count, info.synced.provider)),
      h('p', { class: 'callout__body' }, copy.confirm.syncedBody)),
    info.synced?.can_online_only && h('div', { class: 'modal__online' },
      button({ label: copy.confirm.onlineOnly, variant: 'secondary', size: 'sm', pressed: a.onlineOnly, fk: 'dlg-online', onClick: () => actions.dialogSet({ onlineOnly: !a.onlineOnly }) }),
      a.onlineOnly && h('p', { class: 'note' }, copy.confirm.onlineOnlyOn)),
    h('label', { class: 'ack' },
      h('input', { type: 'checkbox', class: 'checkbox', 'data-fk': 'dlg-ack', checked: a.ack, 'aria-describedby': 'dlg-ack-hint', on: { change: (e) => actions.dialogSet({ ack: !!e.target.checked }) } }),
      h('span', {}, info.acknowledge_text)),
    h('p', { class: 'ack__hint', id: 'dlg-ack-hint' }, a.ack ? info.undo_text : `${info.undo_text} ${copy.confirm.ackRequired}.`),
    h('div', { class: 'modal__foot' },
      !permanent && button({ label: copy.confirm.compare, variant: 'link', fk: 'dlg-compare', onClick: () => actions.compareSample() }),
      h('span', { class: 'modal__spacer' }),
      button({ label: copy.confirm.back, variant: 'secondary', fk: 'dlg-back', onClick: () => actions.dialogBack() }),
      button({ label: info.action_label, variant: permanent ? 'danger' : 'warning', icon: permanent ? undefined : 'warning', disabled: !a.ack, fk: 'dlg-act', extra: 'btn--acting', describedBy: 'dlg-ack-hint', onClick: () => actions.dialogConfirm() })),
  ];
  return modal({ labelId: 'dlg-title', title: info.title, children, onClose: () => actions.dialogBack(), extra: ['modal--confirm', permanent && 'modal--permanent'] });
}
