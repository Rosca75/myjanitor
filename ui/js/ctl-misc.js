// @ts-check
/** History, settings and the stub views' data loading, plus Custom search. */

/** @param {any} c */
export function miscActions(c) {
  const { api, set, get, guard } = c;
  const soft = (p, fb) => p.catch(() => fb);

  return {
    /** Clean / Optimise / Archive lists: cards of one category. @param {string} name */
    loadCategory: guard(async (name) => {
      const list = await api.opportunitiesList({ tab: name, sort: 'saving' });
      set({ category: { name, items: list.items } });
    }),
    loadHistory: guard(async () => {
      await c.actions.refreshHistory();
      set({ quarantine: await soft(api.quarantineInfo(), null) });
    }),
    /** History "Restore" for a whole run. @param {number} runId */
    restoreRun: guard(async (runId) => { await api.undoRun({ run_id: runId }); set({ jobs: await api.jobsList() }); }),
    loadSettings: guard(async () => {
      const [settings, cache, tools] = await Promise.all([api.settingsGet(), soft(api.cacheInfo(), null), soft(api.toolsList(), [])]);
      set({ settings, cache, tools });
    }),
    clearCache: guard(async () => set({ cache: await api.cacheClear() })),

    // ------------------------------------------------------- custom search
    /** Validate the query while typing. @param {string} query */
    setQuery: guard(async (query) => {
      set({ custom: { ...get().custom, query } }); // synchronous, so typing never loses a character
      const parsed = await api.searchParse({ query });
      if (get().custom.query === query) set({ custom: { ...get().custom, errors: parsed.errors } });
    }),
    /** "Search": create a custom opportunity from the quick query. */
    runSearch: guard(async () => {
      const { query } = get().custom;
      const parsed = await api.searchParse({ query });
      if (parsed.errors.length || !query.trim()) { set({ custom: { ...get().custom, errors: parsed.errors } }); return; }
      const opp = await api.searchRun({ rule: parsed.rule, title: 'Custom search', topmost_only: false, action: { kind: 'trash' } });
      set({ custom: { ...get().custom, errors: [], result: opp } });
      await c.actions.refreshOpps();
    }),
  };
}
