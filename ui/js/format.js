// @ts-check
/** Pure formatters (sizes, counts, times). Sizes follow D-107 / mj_core::size. */

const UNITS = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];

/**
 * Binary units with Explorer labels: `0 B`, `1.5 KB`, `48.6 GB`, `354 GB`.
 * One decimal below 100 in the unit, none from 100 up.
 * @param {number} bytes
 */
export function formatBytes(bytes) {
  const n = Math.max(0, Number(bytes) || 0);
  if (n < 1024) return `${Math.round(n)} B`;
  let value = n;
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  let rounded = Math.round(value * 10) / 10;
  if (rounded >= 1024 && unit < UNITS.length - 1) {
    rounded /= 1024;
    unit += 1;
  }
  return rounded >= 100 ? `${Math.round(rounded)} ${UNITS[unit]}` : `${rounded.toFixed(1)} ${UNITS[unit]}`;
}

/**
 * Estimates read in whole gigabytes from 100 MB up ("0.6 GB", as in the
 * wireframe); below that, the normal formatter.
 * @param {number} bytes
 */
export function formatAbout(bytes) {
  const n = Number(bytes) || 0;
  return n >= 100 * 1024 ** 2 && n < 1024 ** 3 ? `${(n / 1024 ** 3).toFixed(1)} GB` : formatBytes(n);
}

/** `~0.6 GB`. @param {number} bytes */
export const formatApprox = (bytes) => `~${formatAbout(bytes)}`;

/** Thousands separators. @param {number} n */
export function formatCount(n) {
  return Math.round(Number(n) || 0).toLocaleString('en-US');
}

/**
 * Relative time from a nanosecond timestamp: "just now", "2 min ago",
 * "3 hours ago", "yesterday", "5 days ago", else a date.
 * @param {number} tsNs
 * @param {number} [nowMs]
 */
export function formatAgo(tsNs, nowMs = Date.now()) {
  const diff = Math.max(0, nowMs - tsNs / 1e6);
  const min = Math.floor(diff / 60000);
  if (min < 1) return 'just now';
  if (min < 60) return `${min} min ago`;
  const hours = Math.floor(min / 60);
  if (hours < 24) return hours === 1 ? '1 hour ago' : `${hours} hours ago`;
  const days = Math.floor(hours / 24);
  if (days === 1) return 'yesterday';
  if (days < 30) return `${days} days ago`;
  return formatDate(tsNs);
}

/** `13 Nov 2026`. @param {number} tsNs */
export function formatDate(tsNs) {
  const d = new Date(tsNs / 1e6);
  const months = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
  return `${d.getUTCDate()} ${months[d.getUTCMonth()]} ${d.getUTCFullYear()}`;
}

/** `1:02:14` or `4:08` from seconds. @param {number} seconds */
export function formatClock(seconds) {
  const s = Math.max(0, Math.round(seconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = String(s % 60).padStart(2, '0');
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${sec}` : `${m}:${sec}`;
}

/** @param {number} n @param {string} one @param {string} [many] */
export const plural = (n, one, many = `${one}s`) => (n === 1 ? one : many);

/** Last path segment for display; both separators. @param {string} path */
export function basename(path) {
  const parts = String(path).split(/[\\/]/).filter(Boolean);
  return parts.length ? parts[parts.length - 1] : String(path);
}

/** Parent folder for display. @param {string} path */
export function dirname(path) {
  const p = String(path);
  const i = Math.max(p.lastIndexOf('\\'), p.lastIndexOf('/'));
  return i > 0 ? p.slice(0, i) : '';
}

/** Show a non-Unicode path ("\u0000" + escaped) as best-effort text. @param {string} path */
export function displayPath(path) {
  const p = String(path);
  return p.startsWith('\u0000') ? p.slice(1).replace(/%u([0-9A-Fa-f]{4})/g, '�').replace(/%([0-9A-Fa-f]{2})/g, '�') : p;
}

/** Percentage of `part` in `whole`, clamped to 0..100. @param {number} part @param {number} whole */
export function pct(part, whole) {
  if (!whole || whole <= 0) return 0;
  return Math.min(100, Math.max(0, (part / whole) * 100));
}

/**
 * Shorten a long text in the middle ("C:\Users\Mic…\report.pdf"), keeping a
 * little more of the end because that is where the file name is. Callers put
 * the full text in a `title` attribute. Texts up to `max` characters are
 * returned unchanged.
 * @param {string} text
 * @param {number} [max]
 */
export function middleEllipsis(text, max = 48) {
  const t = String(text ?? '');
  const limit = Math.max(5, Math.floor(max));
  if (t.length <= limit) return t;
  const keep = limit - 1;
  const tail = Math.ceil(keep * 0.55);
  return `${t.slice(0, keep - tail)}…${t.slice(t.length - tail)}`;
}

/** `13 Nov 2026, 14:05` (UTC). @param {number} tsNs */
export function formatDateTime(tsNs) {
  const d = new Date(tsNs / 1e6);
  const p = (n) => String(n).padStart(2, '0');
  return `${formatDate(tsNs)}, ${p(d.getUTCHours())}:${p(d.getUTCMinutes())}`;
}

/** Whole percent, or one decimal below 10 (`0.4 %`, `37 %`). @param {number} fraction 0..1 */
export function formatPercent(fraction) {
  const p = Math.max(0, Math.min(1, Number(fraction) || 0)) * 100;
  return `${p < 10 && p > 0 ? p.toFixed(1) : Math.round(p)} %`;
}

/**
 * Join a folder and a new file name for display or for an export target. The
 * separator follows the platform; only used for names the user typed, never
 * for paths the engine handed out.
 * @param {string} dir @param {string} name @param {string} [platform]
 */
export function joinPath(dir, name, platform = 'windows') {
  const sep = platform === 'windows' || /^[A-Za-z]:/.test(dir) ? '\\' : '/';
  const base = String(dir).replace(/[\\/]+$/, '');
  return base === '' ? `${sep}${name}` : /^[A-Za-z]:$/.test(base) ? `${base}${sep}${name}` : `${base}${sep}${name}`;
}

/** Compression saved by the file system: 0..1, or null when nothing is compressed. @param {number} size @param {number} allocated */
export function compressionRate(size, allocated) {
  if (!size || !allocated || allocated >= size) return null;
  return 1 - allocated / size;
}
