import { test } from 'node:test';
import assert from 'node:assert/strict';
import { makeEnv, HOSTILE, HOSTILE_FILE } from './testing/env.js';

const ALLOWED_TAGS = new Set(['div', 'span', 'p', 'h1', 'h2', 'h3', 'ul', 'ol', 'li', 'button', 'input', 'select', 'option', 'label', 'nav', 'main', 'aside', 'footer', 'header', 'section', 'article', 'table', 'thead', 'tbody', 'tr', 'th', 'td', 'form', 'strong', 'dl', 'dt', 'dd', 'svg', 'path']);

/** Every element under `root` must be one the UI creates itself; hostile text must never add one. */
function assertNoInjectedMarkup(root) {
  const all = [];
  const walk = (n) => { for (const c of n.children ?? []) { all.push(c); walk(c); } };
  walk(root);
  const bad = all.filter((e) => !ALLOWED_TAGS.has(e.tagName.toLowerCase()));
  assert.deepEqual(bad.map((e) => e.tagName), [], 'unexpected elements (injected markup?)');
  for (const e of all) for (const name of e.attributes.keys()) assert.equal(/^on/i.test(name), false, `inline handler attribute ${name}`);
}

test('a hostile file name in the results table is shown as text only', async () => {
  const env = await makeEnv();
  env.backend.model.rows.get('opt:audio')[0].path = `C:\\Users\\Michel\\Recordings\\${HOSTILE_FILE}`;
  await env.actions.openStep('opt:audio');
  await env.settle();
  const cell = env.$$('.row')[0].querySelector('.cell--name');
  assert.ok(cell.textContent.includes(HOSTILE_FILE));
  assert.equal(cell.querySelectorAll('img').length, 0);
  assert.equal(env.$$('script').length, 0);
  assert.equal(cell.getAttribute('title').endsWith(HOSTILE_FILE), true, 'attributes keep the raw string, unparsed');
  assertNoInjectedMarkup(env.root);
});

test('hostile names in scope chips, card titles, plan steps, history and toast are text only', async () => {
  const env = await makeEnv();
  const m = env.backend.model;
  m.scope.roots = [`C:\\${HOSTILE}`, 'D:\\Projects'];
  m.opps[0].title = HOSTILE;
  m.opps[0].description = HOSTILE;
  m.plan.steps[0].title = HOSTILE;
  m.plan.steps[0].summary = HOSTILE;
  await env.actions.init();
  await env.settle();
  assert.ok(env.$('.chip__text').textContent.includes(HOSTILE));
  assert.equal(env.$('.card__title').textContent, HOSTILE);
  assert.equal(env.$('.step__title').textContent, HOSTILE);
  assert.equal(env.$('.step__title').getAttribute('title'), HOSTILE);
  assertNoInjectedMarkup(env.root);
  env.store.set({ toast: { level: 'error', message: HOSTILE, hint: HOSTILE } });
  assert.ok(env.$('.toast').textContent.includes(HOSTILE));
  m.runs[0].title = HOSTILE;
  m.runs[0].steps = [HOSTILE];
  env.actions.navigate('history');
  await env.settle();
  assert.ok(env.$('.run__title').textContent.includes(HOSTILE));
  assertNoInjectedMarkup(env.root);
});

test('hostile folder names in the folder browser and a hostile scan path are text only', async () => {
  const env = await makeEnv({ scenario: 'empty' });
  env.backend.model.scope.roots = [];
  await env.actions.openBrowser();
  await env.settle();
  const b = env.state().browser;
  env.store.set({ browser: { ...b, entries: [{ name: HOSTILE, path: `C:\\${HOSTILE}`, is_dir: true, is_link: false, hidden: false }], path: `C:\\${HOSTILE}` } });
  assert.ok(env.$('.browser__entry').textContent.includes(HOSTILE));
  assertNoInjectedMarkup(env.root);
  env.store.set({ browser: null, scan: { state: 'scanning', generation: 1 }, scanProgress: { generation: 1, phase: 'walking', files: 1, dirs: 1, bytes: 1, done: 0, current: HOSTILE, elapsed_ms: 1 } });
  assert.equal(env.$('.scanhead__path').textContent, HOSTILE);
  assertNoInjectedMarkup(env.root);
});

test('a hostile name in a dialog and in the report is text only', async () => {
  const env = await makeEnv();
  const info = { step_id: 1, title: HOSTILE, body: HOSTILE, files: 1, bytes_now: 10, bytes_after: 5, saving: 5, confidence: 'exact', originals_options: [{ policy: { policy: 'recycle_bin' }, label: HOSTILE, reversible: true }], default_originals: { policy: 'recycle_bin' }, synced: { count: 1, provider: HOSTILE, can_online_only: true }, acknowledge_text: HOSTILE, undo_text: HOSTILE, action_label: HOSTILE, permanent: false };
  env.store.set({ dialog: { infos: [info], index: 0, answers: [], current: { ack: false, originalsIndex: 0, onlineOnly: false } } });
  assert.equal(env.$('.modal__title').textContent, HOSTILE);
  assert.ok(env.$('.callout__title').textContent.includes(HOSTILE));
  assertNoInjectedMarkup(env.root);
  env.store.set({ dialog: null, report: { run_id: 1, title: HOSTILE, started: 1, totals: { files: 1, succeeded: 1, skipped: 0, failed: 0, bytes_before: 1, bytes_after: 0, bytes_saved: 1 }, sections: [{ title: HOSTILE }] } });
  assert.ok(env.$('.report__sections').textContent.includes(HOSTILE));
  assertNoInjectedMarkup(env.root);
});

test('a hostile engine error message is shown as text', async () => {
  const env = await makeEnv();
  env.actions.fail?.call;
  env.store.set({ toast: { level: 'error', message: `Permission denied: ${HOSTILE}`, hint: 'Skip it.' } });
  assertNoInjectedMarkup(env.root);
  assert.ok(env.$('.toast').textContent.includes('<img src=x'));
});
