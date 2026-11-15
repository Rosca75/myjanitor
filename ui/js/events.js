// @ts-check
/**
 * Event bridge: subscribes to every IPC event through `api.on` and turns each
 * into a store update or a refetch. Events of an older scan generation are
 * ignored (docs/IPC.md: Generations).
 */
import { upsertJob, completedRun } from './ctl-apply.js';
import { withoutProgress } from './controller.js';
import { copy } from './copy.js';
import { formatBytes } from './format.js';

/**
 * @param {{ api: any, store: any, actions: any }} deps
 * @returns {Promise<() => void>} unsubscribe all
 */
export async function bridgeEvents({ api, store, actions }) {
  const { get, set } = store;
  const current = (gen) => gen === undefined || gen >= get().scan.generation;

  /** @type {Record<string, (p: any) => void>} */
  const handlers = {
    'scan://progress': (p) => { if (current(p.generation)) set({ scanProgress: p }); },
    'scan://detector': (p) => { if (current(p.generation)) set({ detectors: { ...get().detectors, [p.opportunity_id]: p } }); },
    'scan://done': (p) => { if (current(p.generation)) { set({ scan: withoutProgress(p), scanProgress: p.progress ?? get().scanProgress, detectors: {} }); actions.refreshOpps(); } },
    'opportunities://changed': () => { actions.refreshOpps(); },
    'plan://changed': (plan) => set({ plan }),
    'plan://dry_run': (r) => {
      set({ dryRunning: false });
      actions.notify?.(copy.plan.dryRunDone(r.total.files, formatBytes(r.total.bytes_before - r.total.bytes_after)), 'success');
    },
    'job://progress': (p) => set({ jobProgress: { ...get().jobProgress, [p.job_id]: p.progress }, jobCurrent: { ...get().jobCurrent, [p.job_id]: p.current ?? '' } }),
    'job://state': (job) => set({ jobs: upsertJob(get().jobs, job), jobProgress: { ...get().jobProgress, [job.id]: job.progress } }),
    'job://done': ({ job }) => {
      const jobs = upsertJob(get().jobs, job);
      const patch = { jobs, jobProgress: { ...get().jobProgress, [job.id]: job.progress } };
      if (job.kind === 'apply') {
        const done = completedRun(jobs, job.run_id);
        if (done) {
          patch.completed = done;
          patch.reclaimed = get().reclaimed + done.saved;
        }
      }
      if (job.kind === 'undo') patch.historyUi = { ...get().historyUi, result: { job } };
      set(patch);
    },
    'history://changed': () => { actions.refreshHistory(); actions.refreshOpenRuns?.(); },
  };

  const offs = [];
  for (const [name, fn] of Object.entries(handlers)) {
    try {
      offs.push(await api.on(name, fn)); // sequential: see main.js
    } catch (e) {
      actions.notify?.(e?.message ?? String(e), 'error');
    }
  }
  return () => { offs.splice(0).forEach((off) => { try { off(); } catch { /* already gone */ } }); };
}
