// @ts-check
/**
 * Row thumbnails of duplicates-type steps (W6-A). The engine makes them
 * (`candidate_thumbnails`, bounded and cached); this loader asks for the rows of
 * the visible page only, a few rows per call with at most two calls in flight,
 * drops answers that arrive after the page changed, keeps a small cache keyed by
 * path + size + modification time, and patches the one cell that is waiting
 * (`[data-thumb]`) instead of drawing the table again. One request per
 * exact-duplicate group: every row of the group shows the same picture.
 */
import { h, icon } from './dom.js';
import { copy } from './copy.js';
import { basename, displayPath } from './format.js';

/** Extensions the engine makes thumbnails for. */
const IMAGE = /\.(jpe?g|jpe|jfif|png|gif|webp|bmp)$/i;
const VIDEO = /\.(mp4|m4v|mov|avi|mkv|wmv|webm|mpe?g|3gp)$/i;
const AUDIO = /\.(mp3|wav|m4a|flac|ogg|opus|aac|wma)$/i;
const DOC = /\.(pdf|docx?|xlsx?|pptx?|odt|ods|odp|rtf|txt|md|csv)$/i;
const ARCHIVE = /\.(zip|7z|rar|tar|gz|tgz|bz2|xz|iso|cab)$/i;
/** Only data URLs of the image types the engine sends are ever put in a `src`. */
const SAFE_URL = /^data:image\/(png|jpeg|gif|webp|bmp);base64,[A-Za-z0-9+/]+=*$/;

export const BATCH = 10;
export const IN_FLIGHT = 2;
const CACHE_MAX = 1000;

/** @param {any} row */
export const thumbKey = (row) => `${row.path}|${row.size}|${row.modified ?? ''}`;

/** File-type icon name for a row without a thumbnail. @param {any} row */
export function typeIcon(row) {
  if (row.target === 'dir') return 'folder';
  const p = String(row.path);
  if (IMAGE.test(p)) return 'image';
  if (VIDEO.test(p)) return 'video';
  if (AUDIO.test(p)) return 'audio';
  if (DOC.test(p)) return 'doc';
  if (ARCHIVE.test(p)) return 'archive';
  return 'file';
}

/** True when the engine may have a thumbnail for this row. @param {any} row */
export const wantsThumb = (row) => row.target !== 'dir' && IMAGE.test(String(row.path)) && !(row.badges ?? []).includes('online_only');

/** The image (or the type icon) inside a thumbnail cell. @param {any} row @param {string|null|undefined} url */
export function thumbContent(row, url) {
  if (url && SAFE_URL.test(url)) {
    return h('img', { class: 'thumb__img', src: url, alt: copy.preview.imageAlt(displayPath(basename(row.path))), decoding: 'async', draggable: 'false' });
  }
  return h('span', { class: 'thumb__icon', role: 'img', 'aria-label': copy.thumbs.typeIcon(typeIcon(row)) }, icon(/** @type {any} */ (typeIcon(row)), 24));
}

/**
 * @param {{ candidateThumbnails: (req: any) => Promise<any> }} api
 * @param {{ doc?: () => any }} [o] where to find the cells (the document by default)
 */
export function createThumbLoader(api, o = {}) {
  /** @type {Map<string, string|null>} */
  const cache = new Map();
  let token = 0;
  const remember = (key, url) => {
    cache.delete(key);
    cache.set(key, url);
    if (cache.size > CACHE_MAX) cache.delete(cache.keys().next().value);
  };
  /** @param {any} row @param {string|null} url */
  function patch(row, url) {
    const doc = o.doc ? o.doc() : globalThis.document;
    for (const cell of doc?.querySelectorAll?.(`[data-thumb="${Number(row.id)}"]`) ?? []) {
      cell.replaceChildren(thumbContent(row, url));
      cell.setAttribute('data-state', url ? 'image' : 'icon');
    }
  }
  return {
    /** Cached URL (string), known to have none (null), or not asked yet (undefined). @param {any} row */
    get(row) {
      if (!wantsThumb(row)) return null;
      return cache.get(thumbKey(row));
    },
    /**
     * Ask for the thumbnails of a page. A later call makes the answers of this one stale.
     * @param {string} opportunityId @param {any[]} rows @param {any[]} [groups]
     */
    async load(opportunityId, rows, groups = []) {
      const mine = ++token;
      const exact = new Set(groups.filter((g) => g.kind === 'exact').map((g) => g.id));
      /** @type {Map<number, any[]>} rows waiting on each requested id */
      const waiting = new Map();
      const leaders = new Map();
      for (const r of rows) {
        if (!wantsThumb(r) || cache.has(thumbKey(r))) continue;
        const lead = r.group !== undefined && exact.has(r.group) ? leaders.get(r.group) : undefined;
        if (lead !== undefined) { waiting.get(lead)?.push(r); continue; }
        if (r.group !== undefined && exact.has(r.group)) leaders.set(r.group, r.id);
        waiting.set(r.id, [r]);
      }
      const ids = [...waiting.keys()];
      const batches = [];
      for (let i = 0; i < ids.length; i += BATCH) batches.push(ids.slice(i, i + BATCH));
      const worker = async () => {
        for (let b = batches.shift(); b; b = batches.shift()) {
          if (mine !== token) return;
          let res;
          try {
            res = await api.candidateThumbnails({ opportunity_id: opportunityId, candidate_ids: b });
          } catch {
            res = { thumbnails: b.map((id) => ({ candidate_id: id, reason: 'unreadable' })) };
          }
          if (mine !== token) return; // the page changed: drop the answer
          for (const t of res?.thumbnails ?? []) {
            const url = typeof t.data_url === 'string' && SAFE_URL.test(t.data_url) ? t.data_url : null;
            for (const r of waiting.get(t.candidate_id) ?? []) {
              remember(thumbKey(r), url);
              patch(r, url);
            }
          }
        }
      };
      await Promise.all(Array.from({ length: Math.min(IN_FLIGHT, batches.length) }, worker));
    },
    /** Forget pending answers (another step was opened). */
    cancel() { token++; },
  };
}
