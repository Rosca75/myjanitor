// @ts-check
/**
 * Request generation counters. Every load takes a number before it awaits and
 * applies its answer only if it is still the newest one for that key, so a slow
 * answer to an old request can never overwrite a newer one.
 */
export function createGens() {
  /** @type {Record<string, number>} */
  const counters = {};
  return {
    /** Start a request for `key`; returns its generation. @param {string} key */
    next(key) {
      counters[key] = (counters[key] ?? 0) + 1;
      return counters[key];
    },
    /** Is `gen` still the newest for `key`? @param {string} key @param {number} gen */
    current: (key, gen) => counters[key] === gen,
  };
}
