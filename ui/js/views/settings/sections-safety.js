// @ts-check
/** Safety and Performance. */
import { h } from '../../dom.js';
import { button, numberInput, radioTile, select, toggle } from '../../components/controls.js';
import { displayPath } from '../../format.js';
import { sc } from './copy.js';
import { advanced, errorLine, row, rows, section } from './ui.js';
import { renderPicker } from './picker.js';

/** @param {any} s @param {any} ctl */
export function safety(s, ctl) {
  const g = s.settings.general;
  const policy = g.default_originals.policy;
  const pick = (value) => () => ctl.commit('originals', (n) => { n.general.default_originals = { policy: value }; });
  const tiles = [
    radioTile({ title: sc.safety.recycle_bin, sub: sc.safety.recycle_binSub, selected: policy === 'recycle_bin', onSelect: pick('recycle_bin'), fk: 'orig-recycle' }),
    radioTile({ title: sc.safety.quarantine, sub: sc.safety.quarantineSub, selected: policy === 'quarantine', onSelect: pick('quarantine'), fk: 'orig-quarantine' }),
    radioTile({ title: sc.safety.backup_folder, sub: sc.safety.backupSub(policy === 'backup_folder' ? displayPath(g.default_originals.folder) : ''), selected: policy === 'backup_folder', onSelect: () => ctl.openPicker('backup'), fk: 'orig-backup' }),
  ];
  const num = (key, value, min, max, apply, label) => numberInput({ value, min, max, label, fk: `set-${key}`, onChange: (n) => ctl.commitNumber(key, String(n), min, max, apply) });
  const flag = (group, key, label, help) => row(ctl, key, label, toggle({ label, checked: s.settings[group][key], onChange: (on) => ctl.commit(key, (n) => { n[group][key] = on; }), fk: `set-${key}` }), help ? { help } : {});
  return section('safety', sc.safety.title,
    h('p', { class: 'kv__k', id: 'originals-label' }, sc.safety.originals),
    h('div', { class: 'tiles', role: 'radiogroup', 'aria-labelledby': 'originals-label' }, tiles),
    policy === 'delete' && h('p', { class: 'notice notice--warning', role: 'note' }, sc.safety.deleteNote),
    ctl.st.picker?.purpose === 'backup' && renderPicker(ctl),
    errorLine(ctl, 'originals'),
    rows(
      row(ctl, 'quarantine_days', sc.safety.retention, num('quarantine_days', g.quarantine_days, 1, 3650, (n, v) => { n.general.quarantine_days = v; }, sc.safety.retention), { help: sc.safety.daysValue(g.quarantine_days) }),
      row(ctl, 'undo_days', sc.safety.undoWindow, num('undo_days', g.undo_days, 1, 3650, (n, v) => { n.general.undo_days = v; }, sc.safety.undoWindow), { help: sc.safety.daysValue(g.undo_days) }),
      flag('optimise', 'preserve_timestamps', sc.safety.preserveTimestamps),
      row(ctl, 'keep_if_not_smaller', sc.safety.keepIfNotSmaller, toggle({ label: sc.safety.keepIfNotSmaller, checked: true, disabled: true, onChange: () => {}, fk: 'set-keep-smaller' }), { help: sc.safety.keepIfNotSmallerHelp })),
    advanced(ctl, 'safety',
      flag('safety', 'skip_signed', sc.safety.skipSigned), flag('safety', 'skip_labelled', sc.safety.skipLabelled),
      flag('safety', 'skip_encrypted', sc.safety.skipEncrypted), flag('safety', 'synced_folder_warning', sc.safety.syncedWarning)));
}

/** @param {any} s @param {any} ctl */
export function performance(s, ctl) {
  const p = s.settings.performance;
  const count = (key, value, max, label) => numberInput({ value, min: 0, max, label, fk: `set-${key}`, onChange: (n) => ctl.commitNumber(key, String(n), 0, max, (st, v) => { st.performance[key] = v; }) });
  const temp = h('div', { class: 'card__actions' },
    h('span', { class: 'tile__sub', title: p.temp_dir ?? '' }, p.temp_dir ? displayPath(p.temp_dir) : sc.perf.tempDefault),
    button({ label: sc.perf.chooseTemp, variant: 'secondary', size: 'sm', fk: 'choose-temp', onClick: () => ctl.openPicker('temp') }),
    p.temp_dir && button({ label: sc.perf.resetTemp, variant: 'quiet', size: 'sm', fk: 'reset-temp', onClick: () => ctl.commit('temp', (n) => { n.performance.temp_dir = null; }) }));
  return section('performance', sc.perf.title, rows(
    row(ctl, 'threads', sc.perf.threads, count('threads', p.threads, 256, sc.perf.threads), { help: sc.perf.threadsHelp }),
    row(ctl, 'low_memory', sc.perf.lowMemory, toggle({ label: sc.perf.lowMemory, checked: p.low_memory, onChange: (on) => ctl.commit('low_memory', (n) => { n.performance.low_memory = on; }), fk: 'set-low-memory' }), { help: sc.perf.lowMemoryHelp }),
    row(ctl, 'temp', sc.perf.tempDir, temp, { help: sc.perf.tempHelp })),
    ctl.st.picker?.purpose === 'temp' && renderPicker(ctl),
    advanced(ctl, 'perf',
      row(ctl, 'parallel_jobs', sc.perf.parallelJobs, count('parallel_jobs', p.parallel_jobs, 64, sc.perf.parallelJobs), { help: sc.perf.parallelJobsHelp }),
      row(ctl, 'cpu_priority', sc.perf.cpu, select({ label: sc.perf.cpu, value: p.cpu_priority, options: [{ value: 'normal', label: sc.perf.cpuNormal }, { value: 'low', label: sc.perf.cpuLow }], fk: 'set-cpu', onChange: (v) => ctl.commit('cpu_priority', (n) => { n.performance.cpu_priority = v; }) }))));
}
