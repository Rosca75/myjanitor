// @ts-check
/**
 * Helpers for the quick-query box. The engine reports error positions and the
 * caret in UTF-8 bytes; the DOM works in UTF-16 units, so these convert.
 */

const enc = new TextEncoder();

/** Byte offset of a UTF-16 index. @param {string} text @param {number} index */
export const byteOffset = (text, index) => enc.encode(text.slice(0, Math.max(0, index))).length;

/** UTF-16 index of a byte offset (rounded to a character boundary). @param {string} text @param {number} bytes */
export function indexOfByte(text, bytes) {
  let b = 0;
  let i = 0;
  for (const ch of text) {
    const n = enc.encode(ch).length;
    if (b + n > bytes) break;
    b += n;
    i += ch.length;
  }
  return i;
}

/**
 * Split the query into plain and flawed segments for underlining.
 * @param {string} query
 * @param {{ offset: number, len: number, message: string }[]} errors
 * @returns {{ text: string, error: string | null }[]}
 */
export function segments(query, errors) {
  const spans = errors
    .map((e) => ({ from: indexOfByte(query, e.offset), to: indexOfByte(query, e.offset + Math.max(1, e.len)), message: e.message }))
    .sort((a, b) => a.from - b.from);
  const out = [];
  let at = 0;
  for (const s of spans) {
    if (s.from < at) continue;
    if (s.from > at) out.push({ text: query.slice(at, s.from), error: null });
    out.push({ text: query.slice(s.from, Math.max(s.to, s.from + 1)) || ' ', error: s.message });
    at = Math.max(s.to, s.from + 1);
  }
  if (at < query.length) out.push({ text: query.slice(at), error: null });
  return out;
}

/** 1-based character column of an error, for "Position 7". @param {string} query @param {{ offset: number }} e */
export const columnOf = (query, e) => indexOfByte(query, e.offset) + 1;

/**
 * Apply a Suggestion: it replaces `replace_len` bytes that end at the caret.
 * Returns the new text and the new caret (UTF-16 index).
 * @param {string} query @param {number} caret UTF-16 index
 * @param {{ insert: string, replace_len: number }} s
 */
export function applySuggestion(query, caret, s) {
  const endByte = byteOffset(query, caret);
  const start = indexOfByte(query, Math.max(0, endByte - s.replace_len));
  const text = query.slice(0, start) + s.insert + query.slice(caret);
  return { text, caret: start + s.insert.length };
}
