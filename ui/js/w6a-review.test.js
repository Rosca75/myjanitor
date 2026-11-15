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
  assert.equal(env.$('[data-fk="scan-stop"]'), null);
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
