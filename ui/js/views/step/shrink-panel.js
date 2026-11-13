// @ts-check
/** Right panel "How to shrink them" for optimise steps. */
import { h } from '../../dom.js';
import { copy } from '../../copy.js';
import { formatBytes, formatApprox } from '../../format.js';
import { button, numberInput, radioTile, toggle } from '../../components/controls.js';

/** Waveform from mock/engine data: bars plus the silences a preset would remove. @param {any} wave */
export function waveform(wave) {
  const { peaks, duration_ms: dur, silences } = wave;
  const max = Math.max(1, ...peaks);
  const bars = peaks.map((p) => { const bar = h('span', { class: ['wave__bar', p <= 4 && 'wave__bar--silent'] }); bar.style.setProperty('height', `${Math.max(4, (p / max) * 100).toFixed(1)}%`); return bar; });
  const bands = silences.map(([a, b]) => { const band = h('span', { class: 'wave__silence' }); band.style.setProperty('left', `${((a / dur) * 100).toFixed(2)}%`); band.style.setProperty('width', `${(((b - a) / dur) * 100).toFixed(2)}%`); return band; });
  return h('div', { class: 'wave', role: 'img', 'aria-label': copy.shrink.waveformAlt }, bands, h('div', { class: 'wave__bars' }, bars));
}

/** Group a number setting with the toggle before it, like the wireframe row. @param {any[]} settings */
function groupSettings(settings) {
  const out = [];
  for (const st of settings) {
    const prev = out[out.length - 1];
    if (st.kind.type === 'number' && prev && prev.main.kind.type === 'toggle') prev.number = st;
    else out.push({ main: st });
  }
  return out;
}

/** @param {any} st one setting @param {any} params @param {any} actions */
function settingControl(st, params, actions) {
  const value = params[st.key] ?? st.kind.default;
  if (st.kind.type === 'toggle') return toggle({ checked: !!value, label: st.label, disabled: !!st.disabled_reason, title: st.disabled_reason, fk: `p-${st.key}`, onChange: (on) => actions.setParam(st.key, on) });
  return numberInput({ value: Number(value), min: st.kind.min, max: st.kind.max, step: st.kind.step, unit: st.kind.unit, label: st.label, fk: `p-${st.key}`, onChange: (n) => actions.setParam(st.key, n) });
}

/** @param {any} entry @param {any} params @param {any} actions */
function settingRow(entry, params, actions) {
  const { main, number } = entry;
  return h('div', { class: 'setting' },
    h('div', { class: 'setting__text' }, h('span', { class: 'setting__label' }, main.label), main.help && h('span', { class: 'setting__help' }, main.help)),
    number && settingControl(number, params, actions),
    settingControl(main, params, actions));
}

/** @param {any} s @param {any} actions */
export function renderShrinkPanel(s, actions) {
  const st = s.step;
  const opp = st.detail.opportunity;
  const sum = st.detail.summary;
  const params = opp.params;
  const current = params.preset ?? opp.presets[0]?.id;
  const basic = opp.settings.filter((x) => !x.advanced);
  const adv = opp.settings.filter((x) => x.advanced);
  const ffmpeg = s.tools.find((t) => t.id === 'ffmpeg');
  const canListen = !ffmpeg || ffmpeg.found; // unknown tools: leave it enabled
  const preview = st.preview;
  const wave = preview?.kind === 'waveform' ? preview.waveform : null;
  const silenceMin = wave ? Math.round(wave.silences.reduce((a, [x, y]) => a + (y - x), 0) / 60000) : 0;
  return h('div', { class: 'shrink' },
    h('h2', { class: 'panel__title' }, copy.shrink.title),
    opp.presets.length > 0 && h('div', { class: 'tiles', role: 'radiogroup', 'aria-label': copy.shrink.presets },
      opp.presets.map((p) => radioTile({ title: p.label, sub: p.description, selected: p.id === current, fk: `preset-${p.id}`, onSelect: () => actions.setPreset(p.id) }))),
    basic.length > 0 && h('div', { class: 'settings', role: 'group', 'aria-label': copy.shrink.options }, groupSettings(basic).map((e) => settingRow(e, params, actions))),
    adv.length > 0 && h('div', { class: 'advanced' },
      h('button', { type: 'button', class: 'disclosure', 'aria-expanded': String(st.advancedOpen), 'data-fk': 'adv-toggle', on: { click: () => actions.toggleAdvanced() } }, st.advancedOpen ? copy.shrink.hideAdvanced : copy.shrink.showAdvanced),
      st.advancedOpen && h('div', { class: 'settings', role: 'group', 'aria-label': copy.shrink.advanced }, groupSettings(adv).map((e) => settingRow(e, params, actions)))),
    wave
      ? h('div', { class: 'preview' }, h('div', { class: 'preview__head' }, h('span', { class: 'preview__title', title: st.previewFor }, (st.previewFor ?? '').split(/[\\/]/).pop()), h('span', { class: 'preview__silence' }, copy.shrink.silence(silenceMin))), waveform(wave))
      : preview && h('p', { class: 'preview__none' }, preview.reason ?? copy.shrink.noPreview),
    h('div', { class: 'estimate', role: 'status', 'aria-live': 'polite' }, h('span', { class: 'estimate__label' }, copy.step.thisStep), h('strong', { class: 'estimate__value', 'data-role': 'step-estimate' }, copy.step.estimateLine(formatBytes(sum.size_before), formatApprox(sum.size_after), formatBytes(sum.saving)))),
    s.step.detail.lossy && h('p', { class: 'note note--warn' }, copy.shrink.lossyNote),
    button({ label: copy.shrink.listen, variant: 'secondary', icon: 'play', block: true, disabled: !canListen, fk: 'listen', onClick: () => actions.previewSample() }),
    !canListen && h('p', { class: 'note' }, copy.shrink.listenOff(copy.shrink.noTool)),
    preview?.kind === 'waveform' && h('p', { class: 'note' }, copy.shrink.previewReady));
}
