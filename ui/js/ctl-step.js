// @ts-check
/**
 * Step review actions (B2): the table, selection, presets, toggles, action
 * choice. The table operations are built by `createStepOps` over a "slot" (where
 * the step state lives), so the Custom search results reuse the very same code
 * and table component as the step review.
 */
import { toErrorInfo } from './errors.js';

export const PAGE_SIZE = 50;

/** Actions the "What happens to them" panel can pick (permanent delete lives in Advanced). */
const ACTIONS = {
  trash: () => ({ kind: 'trash' }),
  quarantine: () => ({ kind: 'quarantine', purge_after_days: 7 }),
  permanent_delete: () => ({ kind: 'permanent_delete' }),
  move: (dest) => ({ kind: 'move', dest, keep_structure: false }),
};

/** A fresh step state. @param {string} id */
export const freshStep = (id) => ({ id, detail: null, page: null, subGroup: null, offset: 0, loading: true, advancedOpen: false, pendingAction: null, preview: null, previewFor: null, sort: 'group', descending: false, filter: '', focusId: null, previewLoading: false });

/**
 * Table operations over one step slot.
 * @param {any} c controller context
 * @param {{ read: () => any, write: (step: any) => void, key: string }} slot
 */
export function createStepOps(c, slot) {
  const { api, get, guard, gens } = c;
  const cur = () => slot.read();
  const patch = (p) => slot.write({ ...cur(), ...p });
  const oppId = () => cur().id;

  /** Reload the page of rows for the current group, offset, sort and filter. */
  async function loadPage() {
    const s = cur();
    const g = gens.next(`${slot.key}:page`);
    const req = { opportunity_id: s.id, sub_group: s.subGroup, offset: s.offset, limit: PAGE_SIZE, sort: s.sort, descending: s.descending };
    if (s.filter) req.filter = s.filter;
    const page = await api.candidatesPage(req);
    if (gens.current(`${slot.key}:page`, g) && cur()?.id === s.id) patch({ page, loading: false });
  }
  async function loadDetail() {
    const id = oppId();
    const g = gens.next(`${slot.key}:detail`);
    const detail = await api.stepDetail({ opportunity_id: id });
    if (gens.current(`${slot.key}:detail`, g) && cur()?.id === id) patch({ detail });
  }
  const reload = () => Promise.all([loadDetail(), loadPage()]);

  /** Fetch a preview for one row; failures show as "no preview" with the reason. @param {any} row */
  async function loadPreview(row) {
    const g = gens.next(`${slot.key}:preview`);
    patch({ focusId: row.id, previewFor: row.path, previewLoading: true });
    let preview;
    try {
      preview = await api.candidatePreview({ candidate_id: row.id });
    } catch (e) {
      preview = { kind: 'none', reason: toErrorInfo(e).message };
    }
    if (gens.current(`${slot.key}:preview`, g)) patch({ preview, previewLoading: false });
  }

  return {
    reloadStep: guard(reload),
    /** @param {string|null} key */
    setSubGroup: guard(async (key) => { patch({ subGroup: key, offset: 0 }); await loadPage(); }),
    /** @param {number} offset */
    setPage: guard(async (offset) => { patch({ offset: Math.max(0, offset) }); await loadPage(); }),
    /** Sort by a column; the same column again reverses. @param {string} key */
    setStepSort: guard(async (key) => {
      const s = cur();
      patch({ sort: key, descending: s.sort === key ? !s.descending : false, offset: 0 });
      await loadPage();
    }),
    /** "Filter these results": case-insensitive path substring. @param {string} text */
    setFilter: guard(async (text) => { patch({ filter: text, offset: 0 }); await loadPage(); }),
    /** Check or uncheck one row. @param {number} id @param {boolean} selected */
    toggleRow: guard(async (id, selected) => {
      await api.selectionSet({ opportunity_id: oppId(), ids: [id], selected });
      await reload();
    }),
    /** "Select all" in the header: the current group, or the whole step. @param {boolean} selected */
    selectAll: guard(async (selected) => {
      const sub = cur().subGroup;
      await api.selectionSet({ opportunity_id: oppId(), ...(sub ? { sub_group: sub } : {}), selected });
      await reload();
    }),
    /** Re-apply the keep rule through the backend. @param {string} rule @param {string} [folder] */
    applyKeepRule: guard(async (rule, folder) => {
      await api.selectionApplyKeepRule({ opportunity_id: oppId(), rule: folder ? { rule, folder } : { rule } });
      await reload();
    }),
    resetSelection: guard(async () => { await api.selectionReset({ opportunity_id: oppId() }); await reload(); }),
    /** Preview one row (a click on its name), else the best sample. @param {number} [id] */
    previewSample: guard(async (id) => {
      const rows = cur().page?.rows ?? [];
      const row = rows.find((r) => r.id === id) ?? rows.find((r) => r.selected && /silence/i.test(r.reason)) ?? rows.find((r) => r.selected) ?? rows[0];
      if (row) await loadPreview(row);
    }),
    /** Keyboard focus on a row loads its preview too. @param {number} id */
    focusRow: guard(async (id) => {
      const row = (cur().page?.rows ?? []).find((r) => r.id === id);
      if (row && cur().focusId !== id) await loadPreview(row);
    }),
    toggleAdvanced: () => patch({ advancedOpen: !cur().advancedOpen }),
    patch,
    loadDetail,
  };
}

/** @param {any} c */
export function stepActions(c) {
  const { api, set, get, guard } = c;
  const ops = createStepOps(c, { key: 'step', read: () => get().step, write: (step) => set({ step }) });
  const cur = () => get().step;

  return {
    ...ops,
    /** Open the step review for an opportunity (from a card's Review button). @param {string} id */
    openStep: guard(async (id) => {
      set({ step: freshStep(id), route: { name: 'step', params: { id }, from: get().route.name === 'step' ? get().route.from : get().route } });
      await ops.reloadStep();
      if (get().step?.detail?.opportunity.category === 'optimise') await ops.previewSample();
    }),
    /** Preset tile. @param {string} presetId */
    setPreset: guard(async (presetId) => { await api.stepParamsSet({ opportunity_id: cur().id, preset_id: presetId }); await ops.reloadStep(); }),
    /** A toggle, number or choice of the options panel. @param {string} key @param {boolean|number|string} value */
    setParam: guard(async (key, value) => { await api.stepParamsSet({ opportunity_id: cur().id, params: { [key]: value } }); await ops.reloadStep(); }),
    /** Action choice of a clean step (Recycle Bin / Quarantine / Move / permanent). @param {keyof typeof ACTIONS} kind @param {string} [dest] */
    setStepAction: guard(async (kind, dest) => {
      const action = ACTIONS[kind](dest);
      const step = get().plan.steps.find((x) => x.opportunity === cur().id);
      if (step) set({ plan: await api.planUpdateStep({ step_id: step.id, action }) });
      ops.patch({ pendingAction: action });
      await ops.loadDetail();
    }),
    /** Archive format of the step (plan_update_step when it is in the plan, else kept for "Add to plan"). @param {string} format */
    setArchiveFormat: guard(async (format) => {
      const opp = cur().detail.opportunity;
      const step = get().plan.steps.find((x) => x.opportunity === opp.id);
      const base = step?.action ?? cur().pendingAction ?? opp.default_action;
      const action = { ...base, format };
      if (step) set({ plan: await api.planUpdateStep({ step_id: step.id, action }) });
      ops.patch({ pendingAction: action });
    }),
    /** "Add to plan" in the step header: honours an action chosen before adding. */
    addStepToPlan: guard(async () => {
      const action = cur().pendingAction;
      set({ plan: await api.planAddStep({ opportunity_id: cur().id, ...(action ? { action } : {}) }) });
      await ops.loadDetail();
    }),
    /** Compare before / after dialog for the focused row. */
    openCompare() {
      const s = cur();
      if (s?.focusId) set({ compare: { rowId: s.focusId } });
    },
    closeCompare: () => set({ compare: null }),
  };
}
