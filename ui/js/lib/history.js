// @ts-check
/** Pure helpers for the History view: run status and entry actions. */

/**
 * One word for the state of a run, from its counters (docs/IPC.md HistoryRun).
 * @param {any} run
 * @returns {'undo'|'failed'|'restored'|'partly_restored'|'with_errors'|'undoable'|'expired'}
 */
export function runStatus(run) {
  if (run.kind === 'undo') return 'undo';
  if (run.done === 0 && run.failed > 0) return 'failed';
  if (run.done > 0 && run.undone >= run.done) return 'restored';
  if (run.undone > 0) return 'partly_restored';
  if (run.undoable) return run.failed > 0 ? 'with_errors' : 'undoable';
  return run.failed > 0 ? 'with_errors' : 'expired';
}

/**
 * Can this entry be restored right now? Done, reversible and inside its undo
 * window (an entry without a deadline is taken as open).
 * @param {any} entry @param {number} [nowNs]
 */
export function canRestore(entry, nowNs = Date.now() * 1e6) {
  return entry.state === 'done' && !!entry.reversible && (entry.undo_deadline === undefined || entry.undo_deadline > nowNs);
}

/** Entries whose earlier restore was refused (the original place was in use, ...). @param {any[]} entries */
export const refused = (entries) => entries.filter((e) => e.state === 'undo_failed' && e.error);
