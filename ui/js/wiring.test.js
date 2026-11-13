import { test } from 'node:test';
import assert from 'node:assert/strict';
import { makeEnv } from './testing/env.js';

const last = (env, cmd) => env.sent(cmd).at(-1)?.req;

test('Add on a card sends plan_add_step with the opportunity id, then the card shows In plan', async () => {
  const env = await makeEnv();
  await env.click(env.$('[data-fk="add-opt:fs_compress"]'));
  assert.deepEqual(last(env, 'plan_add_step'), { opportunity_id: 'opt:fs_compress' });
  assert.ok(env.$('[data-opp="opt:fs_compress"]').classList.contains('card--in-plan'));
  assert.equal(env.$$('.step').length, 5);
  assert.equal(env.$('.plan__count').textContent, '5 steps');
});

test('Review on a card opens the step review for that opportunity', async () => {
  const env = await makeEnv();
  await env.click(env.$('[data-fk="review-opt:video"]'));
  assert.deepEqual(last(env, 'step_detail'), { opportunity_id: 'opt:video' });
  assert.equal(env.state().route.name, 'step');
  assert.equal(env.$('.header__title').textContent, 'Screen recordings');
});

test('the back link returns to Overview', async () => {
  const env = await makeEnv();
  await env.click(env.$('[data-fk="review-opt:video"]'));
  await env.click(env.$('[data-fk="back"]'));
  assert.equal(env.state().route.name, 'overview');
  assert.ok(env.$('.hero'));
});

test('Scan sends scan_start; Pause, Resume and Stop send their commands', async () => {
  const env = await makeEnv({ scenario: 'empty', speed: 1 });
  await env.actions.openBrowser();
  await env.settle();
  await env.actions.chooseFolder();
  await env.settle();
  env.clearCalls();
  env.$('[data-fk="scan"]').click();
  await new Promise((r) => setTimeout(r, 20));
  assert.deepEqual(last(env, 'scan_start'), {});
  env.$('[data-fk="scan-pause"]').click();
  await new Promise((r) => setTimeout(r, 20));
  assert.equal(env.sent('scan_pause').length, 1);
  assert.ok(env.$('[data-fk="scan-resume"]'));
  env.$('[data-fk="scan-resume"]').click();
  await new Promise((r) => setTimeout(r, 20));
  assert.equal(env.sent('scan_resume').length, 1);
  env.$('[data-fk="scan-stop"]').click();
  await new Promise((r) => setTimeout(r, 20));
  assert.equal(env.sent('scan_stop').length, 1);
});

test('Scan again starts a new scan from the results header', async () => {
  const env = await makeEnv();
  await env.actions.startScan();
  assert.equal(env.sent('scan_start').length, 1);
  await env.actions.stopScan();
});

test('the folder browser lists places through fs_list_dir and adds the folder to the scope', async () => {
  const env = await makeEnv({ scenario: 'empty' });
  await env.click(env.$('[data-fk="add-folder"]'));
  assert.deepEqual(env.sent('fs_list_dir')[0].req, {});
  assert.ok(env.$('[role="dialog"]'));
  assert.ok(env.$$('.browser__place').length >= 5);
  await env.click(env.$('[data-fk="dir-Documents"]'));
  assert.equal(last(env, 'fs_list_dir').path, 'C:\\Users\\Michel\\Documents');
  await env.click(env.$('[data-fk="browser-choose"]'));
  assert.deepEqual(last(env, 'scope_add_root'), { path: 'C:\\Users\\Michel\\Documents' });
  assert.equal(env.$('[role="dialog"]'), null);
  assert.equal(env.$$('.chip').length, 1);
  await env.click(env.$('.chip__x'));
  assert.deepEqual(last(env, 'scope_remove_root'), { path: 'C:\\Users\\Michel\\Documents' });
});

test('tabs and sort send opportunities_list with the tab and sort', async () => {
  const env = await makeEnv();
  await env.click(env.$('[data-fk="tab-optimise"]'));
  assert.deepEqual(last(env, 'opportunities_list'), { tab: 'optimise', sort: 'saving' });
  assert.equal(env.$$('.card', env.$('.app__main')).length, 6);
  const sort = env.$('.sort select');
  sort.setValue('name');
  await env.settle();
  assert.deepEqual(last(env, 'opportunities_list'), { tab: 'optimise', sort: 'name' });
  const titles = env.$$('.card__title', env.$('.app__main')).map((e) => e.textContent);
  assert.deepEqual(titles, [...titles].sort((a, b) => a.localeCompare(b)));
});

test('tab keys: arrow right selects the next tab', async () => {
  const env = await makeEnv();
  env.$('[data-fk="tab-all"]').key('ArrowRight');
  await env.settle();
  assert.equal(last(env, 'opportunities_list').tab, 'clean');
});

test('remove (x) sends plan_remove_step and the card loses its In plan state', async () => {
  const env = await makeEnv();
  const id = env.state().plan.steps[0].id;
  await env.click(env.$('[aria-label="Remove Exact duplicates from the plan"]'));
  assert.deepEqual(last(env, 'plan_remove_step'), { step_id: id });
  assert.equal(env.$$('.step').length, 3);
  assert.equal(env.$('[data-opp="det:exact_duplicates"]').classList.contains('card--in-plan'), false);
});

test('reorder by button sends plan_reorder with every id exactly once', async () => {
  const env = await makeEnv();
  const ids = env.state().plan.steps.map((s) => s.id);
  await env.click(env.$('[aria-label="Move Developer artefacts up"]'));
  assert.deepEqual(last(env, 'plan_reorder'), { step_ids: [ids[1], ids[0], ids[2], ids[3]] });
  assert.equal(env.$('.step__title').textContent, 'Developer artefacts');
});

test('reorder by keyboard: Alt+ArrowDown on a step row', async () => {
  const env = await makeEnv();
  const ids = env.state().plan.steps.map((s) => s.id);
  env.$(`[data-step="${ids[0]}"]`).key('ArrowDown', { altKey: true });
  await env.settle();
  assert.deepEqual(last(env, 'plan_reorder'), { step_ids: [ids[1], ids[0], ids[2], ids[3]] });
  env.$(`[data-step="${ids[0]}"]`).key('ArrowDown'); // without Alt nothing happens
  await env.settle();
  assert.equal(env.sent('plan_reorder').length, 1);
});

test('Estimate exactly sends plan_dry_run and the plan turns exact', async () => {
  const env = await makeEnv();
  await env.click(env.byText('button', 'Estimate exactly (dry run)'));
  assert.equal(env.sent('plan_dry_run').length, 1);
  assert.match(env.$('.toast').textContent, /Dry run finished: 233 files, 29\.\d GB saved/);
  assert.ok(env.$('.step__size--exact'));
});

test('Save as weekly schedule sends schedule_from_plan (plans without lossy steps)', async () => {
  const env = await makeEnv();
  await env.click(env.$('[aria-label="Remove Meeting recordings from the plan"]'));
  await env.click(env.byText('button', 'Save as weekly schedule'));
  assert.deepEqual(last(env, 'schedule_from_plan'), { name: 'Weekly cleanup', cadence: { every: 'weekly', weekday: 1, hour: 8, minute: 47 } });
  assert.match(env.$('.toast').textContent, /Weekly schedule saved/);
});

test('Apply plan asks for confirmation first and sends plan_apply only after the acknowledgement', async () => {
  const env = await makeEnv();
  await env.click(env.byText('button', 'Apply plan  ·  29.3 GB'));
  assert.equal(env.sent('plan_apply').length, 0, 'nothing is applied before the dialog is confirmed');
  const stepId = env.state().plan.steps.find((s) => s.lossy).id;
  assert.deepEqual(last(env, 'plan_confirmation_info'), { step_id: stepId });
  env.$('[data-fk="dlg-act"]').click();
  await env.settle();
  assert.equal(env.sent('plan_apply').length, 0, 'the acting button is gated by I understand');
  env.$('[data-fk="dlg-ack"]').click();
  await env.settle();
  await env.click(env.$('[data-fk="dlg-act"]'));
  assert.deepEqual(last(env, 'plan_apply'), { confirmations: [{ step_id: stepId, acknowledged: true, originals: { policy: 'recycle_bin' }, synced_online_only: false }] });
  assert.equal(env.$('[role="dialog"]'), null);
});

test('dropdown and online-only choices travel in the confirmation', async () => {
  const env = await makeEnv();
  await env.click(env.byText('button', 'Apply plan  ·  29.3 GB'));
  const select = env.$('[role="dialog"] select');
  select.setValue('1');
  await env.settle();
  await env.click(env.byText('button', 'Make them online-only instead'));
  assert.equal(env.byText('button', 'Make them online-only instead').getAttribute('aria-pressed'), 'true');
  env.$('[data-fk="dlg-ack"]').click();
  await env.settle();
  await env.click(env.$('[data-fk="dlg-act"]'));
  const conf = last(env, 'plan_apply').confirmations[0];
  assert.deepEqual(conf.originals, { policy: 'quarantine' });
  assert.equal(conf.synced_online_only, true);
  assert.equal(conf.acknowledged, true);
});

test('a plan without lossy steps applies directly with no confirmations', async () => {
  const env = await makeEnv();
  await env.click(env.$('[aria-label="Remove Meeting recordings from the plan"]'));
  await env.click(env.byText('button', 'Apply plan  ·  25.1 GB'));
  assert.deepEqual(last(env, 'plan_apply'), { confirmations: [] });
  assert.equal(env.$('[role="dialog"]'), null);
});

test('Back closes the dialog without applying; Escape does the same', async () => {
  const env = await makeEnv();
  await env.click(env.byText('button', 'Apply plan  ·  29.3 GB'));
  await env.click(env.$('[data-fk="dlg-back"]'));
  assert.equal(env.$('[role="dialog"]'), null);
  assert.equal(env.sent('plan_apply').length, 0);
  await env.click(env.byText('button', 'Apply plan  ·  29.3 GB'));
  env.$('[role="dialog"]').key('Escape');
  await env.settle();
  assert.equal(env.$('[role="dialog"]'), null);
  assert.equal(env.sent('plan_apply').length, 0);
});

test('focus moves into the dialog on open and returns to the opener on close', async () => {
  const env = await makeEnv();
  const apply = env.byText('button', 'Apply plan  ·  29.3 GB');
  apply.focus();
  apply.click();
  await env.settle();
  assert.equal(env.doc.activeElement, env.$('[role="dialog"]'));
  env.$('[role="dialog"]').key('Escape');
  await env.settle();
  assert.equal(env.doc.activeElement.getAttribute('data-fk'), 'apply');
});

test('the dialog traps Tab: last control wraps to the first and Shift+Tab the other way', async () => {
  const env = await makeEnv();
  await env.click(env.byText('button', 'Apply plan  ·  29.3 GB'));
  env.$('[data-fk="dlg-ack"]').click(); // enables the acting button
  await env.settle();
  const dlg = env.$('[role="dialog"]');
  const focusable = dlg.querySelectorAll('button, input, select').filter((e) => !e.disabled);
  focusable.at(-1).focus();
  const ev = { shiftKey: false };
  dlg.key('Tab', ev);
  assert.equal(env.doc.activeElement, focusable[0]);
  focusable[0].focus();
  dlg.key('Tab', { shiftKey: true });
  assert.equal(env.doc.activeElement, focusable.at(-1));
});

test('Compare a sample first closes the dialog and opens the step with a preview', async () => {
  const env = await makeEnv();
  await env.click(env.byText('button', 'Apply plan  ·  29.3 GB'));
  await env.click(env.byText('button', 'Compare a sample first'));
  assert.equal(env.$('[role="dialog"]'), null);
  assert.equal(env.state().route.name, 'step');
  assert.equal(env.state().step.id, 'opt:audio');
  assert.equal(env.sent('plan_apply').length, 0);
  assert.ok(env.sent('candidate_preview').length >= 1);
});

test('a checkbox sends selection_set for that row; select all sends it without ids', async () => {
  const env = await makeEnv();
  await env.actions.openStep('opt:audio');
  await env.settle();
  env.clearCalls();
  const first = env.$$('.row')[0];
  await env.click(first.querySelector('input'));
  const id = Number(first.getAttribute('data-row'));
  assert.deepEqual(last(env, 'selection_set'), { opportunity_id: 'opt:audio', ids: [id], selected: false });
  assert.equal(env.$$('.row')[0].querySelector('input').checked, false);
  assert.equal(env.$('[data-role="step-summary"]').textContent.startsWith('22 recordings'), true);
  await env.click(env.byText('button', 'Same meeting twice'));
  await env.click(env.$('[data-fk="select-all"]'));
  assert.deepEqual(last(env, 'selection_set'), { opportunity_id: 'opt:audio', sub_group: 'same_meeting', selected: true });
  await env.click(env.byText('button', 'All'));
  const all = env.$('[data-fk="select-all"]');
  assert.equal(all.checked, false, 'header box reflects that not every row is checked');
});

test('unchecking updates the card on Overview through opportunities://changed', async () => {
  const env = await makeEnv();
  await env.actions.openStep('opt:audio');
  await env.settle();
  await env.click(env.$$('.row')[2].querySelector('input'));
  await env.click(env.$('[data-fk="back"]'));
  assert.equal(env.$('[data-opp="opt:audio"] .card__size').textContent, '3.7 GB');
});

test('preset tile sends step_params_set with the preset id and updates the estimate', async () => {
  const env = await makeEnv();
  await env.actions.openStep('opt:audio');
  await env.settle();
  env.clearCalls();
  await env.click(env.$('[data-fk="preset-lossless"]'));
  assert.deepEqual(last(env, 'step_params_set'), { opportunity_id: 'opt:audio', preset_id: 'lossless' });
  assert.equal(env.$('[data-fk="preset-lossless"]').getAttribute('aria-checked'), 'true');
  assert.match(env.$('[data-role="step-estimate"]').textContent, /→  ~2\.\d GB/);
  assert.equal(env.$$('.header__badges .badge')[1].textContent, 'Reversible', 'the lossless preset is no longer lossy');
  assert.equal(env.state().plan.steps.find((s) => s.opportunity === 'opt:audio').lossy, false);
});

test('toggles and numbers send step_params_set with the key and value', async () => {
  const env = await makeEnv();
  await env.actions.openStep('opt:audio');
  await env.settle();
  env.clearCalls();
  const sw = env.$('[aria-label="Strip cover art"]');
  await env.click(sw);
  assert.deepEqual(last(env, 'step_params_set'), { opportunity_id: 'opt:audio', params: { strip_cover_art: false } });
  env.$('.number').setValue('5');
  await env.settle();
  assert.deepEqual(last(env, 'step_params_set'), { opportunity_id: 'opt:audio', params: { max_pause_s: 5 } });
});

test('keep rule dropdown sends selection_apply_keep_rule', async () => {
  const env = await makeEnv();
  await env.actions.openStep('det:exact_duplicates');
  await env.settle();
  env.$('[data-fk="keep-rule"]').setValue('oldest');
  await env.settle();
  assert.deepEqual(last(env, 'selection_apply_keep_rule'), { opportunity_id: 'det:exact_duplicates', rule: { rule: 'oldest' } });
  assert.equal(env.state().plan.steps[0].summary, 'Keep oldest · others to Recycle Bin');
});

test('action tiles change the plan step action; before adding, the choice travels with plan_add_step', async () => {
  const env = await makeEnv();
  await env.actions.openStep('det:exact_duplicates');
  await env.settle();
  await env.click(env.$('[data-fk="act-quarantine"]'));
  const stepId = env.state().plan.steps[0].id;
  assert.deepEqual(last(env, 'plan_update_step'), { step_id: stepId, action: { kind: 'quarantine', purge_after_days: 7 } });
  await env.actions.openStep('det:old_installers');
  await env.settle();
  await env.click(env.$('[data-fk="act-quarantine"]'));
  assert.equal(env.sent('plan_update_step').length, 1, 'not in the plan yet: no update');
  await env.click(env.$('[data-fk="step-add"]'));
  assert.deepEqual(last(env, 'plan_add_step'), { opportunity_id: 'det:old_installers', action: { kind: 'quarantine', purge_after_days: 7 } });
});

test('Move opens the folder browser and sends the chosen destination', async () => {
  const env = await makeEnv();
  await env.actions.openStep('det:old_installers');
  await env.settle();
  await env.click(env.$('[data-fk="act-move"]'));
  assert.equal(env.state().browser.mode, 'move');
  await env.click(env.$('[data-fk="browser-choose"]'));
  await env.click(env.$('[data-fk="step-add"]'));
  const req = last(env, 'plan_add_step');
  assert.equal(req.action.kind, 'move');
  assert.equal(typeof req.action.dest, 'string');
});

test('Undo last action sends undo_last once a run exists', async () => {
  const env = await makeEnv();
  await env.click(env.$('[aria-label="Remove Meeting recordings from the plan"]'));
  await env.click(env.byText('button', 'Apply plan  ·  25.1 GB'));
  await env.settle();
  assert.equal(env.state().undoable, true);
  await env.click(env.$('[data-fk="undo-last"]'));
  assert.equal(env.sent('undo_last').length, 1);
  assert.equal(env.state().undoable, false);
  assert.ok(env.$('[data-fk="undo-last"]').disabled);
});

test('Undo on the completed summary sends undo_last; View report sends report_get', async () => {
  const env = await makeEnv();
  await env.click(env.$('[aria-label="Remove Meeting recordings from the plan"]'));
  await env.click(env.byText('button', 'Apply plan  ·  25.1 GB'));
  await env.settle();
  const runId = env.state().completed.run_id;
  await env.click(env.$('[data-fk="done-report"]'));
  assert.deepEqual(last(env, 'report_get'), { run_id: runId });
  assert.ok(env.$('[role="dialog"]'));
  await env.click(env.$('[data-fk="report-close"]'));
  assert.equal(env.$('[role="dialog"]'), null);
  await env.click(env.$('[data-fk="done-undo"]'));
  assert.equal(env.sent('undo_last').length, 1);
  assert.equal(env.$('[data-role="completed"]'), null);
});

test('Last report in the strip sends report_last', async () => {
  const env = await makeEnv();
  await env.click(env.$('[data-fk="last-report"]'));
  assert.equal(env.sent('report_last').length, 1);
  assert.ok(env.$('[role="dialog"]'));
});

test('job controls send job_pause, job_resume and job_cancel', async () => {
  const env = await makeEnv({ speed: 1 });
  await env.actions.startApply();
  await env.settle();
  await env.actions.dialogSet({ ack: true });
  await env.actions.dialogConfirm();
  const first = env.state().jobs[0].id;
  await new Promise((r) => setTimeout(r, 200));
  env.$(`[data-fk="pause-${first}"]`).click();
  await new Promise((r) => setTimeout(r, 20));
  assert.deepEqual(last(env, 'job_pause'), { job_id: first });
  assert.ok(env.$(`[data-fk="resume-${first}"]`));
  env.$(`[data-fk="resume-${first}"]`).click();
  await new Promise((r) => setTimeout(r, 20));
  assert.deepEqual(last(env, 'job_resume'), { job_id: first });
  const queued = env.state().jobs[3].id;
  env.$(`[data-fk="cancel-${queued}"]`).click();
  await new Promise((r) => setTimeout(r, 20));
  assert.deepEqual(last(env, 'job_cancel'), { job_id: queued });
  await env.actions.cancelJob(first);
  await env.settle();
});

test('History Restore sends undo_run for that run', async () => {
  const env = await makeEnv();
  await env.click(env.$('[aria-label="Remove Meeting recordings from the plan"]'));
  await env.click(env.byText('button', 'Apply plan  ·  25.1 GB'));
  await env.settle();
  env.actions.navigate('history');
  await env.settle();
  const runId = env.state().history.runs[0].run_id;
  await env.click(env.$(`[data-fk="restore-${runId}"]`));
  assert.deepEqual(last(env, 'undo_run'), { run_id: runId });
});

test('Custom search validates while typing and runs search_run', async () => {
  const env = await makeEnv();
  env.actions.navigate('clean');
  await env.settle();
  const box = env.$('[data-fk="custom-query"]');
  box.setValue('ext:mp4 size:>1GB');
  await env.settle();
  assert.equal(last(env, 'search_parse').query, 'ext:mp4 size:>1GB');
  await env.click(env.$('[data-fk="custom-run"]'));
  const run = last(env, 'search_run');
  assert.deepEqual(run.rule.all.map((c) => c.cond.field), ['extension', 'size']);
  assert.match(env.$('.custom__result').textContent, /Custom search: \d+ matches/);
  env.$('[data-fk="custom-query"]').setValue('bogus:1');
  await env.settle();
  assert.match(env.$('.custom__help--error').textContent, /Unknown key "bogus"/);
  assert.equal(env.$('[data-fk="custom-query"]').getAttribute('aria-invalid'), 'true');
});

test('errors from the engine show as a toast with the hint, not as a crash', async () => {
  const env = await makeEnv();
  await env.actions.addToPlan('det:exact_duplicates'); // already in the plan: conflict
  await env.settle();
  assert.match(env.$('.toast').textContent, /already in the plan/);
  assert.equal(env.$('.toast').getAttribute('role'), 'alert');
  await env.click(env.$('[data-fk="toast-dismiss"]'));
  assert.equal(env.$('.toast'), null);
});

test('confirmation_required from the engine surfaces if a dialog was somehow skipped', async () => {
  const env = await makeEnv();
  await assert.rejects(env.api.planApply({ confirmations: [] }), (e) => e.code === 'confirmation_required' && /Meeting recordings/.test(e.message));
});
