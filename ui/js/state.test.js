import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createStore, initialState, splitCards, planSaving, isScanning, activeJobs, stepFor, hasResults } from './state.js';
import { upsertJob, completedRun } from './ctl-apply.js';

test('the store replaces state, never mutates it, and notifies subscribers', () => {
  const store = createStore({ a: 1, b: 2 });
  const before = store.get();
  const seen = [];
  store.subscribe((s, changed) => seen.push([s.a, changed]));
  store.set({ a: 5 });
  assert.notEqual(store.get(), before);
  assert.equal(before.a, 1);
  assert.deepEqual(seen, [[5, ['a']]]);
});

test('setting an identical value does not notify', () => {
  const store = createStore({ a: 1 });
  let n = 0;
  store.subscribe(() => n++);
  store.set({ a: 1 });
  assert.equal(n, 0);
});

test('key-filtered subscribers only hear about their keys, and can unsubscribe', () => {
  const store = createStore({ a: 1, b: 1 });
  let n = 0;
  const off = store.subscribe(() => n++, ['b']);
  store.set({ a: 2 });
  assert.equal(n, 0);
  store.set({ b: 2 });
  assert.equal(n, 1);
  off();
  store.set({ b: 3 });
  assert.equal(n, 1);
});

test('set accepts a function of the current state', () => {
  const store = createStore({ n: 1 });
  store.set((s) => ({ n: s.n + 1 }));
  assert.equal(store.get().n, 2);
});

test('initial state is the empty first-run state', () => {
  const s = initialState();
  assert.equal(s.route.name, 'overview');
  assert.equal(s.plan.steps.length, 0);
  assert.equal(isScanning(s), false);
  assert.equal(hasResults(s), false);
  assert.equal(planSaving(s), 0);
});

test('splitCards keeps nine on the first page', () => {
  const items = Array.from({ length: 18 }, (_, i) => ({ id: i }));
  const { main, small } = splitCards(items);
  assert.equal(main.length, 9);
  assert.equal(small.length, 9);
  assert.equal(small[0].id, 9);
});

test('selectors: plan saving, active jobs, step lookup', () => {
  const s = { ...initialState(), plan: { steps: [{ id: 1, opportunity: 'x' }], estimate: { bytes_before: 100, bytes_after: 30 } }, jobs: [{ id: 1, state: 'running' }, { id: 2, state: 'completed' }] };
  assert.equal(planSaving(s), 70);
  assert.deepEqual(activeJobs(s).map((j) => j.id), [1]);
  assert.equal(stepFor(s, 'x').id, 1);
  assert.equal(stepFor(s, 'y'), null);
});

test('upsertJob merges by id and completedRun waits for the last job of the run', () => {
  let jobs = upsertJob([], { id: 1, run_id: 9, kind: 'apply', state: 'running' });
  jobs = upsertJob(jobs, { id: 2, run_id: 9, kind: 'apply', state: 'queued' });
  jobs = upsertJob(jobs, { id: 1, state: 'completed', outcome: { bytes_saved: 10, succeeded: 3, failed: 0 } });
  assert.equal(jobs.length, 2);
  assert.equal(completedRun(jobs, 9), null);
  jobs = upsertJob(jobs, { id: 2, state: 'completed', outcome: { bytes_saved: 5, succeeded: 2, failed: 1 } });
  assert.deepEqual(completedRun(jobs, 9), { run_id: 9, saved: 15, files: 5, failed: 1, cancelled: 0 });
});
