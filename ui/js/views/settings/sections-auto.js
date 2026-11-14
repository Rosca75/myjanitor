// @ts-check
/** Engines (external tools) and Automation (schedules, context menu). */
import { h } from '../../dom.js';
import { button, toggle } from '../../components/controls.js';
import { badge } from '../../components/badge.js';
import { formatDate } from '../../format.js';
import { cadenceText, sc } from './copy.js';
import { errorLine, row, rows, section, textInput } from './ui.js';

/** @param {any} s @param {any} ctl */
export function engines(s, ctl) {
  const overrides = s.settings.tool_paths ?? {};
  const items = s.tools.map((t) => {
    let draft = overrides[t.id] ?? '';
    const key = `tool:${t.id}`;
    return h('li', { class: 'tool', 'data-tool': t.id },
      h('div', { class: 'tile__text' },
        h('div', { class: 'kv' }, h('strong', {}, t.id), t.found ? badge(sc.engines.found, 'success') : badge(sc.engines.missing, 'neutral'), t.version && h('span', { class: 'tool__v' }, t.version)),
        t.found && t.path && h('span', { class: 'tile__sub', title: t.path }, t.path),
        !t.found && h('span', { class: 'tile__sub', 'data-role': 'reason' }, sc.engines.missingReason(t.id)),
        t.used_for?.length > 0 && h('span', { class: 'tile__sub' }, sc.engines.usedFor(t.used_for.join(', '))),
        h('div', { class: 'custom__form' },
          textInput({ value: draft, label: sc.engines.override(t.id), placeholder: sc.engines.overridePlaceholder, fk: `tool-path-${t.id}`, invalid: !!ctl.st.errors[key], onCommit: (v) => { draft = v; } }),
          button({ label: sc.engines.save, variant: 'secondary', size: 'sm', fk: `tool-save-${t.id}`, onClick: () => ctl.setToolPath(t.id, draft) }),
          overrides[t.id] && button({ label: sc.engines.clear, variant: 'quiet', size: 'sm', fk: `tool-clear-${t.id}`, onClick: () => ctl.clearToolPath(t.id) })),
        errorLine(ctl, key)));
  });
  const watcher = s.settings.integration;
  return section('engines', sc.engines.title,
    h('p', { class: 'tile__sub' }, sc.engines.intro),
    h('ul', { class: 'tools' }, items),
    button({ label: sc.engines.refresh, variant: 'secondary', size: 'sm', fk: 'tools-refresh', onClick: () => ctl.refreshTools() }),
    errorLine(ctl, 'tools'),
    rows(row(ctl, 'watcher', sc.engines.watcher, h('div', { class: 'card__actions' }, badge(sc.unavailable, 'neutral'), toggle({ label: sc.engines.watcher, checked: watcher.fs_compression_watcher, disabled: true, title: sc.engines.watcherReason, onChange: () => {}, fk: 'set-watcher' })), { help: `${sc.engines.watcherHelp} ${sc.engines.watcherReason}` })));
}

/** One schedule row. @param {any} sch @param {any} ctl */
function scheduleRow(sch, ctl) {
  const key = `schedule:${sch.id}`;
  const open = !!ctl.st.open[key];
  const confirming = ctl.st.confirmDelete === sch.id;
  return h('li', { class: 'run', 'data-schedule': sch.id },
    h('div', { class: 'run__main' },
      h('strong', {}, sch.name),
      h('span', { class: 'run__meta' }, `${cadenceText(sch.cadence)} · ${sc.auto.steps(sch.steps.length)}`),
      h('span', { class: 'run__meta' }, sch.enabled && sch.next_run ? sc.auto.next(formatDate(sch.next_run)) : sc.auto.disabled, ' · ', sch.last_run ? sc.auto.last(formatDate(sch.last_run)) : sc.auto.never),
      errorLine(ctl, key),
      h('button', { type: 'button', class: 'disclosure', 'aria-expanded': String(open), 'data-fk': `os-${sch.id}`, on: { click: () => ctl.toggleOpen(key) } }, sc.auto.os),
      open && h('div', { class: 'kvs', 'data-role': 'os-commands' },
        h('p', { class: 'tile__sub' }, sc.auto.osHelp),
        h('code', {}, `${sc.auto.osInstall}: myjanitor-cli schedule install ${sch.id}`),
        h('code', {}, `${sc.auto.osUninstall}: myjanitor-cli schedule uninstall ${sch.id}`))),
    h('div', { class: 'run__side' },
      toggle({ label: sc.auto.enable(sch.name), checked: sch.enabled, onChange: (on) => ctl.toggleSchedule(sch.id, on), fk: `sched-on-${sch.id}` }),
      button({ label: sc.auto.runNow, variant: 'secondary', size: 'sm', fk: `sched-run-${sch.id}`, onClick: () => ctl.runSchedule(sch.id) }),
      button({ label: confirming ? sc.auto.confirmDelete : sc.auto.delete, variant: confirming ? 'danger' : 'quiet', size: 'sm', fk: `sched-del-${sch.id}`, onClick: () => ctl.deleteSchedule(sch.id) })));
}

/** @param {any} s @param {any} ctl */
export function automation(s, ctl) {
  const list = ctl.st.schedules;
  const info = s.info;
  const portable = !!info?.portable;
  const note = portable ? sc.auto.contextMenuPortable : info?.platform === 'windows' ? sc.auto.contextMenuWindows : sc.auto.contextMenuLinux;
  return section('automation', sc.auto.title,
    h('div', { class: 'kv' }, h('span', { class: 'kv__k' }, sc.auto.schedules)),
    list === null ? h('p', { class: 'tile__sub' }, sc.loading)
      : list.length ? h('ul', { class: 'runs', 'data-role': 'schedules' }, list.map((x) => scheduleRow(x, ctl)))
        : h('p', { class: 'tile__sub', 'data-role': 'no-schedules' }, sc.auto.noSchedules),
    rows(row(ctl, 'context_menu', sc.auto.contextMenu,
      toggle({ label: sc.auto.contextMenu, checked: s.settings.integration.context_menu, disabled: portable, title: note, onChange: (on) => ctl.commit('context_menu', (n) => { n.integration.context_menu = on; }), fk: 'set-context-menu' }),
      { help: `${sc.auto.contextMenuHelp} ${note}` })));
}
