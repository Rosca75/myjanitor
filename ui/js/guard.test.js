// Source-level guards over ui/: colours only in tokens.css, no unsafe DOM APIs,
// no network, no em dashes in copy, explicit .js imports, responsive layout.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, dirname, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const ui = join(dirname(fileURLToPath(import.meta.url)), '..');
const tokens = join(ui, 'styles', 'tokens.css');

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    if (name === 'node_modules' || name.startsWith('.')) continue;
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, out);
    else out.push(p);
  }
  return out;
}

const all = walk(ui);
const isTest = (p) => p.endsWith('.test.js');
const isDev = (p) => p.includes(`${join('ui', 'dev')}`);
const rel = (p) => relative(ui, p);
const source = (p) => readFileSync(p, 'utf8');
const textFiles = all.filter((p) => /\.(css|js|mjs|html)$/.test(p) && p !== tokens && !p.endsWith('guard.test.js'));
const inTesting = (p) => p.includes(join('js', 'testing'));
const appCode = all.filter((p) => /\.(js|html)$/.test(p) && !isTest(p) && !isDev(p) && !inTesting(p) && !p.endsWith('test-all.js'));
const cssFiles = all.filter((p) => p.endsWith('.css') && p !== tokens);

const NAMED = new Set(('aliceblue antiquewhite aqua aquamarine azure beige bisque black blanchedalmond blue blueviolet brown burlywood cadetblue chartreuse chocolate coral cornflowerblue cornsilk crimson cyan darkblue darkcyan darkgoldenrod darkgray darkgreen darkgrey darkkhaki darkmagenta darkolivegreen darkorange darkorchid darkred darksalmon darkseagreen darkslateblue darkslategray darkslategrey darkturquoise darkviolet deeppink deepskyblue dimgray dimgrey dodgerblue firebrick floralwhite forestgreen fuchsia gainsboro ghostwhite gold goldenrod gray green greenyellow grey honeydew hotpink indianred indigo ivory khaki lavender lavenderblush lawngreen lemonchiffon lightblue lightcoral lightcyan lightgoldenrodyellow lightgray lightgreen lightgrey lightpink lightsalmon lightseagreen lightskyblue lightslategray lightslategrey lightsteelblue lightyellow lime limegreen linen magenta maroon mediumaquamarine mediumblue mediumorchid mediumpurple mediumseagreen mediumslateblue mediumspringgreen mediumturquoise mediumvioletred midnightblue mintcream mistyrose moccasin navajowhite navy oldlace olive olivedrab orange orangered orchid palegoldenrod palegreen paleturquoise palevioletred papayawhip peachpuff peru pink plum powderblue purple rebeccapurple red rosybrown royalblue saddlebrown salmon sandybrown seagreen seashell sienna silver skyblue slateblue slategray slategrey snow springgreen steelblue tan teal thistle tomato turquoise violet wheat white whitesmoke yellow yellowgreen').split(' '));

const stripComments = (css) => css.replace(/\/\*[\s\S]*?\*\//g, '');

test('no colour literal (hex, rgb, hsl, hwb, lab, lch, oklab, oklch, color()) outside tokens.css', () => {
  const literal = /#[0-9a-fA-F]{3,8}\b|\b(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch|color|color-mix)\(/;
  const offenders = [];
  for (const p of textFiles) {
    const text = p.endsWith('.css') ? stripComments(source(p)) : source(p);
    text.split('\n').forEach((line, i) => { if (literal.test(line)) offenders.push(`${rel(p)}:${i + 1}: ${line.trim().slice(0, 80)}`); });
  }
  assert.deepEqual(offenders, []);
});

test('no named colour in any CSS declaration outside tokens.css', () => {
  const skip = /^(font-family|content|grid-template-areas|grid-area|animation-name|will-change|transition-property|list-style-type)$/;
  const offenders = [];
  for (const p of cssFiles) {
    const css = stripComments(source(p));
    for (const m of css.matchAll(/([a-z-]+)\s*:\s*([^;{}]+)[;}]/gi)) {
      if (skip.test(m[1]) || m[1].startsWith('--')) continue;
      const value = m[2].replace(/var\([^)]*\)/g, '').replace(/"[^"]*"/g, '');
      for (const w of value.toLowerCase().split(/[^a-z]+/)) if (NAMED.has(w)) offenders.push(`${rel(p)}: ${m[1]}: ${m[2].trim()}`);
    }
  }
  assert.deepEqual(offenders, []);
});

test('no named colour literal in JavaScript strings', () => {
  const offenders = [];
  for (const p of appCode.filter((f) => f.endsWith('.js'))) {
    for (const m of source(p).matchAll(/(['"`])([A-Za-z]+)\1/g)) if (NAMED.has(m[2].toLowerCase()) && !['tan', 'plum', 'gold', 'olive', 'linen', 'snow', 'peru', 'azure', 'coral', 'khaki', 'violet', 'wheat', 'silver', 'orchid', 'ivory', 'indigo', 'tomato', 'salmon', 'pink', 'teal', 'lime'].includes(m[2].toLowerCase())) offenders.push(`${rel(p)}: ${m[0]}`);
    if (/['"`]\s*(?:color|background(?:-color)?|border-color|fill|stroke)\s*:/.test(source(p))) offenders.push(`${rel(p)}: inline colour declaration`);
  }
  assert.deepEqual(offenders, []);
});

test('every var(--token) used in CSS is defined (tokens.css or locally)', () => {
  const defined = new Set([...source(tokens).matchAll(/(--[a-z0-9-]+)\s*:/gi)].map((m) => m[1]));
  for (const p of cssFiles) for (const m of stripComments(source(p)).matchAll(/(--[a-z0-9-]+)\s*:/gi)) defined.add(m[1]);
  const missing = [];
  for (const p of cssFiles) for (const m of stripComments(source(p)).matchAll(/var\((--[a-z0-9-]+)/gi)) if (!defined.has(m[1])) missing.push(`${rel(p)}: ${m[1]}`);
  assert.deepEqual(missing, []);
});

test('no unsafe DOM APIs, inline handlers, eval or inline styles in app code', () => {
  const banned = [/innerHTML/, /outerHTML/, /insertAdjacentHTML/, /document\.write/, /\beval\s*\(/, /new Function\b/, /javascript:/i, /srcdoc/, /cssText/, /setAttribute\(\s*['"]style['"]/, /\bstyle\s*=\s*["']/, /<script[^>]*>[^<]+/i, /\son[a-z]+\s*=\s*["']/i];
  const offenders = [];
  for (const p of appCode) for (const re of banned) if (re.test(source(p))) offenders.push(`${rel(p)}: ${re}`);
  assert.deepEqual(offenders, []);
});

test('only api.js touches window.__TAURI__ (the dev aid reads globalThis.__mj only)', () => {
  const offenders = appCode.filter((p) => !p.endsWith('api.js') && /__TAURI__|window\.go\b/.test(source(p)));
  assert.deepEqual(offenders.map(rel), []);
});

test('no network resources: no fetch, XHR, absolute URLs, CDN or web fonts', () => {
  const offenders = [];
  for (const p of [...appCode, ...cssFiles]) {
    const s = source(p);
    if (/\bfetch\s*\(|XMLHttpRequest|WebSocket|@import|https?:\/\//.test(s.replace(/http:\/\/www\.w3\.org\/2000\/svg/, ''))) offenders.push(rel(p));
  }
  assert.deepEqual(offenders, []);
  assert.doesNotMatch(source(join(ui, 'index.html')), /(?:src|href)="https?:/);
});

test('user-visible copy has no em dash', () => {
  const offenders = appCode.filter((p) => source(p).includes('\u2014'));
  assert.deepEqual(offenders.map(rel), []);
});

test('all user-visible English lives in copy.js: views carry no sentence literals', () => {
  const views = appCode.filter((p) => p.includes(join('js', 'views')) || p.includes(join('js', 'components')));
  const offenders = [];
  for (const p of views) {
    for (const m of source(p).matchAll(/(?:^|[^\w.-])h\(\s*'(?:p|h1|h2|h3|span|strong|label|button)'\s*,\s*(?:\{[^}]*\}|null)\s*,\s*'([^']{3,})'/g)) offenders.push(`${rel(p)}: ${m[1]}`);
  }
  assert.deepEqual(offenders, []);
});

test('imports are relative and carry the .js extension', () => {
  const offenders = [];
  for (const p of appCode.filter((f) => f.endsWith('.js'))) {
    for (const m of source(p).matchAll(/(?:import|export)[^'"\n]*from\s*['"]([^'"]+)['"]|import\(\s*['"]([^'"]+)['"]\s*\)/g)) {
      const spec = m[1] ?? m[2];
      if (spec.startsWith('node:')) continue;
      if (!/^\.{1,2}\//.test(spec) || !spec.endsWith('.js')) offenders.push(`${rel(p)}: ${spec}`);
    }
  }
  assert.deepEqual(offenders, []);
});

test('index.html loads only local assets and has no inline script or style', () => {
  const html = source(join(ui, 'index.html'));
  assert.doesNotMatch(html, /<style|style=/i);
  assert.doesNotMatch(html, /<script(?![^>]*\bsrc=)/i);
  for (const m of html.matchAll(/(?:src|href)="([^"]+)"/g)) assert.match(m[1], /^\/(?:js|styles)\//);
  for (const css of ['tokens', 'app', 'components', 'overview', 'step', 'overlays', 'views']) assert.match(html, new RegExp(`/styles/${css}\\.css`));
});

test('layout uses the frozen layout tokens and is responsive down to 1100 px', () => {
  const app = source(join(ui, 'styles', 'app.css'));
  for (const t of ['--rail-width', '--header-height', '--panel-width', '--strip-height']) assert.match(app, new RegExp(`var\\(${t}\\)`));
  assert.match(app, /min-width:\s*1100px/);
  assert.match(all.filter((p) => p.endsWith('components.css')).map(source).join('\n'), /@media \(max-width: 1279px\)[\s\S]*repeat\(2/);
  assert.match(app, /grid-template-areas/);
});

test('a visible keyboard focus ring comes from the tokens', () => {
  assert.match(source(join(ui, 'styles', 'app.css')), /:focus-visible\s*\{[^}]*var\(--focus-ring\)/);
});

test('no nested scroll containers inside the main view: only the main region and the side panel scroll', () => {
  const scrollers = [];
  for (const p of cssFiles) {
    for (const m of stripComments(source(p)).matchAll(/([^{}]+)\{([^}]*)\}/g)) if (/overflow(?:-y)?\s*:\s*(?:auto|scroll)/.test(m[2])) scrollers.push(m[1].trim());
  }
  const allowed = new Set(['.app__main', '.app__panel', '.queue', '.scrim', '.modal']);
  assert.deepEqual(scrollers.filter((s) => !allowed.has(s)), []);
});

test('the colour guard really catches a literal (self-check)', () => {
  const literal = /#[0-9a-fA-F]{3,8}\b|\b(?:rgba?|hsla?)\(/;
  assert.ok(literal.test('color: #FF0000;'));
  assert.ok(literal.test('background: rgb(1,2,3)'));
  assert.equal(literal.test('color: var(--text)'), false);
});
