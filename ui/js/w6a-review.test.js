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
