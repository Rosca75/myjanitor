// @ts-check
/** Compare before / after (OPT-ENG-10): a side-by-side shell with sizes. */
import { h } from '../../dom.js';
import { copy } from '../../copy.js';
import { basename, formatBytes, formatApprox, displayPath } from '../../format.js';
import { button } from '../../components/controls.js';
import { modal } from '../../components/modal.js';
import { waveformView } from '../step/preview-pane.js';

/** Waveform with the silences cut out, what the new file would contain. @param {any} wave */
export function afterWaveform(wave) {
  const { peaks, duration_ms: dur, silences } = wave;
  const kept = peaks.filter((_, i) => !silences.some(([a, b]) => (i / peaks.length) * dur >= a && (i / peaks.length) * dur <= b));
  return waveformView({ peaks: kept, duration_ms: dur, silences: [] });
}

/** Can this preview be compared? Only a sample the backend produced. @param {any} preview */
export const canCompare = (preview) => preview?.kind === 'waveform';

/** @param {any} s @param {any} actions */
export function renderCompare(s, actions) {
  const st = s.step;
  const row = st?.page?.rows.find((r) => r.id === s.compare.rowId);
  const preview = st?.preview;
  const name = displayPath(basename(row?.path ?? ''));
  const children = [
    h('h2', { class: 'modal__title', id: 'compare-title' }, copy.compare.title),
    h('p', { class: 'modal__body', title: displayPath(row?.path ?? '') }, name),
    canCompare(preview)
      ? h('div', { class: 'compare' },
        h('section', { class: 'compare__side', 'aria-label': copy.compare.before }, h('h3', { class: 'eyebrow' }, copy.compare.before), waveformView(preview.waveform), h('strong', {}, formatBytes(row.size))),
        h('section', { class: 'compare__side', 'aria-label': copy.compare.after }, h('h3', { class: 'eyebrow' }, copy.compare.after), afterWaveform(preview.waveform), h('strong', {}, formatApprox(row.size_after))),
        h('p', { class: 'note compare__note' }, copy.compare.silenceNote))
      : h('p', { class: 'state state--unavailable' }, copy.compare.noSample(preview?.reason ?? copy.compare.noSampleDefault)),
    h('div', { class: 'modal__foot' }, h('span', { class: 'modal__spacer' }), button({ label: copy.compare.close, variant: 'secondary', fk: 'compare-close', onClick: () => actions.closeCompare() })),
  ];
  return modal({ labelId: 'compare-title', title: copy.compare.title, children, extra: 'modal--wide', onClose: () => actions.closeCompare() });
}
