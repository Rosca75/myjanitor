// @ts-check
/**
 * Applying the plan: confirmation dialogs (lossy and permanent), the job queue
 * and the "completed" summary. `plan_apply` is only called after every step
 * that needs it has been acknowledged in a dialog.
 */
import { copy } from './copy.js';

const NEEDS_CONFIRMATION = ['lossy_opt_in', 'permanent'];
const ACTIVE = ['queued', 'running', 'paused', 'cancelling'];

/** @param {any} info */
const freshAnswer = (info) => ({ ack: false, originalsIndex: Math.max(0, info.originals_options.findIndex((o) => o.policy.policy === info.default_originals?.policy)), onlineOnly: false });

/** @param {any} c */
export function applyActions(c) {
  const { api, set, get, guard } = c;

  /** @param {any[]} confirmations */
  async function doApply(confirmations) {
    set({ completed: null, dialog: null });
    await api.planApply({ confirmations });
    set({ jobs: await api.jobsList() });
  }

  return {
    /** "Apply plan": ask for each lossy or permanent step, then apply. */
    startApply: guard(async () => {
      const steps = get().plan.steps.filter((s) => NEEDS_CONFIRMATION.includes(s.safety));
      if (!steps.length) return doApply([]);
      const infos = await Promise.all(steps.map((s) => api.planConfirmationInfo({ step_id: s.id })));
      set({ dialog: { infos, index: 0, answers: [], current: freshAnswer(infos[0]) } });
    }),
    /** Update the current dialog's answer (checkbox, dropdown, online-only). @param {object} patch */
    dialogSet(patch) {
      const d = get().dialog;
      if (d) set({ dialog: { ...d, current: { ...d.current, ...patch } } });
    },
    /** "Back" and Escape: close without applying anything. */
    dialogBack: () => set({ dialog: null }),
    /** The acting button: needs the acknowledgement. */
    dialogConfirm: guard(async () => {
      const d = get().dialog;
      if (!d || !d.current.ack) return;
      const info = d.infos[d.index];
      const conf = { step_id: info.step_id, acknowledged: true };
      if (info.originals_options.length) conf.originals = info.originals_options[d.current.originalsIndex].policy;
      if (info.synced) conf.synced_online_only = d.current.onlineOnly;
      const answers = [...d.answers, conf];
      if (d.index + 1 < d.infos.length) {
        set({ dialog: { ...d, index: d.index + 1, answers, current: freshAnswer(d.infos[d.index + 1]) } });
        return;
      }
      await doApply(answers);
    }),
    /** "Compare a sample first": close the dialog and open the step review with a preview. */
    compareSample: guard(async () => {
      const d = get().dialog;
      if (!d) return;
      const step = get().plan.steps.find((s) => s.id === d.infos[d.index].step_id);
      set({ dialog: null });
      if (!step) return;
      await c.actions.openStep(step.opportunity);
      await c.actions.previewSample();
    }),

    pauseJob: guard(async (id) => { await api.jobPause({ job_id: id }); }),
    resumeJob: guard(async (id) => { await api.jobResume({ job_id: id }); }),
    cancelJob: guard(async (id) => { await api.jobCancel({ job_id: id }); }),
    /** "Undo last action" (strip and completed summary). */
    undoLast: guard(async () => { set({ completed: null }); await api.undoLast(); set({ jobs: await api.jobsList() }); }),
    dismissCompleted: () => set({ completed: null }),
    /** Move a queued job up (-1) or down (+1) among the queued ones. @param {number} id @param {number} delta */
    moveJob: guard(async (id, delta) => {
      const queued = get().jobs.filter((j) => j.state === 'queued').map((j) => j.id);
      const i = queued.indexOf(id);
      const j = i + delta;
      if (i < 0 || j < 0 || j >= queued.length) return;
      [queued[i], queued[j]] = [queued[j], queued[i]];
      await api.jobsReorder({ job_ids: queued });
      set({ jobs: await api.jobsList() });
    }),
    /** Show or hide the job queue when nothing is running. */
    toggleQueue: () => set({ queueOpen: !get().queueOpen }),
  };
}

/** Insert or replace a job by id. @param {any[]} jobs @param {any} job */
export function upsertJob(jobs, job) {
  const i = jobs.findIndex((j) => j.id === job.id);
  if (i < 0) return [...jobs, job];
  const next = [...jobs];
  next[i] = { ...jobs[i], ...job };
  return next;
}

/** Summary of an apply run once none of its jobs is still active, else null. @param {any[]} jobs @param {number} runId */
export function completedRun(jobs, runId) {
  const mine = jobs.filter((j) => j.run_id === runId && j.kind === 'apply');
  if (!mine.length || mine.some((j) => ACTIVE.includes(j.state))) return null;
  const sum = (f) => mine.reduce((a, j) => a + f(j), 0);
  return { run_id: runId, saved: sum((j) => j.outcome?.bytes_saved ?? 0), files: sum((j) => j.outcome?.succeeded ?? 0), failed: sum((j) => (j.outcome?.failed ?? 0) + (j.state === 'failed' ? 1 : 0)), cancelled: mine.filter((j) => j.state === 'cancelled').length };
}
void copy;
