// @ts-check
/** Helpers for the backend's ErrorInfo `{ code, message, hint, path? }`. */

/** Normalise anything thrown into an ErrorInfo. @param {any} e */
export function toErrorInfo(e) {
  if (e && typeof e === 'object' && typeof e.message === 'string') return { code: String(e.code ?? 'internal'), message: e.message, hint: String(e.hint ?? ''), ...(e.path ? { path: e.path } : {}) };
  return { code: 'internal', message: typeof e === 'string' ? e : 'Something went wrong', hint: '' };
}

/** `not_implemented` and `unsupported`: show the feature disabled, not as a failure. @param {any} e */
export const isUnavailable = (e) => ['not_implemented', 'unsupported'].includes(e?.code);

/**
 * The engine's message often ends with `: <path>`; where the path is already
 * shown next to it, drop that tail so the reason reads once.
 * @param {{ message?: string, hint?: string }} e @param {string} path
 */
export function reasonText(e, path) {
  const m = String(e.message ?? '');
  const short = path && m.endsWith(`: ${path}`) ? m.slice(0, -(path.length + 2)) : m;
  return [short, e.hint].filter(Boolean).join('. ').replace(/\.\./g, '.');
}
