import { test } from 'node:test';
import assert from 'node:assert/strict';
import { formatBytes, formatAbout, formatApprox, formatCount, formatAgo, formatClock, formatDate, plural, basename, dirname, displayPath, pct } from './format.js';

const GB = 1024 ** 3;

test('formatBytes uses binary units with Explorer labels and one decimal below 100', () => {
  assert.equal(formatBytes(0), '0 B');
  assert.equal(formatBytes(512), '512 B');
  assert.equal(formatBytes(1536), '1.5 KB');
  assert.equal(formatBytes(Math.round(48.6 * GB)), '48.6 GB');
  assert.equal(formatBytes(Math.round(12.4 * GB)), '12.4 GB');
  assert.equal(formatBytes(354 * GB), '354 GB');
  assert.equal(formatBytes(14 * GB), '14.0 GB');
  assert.equal(formatBytes(100 * GB), '100 GB');
});

test('formatBytes rounds up across a unit boundary like mj_core::size', () => {
  assert.equal(formatBytes(1024 * 1024 - 1), '1.0 MB');
  assert.equal(formatBytes(-5), '0 B');
  assert.equal(formatBytes(NaN), '0 B');
});

test('formatAbout and formatApprox read estimates in gigabytes from 100 MB', () => {
  assert.equal(formatAbout(644245094), '0.6 GB');
  assert.equal(formatApprox(644245094), '~0.6 GB');
  assert.equal(formatAbout(50 * 1024 ** 2), '50.0 MB');
  assert.equal(formatAbout(2 * GB), '2.0 GB');
});

test('formatCount groups thousands', () => {
  assert.equal(formatCount(184233), '184,233');
  assert.equal(formatCount(7), '7');
});

test('formatAgo gives short relative times from nanosecond timestamps', () => {
  const now = Date.UTC(2026, 10, 13, 12, 0, 0);
  const ns = (msAgo) => (now - msAgo) * 1e6;
  assert.equal(formatAgo(ns(10_000), now), 'just now');
  assert.equal(formatAgo(ns(2 * 60_000), now), '2 min ago');
  assert.equal(formatAgo(ns(59 * 60_000), now), '59 min ago');
  assert.equal(formatAgo(ns(60 * 60_000), now), '1 hour ago');
  assert.equal(formatAgo(ns(3 * 3600_000), now), '3 hours ago');
  assert.equal(formatAgo(ns(30 * 3600_000), now), 'yesterday');
  assert.equal(formatAgo(ns(5 * 86400_000), now), '5 days ago');
  assert.equal(formatAgo(ns(90 * 86400_000), now), '15 Aug 2026');
});

test('formatClock, formatDate, plural', () => {
  assert.equal(formatClock(3734), '1:02:14');
  assert.equal(formatClock(65), '1:05');
  assert.equal(formatDate(Date.UTC(2026, 10, 13) * 1e6), '13 Nov 2026');
  assert.equal(plural(1, 'step'), 'step');
  assert.equal(plural(4, 'step'), 'steps');
  assert.equal(plural(2, 'opportunity', 'opportunities'), 'opportunities');
});

test('path helpers cope with both separators and non-Unicode escapes', () => {
  assert.equal(basename('C:\\Users\\Michel\\a.txt'), 'a.txt');
  assert.equal(basename('/home/me/b.txt'), 'b.txt');
  assert.equal(dirname('C:\\Users\\Michel\\a.txt'), 'C:\\Users\\Michel');
  assert.equal(displayPath('\u0000/home/%FFname'), '/home/\uFFFDname');
  assert.equal(displayPath('/plain'), '/plain');
});

test('pct clamps', () => {
  assert.equal(pct(50, 200), 25);
  assert.equal(pct(5, 0), 0);
  assert.equal(pct(500, 100), 100);
});
