// Pure functions of the W3-D2 screens: treemap, rule builder, query helpers, formatters, trees.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { squarify, aspect } from './lib/treemap.js';
import { newCond, newGroup, toRule, fromRule, validate, splitBytes, parseExtensions, dateToNs, addTo, removeAt, updateAt, initialModel, countConds } from './lib/rulebuilder.js';
import { byteOffset, indexOfByte, segments, applySuggestion, columnOf } from './lib/query.js';
import { middleEllipsis, joinPath, compressionRate, formatBytes, formatCount, formatPercent } from './format.js';
import { flatten, findNode, replaceNode, needsLoad, treemapItems } from './lib/disktree.js';
import { runStatus, canRestore } from './lib/history.js';
import { missingRequirement } from './lib/platform.js';
import { createGens } from './lib/gens.js';
import { parseQuery } from './mock/extras.js';

test('squarify: tile areas add up to the box and tiles stay roughly square', () => {
  const items = [6, 6, 4, 3, 2, 2, 1].map((v, i) => ({ key: `k${i}`, value: v }));
  const rect = { x: 0, y: 0, w: 600, h: 400 };
  const tiles = squarify(items, rect);
  assert.equal(tiles.length, items.length);
  const area = tiles.reduce((a, t) => a + t.w * t.h, 0);
  assert.ok(Math.abs(area - 600 * 400) < 1e-6, `area ${area}`);
  for (const t of tiles) {
    assert.ok(t.x >= -1e-9 && t.y >= -1e-9 && t.x + t.w <= 600 + 1e-6 && t.y + t.h <= 400 + 1e-6, 'inside the box');
    assert.ok(aspect(t) < 4, `aspect ${aspect(t)}`);
  }
  const total = 24;
  for (const t of tiles) assert.ok(Math.abs((t.w * t.h) / (600 * 400) - t.value / total) < 1e-9, 'area proportional to value');
});

test('squarify: ignores empty items, handles one item and degenerate boxes', () => {
  assert.deepEqual(squarify([{ key: 'a', value: 0 }], { x: 0, y: 0, w: 10, h: 10 }), []);
  assert.deepEqual(squarify([{ key: 'a', value: 5 }], { x: 0, y: 0, w: 0, h: 10 }), []);
  const one = squarify([{ key: 'a', value: 5 }], { x: 0, y: 0, w: 10, h: 20 });
  assert.deepEqual([one[0].w, one[0].h], [10, 20]);
  const many = squarify(Array.from({ length: 200 }, (_, i) => ({ key: `${i}`, value: 1 + (i % 7) })), { x: 0, y: 0, w: 1000, h: 520 });
  assert.ok(Math.abs(many.reduce((a, t) => a + t.w * t.h, 0) - 520000) < 1e-3);
});

test('rule builder: model to Rule matches the IPC.md shape', () => {
  const model = { ...newGroup('all'), children: [
    { ...newCond('extension'), values: 'MP4, .mkv' },
    { ...newCond('size'), cmp: 'gt', amount: 1, unit: 'GB' },
    { ...newCond('date'), which: 'modified', cmp: 'lt', mode: 'ago', amount: 6, unit: 'months' },
    { ...newGroup('any'), children: [{ ...newCond('name'), mode: 'wildcard', pattern: '~$*' }, { ...newCond('attribute'), attrs: { hidden: 'set' }, not: true }] },
    { ...newCond('largest_n'), n: 50 },
    newCond('zero_byte'),
  ] };
  assert.deepEqual(toRule(model), { all: [
    { cond: { field: 'extension', values: ['mp4', 'mkv'] } },
    { cond: { field: 'size', cmp: 'gt', bytes: 1073741824 } },
    { cond: { field: 'date', which: 'modified', cmp: 'lt', value: { ago: { amount: 6, unit: 'months' } } } },
    { any: [{ cond: { field: 'name', mode: 'wildcard', pattern: '~$*', case_sensitive: false } }, { not: { cond: { field: 'attribute', attr: ['hidden'], set: true } } }] },
    { cond: { field: 'largest_n', n: 50 } },
    { cond: { field: 'zero_byte' } },
  ] });
});

test('rule builder: Rule to model and back is the identity', () => {
  const rules = [
    { all: [{ cond: { field: 'extension', values: ['mp4'] } }, { cond: { field: 'size', cmp: 'gt', bytes: 1073741824 } }, { cond: { field: 'date', which: 'modified', cmp: 'lt', value: { at: 1704067200000000000 } } }] },
    { any: [{ cond: { field: 'name', mode: 'wildcard', pattern: '~$*', case_sensitive: false } }, { not: { cond: { field: 'attribute', attr: ['hidden'], set: true } } }, { cond: { field: 'largest_n', n: 50 } }, { cond: { field: 'zero_byte' } }] },
    { all: [{ not: { all: [{ cond: { field: 'path', mode: 'regex', pattern: '^C:\\\\Temp', case_sensitive: true } }] } }, { cond: { field: 'depth', cmp: 'ge', value: 4 } }, { cond: { field: 'path_length', cmp: 'gt', value: 240 } }, { cond: { field: 'kind', kind: 'dir' } }, { cond: { field: 'type_group', group: 'video' } }, { cond: { field: 'owner', value: 'uid:1000' } }, { cond: { field: 'content_contains', text: 'invoice', case_sensitive: false } }, { cond: { field: 'invalid_name' } }, { cond: { field: 'extension_mismatch' } }, { cond: { field: 'attribute', attr: ['hidden', 'system'], set: false } }] },
    { all: [{ cond: { field: 'size', cmp: 'lt', bytes: 1536 } }, { cond: { field: 'date', which: 'created', cmp: 'gt', value: { ago: { amount: 2, unit: 'years' } } } }, { cond: { field: 'brand_new_field', x: 1 } }] },
  ];
  for (const r of rules) assert.deepEqual(toRule(fromRule(r)), r);
});

test('rule builder: a non-group root is wrapped, sizes pick a clean unit, helpers', () => {
  assert.equal(fromRule({ cond: { field: 'zero_byte' } }).kind, 'group');
  assert.deepEqual(splitBytes(1073741824), { amount: 1, unit: 'GB' });
  assert.deepEqual(splitBytes(1536), { amount: 1.5, unit: 'KB' });
  assert.deepEqual(splitBytes(900), { amount: 900, unit: 'B' });
  assert.deepEqual(parseExtensions('JPG, .png;*.gif  webp'), ['jpg', 'png', 'gif', 'webp']);
  assert.equal(dateToNs('2024-01-01'), 1704067200000000000);
  assert.ok(Number.isNaN(dateToNs('01/01/2024')));
  let m = initialModel();
  m = addTo(m, [], newGroup('any'));
  m = addTo(m, [1], newCond('size'));
  assert.equal(countConds(m), 2);
  m = updateAt(m, [1, 0], (n) => ({ ...n, amount: 5 }));
  assert.equal(m.children[1].children[0].amount, 5);
  m = removeAt(m, [1, 0]);
  assert.equal(countConds(m), 1);
  assert.equal(removeAt(m, []), m, 'the root stays');
});

test('rule builder: validation names what is wrong and where', () => {
  const m = { ...newGroup('all'), children: [{ ...newCond('name'), pattern: '' }, { ...newCond('name'), mode: 'regex', pattern: '(' }, { ...newCond('extension'), values: ' , ' }, newCond('attribute'), { ...newGroup('all'), children: [{ ...newCond('largest_n'), n: 5 }] }, { ...newCond('date'), mode: 'at', date: 'tomorrow' }] };
  assert.deepEqual(validate(m).map((e) => `${e.path.join('.')}:${e.code}`), ['0:pattern_empty', '1:regex_invalid', '2:extension_empty', '3:attribute_none', '4.0:largest_n_placement', '5:date_invalid']);
  assert.deepEqual(validate({ ...newGroup('all'), children: [] }).map((e) => e.code), ['empty_group']);
  assert.deepEqual(validate({ ...newGroup('all'), children: [{ ...newCond('largest_n'), n: 10 }] }), []);
});

test('query helpers: byte offsets, underline segments, suggestion insertion', () => {
  assert.equal(byteOffset('é:1', 1), 2);
  assert.equal(indexOfByte('é:1', 2), 1);
  const errs = [{ offset: 8, len: 7, message: 'Unknown key "bogus"' }];
  assert.deepEqual(segments('ext:mp4 bogus:1', errs), [{ text: 'ext:mp4 ', error: null }, { text: 'bogus:1', error: 'Unknown key "bogus"' }]);
  assert.equal(columnOf('ext:mp4 bogus:1', errs[0]), 9);
  assert.deepEqual(applySuggestion('ext:mp4 si', 10, { insert: 'size:', replace_len: 2 }), { text: 'ext:mp4 size:', caret: 13 });
  assert.deepEqual(applySuggestion('é si', 4, { insert: 'size:', replace_len: 2 }), { text: 'é size:', caret: 7 });
});

test('mock query parser reports byte positions and omits the rule on errors', () => {
  const ok = parseQuery('ext:mp4 size:>1GB modified:>2y');
  assert.deepEqual(ok.rule.all.map((c) => c.cond.field), ['extension', 'size', 'date']);
  assert.deepEqual(ok.rule.all[2].cond.value, { ago: { amount: 2, unit: 'years' } });
  const bad = parseQuery('é bogus:1 size:xx');
  assert.equal(bad.rule, undefined);
  assert.deepEqual(bad.errors.map((e) => [e.offset, e.len]), [[3, 7], [16, 2]]);
});

test('formatters: middle ellipsis, sizes from bytes to TB, counts, percentages, paths', () => {
  assert.equal(middleEllipsis('short', 20), 'short');
  const long = `C:\\Users\\Michel\\${'deep\\'.repeat(20)}report-final.pdf`;
  const cut = middleEllipsis(long, 40);
  assert.equal(cut.length, 40);
  assert.ok(cut.includes('…') && cut.endsWith('report-final.pdf'.slice(-10)) && cut.startsWith('C:\\Users'));
  assert.equal(middleEllipsis('', 10), '');
  assert.deepEqual(['0', 1023, 1024, 1536, 1048576, 5 * 1024 ** 4, 1.5 * 1024 ** 5].map(formatBytes), ['0 B', '1023 B', '1.0 KB', '1.5 KB', '1.0 MB', '5.0 TB', '1.5 PB']);
  assert.equal(formatCount(1234567), '1,234,567');
  assert.equal(formatCount(NaN), '0');
  assert.equal(formatPercent(0.004), '0.4 %');
  assert.equal(formatPercent(0.37), '37 %');
  assert.equal(joinPath('C:\\Users\\Me', 'a.csv', 'windows'), 'C:\\Users\\Me\\a.csv');
  assert.equal(joinPath('/home/me/', 'a.csv', 'linux'), '/home/me/a.csv');
  assert.equal(compressionRate(100, 54), 0.45999999999999996);
  assert.equal(compressionRate(100, 101), null);
});

const tree = { name: '', path: '', is_dir: true, size: 100, allocated: 100, files: 10, children: [
  { name: 'a', path: 'C:\\a', is_dir: true, size: 70, allocated: 70, files: 6, children: [{ name: 'x', path: 'C:\\a\\x', is_dir: true, size: 50, allocated: 40, files: 3, children: [] }], other: { size: 20, count: 4 } },
  { name: 'b', path: 'D:\\b', is_dir: true, size: 30, allocated: 30, files: 4, children: [] },
] };

test('disk tree helpers: flatten sorted by size, expand, graft a loaded subtree', () => {
  assert.deepEqual(flatten(tree, {}).map((r) => r.node.name), ['a', 'b']);
  const rows = flatten(tree, { 'C:\\a': true });
  assert.deepEqual(rows.map((r) => [r.node.name || 'other', r.depth]), [['a', 0], ['x', 1], ['other', 1], ['b', 0]]);
  assert.equal(rows[1].share, 50 / 70);
  assert.equal(rows.find((r) => r.other).share, 20 / 70);
  assert.deepEqual(flatten(tree, {}, 'name').map((r) => r.node.name), ['a', 'b']);
  assert.equal(findNode(tree, 'C:\\a\\x').size, 50);
  assert.equal(needsLoad(findNode(tree, 'C:\\a\\x')), true);
  const grafted = replaceNode(tree, 'C:\\a\\x', { ...findNode(tree, 'C:\\a\\x'), children: [{ name: 'y', path: 'C:\\a\\x\\y', is_dir: false, size: 50, allocated: 40, files: 3, children: [] }] });
  assert.equal(findNode(grafted, 'C:\\a\\x\\y').name, 'y');
  assert.equal(findNode(tree, 'C:\\a\\x\\y'), null, 'immutable');
  assert.deepEqual(treemapItems(findNode(tree, 'C:\\a')).map((i) => i.value), [50, 20]);
  assert.deepEqual(flatten(null, {}), []);
});

test('history helpers: run status and what can be restored', () => {
  const run = (o) => ({ kind: 'apply', done: 10, failed: 0, undone: 0, undoable: true, ...o });
  assert.equal(runStatus(run({})), 'undoable');
  assert.equal(runStatus(run({ failed: 1 })), 'with_errors');
  assert.equal(runStatus(run({ undone: 10, undoable: false })), 'restored');
  assert.equal(runStatus(run({ undone: 4 })), 'partly_restored');
  assert.equal(runStatus(run({ undoable: false })), 'expired');
  assert.equal(runStatus(run({ done: 0, failed: 3, undoable: false })), 'failed');
  assert.equal(runStatus(run({ kind: 'undo' })), 'undo');
  assert.equal(canRestore({ state: 'done', reversible: true }, 5), true);
  assert.equal(canRestore({ state: 'done', reversible: true, undo_deadline: 4 }, 5), false);
  assert.equal(canRestore({ state: 'undone', reversible: true }), false);
  assert.equal(canRestore({ state: 'done', reversible: false }), false);
});

test('platform labels: Windows only, Btrfs, tools', () => {
  const words = { windowsOnly: 'Windows only', btrfs: 'Needs a Btrfs volume', reflink: 'r', compression: 'c', tool: (id) => `Requires ${id} on PATH` };
  assert.equal(missingRequirement('opt:dehydrate', { platform: 'linux' }, [], [], words), 'Windows only');
  assert.equal(missingRequirement('opt:dehydrate', { platform: 'windows' }, [], [], words), null);
  assert.equal(missingRequirement('opt:btrfs_compress', { platform: 'linux' }, [{ features: { btrfs_compression: false } }], [], words), 'Needs a Btrfs volume');
  assert.equal(missingRequirement('opt:btrfs_compress', { platform: 'linux' }, [{ features: { btrfs_compression: true } }], [], words), null);
  assert.equal(missingRequirement('opt:video', null, [], [{ id: 'ffmpeg', found: false }], words), 'Requires ffmpeg on PATH');
  assert.equal(missingRequirement('det:temp_files', null, [], [], words), null);
});

test('request generations: only the newest request of a key is current', () => {
  const g = createGens();
  const a = g.next('x');
  const b = g.next('x');
  assert.equal(g.current('x', a), false);
  assert.equal(g.current('x', b), true);
  assert.equal(g.current('y', g.next('y')), true);
});
