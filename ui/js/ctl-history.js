// @ts-check
/** History: the journal of runs, expandable entries, restore, undo and purge. */

/** @param {any} c */
export function historyActions(c) {
  const { api, set, get, gens, guard } = c;
  const soft = (p, fb) => p.catch(() => fb);
  const ui = () => get().historyUi;
  const setUi = (p) => set({ historyUi: { ...get().historyUi, ...p } });
  const setDetail = (runId, p) => setUi({ details: { ...ui().details, [runId]: { ...ui().details[runId], ...p } } });

  async function loadDetail(runId, offset = 0) {
    const key = `detail:${runId}`;
    const g = gens.next(key);
    setDetail(runId, { loading: true, error: null, offset });
    try {
      const r = await api.historyRunDetail({ run_id: runId, offset, limit: 100 });
      if (gens.current(key, g)) setDetail(runId, { loading: false, entries: r.entries, total: r.total, offset });
    } catch (e) {
      if (gens.current(key, g)) setDetail(runId, { loading: false, error: e, entries: [], total: 0 });
    }
  }

  async function loadHistory() {
    const g = gens.next('history');
    setUi({ loading: true, error: null });
    try {
      const [history, lastReport, quarantine, settings] = await Promise.all([
        api.historyList({ offset: 0, limit: 50 }), soft(api.reportLast(), null),
        api.quarantineInfo().catch((e) => { if (['not_implemented', 'unsupported'].includes(e?.code)) c.markUnavailable('quarantine', e); return null; }),
        soft(api.settingsGet(), null),
      ]);
      if (!gens.current('history', g)) return;
      set({ history, lastReport, undoable: history.runs.some((r) => r.undoable), quarantine });
      setUi({ loading: false, undoDays: settings?.general?.undo_days ?? null });
      for (const id of Object.keys(ui().expanded)) if (ui().expanded[id]) await loadDetail(Number(id), ui().details[id]?.offset ?? 0);
    } catch (e) {
      if (gens.current('history', g)) setUi({ loading: false, error: e });
    }
  }

  return {
    loadHistory,
    /** Show more runs below the list. */
    loadMoreRuns: guard(async () => {
      const h = get().history;
      setUi({ loadingMore: true });
      try {
        const more = await api.historyList({ offset: h.runs.length, limit: 50 });
        set({ history: { total: more.total, runs: [...h.runs, ...more.runs] } });
      } finally { setUi({ loadingMore: false }); }
    }),
    /** Expand or collapse one run; its entries load on first open. @param {number} runId */
    toggleRun: guard(async (runId) => {
      const open = !ui().expanded[runId];
      setUi({ expanded: { ...ui().expanded, [runId]: open } });
      if (open && !ui().details[runId]?.entries) await loadDetail(runId, 0);
    }),
    /** @param {number} runId @param {number} offset */
    setRunPage: guard(async (runId, offset) => { await loadDetail(runId, Math.max(0, offset)); }),
    /** @param {number} id @param {boolean} on */
    selectEntry: (id, on) => setUi({ selected: { ...ui().selected, [id]: on || undefined } }),
    /** Select or clear every restorable entry of a run's page. @param {number} runId @param {boolean} on @param {(e: any) => boolean} restorable */
    selectAllEntries(runId, on, restorable) {
      const next = { ...ui().selected };
      for (const e of ui().details[runId]?.entries ?? []) if (restorable(e)) next[e.id] = on || undefined;
      setUi({ selected: next });
    },
    /** Restore chosen entries. @param {number[]} ids */
    restoreEntries: c.feature('restore_entries', async (ids) => {
      if (!ids.length) return;
      setUi({ result: null });
      await api.restoreEntries({ entry_ids: ids });
      setUi({ selected: {} });
      set({ jobs: await api.jobsList() });
    }),
    /** Ask before undoing a whole run (items whose place is taken are skipped and listed). @param {number} runId */
    askUndoRun: guard(async (runId) => {
      if (!ui().details[runId]?.entries) await loadDetail(runId, 0);
      set({ ask: { kind: 'undo_run', runId } });
    }),
    askPurge: () => set({ ask: { kind: 'purge', days: get().quarantine?.purge_after_days ?? 30 } }),
    cancelAsk: () => set({ ask: null }),
    /** The accept button of the generic dialog. */
    acceptAsk: guard(async () => {
      const ask = get().ask;
      if (!ask) return;
      set({ ask: null });
      if (ask.kind === 'undo_run') {
        setUi({ result: null });
        await api.undoRun({ run_id: ask.runId });
        set({ jobs: await api.jobsList() });
      } else if (ask.kind === 'purge') {
        await c.feature('quarantine_purge', async () => {
          await api.quarantinePurge({ older_than_days: ask.days });
          set({ jobs: await api.jobsList() });
        })();
      }
    }),
    /** Re-read the entries of expanded runs (history://changed). */
    async refreshOpenRuns() {
      for (const id of Object.keys(ui().expanded)) if (ui().expanded[id]) await loadDetail(Number(id), ui().details[id]?.offset ?? 0);
    },
    dismissResult: () => setUi({ result: null }),
    /** History "Restore" on a run row: asks first. @param {number} runId */
    restoreRun: (runId) => c.actions.askUndoRun(runId),
  };
}
