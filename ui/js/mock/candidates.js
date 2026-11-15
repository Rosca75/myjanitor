// @ts-check
/** Deterministic candidate rows for every opportunity (the step-review tables). */
import { GB, MB } from './fixtures.js';

/** Seeded PRNG so the fixtures never change between runs. @param {number} seed */
function rng(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const HOME = 'C:\\Users\\Michel';
const T0 = 1_792_000_000_000_000_000; // ns, an arbitrary recent time
const DAY = 86_400_000_000_000;

/** @param {any} row */
function stamp(row) {
  return { size: row.size, mtime: row.modified };
}

/**
 * Meeting recordings: the nine rows of the wireframe, then deterministic ones.
 * [name, source, length, MB, afterMB, reason, badges, sub_group, selected]
 */
const AUDIO_HEAD = [
  ['Weekly sync – Nordbank.mp4', 'Teams', '1:02:14', 412, 31, 'same meeting ×2', [], 'same_meeting', true],
  ['Weekly sync – Nordbank (1).m4a', 'Zoom', '1:02:09', 118, 0, 'duplicate', ['duplicate'], 'same_meeting', true],
  ['Steering committee 14 Oct.wav', 'Local', '1:47:30', 1126.4, 52, 'silence 14 min', [], null, true],
  ['Interview – CFO Acme.m4a', 'Local', '0:48:02', 88, 9, 'needs transcript', [], 'needs_transcript', true],
  ['Workshop day 1 – recording.mp4', 'Teams', '3:12:44', 1433.6, 96, 'screen + voice', [], null, true],
  ['Expert call – Supply chain.mp3', 'Zoom', '0:55:10', 76, 10, '', [], null, true],
  ['Board session – privileged.m4a', 'Local', '2:01:00', 210, null, 'confidential', ['confidential'], 'skipped', false],
  ['Kick-off – Project Falcon.wav', 'Local', '1:10:20', 712, 34, 'silence 6 min', [], null, true],
  ['Daily standup 2026-09-30.mp4', 'Teams', '0:16:08', 96, 4, '', [], null, true],
];
const TAIL_MB = [120, 95, 80, 75, 70, 62, 58, 55, 50, 45, 40, 35, 30, 22, 16];
const TAIL_GROUP = ['same_meeting', 'same_meeting', 'same_meeting', 'same_meeting', 'needs_transcript', 'needs_transcript', 'needs_transcript', 'skipped'];

function audioRows() {
  const rows = [];
  let id = 1041;
  const push = (name, source, length, mb, afterMb, reason, badges, sub, selected, i) => {
    const size = Math.round(mb * MB);
    const row = {
      id: id++,
      path: `${HOME}\\Recordings\\${name}`,
      target: 'file',
      source: { optimiser: 'audio' },
      size,
      size_after: afterMb === null ? size : Math.round(afterMb * MB),
      base_after: afterMb === null ? size : Math.round(afterMb * MB),
      confidence: 'estimated',
      reason,
      badges,
      default_selected: selected,
      modified: T0 - (i + 3) * DAY,
      columns: { source, length },
    };
    if (sub) row.sub_group = sub;
    row.stamp = stamp(row);
    rows.push(row);
  };
  AUDIO_HEAD.forEach((r, i) => push(...r, i));
  TAIL_MB.forEach((mb, k) => {
    const day = 1 + k;
    const sub = TAIL_GROUP[k] ?? null;
    const reasons = { same_meeting: 'same meeting ×2', needs_transcript: 'needs transcript', skipped: 'in use by another program' };
    push(`Team call 2026-09-${String(day).padStart(2, '0')}.${k % 2 ? 'm4a' : 'mp4'}`, k % 3 ? 'Teams' : 'Local', `0:${20 + k}:00`, mb, Math.round(mb * 0.48), sub ? reasons[sub] : '', sub === 'skipped' ? ['in_use'] : [], sub, true, 9 + k);
  });
  return rows;
}

const DUP_NAMES = ['IMG_0412.JPG', 'Report_Q3.pdf', 'Budget 2026.xlsx', 'Holiday.mp4', 'scan_0021.png', 'Invoice_8841.pdf', 'Photo 2024-05-02.jpg', 'Contract_draft.docx', 'backup.zip', 'logo_final.png'];
const DUP_FOLDERS = ['Photos', 'Downloads', 'Desktop', 'Documents\\Backup'];

/** 61 groups: 31 of four files and 30 of three = 214 rows, 153 removable. */
function dupRows() {
  const rand = rng(7);
  const rows = [];
  const groups = [];
  let id = 1;
  for (let g = 1; g <= 61; g++) {
    const members = g <= 31 ? 4 : 3;
    const base = DUP_NAMES[g % DUP_NAMES.length];
    const name = g > DUP_NAMES.length ? base.replace(/(\.[^.]+)$/, ` (${g})$1`) : base;
    const size = Math.round((0.3 + rand() * 5) * MB * (g % 7 === 0 ? 40 : 1));
    const paths = [];
    for (let m = 0; m < members; m++) {
      const folder = DUP_FOLDERS[(g + m) % DUP_FOLDERS.length];
      const row = {
        id: id++,
        path: `${HOME}\\${folder}\\${name}`,
        target: 'file',
        source: { detector: 'exact_duplicates' },
        size,
        size_after: 0,
        confidence: 'exact',
        reason: m === 0 ? 'Kept: newest copy' : `Same content as ${members - 1} other files`,
        badges: m === 0 ? ['duplicate', 'keeper'] : ['duplicate'],
        default_selected: m !== 0,
        group: g,
        modified: T0 - (g * 5 + m * 40) * DAY,
      };
      row.stamp = stamp(row);
      paths.push(row.path);
      rows.push(row);
    }
    groups.push({ id: g, kind: 'exact', label: `${members} copies of ${name}`, confidence: 100, keeper: paths[0], members: paths });
  }
  return { rows, groups };
}

const APPS = ['Chrome', 'Edge', 'Firefox', 'Teams', 'Slack', 'Zoom', 'Spotify', 'VS Code', 'Discord', 'Notion', 'OneNote', 'Outlook'];

/** @param {string} id @param {number} n */
function nameFor(id, n, i) {
  switch (id) {
    case 'det:dev_artefacts': {
      const kind = i < 14 ? 'node_modules' : i < 17 ? '.venv' : 'target';
      return { path: `D:\\Projects\\${['app', 'website', 'notes', 'api', 'tools', 'lab', 'demo'][i % 7]}-${i + 1}\\${kind}`, dir: true, reason: kind === 'node_modules' ? 'Rebuild with npm install' : kind === '.venv' ? 'Rebuild with pip install' : 'Rebuild with cargo build' };
    }
    case 'det:app_caches':
      return { path: `${HOME}\\AppData\\Local\\${APPS[i % APPS.length]}${i >= APPS.length ? ` ${Math.floor(i / APPS.length) + 1}` : ''}\\Cache`, dir: true, reason: 'The app recreates this' };
    case 'det:old_installers':
      return { path: `${HOME}\\Downloads\\Setup_${['Office', 'Acrobat', 'Teams', 'Zoom', 'VLC', 'Git'][i % 6]}_${2200 + i}.${['msi', 'exe', 'iso'][i % 3]}`, reason: 'Older than 6 months' };
    case 'opt:images':
      return { path: `${HOME}\\Pictures\\Screenshots\\Screenshot ${2026}-0${1 + (i % 9)}-${String(1 + (i % 28)).padStart(2, '0')} (${i}).png`, reason: 'Lossless recompression' };
    case 'opt:office':
      return { path: `${HOME}\\Documents\\Clients\\Deck_${i + 1}.${i % 2 ? 'pptx' : 'xlsx'}`, reason: 'Embedded media can shrink' };
    case 'opt:pdf':
      return { path: `${HOME}\\Documents\\Scans\\Scan_${i + 1}.pdf`, reason: 'Structure clean-up' };
    case 'opt:video':
      return { path: `${HOME}\\Videos\\Screen Recording 2026-0${1 + (i % 9)}-${String(1 + (i % 28)).padStart(2, '0')}.mp4`, reason: 'Low motion, text stays sharp' };
    case 'opt:fs_compress':
      return { path: `D:\\Projects\\cold-${i + 1}`, dir: true, reason: 'Not opened for 12 months' };
    case 'det:inactive_engagements':
      return { path: `D:\\Clients\\${['Nordbank', 'Acme', 'Falcon', 'Helvetia', 'Orion', 'Baltic', 'Kestrel'][i % 7]}`, dir: true, reason: 'Idle for 18+ months' };
    case 'det:temp_files':
      return { path: `${HOME}\\AppData\\Local\\Temp\\~tmp${1000 + i}.tmp`, reason: 'Leftover temporary file' };
    default:
      return { path: `${HOME}\\Documents\\item-${i + 1}.dat`, reason: 'Found by the scan', dir: /thumbnail/.test(id) };
  }
}

/**
 * Scale the selected rows so they add up to `target` bytes.
 * @param {any[]} rows @param {number} target
 */
function scaleSelected(rows, target) {
  const sel = rows.filter((r) => r.default_selected);
  const sum = sel.reduce((a, r) => a + r.size, 0) || 1;
  const k = target / sum;
  for (const r of rows) {
    r.size = Math.max(1, Math.round(r.size * k));
    r.stamp = stamp(r);
  }
}

/**
 * Build the candidates for every opportunity.
 * @param {any[]} opps
 * @returns {{ rows: Map<string, any[]>, groups: Map<string, any[]> }}
 */
export function buildCandidates(opps) {
  /** @type {Map<string, any[]>} */
  const rows = new Map();
  /** @type {Map<string, any[]>} */
  const groups = new Map();
  for (const opp of opps) {
    if (opp.id === 'opt:audio') {
      rows.set(opp.id, audioRows());
      continue;
    }
    if (opp.id === 'det:exact_duplicates') {
      const d = dupRows();
      scaleSelected(d.rows, opp.size_before);
      rows.set(opp.id, d.rows);
      groups.set(opp.id, d.groups);
      continue;
    }
    const rand = rng(opp.id.length * 131 + opp.count);
    const list = [];
    const review = opp.safety === 'review_only';
    for (let i = 0; i < opp.count; i++) {
      const nm = nameFor(opp.id, opp.count, i);
      const row = {
        id: 100_000 + rows.size * 10_000 + i,
        path: nm.path,
        target: nm.dir ? 'dir' : 'file',
        source: opp.source,
        size: Math.round((0.2 + rand()) * MB),
        size_after: 0,
        confidence: opp.confidence,
        reason: nm.reason,
        badges: opp.id === 'det:dev_artefacts' ? ['rebuildable'] : [],
        default_selected: !review,
        modified: T0 - (30 + i * 9) * DAY,
      };
      list.push(row);
    }
    scaleSelected(list, opp.size_before);
    const keep = opp.size_before === 0 ? 0 : (opp.saving / opp.size_before);
    for (const r of list) {
      r.size_after = opp.category === 'optimise' || opp.category === 'archive' ? Math.round(r.size * (1 - keep)) : opp.id === 'det:zero_byte' ? 0 : 0;
      r.stamp = stamp(r);
    }
    rows.set(opp.id, list);
  }
  void GB;
  return { rows, groups };
}
