// Visual check: serves ui/, opens it in Chromium (mock mode, 1440x900, scale 1),
// writes screenshots to docs/design/screenshots/ and fails on console errors.
// Usage: node ui/dev/screenshots.mjs
import { createRequire } from 'node:module';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { serve } from './serve.mjs';

const { chromium } = createRequire(import.meta.url)('playwright');
const out = join(dirname(fileURLToPath(import.meta.url)), '..', '..', 'docs', 'design', 'screenshots');
const port = 5199;
const server = await serve(port);
const browser = await chromium.launch();
const errors = [];

/** @param {string} query */
async function open(query) {
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1 });
  page.on('console', (m) => { if (['error', 'warning'].includes(m.type())) errors.push(`${m.type()}: ${m.text()}`); });
  page.on('pageerror', (e) => errors.push(`pageerror: ${e.message}`));
  await page.goto(`http://127.0.0.1:${port}/?mock=1&${query}`);
  await page.waitForSelector('.app');
  return page;
}
const shot = (page, name) => page.screenshot({ path: join(out, `${name}.png`) });

// 1. empty state, then the folder browser
let page = await open('scenario=empty');
await page.waitForSelector('.welcome');
await shot(page, 'empty-state');
await page.click('[data-fk="add-folder"]');
await page.waitForSelector('.browser__entry');
await shot(page, 'folder-browser');
await page.click('[data-fk="browser-choose"]');
await page.waitForSelector('.chip');
await shot(page, 'empty-state-with-folder');

// 2. scanning (mid-way)
await page.click('[data-fk="scan"]');
await page.waitForSelector('.card--live');
await page.waitForTimeout(1100);
await shot(page, 'scanning');
await page.waitForSelector('.hero', { timeout: 15000 });
await shot(page, 'results-after-scan');
await page.close();

// 3. B1 results with the four-step plan
page = await open('scenario=results');
await page.waitForSelector('.hero');
await shot(page, 'B1-overview-results');

// 3b. narrowest supported width
await page.setViewportSize({ width: 1100, height: 800 });
await shot(page, 'B1-overview-1100');
await page.setViewportSize({ width: 1440, height: 900 });

// 4. B2 step review, then the lossy confirmation dialog
await page.click('[data-fk="review-opt:audio"]');
await page.waitForSelector('.row');
await page.waitForSelector('.wave');
await shot(page, 'B2-step-review');
await page.click('[data-fk="step-apply"]');
await page.waitForSelector('[role="dialog"]');
await page.check('[data-fk="dlg-ack"]');
await shot(page, 'B2-step-review-lossy-confirm');

// 5. action running, then completed
await page.click('[data-fk="dlg-act"]');
await page.waitForSelector('.queue');
await page.click('[data-fk="back"]');
await page.waitForTimeout(900);
await shot(page, 'action-running');
await page.waitForSelector('[data-role="completed"]', { timeout: 30000 });
await shot(page, 'action-completed');
await page.close();

await browser.close();
server.close();
if (errors.length) {
  console.error(`Console problems:\n${errors.join('\n')}`);
  process.exit(1);
}
console.log('Screenshots written, console clean.');
