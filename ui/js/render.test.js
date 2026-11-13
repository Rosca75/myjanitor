import { test } from 'node:test';
import assert from 'node:assert/strict';
import { makeEnv } from './testing/env.js';

const texts = (els) => els.map((e) => e.textContent);

test('empty state: scope picker and one primary Scan button', async () => {
  const env = await makeEnv({ scenario: 'empty' });
  assert.ok(env.$('.welcome'));
  assert.equal(env.$$('.card').length, 0);
  assert.match(env.$('.picker__none').textContent, /No locations yet/);
  const scan = env.$('[data-fk="scan"]');
  assert.ok(scan.disabled, 'Scan is disabled without a folder');
  assert.ok(scan.getAttribute('title'));
  assert.ok(scan.classList.contains('btn--primary'));
  assert.equal(env.$$('.btn--primary', env.$('.app__main')).length, 1, 'one primary button in the view');
  assert.ok(env.$('[data-fk="add-folder"]'));
});

test('empty state with a folder enables Scan and lists the chip', async () => {
  const env = await makeEnv({ scenario: 'empty' });
  await env.actions.openBrowser();
  await env.settle();
  await env.actions.chooseFolder();
  await env.settle();
  assert.equal(env.$$('.chip').length, 1);
  assert.equal(env.$('[data-fk="scan"]').disabled, false);
});

test('results: nine cards, hero numbers, tabs with counts, plan panel', async () => {
  const env = await makeEnv({ scenario: 'results' });
  const cards = env.$$('.card', env.$('.app__main'));
  assert.equal(cards.length, 9);
  assert.equal(env.$('[data-role="hero-total"]').textContent, '48.6 GB');
  assert.match(env.$('.hero__across').textContent, /across 18 opportunities/);
  assert.deepEqual(texts(env.$$('.legend__item', env.$('.hero__left'))), ['Reversible 41.2 GB', 'Lossy, opt-in 5.1 GB', 'Review only 2.3 GB']);
  assert.deepEqual(texts(env.$$('.tab')), ['All 18', 'Clean 8', 'Optimise 6', 'Review only 3']);
  assert.equal(env.$('.tab--active').textContent, 'All 18');
  assert.equal(env.$('.header__scanned').textContent, 'Scanned 2 min ago  ·  cached');
  assert.equal(env.$$('.chip').length, 2);
  assert.ok(env.byText('button', '+ Add location'));
  assert.ok(env.byText('button', 'Scan again'));
});

test('results: card content, safety badges and the in-plan state', async () => {
  const env = await makeEnv({ scenario: 'results' });
  const card = (id) => env.$(`[data-opp="${id}"]`);
  const dup = card('det:exact_duplicates');
  assert.ok(dup.classList.contains('card--in-plan'));
  assert.equal(dup.querySelector('.card__title').textContent, 'Exact duplicates');
  assert.equal(dup.querySelector('.card__desc').textContent, '214 files in 61 groups · keeps newest');
  assert.equal(dup.querySelector('.card__size').textContent, '12.4 GB');
  assert.equal(dup.querySelector('.card__count').textContent, '214 files');
  assert.equal(dup.querySelector('.badge--select').textContent, 'In plan');
  assert.equal(dup.querySelector('.badge--safety').textContent, 'Reversible');
  assert.equal(texts(dup.querySelectorAll('.btn')).join('|'), 'Review', 'in-plan card has only Review');
  const compress = card('opt:fs_compress');
  assert.equal(compress.classList.contains('card--in-plan'), false);
  assert.equal(texts(compress.querySelectorAll('.btn')).join('|'), 'Review|+ Add');
  const tones = Object.fromEntries(['det:dev_artefacts', 'opt:video', 'opt:office', 'det:exact_duplicates'].map((id) => [id, [card(id).querySelector('.badge--safety').className, card(id).querySelector('.badge--safety').textContent]]));
  assert.match(tones['det:dev_artefacts'][0], /badge--success/);
  assert.equal(tones['det:dev_artefacts'][1], 'Rebuildable');
  assert.match(tones['opt:video'][0], /badge--warning/);
  assert.equal(tones['opt:video'][1], 'Lossy · opt-in');
  assert.equal(tones['opt:office'][1], 'Light copy');
  assert.equal(env.$$('.card--in-plan').length, 4);
});

test('results: the smaller opportunities expander', async () => {
  const env = await makeEnv({ scenario: 'results' });
  const more = env.$('[data-fk="show-small"]');
  assert.equal(more.textContent, 'Show 9 smaller opportunities  ·  1.9 GB');
  await env.click(more);
  assert.equal(env.$$('.card', env.$('.app__main')).length, 18);
  assert.equal(env.$('[data-fk="show-small"]').textContent, 'Hide smaller opportunities');
});

test('results: review only tab shows the three review cards without an Add button', async () => {
  const env = await makeEnv({ scenario: 'results' });
  await env.actions.setTab('review_only');
  await env.settle();
  const cards = env.$$('.card', env.$('.app__main'));
  assert.equal(cards.length, 3);
  for (const c of cards) {
    assert.equal(c.querySelector('.badge--safety').textContent, 'Review only');
    assert.equal(c.querySelectorAll('.btn').some((b) => b.textContent === '+ Add'), false);
  }
});

test('plan panel: four steps with sizes, total, checklist and actions', async () => {
  const env = await makeEnv({ scenario: 'results' });
  const steps = env.$$('.step');
  assert.deepEqual(steps.map((s) => s.querySelector('.step__title').textContent), ['Exact duplicates', 'Developer artefacts', 'App caches & junk', 'Meeting recordings']);
  assert.deepEqual(steps.map((s) => s.querySelector('.step__size').textContent), ['8.6 GB', '9.6 GB', '6.9 GB', '4.2 GB']);
  assert.equal(steps[0].querySelector('.step__sub').textContent, 'Keep newest · others to Recycle Bin');
  assert.equal(steps[3].querySelector('.badge--warning').textContent, 'Lossy');
  assert.equal(env.$('.plan__count').textContent, '4 steps');
  assert.equal(env.$('[data-role="plan-total"]').textContent, '29.3 GB');
  const items = env.$$('.check__item');
  assert.deepEqual(items.map((i) => i.className.split(' ').pop()), ['check__item--ok', 'check__item--warning', 'check__item--ok', 'check__item--info']);
  assert.equal(items[1].textContent, '1 lossy step will ask for confirmation');
  assert.equal(env.byText('button', 'Estimate exactly (dry run)').disabled, false);
  assert.equal(env.byText('button', 'Apply plan  ·  29.3 GB').classList.contains('btn--primary'), true);
  assert.ok(env.byText('button', 'Save as weekly schedule').disabled, 'a plan with a lossy step cannot be scheduled');
});

test('plan panel: reorder buttons are disabled at the ends and have names', async () => {
  const env = await makeEnv({ scenario: 'results' });
  const steps = env.$$('.step');
  assert.ok(steps[0].querySelector('[aria-label="Move Exact duplicates up"]').disabled);
  assert.ok(steps[3].querySelector('[aria-label="Move Meeting recordings down"]').disabled);
  assert.ok(steps[1].querySelector('[aria-label="Remove Developer artefacts from the plan"]'));
});

test('status strip: reclaimable, reclaimed, plan summary, idle state', async () => {
  const env = await makeEnv({ scenario: 'results' });
  assert.equal(env.$('[data-role="reclaimable"]').textContent, '48.6 GB');
  assert.equal(env.$('[data-role="reclaimed"]').textContent, '0 B');
  assert.equal(env.$('[data-role="plan-summary"]').textContent, 'Plan: 4 steps  ·  29.3 GB');
  assert.equal(env.$('[data-role="status"]').textContent, 'Nothing running');
  assert.ok(env.$('[data-fk="undo-last"]').disabled);
  assert.equal(env.$('[data-fk="last-report"]').disabled, false);
  assert.equal(env.$('.queue'), null);
});

test('drive bars show free space before and after the plan', async () => {
  const env = await makeEnv({ scenario: 'results' });
  const rows = env.$$('.drive-row');
  assert.equal(rows.length, 2);
  assert.equal(rows[0].querySelector('.drive-row__name').textContent, 'C:  System');
  assert.match(rows[0].querySelector('.drive-row__free').textContent, /45\.0 GB free.*86\.0 GB after plan/);
  assert.match(rows[1].querySelector('.drive-row__free').textContent, /354 GB free.*362 GB after plan/);
  assert.ok(rows[0].querySelector('[data-seg="freed"]'));
});

test('scanning: progress header, Pause and Stop, live cards fill in', async () => {
  const env = await makeEnv({ scenario: 'empty', speed: 1 });
  await env.actions.openBrowser();
  await env.settle();
  await env.actions.chooseFolder();
  await env.actions.startScan();
  assert.ok(env.$('.scanhead'));
  assert.ok(env.$('[data-fk="scan-pause"]'));
  assert.ok(env.$('[data-fk="scan-stop"]'));
  assert.equal(env.$$('.card--live').length, 0);
  assert.equal(env.$$('.card--placeholder').length, 9);
  env.backend.emit('scan://detector', { generation: env.state().scan.generation, opportunity_id: 'det:exact_duplicates', title: 'Exact duplicates', saving: 1024 ** 3 * 12.4, count: 214, finished: true });
  assert.equal(env.$$('.card--live').length, 1);
  assert.equal(env.$$('.card--placeholder').length, 8);
  assert.equal(env.$('.card--live .card__size').textContent, '12.4 GB');
  env.backend.emit('scan://progress', { generation: env.state().scan.generation, phase: 'hashing', files: 51200, dirs: 4410, bytes: 1, done: 0, total: 18, current: 'C:\\x', elapsed_ms: 10 });
  assert.match(env.$('[data-role="scan-meta"]').textContent, /51,200 files/);
  await env.actions.stopScan();
});

test('scanning ends in results with live cards replaced by the real grid', async () => {
  const env = await makeEnv({ scenario: 'empty', speed: 0 });
  await env.actions.openBrowser();
  await env.settle();
  await env.actions.chooseFolder();
  await env.actions.startScan();
  await env.settle();
  assert.ok(env.$('.hero'));
  assert.equal(env.$$('.card', env.$('.app__main')).length, 9);
  assert.equal(env.$('[data-role="hero-total"]').textContent, '48.6 GB');
});

test('action completed: summary banner with Undo and View report', async () => {
  const env = await makeEnv({ scenario: 'results' });
  await env.actions.startApply();
  await env.settle();
  await env.actions.dialogSet({ ack: true });
  await env.actions.dialogConfirm();
  await env.settle();
  const banner = env.$('[data-role="completed"]');
  assert.ok(banner);
  assert.match(banner.textContent, /All done.*reclaimed from 233 files/);
  assert.ok(env.byText('button', 'Undo', banner));
  assert.ok(env.byText('button', 'View report', banner));
  assert.equal(env.$('[data-role="plan-summary"]').textContent, 'Plan: empty');
  assert.equal(env.$('[data-fk="undo-last"]').disabled, false);
});

test('action running: the strip expands into a job queue with controls', async () => {
  const env = await makeEnv({ scenario: 'results', speed: 1 });
  await env.actions.startApply();
  await env.settle();
  assert.equal(env.state().dialog === null, false);
  await env.actions.dialogSet({ ack: true });
  await env.actions.dialogConfirm();
  assert.ok(env.$('.queue'));
  assert.equal(env.$$('.job').length, 4);
  assert.match(env.$('[data-role="status"]').textContent, /Running 4 jobs/);
  assert.ok(env.$('[data-fk^="pause-"]'));
  assert.equal(env.$$('[data-fk^="cancel-"]').length, 4);
  env.backend.emit('job://progress', { job_id: env.state().jobs[0].id, run_id: 1, progress: { done: 10, total: 153, bytes_done: 1, bytes_total: 2, saved: 0 }, current: 'x' });
  assert.equal(env.$('[data-role="job-count"]').textContent, '10 of 153');
  await env.actions.cancelJob(env.state().jobs[3].id);
});

test('stub views list their category and open the step review', async () => {
  const env = await makeEnv({ scenario: 'results' });
  for (const [view, expected] of [['clean', 8], ['optimise', 6], ['archive', 1]]) {
    env.actions.navigate(view);
    await env.settle();
    assert.equal(env.$$('.card', env.$('.app__main')).length, expected, view);
    assert.equal(env.$('.rail__item--active').getAttribute('data-nav'), view);
    assert.equal(env.$('.header__title').textContent, view[0].toUpperCase() + view.slice(1));
  }
});

test('clean view has a Custom search entry with a quick-query box', async () => {
  const env = await makeEnv({ scenario: 'results' });
  env.actions.navigate('clean');
  await env.settle();
  assert.ok(env.$('.custom'));
  assert.equal(env.$('.custom__title').textContent, 'Custom search');
  assert.ok(env.$('[data-fk="custom-query"]').getAttribute('aria-label'));
});

test('history lists runs with Restore; settings shows the drawer skeleton', async () => {
  const env = await makeEnv({ scenario: 'results' });
  env.actions.navigate('history');
  await env.settle();
  assert.equal(env.$$('.run').length, 1);
  assert.ok(env.byText('button', 'Restore').disabled, 'the seeded run is already restored');
  env.actions.navigate('settings');
  await env.settle();
  assert.ok(env.$('.drawer'));
  assert.match(env.root.textContent, /Undo window30 days/);
  assert.ok(env.byText('li', 'ffmpeg'));
});

test('the right panel only shows on Overview and the step review', async () => {
  const env = await makeEnv({ scenario: 'results' });
  assert.equal(env.$('.app').classList.contains('app--no-panel'), false);
  env.actions.navigate('history');
  assert.equal(env.$('.app').classList.contains('app--no-panel'), true);
});
