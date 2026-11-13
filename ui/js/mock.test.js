import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createMockBackend } from './mock/backend.js';
import { COMMANDS } from './api.js';
import { formatBytes } from './format.js';

const mk = (scenario = 'results') => createMockBackend({ scenario, speed: 0 });
const GB = 1024 ** 3;

test('the mock answers every command of IPC.md or says not_implemented, never crashes', async () => {
  const b = mk();
  const reqs = { scope_set: { scope: { roots: ['C:\\a'], include: [], exclude: [] } }, scope_add_root: { path: 'C:\\x' }, scope_remove_root: { path: 'C:\\x' }, fs_list_dir: {}, opportunities_list: {}, step_detail: { opportunity_id: 'opt:audio' }, candidates_page: { opportunity_id: 'opt:audio' }, selection_set: { opportunity_id: 'opt:audio', selected: true }, selection_apply_keep_rule: { opportunity_id: 'det:exact_duplicates', rule: { rule: 'newest' } }, selection_reset: { opportunity_id: 'opt:audio' }, step_params_set: { opportunity_id: 'opt:audio' }, candidate_preview: { candidate_id: 1041 }, search_parse: { query: 'a' }, search_suggest: { query: 'ex', cursor: 2 }, plan_confirmation_info: { step_id: 12 }, history_run_detail: { run_id: 1 }, settings_set: { settings: {} }, protected_paths_set: { user: [] } };
  for (const cmd of COMMANDS) {
    if (['scan_start', 'plan_apply', 'plan_dry_run', 'undo_last', 'job_pause', 'job_resume', 'job_cancel', 'jobs_reorder', 'undo_run', 'report_get', 'schedule_from_plan', 'search_run', 'plan_add_step', 'plan_update_step', 'plan_remove_step', 'plan_reorder', 'history_run_detail', 'plan_confirmation_info', 'report_export'].includes(cmd)) continue;
    try {
      await b.call(cmd, reqs[cmd]);
    } catch (e) {
      assert.ok(['not_implemented', 'invalid_input'].includes(e.code), `${cmd}: ${e.code} ${e.message}`);
      assert.ok(e.message && e.hint, `${cmd} has message and hint`);
    }
  }
});

test('fixtures: 18 opportunities, 48.6 GB, the wireframe counts', async () => {
  const b = mk();
  const all = await b.call('opportunities_list', { tab: 'all', sort: 'saving' });
  assert.equal(all.items.length, 18);
  assert.equal(formatBytes(all.summary.total), '48.6 GB');
  assert.equal(formatBytes(all.summary.reversible), '41.2 GB');
  assert.equal(formatBytes(all.summary.opt_in), '5.1 GB');
  assert.equal(formatBytes(all.summary.review_only), '2.3 GB');
  assert.deepEqual(all.counts, { all: 18, clean: 8, optimise: 6, archive: 1, review_only: 3 });
  assert.deepEqual(all.items.slice(0, 9).map((o) => o.title), ['Exact duplicates', 'Transparent compression', 'Developer artefacts', 'App caches & junk', 'Screen recordings', 'Inactive engagements', 'Meeting recordings', 'Office decks & workbooks', 'Old installers']);
  assert.equal(formatBytes(all.items.slice(9).reduce((a, o) => a + o.size_before, 0)), '1.9 GB');
  const clean = await b.call('opportunities_list', { tab: 'clean' });
  assert.ok(clean.items.every((o) => o.category === 'clean' && o.safety !== 'review_only'));
  const review = await b.call('opportunities_list', { tab: 'review_only' });
  assert.equal(review.items.length, 3);
});

test('fixtures: drives C: and D:, the plan, the meeting recordings step with 23 selected rows', async () => {
  const b = mk();
  const drives = await b.call('drives_list');
  assert.deepEqual(drives.map((d) => d.id), ['C:', 'D:']);
  assert.equal(formatBytes(drives[0].free_after_plan), '86.0 GB');
  const plan = await b.call('plan_get');
  assert.equal(plan.steps.length, 4);
  assert.equal(formatBytes(plan.estimate.bytes_before - plan.estimate.bytes_after), '29.3 GB');
  const detail = await b.call('step_detail', { opportunity_id: 'opt:audio' });
  assert.equal(detail.summary.selected, 23);
  assert.equal(detail.summary.candidates, 24);
  assert.equal(formatBytes(detail.summary.size_before), '4.8 GB');
  const page = await b.call('candidates_page', { opportunity_id: 'opt:audio', limit: 100 });
  assert.equal(page.total, 24);
  assert.equal(page.rows[6].path.endsWith('Board session – privileged.m4a'), true);
  assert.equal(page.rows[6].selected, false);
  assert.deepEqual(page.rows[6].badges, ['confidential']);
});

test('responses omit absent optional fields and use IPC field names only', async () => {
  const b = mk();
  const page = await b.call('candidates_page', { opportunity_id: 'det:exact_duplicates', limit: 3 });
  for (const r of page.rows) {
    assert.ok(!('base_after' in r));
    for (const k of Object.keys(r)) assert.match(k, /^[a-z_]+$/);
    assert.ok(r.group);
  }
  assert.equal(page.groups.length, 1);
});

test('plan commands follow the contract: conflict, unsupported, reorder validation', async () => {
  const b = mk();
  await assert.rejects(b.call('plan_add_step', { opportunity_id: 'det:exact_duplicates' }), (e) => e.code === 'conflict');
  await assert.rejects(b.call('plan_add_step', { opportunity_id: 'det:large_old' }), (e) => e.code === 'unsupported');
  await assert.rejects(b.call('plan_add_step', { opportunity_id: 'nope' }), (e) => e.code === 'invalid_input');
  await assert.rejects(b.call('plan_reorder', { step_ids: [1] }), (e) => e.code === 'invalid_input');
  const plan = await b.call('plan_get');
  const ids = plan.steps.map((s) => s.id).reverse();
  assert.deepEqual((await b.call('plan_reorder', { step_ids: ids })).steps.map((s) => s.id), ids);
});

test('plan_apply needs the acknowledgement of lossy steps (confirmation_required)', async () => {
  const b = mk();
  await assert.rejects(b.call('plan_apply', { confirmations: [] }), (e) => e.code === 'confirmation_required');
  const plan = await b.call('plan_get');
  const lossy = plan.steps.find((s) => s.lossy);
  await assert.rejects(b.call('plan_apply', { confirmations: [{ step_id: lossy.id, acknowledged: false }] }), (e) => e.code === 'confirmation_required');
  const started = await b.call('plan_apply', { confirmations: [{ step_id: lossy.id, acknowledged: true }] });
  assert.equal(started.jobs.length, 4);
  assert.ok(started.jobs.every((j) => j.state === 'queued' && j.kind === 'apply'));
  await assert.rejects(b.call('plan_apply', { confirmations: [] }), (e) => e.code === 'conflict');
  await b.whenIdle();
  const after = await b.call('plan_get');
  assert.equal(after.steps.length, 0, 'applied steps leave the plan');
  const hist = await b.call('history_list', { offset: 0, limit: 10 });
  assert.equal(hist.runs[0].undoable, true);
  assert.equal(hist.runs[0].done, 233);
});

test('the apply run emits state, progress and done events in order, then history', async () => {
  const b = mk();
  const log = [];
  for (const e of ['job://state', 'job://progress', 'job://done', 'history://changed', 'plan://changed']) await b.listen(e, (p) => log.push([e, p.state ?? p.job?.state ?? '']));
  const plan = await b.call('plan_get');
  await b.call('plan_apply', { confirmations: [{ step_id: plan.steps.find((s) => s.lossy).id, acknowledged: true }] });
  await b.whenIdle();
  const names = log.map((l) => l[0]);
  assert.ok(names.indexOf('job://state') < names.indexOf('job://progress'));
  assert.equal(names.filter((n) => n === 'job://done').length, 4);
  assert.ok(names.indexOf('history://changed') > names.lastIndexOf('job://done') - 1);
  assert.ok(log.some((l) => l[0] === 'job://state' && l[1] === 'running'));
});

test('undo_last restores the newest undoable run and fails when there is none', async () => {
  const b = mk();
  await assert.rejects(b.call('undo_last'), (e) => e.code === 'invalid_input');
  const plan = await b.call('plan_get');
  await b.call('plan_apply', { confirmations: [{ step_id: plan.steps.find((s) => s.lossy).id, acknowledged: true }] });
  await b.whenIdle();
  const job = await b.call('undo_last');
  assert.equal(job.kind, 'undo');
  await b.whenIdle();
  const hist = await b.call('history_list', { offset: 0, limit: 10 });
  assert.equal(hist.runs[0].undoable, false);
  assert.equal(hist.runs[0].undone, hist.runs[0].done);
});

test('scan: events fill cards live, then done; scan_start conflicts while running', async () => {
  const b = mk('empty');
  await assert.rejects(b.call('scan_start', {}), (e) => e.code === 'invalid_input', 'no folder yet');
  await b.call('scope_add_root', { path: 'C:\\Users\\Michel' });
  const events = [];
  for (const e of ['scan://progress', 'scan://detector', 'scan://done', 'opportunities://changed']) await b.listen(e, (p) => events.push([e, p]));
  const started = await b.call('scan_start', {});
  assert.equal(started.generation, 1);
  await assert.rejects(b.call('scan_start', {}), (e) => e.code === 'conflict');
  await b.whenIdle();
  assert.equal(events.filter(([e]) => e === 'scan://detector').length, 18);
  assert.ok(events.filter(([e]) => e === 'scan://progress').length > 10);
  assert.equal(events.at(-2)[0], 'scan://done');
  assert.equal(events.at(-2)[1].state, 'done');
  assert.equal((await b.call('opportunities_list', {})).items.length, 18);
});

test('scan pause, resume and stop', async () => {
  const b = createMockBackend({ scenario: 'empty', speed: 0 });
  await b.call('scope_add_root', { path: 'C:\\a' });
  await b.call('scan_start', {});
  assert.equal((await b.call('scan_pause')).state, 'paused');
  await b.whenIdle();
  assert.equal((await b.call('scan_status')).state, 'paused');
  assert.equal((await b.call('scan_resume')).state, 'scanning');
  await b.call('scan_stop');
  await b.whenIdle();
  const status = await b.call('scan_status');
  assert.equal(status.state, 'cancelled');
  assert.equal((await b.call('opportunities_list', {})).items.length, 0, 'a stopped first scan leaves no results');
});

test('selection: protected and review-only rows never become selected; keep rule and reset', async () => {
  const b = mk();
  const rows = (await b.call('candidates_page', { opportunity_id: 'det:large_old', limit: 10 })).rows;
  assert.ok(rows.every((r) => !r.selected && !r.actionable));
  await b.call('selection_set', { opportunity_id: 'det:large_old', selected: true });
  assert.equal((await b.call('step_detail', { opportunity_id: 'det:large_old' })).summary.selected, 0);
  const dup = await b.call('selection_apply_keep_rule', { opportunity_id: 'det:exact_duplicates', rule: { rule: 'oldest' } });
  assert.equal(dup.selected, 153);
  await b.call('selection_set', { opportunity_id: 'det:exact_duplicates', selected: false });
  assert.equal((await b.call('step_detail', { opportunity_id: 'det:exact_duplicates' })).summary.selected, 0);
  assert.equal((await b.call('selection_reset', { opportunity_id: 'det:exact_duplicates' })).selected, 153 - 0 + 0 === 153 ? 153 : 0);
});

test('step_params_set: a lossless preset is not lossy and re-estimates the step', async () => {
  const b = mk();
  const r = await b.call('step_params_set', { opportunity_id: 'opt:audio', preset_id: 'lossless' });
  assert.equal(r.lossy, false);
  assert.equal(r.safety, 'reversible');
  assert.ok(r.summary.size_after > 2 * GB);
  await assert.rejects(b.call('step_params_set', { opportunity_id: 'opt:audio', preset_id: 'nope' }), (e) => e.code === 'invalid_input');
  const back = await b.call('step_params_set', { opportunity_id: 'opt:audio', preset_id: 'speech' });
  assert.equal(back.lossy, true);
  assert.ok(back.summary.size_after < GB);
});

test('plan_confirmation_info matches the dialog copy of the wireframe', async () => {
  const b = mk();
  const plan = await b.call('plan_get');
  const info = await b.call('plan_confirmation_info', { step_id: plan.steps.find((s) => s.lossy).id });
  assert.equal(info.title, 'Compress 23 recordings?');
  assert.equal(info.action_label, 'Compress 23 files');
  assert.equal(info.originals_options[0].label, 'Move to Recycle Bin (default)');
  assert.equal(info.synced.provider, 'OneDrive – Acme');
  assert.equal(info.synced.count, 6);
  assert.equal(info.permanent, false);
});

test('fs_list_dir without a path returns places; with a path, its folders', async () => {
  const b = mk('empty');
  const places = await b.call('fs_list_dir', {});
  assert.ok(places.places.length >= 5);
  const home = await b.call('fs_list_dir', { path: 'C:\\Users\\Michel' });
  assert.ok(home.entries.some((e) => e.name === 'Documents' && e.is_dir));
  assert.equal(home.parent, 'C:\\Users');
});

test('scope_add_root collapses nested roots and rejects relative paths', async () => {
  const b = mk('empty');
  await b.call('scope_add_root', { path: 'C:\\a\\b' });
  const s = await b.call('scope_add_root', { path: 'C:\\a' });
  assert.deepEqual(s.roots, ['C:\\a']);
  assert.deepEqual((await b.call('scope_add_root', { path: 'C:\\a\\c' })).roots, ['C:\\a']);
  await assert.rejects(b.call('scope_add_root', { path: 'relative' }), (e) => e.code === 'invalid_input');
});

test('schedule_from_plan rejects lossy steps (D-116)', async () => {
  const b = mk();
  await assert.rejects(b.call('schedule_from_plan', { name: 'x', cadence: { every: 'weekly', weekday: 1, hour: 8, minute: 0 } }), (e) => e.code === 'invalid_input');
});

test('the mock is deterministic: two backends give identical fixtures', async () => {
  const a = await mk().call('candidates_page', { opportunity_id: 'det:dev_artefacts', limit: 20 });
  const b = await mk().call('candidates_page', { opportunity_id: 'det:dev_artefacts', limit: 20 });
  assert.deepEqual(a, b);
});
