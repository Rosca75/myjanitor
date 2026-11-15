// @ts-check
/**
 * Right-panel preview (SAF-02): thumbnail, text start or waveform from
 * `candidate_preview`, the row's details, and the size breakdown (OPT-ENG-09).
 */
import { h } from '../../dom.js';
import { copy } from '../../copy.js';
import { basename, dirname, displayPath, formatBytes, formatApprox, formatDate, pct } from '../../format.js';
import { button } from '../../components/controls.js';
import { stackedBar } from '../../components/bars.js';
import { pathText } from '../../components/path.js';

/** Waveform bars plus the silences a preset would remove. @param {any} wave */
export function waveformView(wave) {
  const { peaks, duration_ms: dur, silences } = wave;
  const max = Math.max(1, ...peaks);
  const bars = peaks.map((p) => { const bar = h('span', { class: ['wave__bar', p <= 4 && 'wave__bar--silent'] }); bar.style.setProperty('height', `${Math.max(4, (p / max) * 100).toFixed(1)}%`); return bar; });
  const bands = silences.map(([a, b]) => { const band = h('span', { class: 'wave__silence' }); band.style.setProperty('left', `${((a / dur) * 100).toFixed(2)}%`); band.style.setProperty('width', `${(((b - a) / dur) * 100).toFixed(2)}%`); return band; });
  return h('div', { class: 'wave', role: 'img', 'aria-label': copy.shrink.waveformAlt }, bands, h('div', { class: 'wave__bars' }, bars));
}

/** @param {any} row */
function details(row) {
  const kv = (k, v) => h('div', { class: 'kv' }, h('dt', { class: 'kv__k' }, k), h('dd', { class: 'kv__v' }, v));
  return h('dl', { class: 'kvs preview__meta', 'aria-label': copy.preview.meta },
    kv(copy.preview.name, h('span', { class: 'path', title: displayPath(row.path) }, displayPath(basename(row.path)))),
    kv(copy.preview.folder, pathText(dirname(row.path), { max: 30 })),
    kv(copy.preview.size, formatBytes(row.size)),
    row.size_after !== row.size && kv(copy.preview.after, row.size_after === 0 ? copy.step.removed : formatApprox(row.size_after)),
    row.modified && kv(copy.preview.modified, formatDate(row.modified)),
    row.reason && kv(copy.preview.why, row.reason));
}

/**
 * Per-file size parts when the engine provides them (`columns["part:<name>"]`
 * in bytes); otherwise the plain now-versus-after bar, which is always known.
 * @param {any} row
 */
export function sizeBreakdown(row) {
  const parts = Object.entries(row.columns ?? {}).filter(([k, v]) => k.startsWith('part:') && Number(v) > 0).map(([k, v]) => ({ key: k.slice(5), value: Number(v), tone: ['reversible', 'opt-in', 'review', 'used', 'freed'][Object.keys(row.columns).filter((x) => x.startsWith('part:')).indexOf(k) % 5] }));
  const total = parts.reduce((a, p) => a + p.value, 0);
  if (parts.length) {
    return h('div', { class: 'breakdown' }, h('h3', { class: 'eyebrow' }, copy.preview.breakdown),
      stackedBar({ segments: parts, label: `${copy.preview.breakdown}: ${parts.map((p) => `${p.key} ${formatBytes(p.value)}`).join(', ')}` }),
      h('ul', { class: 'legend' }, parts.map((p) => h('li', { class: 'legend__item' }, h('span', { class: ['swatch', `swatch--${p.tone}`], 'aria-hidden': 'true' }), `${p.key} ${formatBytes(p.value)} (${Math.round(pct(p.value, total))} %)`))));
  }
  return h('div', { class: 'breakdown' }, h('h3', { class: 'eyebrow' }, copy.preview.breakdown),
    stackedBar({ segments: [{ key: 'after', value: row.size_after, tone: 'freed' }, { key: 'saved', value: Math.max(0, row.size - row.size_after), tone: 'review' }], label: copy.preview.sizeBar(formatBytes(row.size), formatApprox(row.size_after)) }),
    h('p', { class: 'note' }, copy.preview.sizeBar(formatBytes(row.size), formatApprox(row.size_after))));
}

/** The "Compare before / after" button, disabled with the reason when no sample exists. @param {any} s @param {any} actions */
export function compareButton(s, actions) {
  const st = s.step;
  const ok = st.preview?.kind === 'waveform';
  const why = !st.focusId ? copy.compare.pickFirst : ok ? undefined : copy.compare.noSample(st.preview?.reason ?? copy.compare.noSampleDefault);
  return h('span', { class: 'compare-btn' },
    button({ label: copy.compare.open, variant: 'secondary', block: true, disabled: !ok, title: why, fk: 'compare-open', onClick: () => actions.openCompare() }),
    !ok && h('p', { class: 'note', 'data-role': 'compare-why' }, why));
}

/** The preview block of the right panel for the focused row. @param {any} s @param {any} actions */
export function previewPane(s) {
  const st = s.step;
  const preview = st.preview;
  const row = st.page?.rows.find((r) => r.id === st.focusId);
  if (!preview) return h('p', { class: 'preview__none' }, copy.preview.pick);
  const title = displayPath(basename(st.previewFor ?? ''));
  let body;
  if (preview.kind === 'waveform') {
    const silenceMin = Math.round(preview.waveform.silences.reduce((a, [x, y]) => a + (y - x), 0) / 60000);
    body = [h('div', { class: 'preview__head' }, h('span', { class: 'preview__title', title: st.previewFor }, title), h('span', { class: 'preview__silence' }, copy.shrink.silence(silenceMin))), waveformView(preview.waveform)];
  } else if (preview.kind === 'image' && preview.data) {
    const img = h('img', { class: 'preview__image', alt: copy.preview.imageAlt(title), src: `data:${preview.mime ?? 'image/png'};base64,${preview.data}` });
    body = [h('div', { class: 'preview__head' }, h('span', { class: 'preview__title', title: st.previewFor }, title)), img];
  } else if (preview.kind === 'text' && preview.text !== undefined) {
    body = [h('div', { class: 'preview__head' }, h('span', { class: 'preview__title', title: st.previewFor }, title), h('span', { class: 'note' }, copy.preview.text)), h('pre', { class: 'preview__text' }, preview.text)];
  } else {
    body = [h('div', { class: 'preview__head' }, h('span', { class: 'preview__title', title: st.previewFor }, title)), h('p', { class: 'preview__none' }, preview.reason ?? copy.preview.none)];
  }
  return h('div', { class: 'preview', 'aria-busy': st.previewLoading ? 'true' : undefined }, body, row && details(row), row && row.target !== 'dir' && sizeBreakdown(row));
}
