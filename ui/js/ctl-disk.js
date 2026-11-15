// @ts-check
/** Disk usage view (SCAN-05/06/07/08): the size tree, the treemap and the breakdowns. */
import { findNode, replaceNode, needsLoad } from './lib/disktree.js';
import { isUnavailable, toErrorInfo } from './errors.js';

const DEPTH = 2;
const MIN_FRACTION = 0.01;

/** @param {any} c */
export function diskActions(c) {
  const { api, set, get, gens, guard } = c;
  const ui = () => get().disk;
  const setUi = (p) => set({ disk: { ...get().disk, ...p } });

  /** Load the subtree below `path` and graft it into the tree. @param {string} path */
  async function graft(path) {
    const key = `disk:${path}`;
    const g = gens.next(key);
    setUi({ loadingPaths: { ...ui().loadingPaths, [path]: true } });
    try {
      const node = await api.diskUsageTree({ path, depth: DEPTH, min_fraction: MIN_FRACTION });
      if (gens.current(key, g) && ui().root) setUi({ root: replaceNode(ui().root, path, node) });
    } finally {
      const { [path]: _, ...rest } = ui().loadingPaths;
      void _;
      setUi({ loadingPaths: rest });
    }
  }

  return {
    /** Load the top of the tree (one node per scan root). */
    async loadDisk() {
      const g = gens.next('disk');
      setUi({ loading: true, error: null });
      try {
        const root = await api.diskUsageTree({ depth: DEPTH, min_fraction: MIN_FRACTION });
        if (!gens.current('disk', g)) return;
        setUi({ loading: false, root, expanded: { [root.path]: true }, focusPath: null, breakdown: {} });
      } catch (e) {
        if (!gens.current('disk', g)) return;
        setUi({ loading: false, error: toErrorInfo(e), root: null });
        if (isUnavailable(e)) set({ unavail: { ...get().unavail, disk: toErrorInfo(e) } });
      }
    },
    /** Expand or collapse a folder row; deeper levels load on demand. @param {string} path */
    toggleDiskNode: guard(async (path) => {
      const open = !ui().expanded[path];
      setUi({ expanded: { ...ui().expanded, [path]: open } });
      const node = findNode(ui().root, path);
      if (open && node && needsLoad(node)) await graft(path);
    }),
    /** @param {'size'|'name'} sort */
    setDiskSort: (sort) => setUi({ sort }),
    /** @param {'folders'|'treemap'|'type'|'age'} tab */
    setDiskTab: guard(async (tab) => {
      setUi({ tab });
      if (tab === 'type' || tab === 'age') await c.actions.loadBreakdown(tab);
    }),
    /** Treemap and breakdowns look at this folder ('' = everything). @param {string|null} path */
    focusDisk: guard(async (path) => {
      setUi({ focusPath: path || null, breakdown: {} });
      const node = path ? findNode(ui().root, path) : null;
      if (node && needsLoad(node)) await graft(path);
      if (ui().tab === 'type' || ui().tab === 'age') await c.actions.loadBreakdown(ui().tab);
    }),
    /** Breakdown by file type or age (cached per folder). @param {'type'|'age'|'extension'|'owner'} by */
    async loadBreakdown(by) {
      const path = ui().focusPath;
      const key = `breakdown:${by}`;
      const g = gens.next(key);
      const put = (v) => setUi({ breakdown: { ...ui().breakdown, [by]: v } });
      put({ loading: true, error: null, data: null });
      try {
        const data = await api.breakdown({ by, ...(path ? { path } : {}) });
        if (gens.current(key, g)) put({ loading: false, error: null, data });
      } catch (e) {
        if (gens.current(key, g)) put({ loading: false, error: toErrorInfo(e), data: null });
      }
    },
  };
}
