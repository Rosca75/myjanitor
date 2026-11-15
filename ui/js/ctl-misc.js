// @ts-check
/** Category lists (Clean / Optimise / Archive) and the settings reads. */

/** @param {any} c */
export function miscActions(c) {
  const { api, set, get, gens } = c;
  const soft = (p, fb) => p.catch(() => fb);

  return {
    /** Clean / Optimise / Archive lists: cards of one category. @param {string} name */
    async loadCategory(name) {
      const g = gens.next('category');
      set({ category: { name, items: get().category.name === name ? get().category.items : [], loading: true, error: null } });
      try {
        const list = await api.opportunitiesList({ tab: name, sort: 'saving' });
        if (gens.current('category', g)) set({ category: { name, items: list.items, loading: false, error: null } });
      } catch (e) {
        if (gens.current('category', g)) set({ category: { name, items: [], loading: false, error: e } });
      }
    },
    /** Archive view: cold-folder months, through the step parameters. @param {string} id @param {string} key @param {number} value */
    setOppParam: c.guard(async (id, key, value) => {
      await api.stepParamsSet({ opportunity_id: id, params: { [key]: value } });
      await c.actions.loadCategory(get().category.name);
    }),
    loadSettings: c.guard(async () => {
      // app_info too: its trash_route follows general.windows_recycle_bin (IPC-5).
      const [settings, cache, tools, info] = await Promise.all([api.settingsGet(), soft(api.cacheInfo(), null), soft(api.toolsList(), []), soft(api.appInfo(), get().info)]);
      set({ settings, cache, tools, info });
    }),
    clearCache: c.guard(async () => set({ cache: await api.cacheClear() })),
  };
}
