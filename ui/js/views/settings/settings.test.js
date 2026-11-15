// Settings drawer tests: a fake DOM and a local fake backend (the shared mock has no
// validation, no schedule_save and no persisted protected paths), asserting the
// command names and exact payloads each control sends.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { installFakeDom } from '../../testing/fakedom.js';
import { SETTINGS, TOOLS } from '../../mock/fixtures.js';
import { renderSettings } from './index.js';
import { attachSettingsApi, isAbsolutePath } from './controller.js';

const tick = async () => { for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0)); };

const SCHEDULE = { id: 'sched_1', name: 'Weekly Downloads', enabled: true, cadence: { every: 'weekly', weekday: 1, hour: 8, minute: 47 }, scope: { roots: ['/home/m/Downloads'] }, steps: [{ opportunity: 'det:temp_files', action: { kind: 'trash' }, params: {} }], last_run: null, next_run: 1_800_000_000_000_000_000 };

/** @param {{ portable?: boolean, settingsError?: any, scheduleError?: any, schedules?: any[] }} [o] */
async function setup(o = {}) {
  const doc = installFakeDom();
  const calls = [];
  const rec = (cmd, req) => calls.push({ cmd, req });
  let settings = structuredClone(SETTINGS);
  let schedules = o.schedules ?? [];
  let prot = { builtin: ['C:\\Windows', 'C:\\Program Files'], builtin_patterns: ['**/.git'], user: [] };
  const api = {
    settingsSet: async (req) => { rec('settings_set', req); if (o.settingsError) throw o.settingsError; settings = structuredClone(req.settings); return settings; },
    protectedPathsGet: async () => prot,
    protectedPathsSet: async (req) => { rec('protected_paths_set', req); prot = { ...prot, user: req.user }; return prot; },
    schedulesList: async () => schedules,
    scheduleSave: async (req) => { rec('schedule_save', req); if (o.scheduleError) throw o.scheduleError; schedules = schedules.map((s) => (s.id === req.schedule.id ? req.schedule : s)); return schedules; },
    scheduleDelete: async (req) => { rec('schedule_delete', req); schedules = schedules.filter((s) => s.id !== req.id); return schedules; },
    scheduleRunNow: async (req) => { rec('schedule_run_now', req); return { run_id: 1, jobs: [] }; },
    toolsRefresh: async () => { rec('tools_refresh'); return TOOLS; },
    fsListDir: async (req) => {
      rec('fs_list_dir', req);
      if (!req.path) return { places: [{ label: 'Home', path: '/home/m', kind: 'home' }], entries: [] };
      return { path: req.path, parent: req.path === '/home/m' ? null : '/home/m', entries: [{ name: 'Clients', path: `${req.path}/Clients`, is_dir: true, is_link: false, hidden: false }], places: [] };
    },
  };
  const actions = { loadSettings: async () => { rec('load_settings'); }, clearCache: async () => rec('cache_clear') };
  attachSettingsApi(actions, api);
  const state = { settings, cache: { entries: 12, bytes: 2048 }, tools: TOOLS, info: { version: '0.1.0', platform: 'linux', portable: !!o.portable, data_dir: '/home/m/.local/share/myjanitor', journal_persistent: true } };
  const root = doc.createElement('div');
  doc.body.append(root);
  root.replaceChildren(renderSettings(state, actions));
  await tick();
  const $ = (sel) => root.querySelector(sel);
  const sent = (cmd) => calls.filter((c) => c.cmd === cmd);
  return { root, $, calls, sent, settings: () => settings };
}

test('renders every section, the offline line, the portable indicator and locked built-ins', async () => {
  const t = await setup();
  const ids = t.root.querySelectorAll('[data-section]').map((e) => e.getAttribute('data-section'));
  assert.deepEqual(ids, ['general', 'scan', 'protected', 'exclusions', 'safety', 'performance', 'engines', 'automation']);
  assert.match(t.$('[data-role="offline"]').textContent, /no telemetry/);
  assert.match(t.$('[data-role="mode"]').textContent, /Installed mode/);
  const builtin = t.$('[data-role="builtin"]');
  assert.match(builtin.textContent, /C:\\Windows/);
  assert.ok(builtin.querySelectorAll('.badge').length >= 3, 'every built-in row carries the Locked badge');
  assert.equal(builtin.querySelectorAll('button').filter((b) => /Remove/.test(b.textContent)).length, 0, 'built-ins cannot be removed');
  const p = await setup({ portable: true });
  assert.match(p.$('[data-role="mode"]').textContent, /Portable mode/);
});

test('a toggle sends the whole Settings object with only that field changed', async () => {
  const t = await setup();
  t.$('[data-fk="set-use-cache"]').setChecked(false);
  await tick();
  const [req] = t.sent('settings_set');
  assert.equal(req.req.settings.scan.use_cache, false);
  assert.deepEqual({ ...req.req.settings, scan: SETTINGS.scan }, SETTINGS);
  assert.ok(t.sent('load_settings').length >= 1, 'the shell store is refreshed');
});

test('numbers are validated before they are sent, and valid ones are sent exactly', async () => {
  const t = await setup();
  t.$('[data-fk="set-threads"]').setValue('999');
  await tick();
  assert.equal(t.sent('settings_set').length, 0);
  assert.match(t.$('[data-error="threads"]').textContent, /whole number from 0 to 256/);
  t.$('[data-fk="set-threads"]').setValue('4');
  await tick();
  assert.equal(t.sent('settings_set')[0].req.settings.performance.threads, 4);
  assert.equal(t.$('[data-error="threads"]'), null, 'the error clears on success');
  t.$('[data-fk="set-undo_days"]').setValue('0');
  await tick();
  assert.match(t.$('[data-error="undo_days"]').textContent, /1 to 3650/);
});

test('an engine rejection shows inline under the control that caused it', async () => {
  const t = await setup({ settingsError: { code: 'invalid_input', message: 'performance.threads must be between 0 and 256', hint: '' } });
  t.$('[data-fk="set-low-memory"]').setChecked(true);
  await tick();
  const err = t.$('[data-error="low_memory"]');
  assert.match(err.textContent, /performance\.threads must be between/);
  assert.equal(err.getAttribute('role'), 'alert');
});

test('exclusions: add (trimmed, Enter or button) and remove', async () => {
  const t = await setup();
  t.$('[data-fk="add-exclusion"]').click();
  await tick();
  assert.match(t.$('[data-error="exclusions"]').textContent, /Type a path/);
  t.$('[data-fk="exclusion-input"]').value = '  **/node_modules ';
  t.$('[data-fk="add-exclusion"]').click();
  await tick();
  assert.deepEqual(t.sent('settings_set')[0].req.settings.safety.exclusions, ['**/node_modules']);
  t.$('[data-fk="rm-excl-**/node_modules"]').click();
  await tick();
  assert.deepEqual(t.sent('settings_set')[1].req.settings.safety.exclusions, []);
});

test('protected paths: choose with the in-app browser, then remove', async () => {
  const t = await setup();
  t.$('[data-fk="add-protected"]').click();
  await tick();
  assert.deepEqual(t.sent('fs_list_dir')[0].req, {});
  assert.deepEqual(t.sent('fs_list_dir')[1].req, { path: '/home/m', show_hidden: false, include_files: false });
  t.$('[data-fk="pick-dir-Clients"]').click();
  await tick();
  assert.match(t.$('.browser__current').textContent, /Clients/);
  t.$('[data-fk="pick-choose"]').click();
  await tick();
  assert.deepEqual(t.sent('protected_paths_set')[0].req, { user: ['/home/m/Clients'] });
  assert.equal(t.$('[data-picker]'), null, 'the picker closes');
  t.$('[data-fk="rm-protected-/home/m/Clients"]').click();
  await tick();
  assert.deepEqual(t.sent('protected_paths_set')[1].req, { user: [] });
});

test('originals radio group sets the default policy; backup folder goes through the picker', async () => {
  const t = await setup();
  assert.equal(t.$('[role="radiogroup"]').querySelectorAll('[role="radio"]').length, 3);
  t.$('[data-fk="orig-quarantine"]').click();
  await tick();
  assert.deepEqual(t.sent('settings_set')[0].req.settings.general.default_originals, { policy: 'quarantine' });
  t.$('[data-fk="orig-backup"]').click();
  await tick();
  t.$('[data-fk="pick-choose"]').click();
  await tick();
  assert.deepEqual(t.sent('settings_set')[1].req.settings.general.default_originals, { policy: 'backup_folder', folder: '/home/m' });
});

test('the keep-if-not-smaller guarantee is shown locked on', async () => {
  const t = await setup();
  const sw = t.$('[data-fk="set-keep-smaller"]');
  assert.ok(sw.disabled);
  assert.equal(sw.checked, true);
});

test('temp folder: choose and reset; advanced sections are collapsed until opened', async () => {
  const t = await setup();
  assert.equal(t.$('[data-fk="set-parallel_jobs"]'), null, 'Advanced is collapsed by default');
  t.$('[data-fk="adv-perf"]').click();
  await tick();
  assert.ok(t.$('[data-fk="set-parallel_jobs"]'));
  assert.equal(t.$('[data-fk="adv-perf"]').getAttribute('aria-expanded'), 'true');
  t.$('[data-fk="choose-temp"]').click();
  await tick();
  t.$('[data-fk="pick-choose"]').click();
  await tick();
  assert.equal(t.sent('settings_set')[0].req.settings.performance.temp_dir, '/home/m');
});

test('engines: unavailable tools say why; overrides must be absolute and trigger a re-detect', async () => {
  const t = await setup();
  assert.match(t.$('[data-tool="qpdf"] [data-role="reason"]').textContent, /qpdf was not found on your PATH/);
  const input = t.$('[data-fk="tool-path-qpdf"]');
  input.setValue('qpdf');
  t.$('[data-fk="tool-save-qpdf"]').click();
  await tick();
  assert.match(t.$('[data-error="tool:qpdf"]').textContent, /full path/);
  assert.equal(t.sent('settings_set').length, 0);
  t.$('[data-fk="tool-path-qpdf"]').setValue('/opt/qpdf/bin/qpdf');
  t.$('[data-fk="tool-save-qpdf"]').click();
  await tick();
  assert.equal(t.sent('settings_set')[0].req.settings.tool_paths.qpdf, '/opt/qpdf/bin/qpdf');
  assert.equal(t.sent('tools_refresh').length, 1);
});

test('the FS-compression watcher is stored but shown unavailable', async () => {
  const t = await setup();
  assert.ok(t.$('[data-fk="set-watcher"]').disabled);
  assert.match(t.$('[data-section="engines"]').textContent, /Not available yet/);
});

test('schedules: list with cadence and next run, pause, run now, two-step delete, OS commands', async () => {
  const t = await setup({ schedules: [SCHEDULE] });
  const row = t.$('[data-schedule="sched_1"]');
  assert.match(row.textContent, /Weekly Downloads/);
  assert.match(row.textContent, /Every Monday at 08:47/);
  assert.match(row.textContent, /Next run/);
  t.$('[data-fk="sched-on-sched_1"]').setChecked(false);
  await tick();
  assert.deepEqual(t.sent('schedule_save')[0].req.schedule, { ...SCHEDULE, enabled: false });
  assert.match(t.$('[data-schedule="sched_1"]').textContent, /Paused/);
  t.$('[data-fk="sched-run-sched_1"]').click();
  await tick();
  assert.deepEqual(t.sent('schedule_run_now')[0].req, { id: 'sched_1' });
  t.$('[data-fk="os-sched_1"]').click();
  await tick();
  assert.match(t.$('[data-role="os-commands"]').textContent, /myjanitor-cli schedule install sched_1/);
  t.$('[data-fk="sched-del-sched_1"]').click();
  await tick();
  assert.equal(t.sent('schedule_delete').length, 0, 'the first click only asks');
  t.$('[data-fk="sched-del-sched_1"]').click();
  await tick();
  assert.deepEqual(t.sent('schedule_delete')[0].req, { id: 'sched_1' });
  assert.ok(t.$('[data-role="no-schedules"]'));
});

test('a rejected schedule save shows the engine message on that schedule', async () => {
  const t = await setup({ schedules: [SCHEDULE], scheduleError: { message: 'lossy or permanent steps cannot run unattended' } });
  t.$('[data-fk="sched-on-sched_1"]').setChecked(false);
  await tick();
  assert.match(t.$('[data-error="schedule:sched_1"]').textContent, /cannot run unattended/);
  assert.match(t.$('[data-schedule="sched_1"]').textContent, /Next run/, 'the list keeps the stored state');
});

test('context menu toggle sends integration.context_menu; portable mode disables it with the reason', async () => {
  const t = await setup();
  t.$('[data-fk="set-context-menu"]').setChecked(true);
  await tick();
  assert.equal(t.sent('settings_set')[0].req.settings.integration.context_menu, true);
  const p = await setup({ portable: true });
  assert.ok(p.$('[data-fk="set-context-menu"]').disabled);
  assert.match(p.$('[data-section="automation"]').textContent, /portable mode/i);
});

test('clear cache goes through the shell action', async () => {
  const t = await setup();
  assert.match(t.$('[data-section="scan"]').textContent, /12 cached, 2.0 KB/);
  t.$('[data-fk="clear-cache"]').click();
  await tick();
  assert.equal(t.sent('cache_clear').length, 1);
});

test('absolute path check', () => {
  for (const p of ['/usr/bin/x', 'C:\\Tools\\x.exe', 'C:/x', '\\\\srv\\share']) assert.ok(isAbsolutePath(p), p);
  for (const p of ['x', 'rel/x', '', 'C:x']) assert.ok(!isAbsolutePath(p), p);
});
