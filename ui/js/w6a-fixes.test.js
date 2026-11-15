// W6-A (third task): review findings on entry ids, "select all" under a filter and the
// quarantine action's retention.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { makeEnv } from './testing/env.js';

const last = (env, cmd) => env.sent(cmd).at(-1)?.req;

/** Answers pass through JSON text, as Tauri delivers them to the webview. */
function asWebview(env) {
  const call = env.backend.call.bind(env.backend);
  env.backend.call = async (cmd, req) => {
    const r = await call(cmd, req);
    return r === undefined ? r : JSON.parse(JSON.stringify(r));
  };
}

// ---------------------------------------------------------------- IPC-1 entry ids

test('history: entry ids have the real magnitude and Restore sends back exactly the clicked id', async () => {
  const env = await makeEnv({ scenario: 'rich', mount: false });
  asWebview(env);
  const { mountApp } = await import('./app.js');
  mountApp(env.root, env.store, env.actions, {});
  await env.actions.init();
  await env.settle();
  env.actions.navigate('history');
  await env.settle();
  await env.click(env.$('.run .run__toggle'));
  const entries = Object.values(env.state().historyUi.details)[0].entries;
  assert.ok(entries.length >= 6, `${entries.length} entries`);
  for (const e of entries) {
    assert.equal(typeof e.id, 'string');
    assert.ok(BigInt(e.id) > BigInt(Number.MAX_SAFE_INTEGER), `${e.id} is beyond 2^53`);
  }
  assert.equal(new Set(entries.map((e) => e.id)).size, entries.length, 'ids stay distinct');
  const rows = env.$$('.entry').filter((r) => !r.querySelector('button[disabled]'));
  assert.ok(rows.length >= 2);
  const sixth = rows[Math.min(5, rows.length - 1)];
  const id = sixth.getAttribute('data-entry');
  // Ticking that row ticks it alone.
  sixth.querySelector('input[type="checkbox"]').setChecked(true);
  await env.settle();
  const ticked = env.$$('.entry input[type="checkbox"]').filter((b) => b.checked);
  assert.equal(ticked.length, 1);
  assert.equal(env.$$('.entry')[0].querySelector('input[type="checkbox"]').checked, sixth === env.$$('.entry')[0]);
  await env.click(env.$(`.entry[data-entry="${id}"]`).querySelector('button'));
  assert.deepEqual(last(env, 'restore_entries'), { entry_ids: [id] });
});

// ---------------------------------------------------------------- ENG-3 select all under a filter

/** Every row of a step from the mock model, with its selection. */
const allRows = async (env, id) => (await env.backend.call('candidates_page', { opportunity_id: id, limit: 500, sort: 'path' })).rows;

test('select all under a text filter sends the filter and leaves hidden rows alone (flat)', async () => {
  const env = await makeEnv();
  await env.actions.openStep('det:temp_files');
  await env.settle();
  await env.actions.selectAll(false);
  await env.settle();
  const every = await allRows(env, 'det:temp_files');
  assert.ok(every.length > 3);
  const word = every[1].path.split(/[\\/]/).pop();
  env.actions.setKeepGroups(false);
  await env.actions.setFilter(word);
  await env.settle();
  const shown = env.state().step.page.rows.map((r) => r.id);
  assert.ok(shown.length >= 1 && shown.length < every.length);
  await env.actions.selectAll(true);
  await env.settle();
  assert.deepEqual(last(env, 'selection_set'), { opportunity_id: 'det:temp_files', filter: word, keep_groups: false, selected: true });
  for (const r of await allRows(env, 'det:temp_files')) assert.equal(r.selected, shown.includes(r.id), r.path);
});

test('select all with groups kept together selects the shown groups minus their kept copies', async () => {
  const env = await makeEnv();
  await env.actions.openStep('det:exact_duplicates');
  await env.settle();
  await env.actions.selectAll(false);
  await env.settle();
  const every = await allRows(env, 'det:exact_duplicates');
  const target = every.find((r) => r.group && !r.badges.includes('keeper'));
  const word = target.path.split(/[\\/]/).pop();
  await env.actions.setFilter(word);
  await env.settle();
  const shown = env.state().step.page.rows;
  const groups = new Set(shown.map((r) => r.group));
  await env.actions.selectAll(true);
  await env.settle();
  assert.deepEqual(last(env, 'selection_set'), { opportunity_id: 'det:exact_duplicates', filter: word, keep_groups: true, selected: true });
  for (const r of await allRows(env, 'det:exact_duplicates')) {
    assert.equal(r.selected, groups.has(r.group) && !r.badges.includes('keeper'), r.path);
  }
});

// ---------------------------------------------------------------- ENG-7 kept badge after a reset

test('after a keep rule and Reset selection the kept badge sits on the copy that stays', async () => {
  const env = await makeEnv();
  await env.actions.openStep('det:exact_duplicates');
  await env.settle();
  for (const rule of ['newest', 'oldest']) {
    await env.actions.applyKeepRule(rule);
    await env.settle();
    await env.actions.resetSelection();
    await env.settle();
    const rows = await allRows(env, 'det:exact_duplicates');
    const groups = new Map();
    for (const r of rows) if (r.group) groups.set(r.group, [...(groups.get(r.group) ?? []), r]);
    for (const [g, members] of groups) {
      const left = members.filter((r) => !r.selected);
      const kept = members.filter((r) => r.badges.includes('keeper'));
      if (left.length === 0) continue;
      assert.equal(kept.length, 1, `group ${g}`);
      assert.ok(!kept[0].selected, `group ${g}: the kept copy is not selected`);
    }
  }
});

// ---------------------------------------------------------------- ENG-9 quarantine days

test('the quarantine action holds items for the Settings value, not a fixed 7 days', async () => {
  const env = await makeEnv();
  const s = await env.backend.call('settings_get', {});
  s.general.quarantine_days = 12;
  await env.backend.call('settings_set', { settings: s });
  await env.actions.openStep('det:old_installers');
  await env.settle();
  await env.actions.setStepAction('quarantine');
  await env.settle();
  assert.deepEqual(env.state().step.pendingAction, { kind: 'quarantine', purge_after_days: 12 });
  env.store.set({ settings: { ...s, general: { ...s.general, quarantine_days: 9 } } });
  await env.actions.setStepAction('quarantine');
  await env.settle();
  assert.deepEqual(env.state().step.pendingAction, { kind: 'quarantine', purge_after_days: 9 }, 'the loaded settings win');
});

// ---------------------------------------------------------------- IPC-5 trash route follows settings

test('app_info is refetched after a settings change, so the trash route label follows the toggle', async () => {
  const env = await makeEnv();
  assert.equal(env.state().info.trash_route, 'quarantine');
  const s = await env.backend.call('settings_get', {});
  s.general.windows_recycle_bin = true;
  await env.backend.call('settings_set', { settings: s });
  env.clearCalls();
  await env.actions.loadSettings();
  await env.settle();
  assert.equal(env.sent('app_info').length, 1);
  assert.equal(env.state().info.trash_route, 'recycle_bin');
  s.general.windows_recycle_bin = false;
  await env.backend.call('settings_set', { settings: s });
  await env.actions.loadSettings();
  assert.equal(env.state().info.trash_route, 'quarantine');
});

// ---------------------------------------------------------------- IPC-8 no report fetch for a button

test('the Last report button is enabled from the history list; the report is fetched on click only', async () => {
  const env = await makeEnv();
  await env.actions.refreshHistory();
  await env.settle();
  assert.equal(env.sent('report_last').length, 0);
  const runs = env.state().history.runs;
  const want = runs.some((r) => r.kind !== 'undo' && r.items > 0);
  assert.equal(!!env.state().lastReport, want);
  assert.ok(want, 'the fixture history has an apply run');
  await env.click(env.$('[data-fk="last-report"]'));
  assert.equal(env.sent('report_last').length, 1);
});
