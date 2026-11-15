// W6-A: duplicate review with thumbnails and groups kept together.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { makeEnv, HOSTILE_FILE } from './testing/env.js';
import { createThumbLoader, thumbContent, typeIcon, wantsThumb, BATCH } from './thumbs.js';
import { orderPage } from './mock/review-order.js';
import { installFakeDom } from './testing/fakedom.js';
import { h } from './dom.js';

const last = (env, cmd) => env.sent(cmd).at(-1)?.req;
const groupOf = (env, row) => env.state().step.page.rows.find((r) => String(r.id) === row.getAttribute('data-row'))?.group;

async function openDups() {
  const env = await makeEnv();
  await env.actions.openStep('det:exact_duplicates');
  await env.settle();
  return env;
}

/** Rows of the table grouped in contiguous runs, the kept copy first in each. */
function assertContiguous(env) {
  const seen = new Set();
  let prev;
  for (const row of env.$$('.row')) {
    const g = groupOf(env, row);
    if (g !== prev) {
      assert.ok(!seen.has(g), `group ${g} is split`);
      seen.add(g);
      assert.ok(row.classList.contains('row--keeper'), 'the kept copy opens its group');
    }
    prev = g;
  }
  return seen.size;
}

test('duplicates: "Keep groups together" is on by default and sorting by size keeps groups whole', async () => {
  const env = await openDups();
  const toggle = env.$('[data-fk="filter-keep-groups"]');
  assert.ok(toggle, 'visible control');
  assert.equal(toggle.checked, true);
  assert.equal(toggle.getAttribute('aria-label'), 'Keep groups together');
  assert.equal(last(env, 'candidates_page').keep_groups, true);
  await env.click(env.$('[data-fk="sort-size"]'));
  const req = last(env, 'candidates_page');
  assert.equal(req.sort, 'size');
  assert.equal(req.keep_groups, true);
  const groups = assertContiguous(env);
  assert.equal(env.$$('.group-head').length, groups, 'a header for every group, whatever the sort');
  assert.match(env.$('.group-head').textContent, /copies of .*100 % match {2}· {2}\d files {2}· {2}.+ each {2}· {2}.+ reclaimable/);
  // Biggest group first (the largest file of each group).
  const stats = env.state().step.page.group_stats;
  const order = [...new Set(env.state().step.page.rows.map((r) => r.group))].map((g) => stats.find((s) => s.id === g).largest);
  assert.deepEqual(order, [...order].sort((a, b) => b - a));
});

test('duplicates: turning the mode off gives the flat list, on again restores groups; it survives paging and selection, resets on another step', async () => {
  const env = await openDups();
  await env.click(env.$('[data-fk="sort-size"]'));
  await env.click(env.$('[data-fk="filter-keep-groups"]'));
  assert.equal(last(env, 'candidates_page').keep_groups, false);
  assert.equal(env.$$('.group-head').length, 0, 'flat: no headers when not sorted by group');
  assert.equal(env.state().step.sort, 'size', 'the sort stays');
  await env.click(env.$('[data-fk="filter-keep-groups"]'));
  assert.ok(env.$$('.group-head').length > 0);
  assertContiguous(env);
  // Paging and a selection change keep the mode and the sort.
  await env.click(env.$('[data-fk="page-next"]'));
  assert.equal(last(env, 'candidates_page').keep_groups, true);
  assert.equal(last(env, 'candidates_page').sort, 'size');
  await env.click(env.$('[data-fk="filter-keep-groups"]'));
  await env.click(env.$$('.row .checkbox').find((c) => !c.disabled));
  assert.equal(env.$('[data-fk="filter-keep-groups"]').checked, false);
  assert.equal(last(env, 'candidates_page').keep_groups, false);
  // Another step starts fresh.
  await env.actions.openStep('det:old_installers');
  await env.settle();
  await env.actions.openStep('det:exact_duplicates');
  await env.settle();
  assert.equal(env.$('[data-fk="filter-keep-groups"]').checked, true);
  assert.equal(env.state().step.sort, 'group');
});

test('duplicates: the size filter keeps whole groups (their largest file decides)', async () => {
  const env = await openDups();
  const sel = env.$('[data-fk="filter-min-size"]');
  assert.equal(sel.getAttribute('aria-label'), 'At least');
  sel.setValue(String(10 * 1024 ** 2));
  await env.settle();
  const req = last(env, 'candidates_page');
  assert.equal(req.min_size, 10 * 1024 ** 2);
  assert.equal(req.offset, 0);
  const page = env.state().step.page;
  assert.ok(page.total > 0 && page.total < 214);
  assertContiguous(env);
  for (const s of page.group_stats) assert.ok(s.largest >= 10 * 1024 ** 2);
});

test('select all and the keep rule behave the same with groups kept together', async () => {
  const env = await openDups();
  await env.click(env.$('[data-fk="sort-size"]'));
  // Everything but the kept copies starts selected: the header box clears, then selects all.
  await env.click(env.$('[data-fk="select-all"]'));
  assert.deepEqual(last(env, 'selection_set'), { opportunity_id: 'det:exact_duplicates', selected: false });
  for (const r of env.state().step.page.rows) assert.equal(r.selected, false);
  await env.click(env.$('[data-fk="select-all"]'));
  assert.deepEqual(last(env, 'selection_set'), { opportunity_id: 'det:exact_duplicates', selected: true });
  for (const r of env.state().step.page.rows) assert.equal(r.selected, !r.badges.includes('keeper'), r.path);
  env.$('[data-fk="keep-rule"]').setValue('oldest');
  await env.settle();
  assertContiguous(env);
  assert.equal(last(env, 'candidates_page').keep_groups, true);
});

test('thumbnails: one request per exact group, bounded batches, images patched into their cells, type icons otherwise', async () => {
  const env = await openDups();
  const calls = env.sent('candidate_thumbnails');
  assert.ok(calls.length > 0);
  const rows = env.state().step.page.rows;
  const imageGroups = new Set(rows.filter((r) => /\.(jpe?g|png)$/i.test(r.path)).map((r) => r.group));
  const asked = calls.flatMap((c) => c.req.candidate_ids);
  assert.equal(asked.length, imageGroups.size, 'one id per exact group with images');
  for (const c of calls) assert.ok(c.req.candidate_ids.length <= BATCH);
  for (const row of env.$$('.row')) {
    const r = rows.find((x) => String(x.id) === row.getAttribute('data-row'));
    const cell = row.querySelector('.thumb');
    assert.ok(cell, 'every row has a thumbnail cell');
    if (/\.(jpe?g|png)$/i.test(r.path)) {
      const img = cell.querySelector('img');
      assert.ok(img, `image row ${r.path}`);
      assert.ok(img.getAttribute('src').startsWith('data:image/png;base64,'));
      assert.match(img.getAttribute('alt'), /^Thumbnail of /);
    } else {
      assert.equal(cell.querySelector('img'), null);
      assert.ok(cell.querySelector('.thumb__icon svg'), `type icon for ${r.path}`);
      assert.ok(cell.querySelector('.thumb__icon').getAttribute('aria-label'));
    }
  }
  // Cached: the next render (a selection change) asks nothing again.
  env.clearCalls();
  await env.click(env.$$('.row .checkbox').find((c) => !c.disabled));
  assert.equal(env.sent('candidate_thumbnails').length, 0);
  assert.ok(env.$('.thumb img'));
});

test('thumbnails are not shown for steps without groups (audio, installers)', async () => {
  const env = await makeEnv();
  await env.actions.openStep('det:old_installers');
  await env.settle();
  assert.equal(env.$('.thumb'), null);
  assert.equal(env.$('[data-fk="filter-keep-groups"]'), null);
  assert.equal(env.sent('candidate_thumbnails').length, 0);
});

test('thumb loader: at most two calls in flight, stale answers dropped, cells patched in place', async () => {
  const doc = installFakeDom();
  const pending = [];
  let inFlight = 0;
  let peak = 0;
  const api = {
    candidateThumbnails: (req) => new Promise((resolve) => {
      inFlight++;
      peak = Math.max(peak, inFlight);
      pending.push(() => { inFlight--; resolve({ thumbnails: req.candidate_ids.map((id) => ({ candidate_id: id, data_url: 'data:image/png;base64,AAAA' })) }); });
    }),
  };
  const loader = createThumbLoader(api);
  const rows = Array.from({ length: 35 }, (_, i) => ({ id: i + 1, path: `C:\\p\\${i}.jpg`, size: 10, modified: 1, badges: [] }));
  const table = h('div', {}, rows.map((r) => h('span', { class: 'thumb', 'data-thumb': String(r.id) })));
  doc.body.append(table);
  const done = loader.load('det:x', rows);
  await Promise.resolve();
  assert.equal(inFlight, 2, 'two batches at a time');
  while (pending.length) { pending.shift()(); await new Promise((r) => setTimeout(r, 0)); }
  await done;
  assert.equal(peak, 2);
  assert.equal(table.querySelectorAll('img').length, 35, 'every cell patched');
  assert.equal(loader.get(rows[0]), 'data:image/png;base64,AAAA');
  // A newer page makes the older answers stale.
  const more = Array.from({ length: 3 }, (_, i) => ({ id: 100 + i, path: `C:\\q\\${i}.png`, size: 1, modified: 2, badges: [] }));
  const first = loader.load('det:x', more);
  await Promise.resolve();
  void loader.load('det:x', []);
  pending.shift()();
  await first;
  assert.equal(loader.get(more[0]), undefined, 'stale answer dropped');
});

test('thumbnails: hostile names stay text, only image data URLs reach src', () => {
  installFakeDom();
  const row = { id: 1, path: `C:\\x\\${HOSTILE_FILE}.png`, size: 1, badges: [] };
  const img = thumbContent(row, 'data:image/png;base64,AAAA');
  assert.equal(img.tagName.toLowerCase(), 'img');
  assert.ok(img.getAttribute('alt').includes('<img src=x'), 'alt keeps the raw string as an attribute');
  assert.equal(img.children.length, 0);
  for (const bad of ['javascript:alert(1)', 'data:text/html;base64,PHNjcmlwdD4=', 'data:image/svg+xml;base64,PHN2Zz4=', 'http://x/y.png']) {
    assert.equal(thumbContent(row, bad).tagName.toLowerCase(), 'span', bad);
  }
  assert.equal(typeIcon({ path: 'a.mp4' }), 'video');
  assert.equal(typeIcon({ path: 'a.pdf' }), 'doc');
  assert.equal(typeIcon({ path: 'a', target: 'dir' }), 'folder');
  assert.equal(wantsThumb({ path: 'a.jpg', badges: ['online_only'] }), false, 'online-only rows are not asked for');
});

test('mock parity: grouped order, whole-group filters and pages mirror the engine', () => {
  const k = (id, path, size, group, keep) => ({ id, path, size, size_after: 0, group, badges: keep ? ['keeper'] : [], modified: id });
  const rows = [k(1, '/a/one', 10, 1, false), k(2, '/b/one', 10, 1, true), k(3, '/c/two', 50, 2, true), k(4, '/d/two-small', 5, 2, false), k(5, '/e/alone', 30, undefined, false)];
  const ids = (r) => r.rows.map((x) => x.id);
  assert.deepEqual(ids(orderPage(rows, { sort: 'size', keep_groups: true }, 50, true)), [3, 4, 5, 2, 1]);
  assert.deepEqual(ids(orderPage(rows, { sort: 'size', descending: true, keep_groups: true }, 50, true)), [2, 1, 5, 3, 4]);
  assert.deepEqual(ids(orderPage(rows, { sort: 'size', keep_groups: false }, 50, true)), [3, 5, 1, 2, 4]);
  assert.deepEqual(ids(orderPage(rows, { sort: 'size', keep_groups: true, min_size: 40 }, 50, true)), [3, 4]);
  assert.deepEqual(ids(orderPage(rows, { sort: 'size', keep_groups: true, filter: '/a/' }, 50, true)), [2, 1]);
  const p = orderPage(rows, { sort: 'size', keep_groups: true }, 2, true);
  assert.deepEqual([p.offset, p.rows.length, p.page, p.pages, p.next, p.prev], [0, 2, 1, 3, 2, null]);
  const g2 = orderPage(rows, { sort: 'size', keep_groups: true }, 50, true).stats.find((s) => s.id === 2);
  assert.deepEqual(g2, { id: 2, files: 2, largest: 50, smallest: 5, reclaimable: 5 });
});
