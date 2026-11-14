// @ts-check
/** Reports (SAF-04), exports (CLN-ACT-11, SAF-12) and the export destination flow. */
import { joinPath } from './format.js';
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
     * Start an export: choose the folder (and file name) in the in-app browser.
     * @param {{ type: 'report'|'audit'|'results', format: 'csv'|'html', run_id?: number, opportunity_id?: string, selected_only?: boolean }} job
     */
    startExport: guard(async (job) => { await c.actions.openBrowser({ mode: 'export', exportJob: job, filename: exportName(job) }); }),
    /** @param {string} filename */
    setExportName(filename) {
      const b = get().browser;
      if (b) set({ browser: { ...b, filename } });
    },
    /** "Save here" in the browser: write the export. */
    finishExport: c.feature('export', async () => {
      const b = get().browser;
      const name = String(b?.filename ?? '').trim();
      if (!b?.path || !name || /[\\/]/.test(name)) { c.fail({ message: copy.report.badName, hint: copy.report.badNameHint }); return; }
      const job = b.exportJob;
      const dest = joinPath(b.path, name, get().info?.platform);
      set({ browser: null });
      const r = job.type === 'results'
        ? await api.resultsExport({ ...(job.opportunity_id ? { opportunity_id: job.opportunity_id } : {}), selected_only: !!job.selected_only, format: job.format, dest })
        : await api.reportExport({ ...(job.run_id ? { run_id: job.run_id } : {}), kind: job.type, format: job.format, dest });
      c.notify(copy.report.saved(r.rows, r.path), 'success');
    }),
  };
}
