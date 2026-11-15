// W6-A (fourth task): front-end review findings (UI-2, UI-4..UI-11, DUP-2).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { makeEnv } from './testing/env.js';

const last = (env, cmd) => env.sent(cmd).at(-1)?.req;

// ---------------------------------------------------------------- UI-2 undo the run that finished

test('Undo on a finished run undoes that run, never the newest undoable one', async () => {
  for (const fk of ['done-undo', 'queue-undo']) {
    const env = await makeEnv({ scenario: 'rich' });
    const runs = env.state().history.runs;
    const older = runs.filter((r) => r.undoable)[1];
    assert.ok(older && older.run_id !== runs[0].run_id, 'an undoable run that is not the newest');
    env.store.set({ completed: { run_id: older.run_id, saved: 10, files: 2, failed: 0, cancelled: 0 } });
    await env.settle();
    env.clearCalls();
    await env.click(env.$(`[data-fk="${fk}"]`));
    assert.equal(env.sent('undo_last').length, 0, `${fk} must not undo the newest run`);
    assert.deepEqual(last(env, 'undo_run'), { run_id: older.run_id }, fk);
  }
});

test('Undo on a finished run with nothing undoable is disabled and says why', async () => {
  const env = await makeEnv({ scenario: 'rich' });
  const none = env.state().history.runs.find((r) => !r.undoable);
  env.store.set({ completed: { run_id: none.run_id, saved: 0, files: 0, failed: 3, cancelled: 0 } });
  await env.settle();
  for (const fk of ['done-undo', 'queue-undo']) {
    const b = env.$(`[data-fk="${fk}"]`);
    assert.ok(b.disabled, `${fk} disabled`);
    assert.match(b.getAttribute('title') ?? '', /nothing/i);
  }
  env.clearCalls();
  await env.click(env.$('[data-fk="done-undo"]'));
  assert.equal(env.sent('undo_last').length + env.sent('undo_run').length, 0);
});

// ---------------------------------------------------------------- DUP-2 the keep rule is recorded

test('the keep-rule dropdown shows the rule last applied, and the preferred folder reopens the dialog there', async () => {
  const env = await makeEnv({ scenario: 'rich', dialog: { folder: '/data/keep here' } });
  await env.actions.openStep('det:exact_duplicates');
  await env.settle();
  const chosen = () => Array.from(env.$('[data-fk="keep-rule"]').querySelectorAll('option')).find((o) => o.selected)?.getAttribute('value') ?? env.$('[data-fk="keep-rule"]').value;
  for (const rule of ['oldest', 'shortest_path', 'newest']) {
    env.$('[data-fk="keep-rule"]').setValue(rule);
    await env.settle();
    assert.deepEqual(last(env, 'selection_apply_keep_rule').rule, { rule });
    assert.equal(chosen(), rule, 'the dropdown follows the rule just applied');
    assert.equal(env.state().step.detail.opportunity.params.keep_rule, rule);
  }
  env.$('[data-fk="keep-rule"]').setValue('preferred_folder');
  await env.settle();
  assert.deepEqual(last(env, 'selection_apply_keep_rule').rule, { rule: 'preferred_folder', folder: '/data/keep here' });
  assert.equal(chosen(), 'preferred_folder');
  env.$('[data-fk="keep-rule"]').setValue('preferred_folder');
  await env.settle();
  assert.equal(last(env, 'dialog_pick_folder').start_dir, '/data/keep here', 'the dialog opens at the recorded folder');
});

test('without a recorded rule the dropdown shows the default of the card setting, not a fixed one', async () => {
  const { keepValue } = await import('./views/step/clean-panel.js');
  const settings = [{ key: 'keep_rule', kind: { default: 'oldest', options: [] } }];
  assert.equal(keepValue({ params: {}, settings }), 'oldest');
  assert.equal(keepValue({ params: { keep_rule: 'newest' }, settings }), 'newest');
  assert.equal(keepValue({ params: { keep_rule: { rule: 'shortest_path' } }, settings }), 'shortest_path');
});

// ---------------------------------------------------------------- UI-4 scan responses after scan://done

/** `cmd` answers only after the scan's done event reached the UI (another channel, no order). */
function doneBeforeAnswer(env, cmd, state) {
  const call = env.backend.call.bind(env.backend);
  env.backend.call = async (c, req) => {
    const r = await call(c, req);
    if (c === cmd) {
      const status = { ...env.backend.model.scan, generation: r.generation, state };
      env.backend.emit('scan://done', structuredClone(status));
    }
    return r;
  };
}

test('a scan_stop answer that arrives after scan://done does not leave the overview stopping', async () => {
  const env = await makeEnv({ scenario: 'results', speed: 1 });
  await env.actions.startScan();
  assert.equal(env.state().scan.state, 'scanning');
  doneBeforeAnswer(env, 'scan_stop', 'cancelled');
  await env.actions.stopScan();
  assert.equal(env.state().scan.state, 'cancelled');
  assert.ok(!env.$('[data-fk="scan-stop"]')); // never inspects a DOM node on failure
  await env.settle();
});

test('a scan_start answer that arrives after its own scan://done does not leave it scanning', async () => {
  const env = await makeEnv({ scenario: 'results' });
  doneBeforeAnswer(env, 'scan_start', 'done');
  await env.actions.startScan();
  assert.equal(env.state().scan.state, 'done');
  await env.settle();
  assert.equal(env.state().scan.state, 'done');
});

// ---------------------------------------------------------------- UI-6 the open review follows a rescan

const rowIds = (env) => env.$$('[data-row]').map((r) => r.getAttribute('data-row'));

test('the open step review reloads its rows after a rescan instead of keeping stale ids', async () => {
  const env = await makeEnv({ scenario: 'results' });
  await env.actions.openStep('det:exact_duplicates');
  await env.settle();
  const before = rowIds(env);
  assert.ok(before.length > 0);
  // The rescan gives every candidate a new id (ids are valid for one generation).
  const m = env.backend.model;
  for (const r of m.rows.get('det:exact_duplicates')) r.id += 1_000_000;
  m.selection.set('det:exact_duplicates', new Set(m.rows.get('det:exact_duplicates').filter((r) => r.default_selected).map((r) => r.id)));
  m.scan.generation += 1;
  env.clearCalls();
  env.backend.emit('scan://done', { ...structuredClone(m.scan), state: 'done' });
  env.backend.emit('opportunities://changed', { generation: m.scan.generation });
  await env.settle();
  assert.equal(env.sent('candidates_page').length, 1, 'the rows are read again, once for both events');
  const after = rowIds(env);
  assert.equal(after.length, before.length);
  assert.ok(after.every((id) => !before.includes(id)), 'only the new ids are shown');
});

test('the open Custom-search results reload after a finished apply', async () => {
  const env = await makeEnv({ scenario: 'results' });
  const { freshStep } = await import('./ctl-step.js');
  env.store.set({ custom: { ...env.state().custom, result: { id: 'det:exact_duplicates', title: 'x', count: 3 }, step: freshStep('det:exact_duplicates') } });
  env.clearCalls();
  env.backend.emit('history://changed', { run_id: 1 });
  await env.settle();
  assert.deepEqual(env.sent('candidates_page').map((c) => c.req.opportunity_id), ['det:exact_duplicates']);
  assert.ok(env.state().custom.step.page, 'the results slot holds the fresh page');
});

// ---------------------------------------------------------------- UI-10 a failed load is not "Loading..." forever

test('a failed step_detail shows an error with Retry instead of Loading forever', async () => {
  const env = await makeEnv({ scenario: 'results' });
  env.backend.failNext('step_detail', { code: 'invalid_input', message: 'unknown opportunity det:exact_duplicates' });
  await env.actions.openStep('det:exact_duplicates');
  await env.settle();
  const err = env.$('.state--error');
  assert.ok(err, 'an error block');
  assert.match(err.textContent, /unknown opportunity/);
  assert.equal(env.$('.view').textContent.includes('Loading'), false);
  await env.click(err.querySelector('button'));
  assert.ok(!env.$('.state--error')); // never inspects a DOM node on failure
  assert.ok(env.$$('[data-row]').length > 0, 'Retry loads the review');
});

test('a failed candidates_page shows an error with Retry under the step header', async () => {
  const env = await makeEnv({ scenario: 'results' });
  env.backend.failNext('candidates_page', { code: 'io', message: 'disk went away' });
  await env.actions.openStep('det:exact_duplicates');
  await env.settle();
  const err = env.$('.state--error');
  assert.ok(err, 'an error block');
  assert.match(err.textContent, /disk went away/);
  await env.click(err.querySelector('button'));
  assert.ok(!env.$('.state--error')); // never inspects a DOM node on failure
  assert.ok(env.$$('[data-row]').length > 0);
});

// ---------------------------------------------------------------- UI-5 the category lists follow the plan and rescans

test('Clean: "+ Add" turns the card into an in-plan card at once, and a rescan refreshes the list', async () => {
  const env = await makeEnv({ scenario: 'results' });
  env.actions.navigate('clean');
  await env.settle();
  const add = env.$('.view [data-fk^="add-"]');
  assert.ok(add, 'a card that is not in the plan');
  const id = add.getAttribute('data-fk').slice(4);
  await env.click(add);
  assert.ok(!env.$(`[data-fk="add-${id}"]`), 'no second "+ Add"'); // never inspects a DOM node on failure
  assert.ok(env.state().category.items.find((o) => o.id === id).in_plan);
  // A finished scan re-reads the list on screen.
  env.clearCalls();
  env.backend.emit('scan://done', { ...structuredClone(env.backend.model.scan), state: 'done' });
  await env.settle();
  assert.ok(env.sent('opportunities_list').some((c) => c.req.tab === 'clean'), 'the Clean list is read again');
});
