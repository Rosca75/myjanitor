// @ts-check
/** Step review actions (B2): the table, selection, presets, toggles, action choice. */

export const PAGE_SIZE = 50;

/** Actions the "What happens to them" panel can pick (permanent delete lives in Advanced). */
const ACTIONS = {
  trash: () => ({ kind: 'trash' }),
  quarantine: () => ({ kind: 'quarantine', purge_after_days: 7 }),
  permanent_delete: () => ({ kind: 'permanent_delete' }),
  move: (dest) => ({ kind: 'move', dest, keep_structure: false }),
};

/** @param {any} c */
export function stepActions(c) {
  const { api, set, get, guard } = c;
  const cur = () => get().step;
  const oppId = () => cur().id;

  /** Reload the page of rows for the current group, offset and size. */
  async function loadPage() {
    const s = cur();
    const page = await api.candidatesPage({ opportunity_id: s.id, sub_group: s.subGroup, offset: s.offset, limit: PAGE_SIZE, sort: 'group', descending: false });
    set({ step: { ...cur(), page, loading: false } });
  }

  async function loadDetail() {
    const detail = await api.stepDetail({ opportunity_id: oppId() });
    set({ step: { ...cur(), detail } });
  }

  async function reload() {
    await Promise.all([loadDetail(), loadPage()]);
  }

  return {
    /** Open the step review for an opportunity (from a card's Review button). @param {string} id */
    openStep: guard(async (id) => {
      set({ step: { id, detail: null, page: null, subGroup: null, offset: 0, loading: true, advancedOpen: false, pendingAction: null, preview: null }, route: { name: 'step', params: { id }, from: get().route.name === 'step' ? get().route.from : get().route } });
      await reload();
      if (get().step?.detail?.opportunity.category === 'optimise') await c.actions.previewSample();
    }),
    /** @param {string|null} key */
    setSubGroup: guard(async (key) => { set({ step: { ...cur(), subGroup: key, offset: 0 } }); await loadPage(); }),
    /** @param {number} offset */
    setPage: guard(async (offset) => { set({ step: { ...cur(), offset: Math.max(0, offset) } }); await loadPage(); }),
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
    /** @param {string} rule newest | oldest | shortest_path | highest_resolution */
    applyKeepRule: guard(async (rule) => { await api.selectionApplyKeepRule({ opportunity_id: oppId(), rule: { rule } }); await reload(); }),
    resetSelection: guard(async () => { await api.selectionReset({ opportunity_id: oppId() }); await reload(); }),

    /** Preset tile. @param {string} presetId */
    setPreset: guard(async (presetId) => { await api.stepParamsSet({ opportunity_id: oppId(), preset_id: presetId }); await reload(); }),
    /** A toggle or number of "How to shrink them". @param {string} key @param {boolean|number} value */
    setParam: guard(async (key, value) => { await api.stepParamsSet({ opportunity_id: oppId(), params: { [key]: value } }); await reload(); }),
    toggleAdvanced: () => set({ step: { ...cur(), advancedOpen: !cur().advancedOpen } }),

    /** Action choice of a clean step (Recycle Bin / Quarantine / Move / permanent). @param {keyof typeof ACTIONS} kind @param {string} [dest] */
    setStepAction: guard(async (kind, dest) => {
      const action = ACTIONS[kind](dest);
      const step = get().plan.steps.find((x) => x.opportunity === oppId());
      if (step) set({ plan: await api.planUpdateStep({ step_id: step.id, action }) });
      set({ step: { ...cur(), pendingAction: action } });
      await loadDetail();
    }),
    /** "Add to plan" in the step header: honours an action chosen before adding. */
    addStepToPlan: guard(async () => {
      const action = cur().pendingAction;
      set({ plan: await api.planAddStep({ opportunity_id: oppId(), ...(action ? { action } : {}) }) });
      await loadDetail();
    }),
    /** Listen / Compare: fetch a preview of the first selected row. */
    /** Preview one row (a click on its name), else the best sample. @param {number} [id] */
    previewSample: guard(async (id) => {
      const rows = cur().page?.rows ?? [];
      const row = rows.find((r) => r.id === id) ?? rows.find((r) => r.selected && /silence/i.test(r.reason)) ?? rows.find((r) => r.selected) ?? rows[0];
      if (!row) return;
      set({ step: { ...cur(), preview: await api.candidatePreview({ candidate_id: row.id }), previewFor: row.path } });
    }),
  };
}
