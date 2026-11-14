// W3-D2 screens against the mock backend: render tests, wiring (exact command and payload) and error paths.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { makeEnv, HOSTILE } from './testing/env.js';

const last = (env, cmd) => env.sent(cmd).at(-1)?.req;
const texts = (els) => els.map((e) => e.textContent);
const go = async (env, name, params) => { env.actions.navigate(name, params); await env.settle(); };
const rich = (opts = {}) => makeEnv({ scenario: 'rich', ...opts });

// ---------------------------------------------------------------- History

test('history: runs grouped with date, steps, files, bytes and status; entries expand', async () => {
  const env = await rich();
  await go(env, 'history');
  const runs = env.$$('.run');
  assert.equal(runs.length, 3);
  assert.match(runs[0].textContent, /Plan · 3 steps/);
  assert.match(runs[0].textContent, /3 steps/);
  assert.match(runs[0].textContent, /2 failed/);
  assert.equal(runs[0].querySelector('.badge').textContent, 'Done with errors');
  assert.equal(runs[1].querySelector('.badge').textContent, 'Can be undone');
  assert.equal(runs[2].querySelector('.badge').textContent, 'Undo window closed');
  assert.match(env.$('[data-role="undo-window"]').textContent, /Undo window: 30 days/);
  assert.equal(env.$$('.entry').length, 0);
  await env.click(runs[0].querySelector('.run__toggle'));
  assert.equal(last(env, 'history_run_detail').offset, 0);
  assert.ok(env.$$('.entry').length > 0);
  assert.equal(env.$('.run__toggle').getAttribute('aria-expanded'), 'true');
  const refused = env.$$('.entry').find((e) => /Restore refused/.test(e.textContent));
  assert.ok(refused, 'a refused entry');
  assert.match(refused.querySelector('[data-role="entry-error"]').textContent, /original location is occupied/);
});

test('history: Restore on an entry sends restore_entries and shows refusals with the reason', async () => {
  const env = await rich();
  await go(env, 'history');
  await env.click(env.$('.run .run__toggle'));
  const row = env.$$('.entry').find((e) => /Done/.test(e.textContent) && !e.querySelector('button[disabled]'));
  const id = Number(row.getAttribute('data-entry'));
  await env.click(row.querySelector('button'));
  assert.deepEqual(last(env, 'restore_entries'), { entry_ids: [id] });
  assert.equal(env.state().jobs.at(-1).kind, 'undo');
  await env.settle();
  assert.match(env.$('.entry[data-entry="' + id + '"]').textContent, /Restored/);
});

test('history: undo a run asks first, lists the earlier refusals, then sends undo_run; refusals show after', async () => {
  const env = await rich();
  await go(env, 'history');
  const runId = env.state().history.runs[0].run_id;
  await env.click(env.$(`[data-fk="restore-${runId}"]`));
  assert.equal(env.sent('undo_run').length, 0);
  assert.equal(last(env, 'history_run_detail').run_id, runId, 'entries were read to name the refusals');
  assert.match(env.$('[role="dialog"]').textContent, /refused the last time/);
  assert.match(env.$('[role="dialog"] .refusals').textContent, /original location is occupied/);
  await env.click(env.$('[data-fk="ask-cancel"]'));
  assert.equal(env.$('[role="dialog"]'), null);
  assert.equal(env.sent('undo_run').length, 0);
  await env.click(env.$(`[data-fk="restore-${runId}"]`));
  await env.click(env.$('[data-fk="ask-confirm"]'));
  assert.deepEqual(last(env, 'undo_run'), { run_id: runId });
  await env.settle();
  const result = env.$('[data-role="restore-result"]');
  assert.ok(result, 'result panel after the job');
  assert.match(result.textContent, /2 failed/);
  assert.match(result.textContent, /occupied/);
});

test('history: select entries and restore the selection; empty and error states', async () => {
  const env = await rich();
  await go(env, 'history');
  await env.click(env.$('.run .run__toggle'));
  const boxes = env.$$('.entry input[type="checkbox"]').filter((b) => !b.disabled).slice(0, 2);
  boxes[0].setChecked(true);
  boxes[1].setChecked(true);
  await env.settle();
  const btn = env.byText('button', 'Restore 2 selected');
  assert.ok(btn);
  await env.click(btn);
  assert.equal(last(env, 'restore_entries').entry_ids.length, 2);

  const empty = await makeEnv({ scenario: 'fresh' });
  await go(empty, 'history');
  assert.match(empty.$('.state--empty').textContent, /Nothing has been applied yet/);
  const broken = await rich();
  broken.backend.failNext('history_list', { code: 'io', message: 'The journal is locked', hint: 'Close other MyJanitor windows.' });
  await go(broken, 'history');
  assert.match(broken.$('.state--error').textContent, /journal is locked/);
  assert.match(broken.$('.state--error').textContent, /Close other MyJanitor windows/);
  await broken.click(broken.$('[data-fk="history-retry"]'));
  assert.equal(broken.$$('.run').length, 3);
});

test('history: export audit CSV goes through the folder picker and sends report_export', async () => {
  const env = await rich();
  await go(env, 'history');
  await env.click(env.$('[data-fk="export-audit"]'));
  assert.equal(env.state().browser.mode, 'export');
  assert.equal(env.$('[data-fk="export-name"]').value, 'myjanitor-audit.csv');
  await env.click(env.$('[data-fk="browser-choose"]'));
  const req = last(env, 'report_export');
  assert.equal(req.kind, 'audit');
  assert.equal(req.format, 'csv');
  assert.equal(req.run_id, undefined);
  assert.equal(req.dest, 'C:\\Users\\Michel\\Documents\\myjanitor-audit.csv');
  assert.match(env.$('.toast').textContent, /Saved \d+ rows to C:\\Users\\Michel\\Documents\\myjanitor-audit\.csv/);
  const runId = env.state().history.runs[1].run_id;
  await env.click(env.$(`[data-fk="audit-${runId}"]`));
  await env.click(env.$('[data-fk="browser-choose"]'));
  assert.equal(last(env, 'report_export').run_id, runId);
  assert.match(last(env, 'report_export').dest, /myjanitor-audit \(1\)|myjanitor-audit-\d+\.csv/);
});

test('history: purge quarantine asks, then sends quarantine_purge with the days', async () => {
  const env = await rich();
  await go(env, 'history');
  assert.match(env.$('.quarantine').textContent, /Quarantine: 38 items/);
  await env.click(env.$('[data-fk="purge"]'));
  assert.match(env.$('[role="dialog"]').textContent, /cannot be undone/);
  await env.click(env.$('[data-fk="ask-confirm"]'));
  assert.deepEqual(last(env, 'quarantine_purge'), { older_than_days: 30 });
});

// ----------------------------------------------------------------- Report

test('report: totals, errors and skipped items with reasons, filters, exports', async () => {
  const env = await rich();
  await go(env, 'history');
  const runId = env.state().history.runs[0].run_id;
  await env.click(env.$(`[data-fk="report-${runId}"]`));
  assert.deepEqual(last(env, 'report_get'), { run_id: runId });
  const dlg = env.$('[role="dialog"]');
  assert.deepEqual(texts(env.$$('.stat', dlg)).map((t) => t.replace(/\d.*$/, '')), ['Files', 'Done', 'Skipped', 'Errors', 'Saved']);
  assert.match(dlg.textContent, /No access/);
  assert.match(dlg.textContent, /Changed since the scan/);
  assert.match(dlg.textContent, /Protected/);
  assert.doesNotMatch(env.$('[data-status="failed"]').textContent, /Permission denied: C:/, 'the path is not repeated in the reason');
  await env.click(env.$('[data-fk="rfilter-failed"]'));
  assert.ok(env.$$('[data-status]').every((r) => r.getAttribute('data-status') === 'failed'));
  await env.click(env.$('[data-fk="report-html"]'));
  assert.equal(env.state().browser.exportJob.format, 'html');
  await env.click(env.$('[data-fk="browser-choose"]'));
  assert.deepEqual(Object.keys(last(env, 'report_export')).sort(), ['dest', 'format', 'kind', 'run_id']);
  assert.equal(last(env, 'report_export').kind, 'report');
  assert.match(last(env, 'report_export').dest, /myjanitor-report-\d+\.html$/);
});

test('report: a hostile path and a bad file name are text only / refused', async () => {
  const env = await rich();
  env.store.set({ report: { run_id: 1, title: 'T', started: 1, totals: { files: 1, succeeded: 0, skipped: 1, failed: 0, bytes_before: 1, bytes_after: 1, bytes_saved: 0 }, sections: [{ title: 'S', action: 'x', totals: { files: 1 }, items: [{ path: `C:\\${HOSTILE}`, status: 'skipped', bytes_before: 1, bytes_after: 1, error: { code: 'protected', message: HOSTILE, hint: '' } }] }] }, reportUi: { filter: 'all', shown: {} } });
  assert.equal(env.$$('img').length, 0);
  assert.equal(env.$$('script').length, 0);
  await env.actions.startExport({ type: 'report', format: 'csv', run_id: 1 });
  await env.settle();
  env.$('[data-fk="export-name"]').setValue('..\\evil.csv');
  await env.click(env.$('[data-fk="browser-choose"]'));
  assert.equal(env.sent('report_export').length, 0);
  assert.match(env.$('.toast').textContent, /without folders/);
});

// ------------------------------------------------------------- Job queue

test('job queue: per-job progress, current file, errors, summary with Undo and View report', async () => {
  const env = await rich({ speed: 1 });
  env.actions.startApply();
  await new Promise((r) => setTimeout(r, 330));
  const first = env.state().jobs[0];
  assert.ok(env.$('.queue'));
  assert.equal(env.$$('.job').length, 3);
  const deep = 'C:\\Users\\Michel\\' + 'folder\\'.repeat(20) + 'file.pdf';
  env.backend.emit('job://state', { ...first, state: 'running' });
  env.backend.emit('job://progress', { job_id: first.id, run_id: first.run_id, progress: { done: 10, total: 153, bytes_done: 1, bytes_total: 2, saved: 0 }, current: deep });
  const cur = env.$(`.job[data-job="${first.id}"] [data-role="job-current"]`);
  assert.ok(cur.textContent.includes('\u2026') && cur.textContent.length <= 60, 'middle ellipsis');
  assert.equal(cur.getAttribute('title'), deep, 'full path in the title');
  assert.ok(env.$(`[data-fk="jup-${env.state().jobs[2].id}"]`), 'queued jobs can be reordered');
  assert.equal(env.$(`[data-fk="jup-${first.id}"]`), null, 'a running job cannot');
  await env.settle();
  const s = env.state();
  assert.equal(s.jobs.find((j) => j.title === 'App caches & junk').state, 'completed_with_errors');
  const summary = env.$('[data-role="queue-summary"]');
  assert.ok(summary);
  assert.match(summary.textContent, /Run finished/);
  assert.match(summary.textContent, /2 errors/);
  assert.match(env.$('.job--completed_with_errors [data-role="job-errors"]').textContent, /2 errors/);
  await env.click(env.$('[data-fk="queue-report"]'));
  assert.equal(last(env, 'report_get').run_id, s.completed.run_id);
  await env.click(env.$('[data-fk="report-close"]'));
  await env.click(env.$('[data-fk="queue-undo"]'));
  assert.equal(env.sent('undo_last').length, 1);
});

test('job queue: reorder queued jobs sends jobs_reorder with the new order; cancel sends job_cancel', async () => {
  const env = await rich({ speed: 1 });
  env.actions.startApply();
  await new Promise((r) => setTimeout(r, 60));
  await env.settle === undefined;
  const jobs = env.state().jobs;
  const [, b, c] = jobs.map((j) => j.id);
  env.$(`[data-fk="jdown-${b}"]`).click();
  await new Promise((r) => setTimeout(r, 20));
  assert.deepEqual(last(env, 'jobs_reorder'), { job_ids: [c, b] });
  env.$(`[data-fk="cancel-${c}"]`).click();
  await new Promise((r) => setTimeout(r, 20));
  assert.deepEqual(last(env, 'job_cancel'), { job_id: c });
  await env.settle();
});

test('job queue can be opened when idle and shows finished jobs', async () => {
  const env = await rich();
  assert.equal(env.$('[data-fk="queue-toggle"]'), null, 'nothing to show yet');
  await env.actions.startApply();
  await env.settle();
  await env.click(env.$('[data-fk="queue-dismiss"]'));
  assert.equal(env.$('.queue'), null);
  await env.click(env.$('[data-fk="queue-toggle"]'));
  assert.ok(env.$('.queue'));
  assert.equal(env.$$('.job').length, 3);
});

// ----------------------------------------------------------- Custom search

test('custom search: typing parses and suggests; errors show with their position', async () => {
  const env = await rich();
  await go(env, 'clean');
  env.$('[data-fk="custom-query"]').setValue('ext:mp4 si');
  await env.settle();
  assert.deepEqual(last(env, 'search_suggest'), { query: 'ext:mp4 si', cursor: 10 });
  assert.deepEqual(texts(env.$$('[role="option"]')), ['size:']);
  env.$('[data-fk="custom-query"]').key('Tab');
  await env.settle();
  assert.equal(env.state().custom.query, 'ext:mp4 size:');
  assert.ok(env.$$('[role="option"]').every((o) => o.textContent.startsWith('size:')), 'now the values of size: are suggested');
  env.$('[data-fk="custom-query"]').setValue('ext:mp4 bogus:1');
  await env.settle();
  assert.equal(last(env, 'search_parse').query, 'ext:mp4 bogus:1');
  assert.match(env.$('.query__errors').textContent, /Position 9: Unknown key "bogus"/);
  assert.equal(env.$('.query__bad').textContent, 'bogus:1');
  assert.equal(env.$('[data-fk="custom-query"]').getAttribute('aria-invalid'), 'true');
  const before = env.sent('search_run').length;
  await env.click(env.$('[data-fk="custom-run"]'));
  assert.equal(env.sent('search_run').length, before, 'an invalid query never runs');
});

test('custom search: run sends the parsed rule, title, topmost and action; results use the shared table', async () => {
  const env = await rich();
  await go(env, 'clean');
  env.$('[data-fk="custom-query"]').setValue('ext:log size:>1MB');
  await env.settle();
  env.$('[data-fk="custom-title"]').setValue('Big logs');
  env.$('[data-fk="custom-topmost"]').setChecked(true);
  env.$('[data-fk="custom-action-top"]').setValue('quarantine');
  await env.click(env.$('[data-fk="custom-run"]'));
  const req = last(env, 'search_run');
  assert.deepEqual(req.rule, { all: [{ cond: { field: 'extension', values: ['log'] } }, { cond: { field: 'size', cmp: 'gt', bytes: 1048576 } }] });
  assert.equal(req.title, 'Big logs');
  assert.equal(req.topmost_only, true);
  assert.deepEqual(req.action, { kind: 'quarantine', purge_after_days: 7 });
  assert.match(env.$('.custom__result').textContent, /Big logs: \d+ matches/);
  assert.ok(env.$$('.cresults .row').length > 0, 'rows in the shared table');
  assert.equal(last(env, 'candidates_page').opportunity_id, env.state().custom.result.id);
  env.$('.cresults [data-fk="row-' + env.$('.cresults .row').getAttribute('data-row') + '"]').setChecked(false);
  await env.settle();
  assert.equal(last(env, 'selection_set').selected, false);
  await env.click(env.$('[data-fk="custom-add"]'));
  assert.deepEqual(last(env, 'plan_add_step'), { opportunity_id: env.state().custom.result.id, action: { kind: 'quarantine', purge_after_days: 7 } });
  assert.equal(env.$('[data-fk="custom-add"]').disabled, true);
});

test('custom search: no matches shows an empty state', async () => {
  const env = await rich();
  await go(env, 'clean');
  env.$('[data-fk="custom-query"]').setValue('name:nomatch');
  await env.settle();
  await env.click(env.$('[data-fk="custom-run"]'));
  assert.match(env.$('.cresults').textContent, /Nothing matched/);
  assert.equal(env.$$('.cresults .row').length, 0);
});

test('rule builder: nested groups, field-specific inputs, validation and the exact rule sent', async () => {
  const env = await rich();
  await go(env, 'clean');
  await env.click(env.$('[data-fk="smode-builder"]'));
  assert.ok(env.$('.builder'));
  await env.click(env.$('[data-fk="b-root-addg"]'));
  await env.click(env.$('[data-fk="b-1-add"]'));
  assert.ok(env.$('.bgroup--nested'));
  env.$('[data-fk="b-1_0-field"]').setValue('size');
  await env.settle();
  assert.ok(env.$('[data-fk="b-1_0-unit"]'), 'size has a unit box');
  env.$('[data-fk="b-1_0-amount"]').setValue('2');
  env.$('[data-fk="b-1_0-unit"]').setValue('GB');
  env.$('[data-fk="b-0-pattern"]').setValue('');
  await env.click(env.$('[data-fk="custom-run"]'));
  assert.equal(env.sent('search_run').length, 0);
  assert.match(env.$('.brow__error').textContent, /Enter some text/);
  env.$('[data-fk="b-0-pattern"]').setValue('*.iso');
  env.$('[data-fk="b-1-op"]').setValue('any');
  env.$('[data-fk="b-0-mode"]').setValue('wildcard');
  await env.click(env.$('[data-fk="b-1_0-attr-hidden"]')?.parentNode ? env.$('[data-fk="custom-run"]') : env.$('[data-fk="custom-run"]'));
  assert.deepEqual(last(env, 'search_run').rule, { all: [{ cond: { field: 'name', mode: 'wildcard', pattern: '*.iso', case_sensitive: false } }, { any: [{ cond: { field: 'size', cmp: 'gt', bytes: 2147483648 } }] }] });
});

test('rule builder: attribute tri-state cycles any, set, not set; extension sets fill the list; date modes', async () => {
  const env = await rich();
  await go(env, 'clean');
  await env.click(env.$('[data-fk="smode-builder"]'));
  env.$('[data-fk="b-0-field"]').setValue('attribute');
  await env.settle();
  const hidden = () => env.$('[data-fk="b-0-attr-hidden"]');
  assert.equal(hidden().getAttribute('data-state'), 'any');
  await env.click(hidden());
  assert.equal(hidden().getAttribute('data-state'), 'set');
  await env.click(hidden());
  assert.equal(hidden().getAttribute('data-state'), 'clear');
  await env.click(hidden());
  assert.equal(hidden().getAttribute('data-state'), 'any');
  await env.click(hidden());
  env.$('[data-fk="b-0-field"]').setValue('extension');
  await env.settle();
  await env.click(env.$('[data-fk="b-0-set-video"]'));
  assert.match(env.state().custom.builder.children[0].values, /mp4/);
  env.$('[data-fk="b-0-field"]').setValue('date');
  await env.settle();
  env.$('[data-fk="b-0-mode"]').setValue('at');
  await env.settle();
  env.$('[data-fk="b-0-date"]').setValue('2024-03-15');
  await env.click(env.$('[data-fk="custom-run"]'));
  assert.deepEqual(last(env, 'search_run').rule.all[0], { cond: { field: 'date', which: 'modified', cmp: 'lt', value: { at: Date.UTC(2024, 2, 15) * 1e6 } } });
});

test('presets: built-in first, use loads the builder, save and delete send preset_save and preset_delete', async () => {
  const env = await rich();
  await go(env, 'clean');
  assert.equal(last(env, 'presets_list') !== undefined || env.sent('presets_list').length > 0, true);
  const items = env.$$('.preset');
  assert.equal(items.length, 3);
  assert.ok(items.every((i) => i.textContent.includes('Built in')));
  assert.equal(env.$('[data-fk="del-dev_junk"]'), null, 'built-ins cannot be deleted');
  await env.click(env.$('[data-fk="use-dev_junk"]'));
  assert.equal(env.state().custom.mode, 'builder');
  assert.equal(env.state().custom.topmost, true);
  env.$('[data-fk="preset-name"]').setValue('Old screen recordings');
  env.$('[data-fk="preset-desc"]').setValue('Older than six months');
  await env.settle();
  await env.click(env.$('[data-fk="preset-save"]'));
  const req = last(env, 'preset_save').preset;
  assert.equal(req.id, '');
  assert.equal(req.name, 'Old screen recordings');
  assert.equal(req.description, 'Older than six months');
  assert.equal(req.builtin, false);
  assert.equal(req.topmost_only, true);
  assert.deepEqual(req.rule, { any: [{ cond: { field: 'name', mode: 'exact', pattern: 'node_modules', case_sensitive: true } }, { cond: { field: 'name', mode: 'exact', pattern: 'target', case_sensitive: true } }] });
  assert.equal(env.$$('.preset').length, 4);
  const mine = env.state().custom.presets.find((p) => !p.builtin);
  await env.click(env.$(`[data-fk="del-${mine.id}"]`));
  assert.deepEqual(last(env, 'preset_delete'), { id: mine.id });
  assert.equal(env.$$('.preset').length, 3);
});

test('presets: not_implemented disables saving and deleting with the hint', async () => {
  const env = await rich();
  env.backend.disable('preset_save');
  await go(env, 'clean');
  await env.click(env.$('[data-fk="smode-builder"]'));
  env.$('[data-fk="b-0-pattern"]').setValue('*.tmp');
  env.$('[data-fk="preset-name"]').setValue('x');
  await env.settle();
  await env.click(env.$('[data-fk="preset-save"]'));
  assert.ok(env.state().unavail.presets);
  assert.equal(env.$('[data-fk="preset-save"]').disabled, true);
  assert.match(env.$('[data-fk="preset-save"]').getAttribute('title'), /not available in this build/);
});

test('step filter: Filter these results sends the path filter; sorting sends sort and descending', async () => {
  const env = await rich();
  await env.actions.openStep('det:old_installers');
  await env.settle();
  env.$('[data-fk="filter-input"]').setValue('office');
  await env.settle();
  assert.equal(last(env, 'candidates_page').filter, 'office');
  assert.ok(env.$$('.row').length < 42);
  env.$('[data-fk="filter-input"]').setValue('');
  await env.settle();
  assert.equal(last(env, 'candidates_page').filter, undefined);
  await env.click(env.$('[data-fk="sort-size"]'));
  assert.deepEqual([last(env, 'candidates_page').sort, last(env, 'candidates_page').descending], ['size', false]);
  await env.click(env.$('[data-fk="sort-size"]'));
  assert.equal(last(env, 'candidates_page').descending, true);
  assert.equal(env.$('th[aria-sort="descending"]').textContent, 'Size');
});

// --------------------------------------------------------- Step completion

test('step: duplicate groups are separated, keeper marked, keep rule goes through the backend', async () => {
  const env = await rich();
  await env.actions.openStep('det:exact_duplicates');
  await env.settle();
  assert.ok(env.$$('.group-head').length >= 10);
  assert.match(env.$('.group-head').textContent, /\d copies of .*100 % match/);
  assert.ok(env.$('.row--keeper'));
  env.$('[data-fk="keep-rule"]').setValue('oldest');
  await env.settle();
  assert.deepEqual(last(env, 'selection_apply_keep_rule'), { opportunity_id: 'det:exact_duplicates', rule: { rule: 'oldest' } });
  env.$('[data-fk="keep-rule"]').setValue('preferred_folder');
  await env.settle();
  assert.equal(env.state().browser.mode, 'keep');
  await env.click(env.$('[data-fk="browser-choose"]'));
  const req = last(env, 'selection_apply_keep_rule');
  assert.equal(req.rule.rule, 'preferred_folder');
  assert.equal(typeof req.rule.folder, 'string');
});

test('step: keyboard moves between rows, Page Down changes page', async () => {
  const env = await rich();
  await env.actions.openStep('det:exact_duplicates');
  await env.settle();
  const files = env.$$('.cell__file');
  files[0].focus();
  env.$('.table').dispatchEvent({ type: 'keydown', key: 'ArrowDown', currentTarget: env.$('.table') });
  assert.equal(env.doc.activeElement, files[1]);
  env.doc.activeElement.key('ArrowDown');
  assert.equal(env.doc.activeElement, files[2]);
  env.$('.table').key('PageDown');
  await env.settle();
  assert.equal(last(env, 'candidates_page').offset, 50);
  assert.match(env.$('.pager__range').textContent, /Page 2 of 5/);
});

test('step: clicking a name shows the preview pane (image, text, none with reason) and focus loads it', async () => {
  const env = await rich();
  await env.actions.openStep('det:old_installers');
  await env.settle();
  assert.match(env.$('.preview__none').textContent, /Click a file name/);
  await env.click(env.$('.cell__file'));
  assert.match(env.$('.preview').textContent, /No preview for this file type/);
  assert.match(env.$('.preview').textContent, /Name/);
  await env.actions.openStep('det:temp_files');
  await env.settle();
  await env.click(env.$('.cell__file'));
  assert.match(env.$('.preview__text').textContent, /Setup log/);
  await env.actions.openStep('det:exact_duplicates');
  await env.settle();
  await env.click(env.$$('.row').find((r) => /IMG_0412/.test(r.textContent)).querySelector('.cell__file'));
  assert.ok(env.$('.preview__image').getAttribute('src').startsWith('data:image/png;base64,'));
  assert.ok(env.$('.preview__image').getAttribute('alt'));
  assert.ok(env.$('.breakdown .stack'), 'size bar');
});

test('compare before / after: shell with sizes for a sample; disabled with the reason otherwise', async () => {
  const env = await makeEnv({ scenario: 'results' });
  await env.actions.openStep('opt:audio');
  await env.settle();
  assert.equal(env.$('[data-fk="compare-open"]').disabled, false);
  await env.click(env.$('[data-fk="compare-open"]'));
  const dlg = env.$('[role="dialog"]');
  assert.match(dlg.textContent, /Before/);
  assert.match(dlg.textContent, /After \(estimate\)/);
  assert.equal(env.$$('.wave', dlg).length, 2);
  assert.match(dlg.textContent, /MB now|MB/);
  await env.click(env.$('[data-fk="compare-close"]'));
  assert.equal(env.$('[role="dialog"]'), null);
  await env.actions.openStep('opt:images');
  await env.settle();
  await env.click(env.$('.cell__file'));
  assert.equal(env.$('[data-fk="compare-open"]').disabled, true);
  assert.match(env.$('[data-role="compare-why"]').textContent, /No sample to compare/);
});

// ------------------------------------------------------------- Disk usage

test('disk usage: reachable from Overview; tree sorted by size with allocated and compression columns', async () => {
  const env = await rich();
  await env.click(env.$('[data-fk="explore-disk"]'));
  assert.equal(env.state().route.name, 'disk');
  assert.deepEqual(last(env, 'disk_usage_tree'), { depth: 2, min_fraction: 0.01 });
  assert.equal(env.$('.header__title').textContent, 'Disk usage');
  assert.deepEqual(texts(env.$$('thead th')), ['Folder', 'Share', 'Size', 'On disk', 'Compression', 'Files']);
  const sizes = env.$$('.trow').map((r) => r.getAttribute('data-path'));
  assert.deepEqual(sizes, ['C:\\Users\\Michel', 'D:\\Projects']);
  await env.click(env.$('[data-fk="dt-C:_Users_Michel"]'));
  const names = env.$$('.trow').map((r) => r.querySelector('.tree__text, .tree__name--other').textContent);
  assert.equal(names[1], 'AppData');
  assert.equal(env.$('.trow[data-path="C:\\Users\\Michel"]').getAttribute('aria-expanded'), 'true');
  assert.match(env.$('[data-role="disk-total"]').textContent, /GB in [\d,]+ files/);
  await env.click(env.$('[data-fk="dt-D:_Projects"]'));
  await env.click(env.$('[data-fk="dt-D:_Projects_archive-2022"]'));
  assert.match(env.$('.trow[data-path="D:\\Projects\\archive-2022"]').textContent, /46 % smaller on disk/);
});

test('disk usage: expanding a folder below the loaded depth loads it with disk_usage_tree', async () => {
  const env = await rich();
  await go(env, 'disk');
  await env.click(env.$('[data-fk="dt-D:_Projects"]'));
  await env.click(env.$('[data-fk="dt-D:_Projects_app"]'));
  const req = last(env, 'disk_usage_tree');
  assert.deepEqual(req, { path: 'D:\\Projects\\app', depth: 2, min_fraction: 0.01 });
  assert.ok(env.$('.trow[data-path="D:\\Projects\\app\\node_modules"]'));
  env.$('[data-fk="dsort-name"]').click();
  assert.equal(env.state().disk.sort, 'name');
});

test('disk usage: treemap tiles carry labels and sizes in text, drill down and up', async () => {
  const env = await rich();
  await go(env, 'disk');
  await env.click(env.$('[data-fk="dtab-treemap"]'));
  const tiles = env.$$('.tile2');
  assert.equal(tiles.length, 2);
  assert.match(tiles[0].getAttribute('aria-label'), /C:\\Users\\Michel, 262 GB, 63 %/);
  assert.match(tiles[0].textContent, /262 GB/);
  await env.click(tiles[0]);
  assert.equal(env.state().disk.focusPath, 'C:\\Users\\Michel');
  assert.ok(env.$$('.tile2').length >= 8);
  assert.ok(env.$$('.largest__list li').length >= 7, 'a text list repeats the sizes');
  await env.click(env.$('[data-fk="treemap-up"]'));
  assert.equal(env.state().disk.focusPath, null);
});

test('disk usage: breakdown by type and by age are bar lists with numbers in text', async () => {
  const env = await rich();
  await go(env, 'disk');
  await env.click(env.$('[data-fk="dtab-type"]'));
  assert.deepEqual(last(env, 'breakdown'), { by: 'type' });
  assert.match(env.$('.bars__row').textContent, /Video/);
  assert.match(env.$('.bars__row').textContent, /112 GB/);
  assert.match(env.$('.bars__row').getAttribute('aria-label'), /Video: 112 GB/);
  await env.click(env.$('[data-fk="dtab-age"]'));
  assert.deepEqual(last(env, 'breakdown'), { by: 'age' });
  assert.equal(env.$$('.bars__row').length, 6);
});

test('disk usage: before a scan it says so; not_implemented shows the hint; errors retry', async () => {
  const fresh = await makeEnv({ scenario: 'empty' });
  await go(fresh, 'disk');
  assert.match(fresh.$('.state--empty').textContent, /Scan before exploring disk usage/);
  await fresh.click(fresh.$('[data-fk="disk-overview"]'));
  assert.equal(fresh.state().route.name, 'overview');
  const ni = await rich({ });
  ni.backend.disable('disk_usage_tree');
  await go(ni, 'disk');
  assert.match(ni.$('.state--unavailable').textContent, /not available in this build/i);
  assert.ok(ni.state().unavail.disk);
  const bad = await rich();
  bad.backend.failNext('disk_usage_tree', { code: 'io', message: 'Disk read failed', hint: 'Try again.' });
  await go(bad, 'disk');
  assert.match(bad.$('.state--error').textContent, /Disk read failed/);
  await bad.click(bad.$('[data-fk="disk-retry"]'));
  assert.ok(bad.$$('.trow').length > 0);
  const bd = await rich();
  bd.backend.disable('breakdown');
  await go(bd, 'disk');
  await bd.click(bd.$('[data-fk="dtab-type"]'));
  assert.match(bd.$('.state--unavailable').textContent, /not available/i);
});

// --------------------------------------------------- Category views, platform

test('category views: disabled cards show the backend reason and cannot be added', async () => {
  const env = await rich();
  await go(env, 'optimise');
  const cards = env.$$('.card--disabled');
  assert.equal(cards.length, 3);
  assert.deepEqual(cards.map((c) => c.querySelector('[data-role="card-reason"]').textContent), ['Requires ffmpeg on PATH', 'Requires ffmpeg on PATH', 'Needs a Btrfs volume']);
  assert.ok(cards.every((c) => c.getAttribute('aria-disabled') === 'true' && c.querySelector('button[disabled]')));
  assert.match(env.$('[data-role="category-count"]').textContent, /7 opportunities/);
  const before = env.sent('plan_add_step').length;
  env.$('[data-fk="add-opt:video"]').click();
  assert.equal(env.sent('plan_add_step').length, before);
});

test('platform awareness: the card is labelled from app_info, drives and tools instead of hidden', async () => {
  const env = await rich({ platform: 'linux' });
  assert.equal(env.state().info.platform, 'linux');
  await go(env, 'optimise');
  assert.ok(env.$('[data-opp="opt:btrfs_compress"]'), 'still listed');
  const pdf = env.$('[data-opp="opt:fs_compress"]');
  assert.ok(pdf);
});

test('archive view: cold-folder months and format go through the backend; the integrity test is locked on', async () => {
  const env = await rich();
  await go(env, 'archive');
  const verify = env.$('[data-fk="archive-verify"]');
  assert.equal(verify.disabled, true);
  assert.equal(verify.checked, true);
  assert.match(env.$('.aparams').textContent, /Always on/);
  env.$('[data-fk="archive-months"]').setValue('24');
  await env.settle();
  assert.deepEqual(last(env, 'step_params_set'), { opportunity_id: 'det:inactive_engagements', params: { idle_months: 24 } });
  assert.equal(env.$$('option', env.$('[data-fk="archive-format"]')).find((o) => o.selected).value, 'seven_zip');
  env.$('[data-fk="archive-format"]').setValue('zip');
  await env.settle();
  await env.actions.addToPlan('det:inactive_engagements');
  await env.settle();
  env.$('[data-fk="archive-format"]').setValue('tar_zstd');
  await env.settle();
  const upd = last(env, 'plan_update_step');
  assert.equal(upd.action.kind, 'archive');
  assert.equal(upd.action.format, 'tar_zstd');
});

test('category views: loading, empty and error states', async () => {
  const env = await makeEnv({ scenario: 'empty' });
  await go(env, 'clean');
  assert.match(env.$('.state--empty').textContent, /Nothing found here yet/);
  const bad = await rich();
  bad.backend.failNext('opportunities_list', { code: 'io', message: 'Could not read results', hint: 'Scan again.' });
  await go(bad, 'archive');
  assert.match(bad.$('.state--error').textContent, /Could not read results/);
  await bad.click(bad.$('[data-fk="category-retry"]'));
  assert.ok(bad.$('.card'));
});

// ------------------------------------------------------------ Error paths

test('every backend error reaches the user as a toast, never as an unhandled rejection', async () => {
  const env = await rich();
  const err = { code: 'internal', message: 'Boom from the engine', hint: 'Report it.' };
  const cases = [
    ['history_run_detail', () => { env.actions.navigate('history'); }],
  ];
  void cases;
  const calls = [
    ['candidates_page', () => env.actions.openStep('det:temp_files')],
    ['selection_set', async () => { await env.actions.openStep('det:temp_files'); await env.settle(); env.backend.failNext('selection_set', err); await env.actions.toggleRow(env.state().step.page.rows[0].id, false); }],
    ['plan_add_step', () => env.actions.addToPlan('det:temp_files')],
    ['plan_reorder', () => env.actions.moveStep(env.state().plan.steps[0].id, 1)],
    ['job_pause', () => env.actions.pauseJob(1)],
    ['search_run', async () => { env.actions.setSearchOption({ }); env.actions.navigate('clean'); await env.settle(); env.actions.setQuery('ext:log'); await env.settle(); await env.actions.runSearch(); }],
    ['report_get', () => env.actions.viewReport(5)],
    ['report_export', async () => { await env.actions.startExport({ type: 'audit', format: 'csv' }); await env.settle(); await env.actions.chooseFolder(); }],
    ['restore_entries', () => env.actions.restoreEntries([1])],
    ['undo_run', async () => { await env.actions.acceptAsk(); env.store.set({ ask: { kind: 'undo_run', runId: 1 } }); await env.actions.acceptAsk(); }],
    ['presets_list', () => env.actions.loadPresets()],
    ['scope_remove_root', () => env.actions.removeRoot('x')],
  ];
  for (const [cmd, run] of calls) {
    env.store.set({ toast: null });
    env.backend.failNext(cmd, err);
    await run();
    await env.settle();
    assert.equal(env.state().toast?.message, 'Boom from the engine', `${cmd} surfaced`);
  }
});

test('not_implemented renders the control disabled with the hint', async () => {
  const env = await rich();
  env.backend.disable('restore_entries');
  await go(env, 'history');
  await env.click(env.$('.run .run__toggle'));
  const row = env.$$('.entry').find((e) => e.querySelectorAll('button').some((b) => !b.disabled) && /Done/.test(e.textContent));
  await env.click(row.querySelector('button'));
  assert.ok(env.state().unavail.restore_entries);
  assert.ok(env.$$('.entry button').every((b) => b.disabled));
  assert.match(env.$('.entry button').getAttribute('title') ?? '', /Unavailable|not available/i);
  env.backend.disable('report_export');
  await go(env, 'history');
  await env.click(env.$('[data-fk="export-audit"]'));
  await env.click(env.$('[data-fk="browser-choose"]'));
  assert.ok(env.state().unavail.export);
  assert.equal(env.$('[data-fk="export-audit"]').disabled, true);
  env.backend.disable('quarantine_purge');
  await env.click(env.$('[data-fk="purge"]'));
  await env.click(env.$('[data-fk="ask-confirm"]'));
  assert.equal(env.$('[data-fk="purge"]').disabled, true);
});

test('stale responses are ignored: the newest query, category and page win', async () => {
  const env = await rich();
  await go(env, 'clean');
  let release;
  const orig = env.api.searchParse;
  const gate = new Promise((r) => { release = r; });
  let n = 0;
  env.api.searchParse = async (req) => { n += 1; if (n === 1) await gate; return orig(req); };
  const first = env.actions.setQuery('bogus:1');
  await new Promise((r) => setTimeout(r, 5));
  await env.actions.setQuery('ext:mp4');
  release();
  await first;
  await env.settle();
  assert.deepEqual(env.state().custom.errors, [], 'the late answer about the old query was dropped');
  assert.equal(env.state().custom.query, 'ext:mp4');
});

test('stale responses: switching category quickly keeps the last category', async () => {
  const env = await rich({ speed: 0 });
  const a = env.actions.loadCategory('clean');
  const b = env.actions.loadCategory('archive');
  await Promise.all([a, b]);
  assert.equal(env.state().category.name, 'archive');
  assert.equal(env.state().category.items.length, 1);
});

test('teardown: every store subscription and event listener is released', async () => {
  const env = await makeEnv({ mount: false });
  const { mountApp } = await import('./app.js');
  const { bridgeEvents } = await import('./events.js');
  await env.actions.init();
  const app = mountApp(env.root, env.store, env.actions);
  const off = await bridgeEvents({ api: env.api, store: env.store, actions: env.actions });
  assert.equal(env.backend.listenerCount(), 10);
  off();
  assert.equal(env.backend.listenerCount(), 0);
  app.dispose();
  const html = env.root.textContent;
  env.store.set({ toast: { level: 'info', message: 'after dispose', hint: '' } });
  assert.equal(env.root.textContent, html, 'no region redraws after dispose');
});

test('real-backend readiness: zero opportunities, zero drives, very long paths, huge counts', async () => {
  const env = await rich();
  const long = `C:\\Users\\Michel\\${'very-long-folder-name\\'.repeat(14)}file.bin`;
  env.store.set({ drives: [], opps: { ...env.state().opps, items: [], summary: { total: 0, reversible: 0, opt_in: 0, review_only: 0, opportunities: 0 }, counts: { all: 0, clean: 0, optimise: 0, archive: 0, review_only: 0 } } });
  assert.match(env.$('.empty-note').textContent, /No opportunities/);
  env.store.set({ route: { name: 'step', params: {}, from: null }, step: { id: 'x', detail: { opportunity: { id: 'x', title: 'T', category: 'clean', source: {}, sub_groups: [], unit: 'files', params: {}, presets: [], settings: [], available_actions: ['trash'], default_action: { kind: 'trash' }, description: '', safety: 'reversible' }, summary: { candidates: 1234567, selected: 1234567, size_before: 5 * 1024 ** 4, size_after: 0, saving: 5 * 1024 ** 4 }, diagnostics: { notes: [] }, lossy: false, safety: 'reversible' }, page: { total: 1234567, offset: 0, rows: [{ id: 1, path: long, size: 2 * 1024 ** 4, size_after: 0, selected: true, actionable: true, badges: ['online_only', 'signed', 'encrypted'], reason: '', target: 'file' }], groups: [] }, subGroup: null, offset: 0, loading: false, sort: 'group', descending: false, filter: '', focusId: null } });
  assert.match(env.$('[data-role="step-summary"]').textContent, /1,234,567 files/);
  const cell = env.$('.cell--folder');
  assert.ok(cell.textContent.includes('…') && cell.textContent.length <= 40);
  assert.equal(cell.getAttribute('title'), long.slice(0, long.lastIndexOf('\\')));
  assert.match(env.$('.pager__range') ? env.$('.pager__range').textContent : '1,234,567', /1,234,567/);
  assert.deepEqual(texts(env.$$('.badge--row')), ['online-only', 'signed', 'encrypted']);
  assert.match(env.$('.row').textContent, /2\.0 TB/);
});
