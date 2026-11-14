// @ts-check
/**
 * Synthetic disk tree and breakdowns for the mock (disk_usage_tree, breakdown).
 * Definition: [name, GB, thousands of files, children?, compressedFraction?].
 */
import { GB } from './fixtures.js';

const DEF = [
  ['C:\\Users\\Michel', 262, 410, [
    ['AppData', 88, 190, [['Local', 61, 140, [['Google', 18, 40], ['Microsoft', 14, 36], ['Temp', 9, 30], ['Packages', 12, 20], ['Other', 8, 14]]], ['Roaming', 24, 48, [['Teams', 9, 12], ['Code', 6, 20], ['Slack', 4, 6], ['Other', 5, 10]]], ['LocalLow', 3, 2]]],
    ['Documents', 52, 61, [['Clients', 34, 40, [['Nordbank', 12, 14], ['Acme', 9, 11], ['Falcon', 8, 9], ['Other', 5, 6]]], ['Scans', 11, 8], ['Templates', 7, 13]]],
    ['Videos', 47, 1, [['Screen Recordings', 31, 1], ['Family', 16, 0.2]]],
    ['Pictures', 31, 22, [['2025', 14, 9], ['2026', 12, 8], ['Screenshots', 5, 5]]],
    ['Downloads', 22, 1.2, [['Setup_Office_2200.msi', 4.2, 0.001], ['Windows ISO.iso', 5.6, 0.001], ['Other', 12.2, 1.2]]],
    ['Recordings', 12, 0.1],
    ['Desktop', 3, 0.4],
    ['.nuget', 2, 90],
  ]],
  ['D:\\Projects', 152, 1300, [
    ['app', 52, 420, [['node_modules', 31, 330], ['src', 3, 60], ['dist', 18, 30]]],
    ['website', 38, 380, [['node_modules', 22, 300], ['.next', 11, 50], ['src', 5, 30]]],
    ['api', 26, 190, [['target', 19, 90], ['src', 7, 100]]],
    ['notes', 11, 12],
    ['archive-2022', 25, 300, [['cold-data', 25, 300]], 0.46],
  ]],
];

/** @param {any[]} d @param {string} parent @returns {any} */
function build(d, parent) {
  const [name, gb, kfiles, kids, comp] = d;
  const path = parent ? `${parent}\\${name}` : name;
  const size = Math.round(gb * GB);
  const children = (kids ?? []).map((k) => build(k, path)).sort((a, b) => b.size - a.size);
  return { name: parent ? name : name, path, is_dir: !/\.\w{2,4}$/.test(name), size, allocated: Math.round(size * (comp ? 1 - comp : 1.012)), files: Math.round(kfiles * 1000), children };
}

const ROOTS = DEF.map((d) => build(d, ''));
const virtual = () => ({ name: '', path: '', is_dir: true, size: ROOTS.reduce((a, r) => a + r.size, 0), allocated: ROOTS.reduce((a, r) => a + r.allocated, 0), files: ROOTS.reduce((a, r) => a + r.files, 0), children: ROOTS });

/** @param {string} path */
function find(path) {
  const walk = (n) => { if (n.path === path) return n; for (const c of n.children) { const f = walk(c); if (f) return f; } return null; };
  return walk(virtual());
}

/** Cut a subtree to `depth` levels, folding small children into `other`. */
function cut(n, depth, minFraction) {
  if (depth <= 0) return { ...n, children: [] };
  const keep = [];
  let other = null;
  for (const c of n.children) {
    if (n.size > 0 && c.size / n.size < minFraction) other = { size: (other?.size ?? 0) + c.size, count: (other?.count ?? 0) + 1 };
    else keep.push(cut(c, depth - 1, minFraction));
  }
  return { ...n, children: keep, ...(other ? { other } : {}) };
}

/** @param {{ path?: string, depth?: number, min_fraction?: number }} req */
export function diskTree(req) {
  const node = req.path ? find(req.path) : virtual();
  if (!node) return null;
  return cut(node, Math.min(4, Math.max(1, req.depth ?? 2)), req.min_fraction ?? 0.01);
}

const TYPE = [['video', 'Video', 112, 2400], ['documents', 'Documents', 71, 98000], ['images', 'Images', 49, 61000], ['apps', 'Applications', 41, 210000], ['archives', 'Archives', 38, 1700], ['code', 'Code and build output', 22, 880000], ['audio', 'Audio', 19, 4300], ['other', 'Other', 62, 52000]];
const AGE = [['>5y', 'Older than 5 years', 41, 62000], ['2-5y', '2 to 5 years', 88, 240000], ['1-2y', '1 to 2 years', 74, 310000], ['6-12m', '6 to 12 months', 52, 380000], ['1-6m', '1 to 6 months', 47, 450000], ['<1m', 'Last month', 12, 270000]];
const OWNER = [['michel', 'Michel', 380, 1600000], ['system', 'SYSTEM', 34, 90000]];

/** @param {{ by: string, path?: string }} req */
export function breakdown(req) {
  const rows = { type: TYPE, age: AGE, owner: OWNER, extension: TYPE.map(([k, l, g, f]) => [k, `.${k}`, g, f]) }[req.by] ?? TYPE;
  const scale = req.path ? (find(req.path)?.size ?? 0) / (virtual().size || 1) : 1;
  return { by: req.by, rows: rows.map(([key, label, gb, files]) => ({ key, label, bytes: Math.round(gb * GB * scale), files: Math.round(files * scale) })).filter((r) => r.bytes > 0).sort((a, b) => b.bytes - a.bytes) };
}
