// Visual check of the W3-D2 screens (rich mock scenario). Usage: node ui/dev/screenshots-w3.mjs
import { createRequire } from 'node:module';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { serve } from './serve.mjs';

const { chromium } = createRequire(import.meta.url)('playwright');
const out = join(dirname(fileURLToPath(import.meta.url)), '..', '..', 'docs', 'design', 'screenshots');
const port = 5198;
const server = await serve(port);
const browser = await chromium.launch();
const errors = [];

async function open(query, width = 1440, height = 900) {
  const page = await browser.newPage({ viewport: { width, height }, deviceScaleFactor: 1 });
  page.on('console', (m) => { if (['error', 'warning'].includes(m.type())) errors.push(`${m.type()}: ${m.text()}`); });
  page.on('pageerror', (e) => errors.push(`pageerror: ${e.message}`));
  await page.goto(`http://127.0.0.1:${port}/?mock=1&${query}`);
  await page.waitForSelector('.app');
  return page;
}
const shot = (page, name) => page.screenshot({ path: join(out, `${name}.png`) });
const nav = (page, name) => page.click(`[data-fk="nav-${name}"]`);

// History and report
let page = await open('scenario=rich');
await page.waitForSelector('.hero');
await nav(page, 'history');
await page.waitForSelector('.run');
await page.click('.run:first-child .run__toggle');
await page.waitForSelector('.entry');
await shot(page, 'W3-history');
await page.setViewportSize({ width: 1100, height: 800 });
await shot(page, 'W3-history-1100');
await page.setViewportSize({ width: 1440, height: 900 });
await page.click('.run:first-child [data-fk^="report-"]');
await page.waitForSelector('[role="dialog"] .rsec');
await shot(page, 'W3-report');
await page.click('[data-fk="report-csv"]');
await page.waitForSelector('[data-fk="export-name"]');
await shot(page, 'W3-export-picker');
await page.click('[data-fk="browser-choose"]');
await page.waitForSelector('.toast');
await page.close();

// Job queue running, then the summary
page = await open('scenario=rich');
await page.waitForSelector('.hero');
await page.click('[data-fk="apply"]');
await page.waitForSelector('.job');
await page.waitForTimeout(500);
await shot(page, 'W3-job-queue-running');
await page.waitForSelector('[data-role="queue-summary"]', { timeout: 30000 });
await shot(page, 'W3-job-queue-done');
await page.close();

// Custom search
page = await open('scenario=rich');
await page.waitForSelector('.hero');
await nav(page, 'clean');
await page.waitForSelector('.custom');
await page.type('[data-fk="custom-query"]', 'ext:mp4 si');
await page.waitForSelector('.suggest');
await shot(page, 'W3-search-suggest');
await page.keyboard.press('Escape');
await page.fill('[data-fk="custom-query"]', 'ext:mp4 bogus:1 size:>1GB');
await page.waitForSelector('.query__bad');
await shot(page, 'W3-search-error');
await page.click('[data-fk="smode-builder"]');
await page.click('[data-fk="b-root-addg"]');
await page.waitForSelector('.bgroup--nested');
await page.click('[data-fk="b-1-add"]');
await page.fill('[data-fk="b-0-pattern"]', '*.log');
await page.selectOption('[data-fk="b-1_0-field"]', 'size');
await page.fill('[data-fk="b-1_0-amount"]', '10');
await page.fill('[data-fk="custom-title"]', 'Big logs');
await page.evaluate(() => document.querySelector('.custom').scrollIntoView());
await shot(page, 'W3-search-builder');
await page.click('[data-fk="custom-run"]');
await page.waitForSelector('.cresults .row');
await page.evaluate(() => document.querySelector('.cresults').scrollIntoView());
await shot(page, 'W3-search-builder-results');
await page.close();

// Disk usage
page = await open('scenario=rich');
await page.waitForSelector('.hero');
await page.click('[data-fk="explore-disk"]');
await page.waitForSelector('.trow');
await page.click('[data-fk^="dt-C"]');
await shot(page, 'W3-disk-tree');
await page.click('[data-fk="dtab-treemap"]');
await page.waitForSelector('.tile2');
await page.click('.tile2:first-child');
await page.waitForSelector('.tile2__name');
await shot(page, 'W3-disk-treemap');
await page.setViewportSize({ width: 1100, height: 800 });
await shot(page, 'W3-disk-treemap-1100');
await page.setViewportSize({ width: 1440, height: 900 });
await page.click('[data-fk="dtab-age"]');
await page.waitForSelector('.bars__row');
await shot(page, 'W3-disk-age');
await page.close();

// Category views
page = await open('scenario=rich');
await page.waitForSelector('.hero');
await nav(page, 'optimise');
await page.waitForSelector('.card--disabled');
await shot(page, 'W3-category-disabled');
await nav(page, 'archive');
await page.waitForSelector('.aparams');
await shot(page, 'W3-archive-params');
await page.close();

// Step review with preview
page = await open('scenario=rich');
await page.waitForSelector('.hero');
await page.click('[data-fk="review-det:exact_duplicates"]');
await page.waitForSelector('.group-head');
await page.click('.row:has-text("IMG_0412") .cell__file');
await page.waitForSelector('.preview__image');
await shot(page, 'W3-step-preview');
await page.close();

await browser.close();
server.close();
if (errors.length) { console.error(`Console problems:\n${errors.join('\n')}`); process.exit(1); }
console.log('Screenshots written, console clean.');
