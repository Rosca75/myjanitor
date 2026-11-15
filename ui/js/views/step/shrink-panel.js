// @ts-check
/** Right panel "How to shrink them" for optimise steps. */
import { h } from '../../dom.js';
import { copy } from '../../copy.js';
import { formatBytes, formatApprox } from '../../format.js';
import { button, radioTile } from '../../components/controls.js';
import { previewPane, compareButton } from './preview-pane.js';
import { settingControl } from '../../components/setting-control.js';

export { waveformView as waveform } from './preview-pane.js';

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
const control = (st, params, actions) => settingControl(st, params, { onChange: (k, v) => actions.setParam(k, v) });

/** @param {any} entry @param {any} params @param {any} actions */
function settingRow(entry, params, actions) {
  const { main, number } = entry;
  return h('div', { class: 'setting' },
    h('div', { class: 'setting__text' }, h('span', { class: 'setting__label' }, main.label), main.help && h('span', { class: 'setting__help' }, main.help)),
    number && control(number, params, actions),
    control(main, params, actions));
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
    previewPane(s, actions),
    h('div', { class: 'estimate', role: 'status', 'aria-live': 'polite' }, h('span', { class: 'estimate__label' }, copy.step.thisStep), h('strong', { class: 'estimate__value', 'data-role': 'step-estimate' }, copy.step.estimateLine(formatBytes(sum.size_before), formatApprox(sum.size_after), formatBytes(sum.saving)))),
    s.step.detail.lossy && h('p', { class: 'note note--warn' }, copy.shrink.lossyNote),
    button({ label: copy.shrink.listen, variant: 'secondary', icon: 'play', block: true, disabled: !canListen, fk: 'listen', onClick: () => actions.previewSample() }),
    compareButton(s, actions),
    !canListen && h('p', { class: 'note' }, copy.shrink.listenOff(copy.shrink.noTool)),
    preview?.kind === 'waveform' && h('p', { class: 'note' }, copy.shrink.previewReady));
}
