import { test } from 'node:test';
import assert from 'node:assert/strict';
import { makeEnv } from './testing/env.js';

const texts = (els) => els.map((e) => e.textContent);

async function openAudio(opts) {
  const env = await makeEnv(opts);
  await env.actions.openStep('opt:audio');
  await env.settle();
  env.clearCalls();
  return env;
}

test('step review: header, summary line, sub-group tabs', async () => {
  const env = await openAudio();
  assert.equal(env.$('.crumbs__back').textContent, 'Overview');
  assert.equal(env.$('.header__title').textContent, 'Meeting recordings');
  assert.deepEqual(texts(env.$$('.header__badges .badge')), ['In plan', 'Lossy · opt-in']);
  assert.equal(env.$('[data-role="step-summary"]').textContent, '23 recordings  ·  4.8 GB  →  about 0.6 GB');
  assert.match(env.$('.step-head__sub').textContent, /^Found in Teams, Zoom and local Recordings folders\./);
  assert.deepEqual(texts(env.$$('.tab')), ['All 24', 'Same meeting twice 6', 'Need transcript 4', 'Skipped 2']);
  assert.equal(env.$('.rail__item--active').getAttribute('data-nav'), 'optimise');
});

test('step review: table columns, checked rows, per-row after size and badges', async () => {
  const env = await openAudio();
  assert.deepEqual(texts(env.$$('th')).slice(1), ['Recording', 'Source', 'Length', 'Size', 'After', 'Notes']);
  const rows = env.$$('.row');
  assert.equal(rows.length, 24);
  const cells = (r) => texts(r.querySelectorAll('td')).slice(1);
  assert.deepEqual(cells(rows[0]).slice(0, 5), ['Weekly sync – Nordbank.mp4', 'Teams', '1:02:14', '412 MB', '~31.0 MB']);
  assert.equal(cells(rows[1])[4], 'remove');
  assert.equal(rows[1].querySelector('.badge--info').textContent, 'duplicate');
  assert.equal(rows[0].querySelector('.badge--info').textContent, 'same meeting ×2');
  assert.equal(rows[2].querySelector('.badge--warning').textContent, 'silence 14 min');
  const board = rows[6];
  assert.equal(board.querySelector('input').checked, false, 'confidential is auto-unchecked');
  assert.equal(cells(board)[4], '–');
  assert.equal(board.querySelector('.badge--neutral').textContent, 'confidential');
  assert.equal(rows[0].querySelector('input').checked, true);
  assert.ok(rows[0].querySelector('input').getAttribute('aria-label'));
});

test('step review: sub-group tab filters the table through candidates_page', async () => {
  const env = await openAudio();
  await env.click(env.byText('button', 'Same meeting twice'));
  assert.equal(env.$$('.row').length, 6);
  const page = env.sent('candidates_page').at(-1).req;
  assert.equal(page.sub_group, 'same_meeting');
  assert.equal(page.opportunity_id, 'opt:audio');
});

test('step review: only a window of rows is built and paging moves the window', async () => {
  const env = await makeEnv();
  await env.actions.openStep('det:exact_duplicates');
  await env.settle();
  assert.equal(env.$$('.row').length, 50, 'never more than one page of rows');
  assert.equal(env.$('.pager__range').textContent, '1 to 50 of 214');
  assert.ok(env.$('[data-fk="page-prev"]').disabled);
  env.clearCalls();
  await env.click(env.$('[data-fk="page-next"]'));
  assert.equal(env.$('.pager__range').textContent, '51 to 100 of 214');
  assert.equal(env.sent('candidates_page')[0].req.offset, 50);
  assert.equal(env.sent('candidates_page')[0].req.limit, 50);
});

test('step review has one scroll container: the table is not wrapped in its own', async () => {
  const env = await openAudio();
  assert.equal(env.$$('[style]', env.$('.view--step')).filter((e) => e.getAttribute('style')).length, 0);
  assert.ok(env.$('.app__main').contains(env.$('.table')));
});

test('how to shrink them: presets first, then toggles, waveform, estimate, listen', async () => {
  const env = await openAudio();
  const panel = env.$('.shrink');
  assert.equal(panel.querySelector('.panel__title').textContent, 'How to shrink them');
  const tiles = panel.querySelectorAll('.tile');
  assert.deepEqual(tiles.map((t) => t.querySelector('.tile__title').textContent), ['Speech', 'Transcription-safe', 'Lossless']);
  assert.equal(tiles[0].getAttribute('aria-checked'), 'true');
  assert.equal(tiles[1].getAttribute('aria-checked'), 'false');
  const order = [...panel.children].map((c) => c.className.split(' ')[0]);
  assert.ok(order.indexOf('tiles') < order.indexOf('settings'), 'presets come before toggles');
  assert.deepEqual(texts(panel.querySelectorAll('.setting__label')), ['Downmix to mono', 'Remove long silences', 'Strip cover art', 'Transcribe on this PC, then archive audio']);
  assert.equal(panel.querySelectorAll('[role="switch"]').filter((x) => x.checked).length, 3);
  assert.equal(panel.querySelector('.number').value, '2');
  assert.equal(panel.querySelector('.number-unit').textContent, 's max');
  assert.equal(panel.querySelector('.setting__help').textContent, 'Keeps a searchable text file next to each meeting');
  assert.ok(panel.querySelector('.wave'), 'waveform drawn from mock data');
  assert.ok(panel.querySelectorAll('.wave__bar').length >= 90);
  assert.ok(panel.querySelectorAll('.wave__silence').length >= 1);
  assert.equal(panel.querySelector('.preview__title').textContent, 'Steering committee 14 Oct.wav');
  assert.match(panel.querySelector('.preview__silence').textContent, /min of silence/);
  assert.equal(panel.querySelector('[data-role="step-estimate"]').textContent, '4.8 GB  →  ~0.6 GB   (−4.2 GB)');
  assert.ok(env.byText('button', 'Listen before / after', panel));
});

test('how to shrink them: advanced options stay collapsed until asked for', async () => {
  const env = await openAudio();
  const panel = env.$('.shrink');
  assert.equal(panel.querySelector('.disclosure').getAttribute('aria-expanded'), 'false');
  assert.equal(panel.querySelectorAll('.setting').length, 4);
  await env.click(panel.querySelector('.disclosure'));
  assert.equal(env.$('.shrink .disclosure').getAttribute('aria-expanded'), 'true');
  assert.equal(env.$$('.shrink .setting').length, 6);
});

test('listen is disabled with its reason when the audio tool is missing', async () => {
  const env = await makeEnv();
  env.backend.model.tools.find((t) => t.id === 'ffmpeg').found = false;
  await env.actions.init();
  await env.actions.openStep('opt:audio');
  await env.settle();
  const listen = env.byText('button', 'Listen before / after');
  assert.ok(listen.disabled);
  assert.match(env.$('.shrink').textContent, /Unavailable: The audio tool \(ffmpeg\) was not detected/);
});

test('what happens to them: action tiles with Recycle Bin by default, keep rule, collapsed advanced', async () => {
  const env = await makeEnv();
  await env.actions.openStep('det:exact_duplicates');
  await env.settle();
  const panel = env.$('.clean');
  assert.equal(panel.querySelector('.panel__title').textContent, 'What happens to them');
  const tiles = panel.querySelectorAll('.tile');
  assert.deepEqual(tiles.map((t) => t.querySelector('.tile__title').textContent), ['Recycle Bin', 'Quarantine', 'Move…']);
  assert.equal(tiles[0].getAttribute('aria-checked'), 'true');
  const keep = panel.querySelector('select');
  assert.equal(keep.getAttribute('aria-label'), 'Keep the');
  assert.deepEqual(keep.querySelectorAll('option').map((o) => o.textContent), ['Newest copy', 'Oldest copy', 'Shortest path', 'Highest resolution']);
  assert.equal(panel.querySelector('.disclosure').getAttribute('aria-expanded'), 'false');
  assert.equal(panel.querySelectorAll('.tile--selected').length, 1);
});

test('header offers Add to plan, or Apply plan when the step is already in it', async () => {
  const env = await makeEnv();
  await env.actions.openStep('det:old_installers');
  await env.settle();
  assert.ok(env.$('[data-fk="step-add"]'));
  assert.equal(env.$('.header__badges').textContent.includes('In plan'), false);
  await env.actions.openStep('det:exact_duplicates');
  await env.settle();
  assert.equal(env.$('[data-fk="step-apply"]').textContent, 'Apply plan  ·  29.3 GB');
});

test('lossy confirmation dialog: what happens, tiles, originals, synced warning, gating', async () => {
  const env = await openAudio();
  await env.click(env.$('[data-fk="step-apply"]'));
  const dlg = env.$('[role="dialog"]');
  assert.ok(dlg);
  assert.equal(dlg.getAttribute('aria-modal'), 'true');
  assert.equal(env.$(`#${dlg.getAttribute('aria-labelledby')}`).textContent, 'Compress 23 recordings?');
  assert.equal(dlg.querySelector('.modal__body').textContent, 'This is lossy. The new files sound fine for speech but cannot be turned back into the originals.');
  const stats = Object.fromEntries(dlg.querySelectorAll('.stat').map((s) => [s.getAttribute('data-stat'), s.querySelector('.stat__value').textContent]));
  assert.deepEqual(stats, { Files: '23', Now: '4.8 GB', After: '~0.6 GB', Saving: '4.2 GB' });
  const sel = dlg.querySelector('select');
  assert.equal(sel.getAttribute('aria-label'), 'Originals');
  assert.equal(sel.querySelectorAll('option')[0].textContent, 'Move to Recycle Bin (default)');
  assert.equal(sel.querySelectorAll('option').find((o) => o.selected).textContent, 'Move to Recycle Bin (default)');
  assert.equal(dlg.querySelector('.callout__title').textContent, '6 files are in OneDrive – Acme');
  assert.match(dlg.querySelector('.callout__body').textContent, /also removes them in the cloud/);
  assert.ok(env.byText('button', 'Make them online-only instead', dlg));
  assert.ok(env.byText('button', 'Compare a sample first', dlg));
  assert.ok(env.byText('button', 'Back', dlg));
  const act = env.$('[data-fk="dlg-act"]');
  assert.equal(act.textContent, 'Compress 23 files');
  assert.ok(act.disabled, 'gated by I understand');
  assert.ok(act.classList.contains('btn--warning'), 'warning style');
  assert.equal(act.classList.contains('btn--primary'), false, 'never the primary style');
  assert.equal(env.$$('.btn--primary', dlg).length, 0);
  assert.match(env.$('[data-fk="dlg-ack"]').parentNode.textContent, /I understand the new files are lower quality than the originals/);
  env.$('[data-fk="dlg-ack"]').setChecked(true);
  assert.equal(env.$('[data-fk="dlg-act"]').disabled, false);
  assert.match(env.$('#dlg-ack-hint').textContent, /History → Restore brings the originals back for 30 days/);
});

test('permanent delete confirmation variant uses the danger style and no originals dropdown', async () => {
  const env = await makeEnv();
  await env.actions.openStep('det:old_installers');
  await env.settle();
  await env.actions.addStepToPlan();
  await env.actions.setStepAction('permanent_delete');
  await env.settle();
  assert.equal(env.state().plan.steps.find((s) => s.opportunity === 'det:old_installers').safety, 'permanent');
  // apply only this step plus the lossy one: both dialogs come in plan order
  await env.actions.startApply();
  await env.settle();
  const d = env.state().dialog;
  assert.equal(d.infos.length, 2);
  await env.actions.dialogSet({ ack: true });
  await env.actions.dialogConfirm();
  await env.settle();
  assert.match(env.$('.eyebrow', env.$('[role="dialog"]')).textContent, /Step 2 of 2/);
  const dlg = env.$('[role="dialog"]');
  const act = env.$('[data-fk="dlg-act"]');
  assert.ok(act.classList.contains('btn--danger'));
  assert.equal(act.classList.contains('btn--warning'), false);
  assert.equal(dlg.querySelector('select'), null);
  assert.ok(dlg.classList.contains('modal--permanent'));
  assert.match(dlg.querySelector('.modal__title').textContent, /permanently\?$/);
  assert.equal(env.byText('button', 'Compare a sample first', dlg), null);
});
