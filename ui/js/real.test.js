// The views render REAL engine responses, not only mock data (W4-INT item 6).
// Fixtures in testing/real/ are serialised by crates/engine/tests/ipc_shapes.rs from a
// real session on a generated tree; regenerate with
//   MJ_WRITE_UI_FIXTURES=1 cargo test -p mj-engine --test ipc_shapes
// Every command with a fixture answers with it; the rest (events, writes) use the mock.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { makeEnv } from './testing/env.js';

const dir = join(dirname(fileURLToPath(import.meta.url)), 'testing', 'real');
const real = Object.fromEntries(readdirSync(dir).filter((f) => f.endsWith('.json')).map((f) => [f.replace(/\.json$/, ''), JSON.parse(readFileSync(join(dir, f), 'utf8'))]));

/** A mounted app whose backend answers from the real fixtures. */
async function realEnv() {
  const env = await makeEnv({ scenario: 'results', mount: false });
  const mock = env.backend.call.bind(env.backend);
  const errors = [];
  env.backend.call = async (cmd, req) => {
    if (cmd === 'jobs_list' && real.jobs_list_all) return structuredClone(real.jobs_list_all);
    if (real[cmd] !== undefined) return structuredClone(real[cmd]);
    return mock(cmd, req);
  };
  const { mountApp } = await import('./app.js');
  const { bridgeEvents } = await import('./events.js');
  const origError = console.error;
  console.error = (...a) => { errors.push(a.join(' ')); };
  mountApp(env.root, env.store, env.actions, {});
  await bridgeEvents({ api: env.api, store: env.store, actions: env.actions });
  await env.actions.init();
  await env.settle();
  console.error = origError;
  env.errors = errors;
  return env;
}

test('real fixtures exist for the start-up and view commands', () => {
  for (const cmd of ['app_info', 'scope_get', 'scan_status', 'opportunities_list', 'plan_get', 'history_list', 'history_run_detail', 'step_detail', 'candidates_page', 'settings_get', 'disk_usage_tree', 'report_last']) {
    assert.ok(real[cmd] !== undefined, `testing/real/${cmd}.json missing`);
  }
});

test('overview renders the real opportunities, hero summary and plan', async () => {
  const env = await realEnv();
  assert.equal(env.state().ready, true);
  assert.deepEqual(env.errors, []);
  const text = env.root.textContent;
  const available = real.opportunities_list.items.filter((o) => o.availability.state === 'available');
  assert.ok(available.length >= 3);
  for (const o of available) assert.ok(text.includes(o.title), `card "${o.title}" rendered`);
  for (const s of real.plan_get.steps) assert.ok(text.includes(s.title), `plan step "${s.title}" rendered`);
  assert.doesNotMatch(text, /undefined|NaN|\[object Object\]/);
});

test('step review renders real candidate rows', async () => {
  const env = await realEnv();
  await env.actions.openStep(real.step_detail.opportunity.id);
  await env.settle();
  const text = env.root.textContent;
  assert.ok(text.includes(real.step_detail.opportunity.title));
  const names = real.candidates_page.rows.map((r) => r.path.split('/').pop());
  assert.ok(names.length > 0);
  for (const n of names) assert.ok(text.includes(n), `row ${n} rendered`);
  assert.doesNotMatch(text, /undefined|NaN|\[object Object\]/);
});

test('history renders the real run and its journal entries', async () => {
  const env = await realEnv();
  env.actions.navigate('history');
  await env.settle();
  const runs = env.$$('.run');
  assert.equal(runs.length, real.history_list.runs.length);
  await env.click(runs[0].querySelector('.run__toggle'));
  assert.equal(env.$$('.entry').length, real.history_run_detail.entries.length);
  assert.doesNotMatch(env.root.textContent, /undefined|NaN|\[object Object\]/);
});

test('settings, disk usage and the completed report render real data', async () => {
  const env = await realEnv();
  for (const view of ['settings', 'disk']) {
    env.actions.navigate(view);
    await env.settle();
    assert.doesNotMatch(env.root.textContent, /undefined|NaN|\[object Object\]/, view);
  }
  env.actions.navigate('disk');
  await env.settle();
  const top = (real.disk_usage_tree.children ?? [])[0];
  if (top) assert.ok(env.root.textContent.includes(top.name), `disk tree shows ${top.name}`);
  assert.deepEqual(env.errors, []);
});
