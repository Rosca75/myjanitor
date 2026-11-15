// @ts-check
/** Reports (SAF-04) and exports (CLN-ACT-11, SAF-12). */
import { copy } from './copy.js';

/** @param {string} s */
const slug = (s) => String(s).replace(/[^A-Za-z0-9_-]+/g, '-').replace(/^-|-$/g, '');

/** Default file name of an export. @param {any} job */
export function exportName(job) {
  if (job.type === 'results') return `myjanitor-results${job.opportunity_id ? `-${slug(job.opportunity_id)}` : ''}.${job.format}`;
  if (job.type === 'audit') return `myjanitor-audit${job.run_id ? `-${job.run_id}` : ''}.csv`;
  return `myjanitor-report-${job.run_id}.${job.format}`;
}

/** @param {any} c */
export function reportActions(c) {
  const { api, set, get, guard } = c;
  return {
    /** "View report" / "Last report". @param {number} [runId] */
    viewReport: guard(async (runId) => {
      const report = runId ? await api.reportGet({ run_id: runId }) : await api.reportLast();
      if (!report) { c.notify(copy.report.none, 'info'); return; }
      set({ report, reportUi: { filter: 'all', shown: {} } });
    }),
    closeReport: () => set({ report: null }),
    /** @param {'all'|'done'|'skipped'|'failed'} filter */
    setReportFilter: (filter) => set({ reportUi: { filter, shown: {} } }),
    /** Show more items of one report section. @param {number} section */
    showMoreReport(section) {
      const ui = get().reportUi;
      set({ reportUi: { ...ui, shown: { ...ui.shown, [section]: (ui.shown[section] ?? 100) + 200 } } });
    },
    /**
     * Export: choose the target in the operating system's "Save as" dialog, then write
     * it. A cancelled dialog writes nothing and says nothing. The dialog itself asks
     * before an existing file is chosen, hence `overwrite`.
     * @param {{ type: 'report'|'audit'|'results', format: 'csv'|'html'|'pdf', run_id?: number, opportunity_id?: string, selected_only?: boolean }} job
     */
    startExport: c.feature('export', async (job) => {
      const dest = await api.dialogSaveFile({ title: copy.dialogs.saveExport[job.type], file_name: exportName(job), filters: [{ name: copy.dialogs.fileTypes[job.format], extensions: [job.format] }] });
      if (!dest) return;
      const r = job.type === 'results'
        ? await api.resultsExport({ ...(job.opportunity_id ? { opportunity_id: job.opportunity_id } : {}), selected_only: !!job.selected_only, format: job.format, dest, overwrite: true })
        : await api.reportExport({ ...(job.run_id ? { run_id: job.run_id } : {}), kind: job.type, format: job.format, dest, overwrite: true });
      c.notify(copy.report.saved(r.rows, r.path), 'success');
    }),
  };
}
