/**
 * The strip's "Last report" button only needs to know whether a report exists: derive it
 * from the history list instead of fetching the whole report (IPC-8). viewReport fetches
 * the report itself on click. @param {any} history
 */
export const lastReportOf = (history) => {
  const run = [...(history?.runs ?? [])].sort((a, b) => b.run_id - a.run_id).find((r) => r.kind !== 'undo' && r.items > 0);
  return run ? { run_id: run.run_id } : null;
};

