import { test } from 'node:test';
import assert from 'node:assert/strict';
import { makeEnv } from './testing/env.js';
import { EVENTS } from './api.js';

test('the event bridge subscribes to every event of docs/IPC.md', async () => {
  const env = await makeEnv({ mount: false });
  const seen = new Set();
  const wrapped = { on: async (name, cb) => { seen.add(name); return env.api.on(name, cb); } };
  const { bridgeEvents } = await import('./events.js');
  const off = await bridgeEvents({ api: wrapped, store: env.store, actions: env.actions });
  assert.deepEqual([...seen].sort(), [...EVENTS].sort());
  off();
});

test('scan events of an older generation are ignored', async () => {
  const env = await makeEnv({ scenario: 'fresh' });
  env.store.set({ scan: { ...env.state().scan, state: 'scanning', generation: 5 } });
  const before = env.state().scanProgress;
  env.backend.emit('scan://detector', { generation: 4, opportunity_id: 'x', title: 'Old', saving: 1, count: 1, finished: true });
  env.backend.emit('scan://progress', { generation: 4, phase: 'walking', files: 1, dirs: 1, bytes: 1, done: 0, elapsed_ms: 1 });
  assert.deepEqual(env.state().detectors, {});
  assert.equal(env.state().scanProgress, before, 'progress of an old generation is dropped');
  env.backend.emit('scan://detector', { generation: 5, opportunity_id: 'y', title: 'New', saving: 1, count: 1, finished: true });
  assert.deepEqual(Object.keys(env.state().detectors), ['y']);
});

test('plan://changed replaces the plan; opportunities://changed refetches the list', async () => {
  const env = await makeEnv();
  env.clearCalls();
  env.backend.emit('plan://changed', { id: 9, name: 'Your plan', steps: [], estimate: { files: 0, bytes_before: 0, bytes_after: 0, confidence: 'exact', skipped: 0 }, checklist: [] });
  assert.equal(env.state().plan.id, 9);
  env.backend.emit('opportunities://changed', { generation: 3 });
  await env.settle();
  assert.equal(env.sent('opportunities_list').length, 1);
  assert.equal(env.sent('drives_list').length, 1);
});

test('job events update the queue and finishing every job of a run completes it', async () => {
  const env = await makeEnv();
  const job = (id, state, extra = {}) => ({ id, kind: 'apply', run_id: 7, title: `J${id}`, state, progress: { done: 0, total: 1, bytes_done: 0, bytes_total: 1, saved: 0 }, ...extra });
  env.backend.emit('job://state', job(1, 'running'));
  env.backend.emit('job://state', job(2, 'queued'));
  assert.equal(env.$$('.job').length, 2);
  env.backend.emit('job://done', { job: job(1, 'completed', { outcome: { succeeded: 1, skipped: 0, failed: 0, bytes_saved: 100, errors: [] } }), report_available: true });
  assert.equal(env.state().completed, null);
  env.backend.emit('job://done', { job: job(2, 'completed', { outcome: { succeeded: 1, skipped: 0, failed: 1, bytes_saved: 50, errors: [] } }), report_available: true });
  assert.deepEqual(env.state().completed, { run_id: 7, saved: 150, files: 2, failed: 1, cancelled: 0 });
  assert.equal(env.state().reclaimed, 150);
  assert.match(env.$('[data-role="completed"]').textContent, /1 item could not be changed/);
});

test('history://changed refreshes history, undo availability and the last report', async () => {
  const env = await makeEnv();
  env.clearCalls();
  env.backend.emit('history://changed', { run_id: 1 });
  await env.settle();
  assert.equal(env.sent('history_list').length, 1);
  assert.equal(env.sent('report_last').length, 1);
});

test('plan://dry_run clears the running flag', async () => {
  const env = await makeEnv();
  env.store.set({ dryRunning: true });
  env.backend.emit('plan://dry_run', { plan_id: 1, job_id: 1, steps: [], total: { files: 3, bytes_before: 2048, bytes_after: 0, confidence: 'exact', skipped: 0 } });
  assert.equal(env.state().dryRunning, false);
});

test('progress events never re-render the controls (clicks would be swallowed)', async () => {
  const env = await makeEnv({ speed: 1 });
  await env.actions.startApply();
  await env.settle();
  await env.actions.dialogSet({ ack: true });
  await env.actions.dialogConfirm();
  const pause = env.$('[data-fk^="pause-"]');
  env.backend.emit('job://progress', { job_id: env.state().jobs[0].id, run_id: 1, progress: { done: 50, total: 153, bytes_done: 1, bytes_total: 2, saved: 0 }, current: 'x' });
  assert.equal(env.$('[data-fk^="pause-"]'), pause, 'same element after a progress event');
  await env.actions.cancelJob(env.state().jobs[0].id);
});
