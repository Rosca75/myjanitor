// @ts-check
/**
 * Controller of the Settings drawer. The shell's controller only knows
 * `loadSettings` and `clearCache`; everything else the drawer does (settings,
 * protected paths, schedules, tools, the folder picker) is handled here, with
 * the exact payloads of docs/IPC.md. The drawer's own transient state (inline
 * errors, open "Advanced" sections, the picker, loaded lists) lives in the
 * controller, one per shell `actions` object, because the shell store has no
 * keys for it. Settings values themselves always come from the store.
 *
 * `api` defaults to the real `api.js` and is injected in tests.
 */
import defaultApi from '../../api.js';
import { replaceKeepingFocus } from '../../dom.js';
import { errors } from './copy.js';

/** @type {WeakMap<object, any>} */
const registry = new WeakMap();

/** True for `/x`, `C:\x`, `C:/x` and `\\server\share`. @param {string} p */
export const isAbsolutePath = (p) => /^(?:\/|[A-Za-z]:[\\/]|\\\\)/.test(p);

/** The controller for these shell actions (created on first use). @param {any} actions @param {any} [api] */
export function settingsController(actions, api) {
  let ctl = registry.get(actions);
  if (!ctl) {
    ctl = createController(actions, api ?? defaultApi);
    registry.set(actions, ctl);
  }
  return ctl;
}

/** Replace the controller of `actions` (tests inject a fake api). @param {any} actions @param {any} api */
export function attachSettingsApi(actions, api) {
  const ctl = createController(actions, api);
  registry.set(actions, ctl);
  return ctl;
}

/** @param {any} actions @param {any} api */
function createController(actions, api) {
  const st = {
    prot: null, schedules: null, loaded: false, loading: false,
    /** @type {Record<string, string|null>} */ errors: {},
    /** @type {Record<string, boolean>} */ open: {},
    /** @type {any} */ picker: null,
    /** @type {string|null} */ confirmDelete: null,
    /** @type {Record<string, string>} */ notes: {},
  };
  /** @type {{ host: any, s: any, build: ((s: any) => any) | null }} */
  const view = { host: null, s: null, build: null };

  const rerender = () => { if (view.host && view.build && view.s) replaceKeepingFocus(view.host, view.build(view.s)); };
  const fail = (field, e) => { st.errors[field] = e?.message ?? String(e); rerender(); };
  const ok = (field) => { st.errors[field] = null; };

  const ctl = {
    st,
    /** Called by the view on every render. */
    bind(host, s, build) { view.host = host; view.s = s; view.build = build; },
    rerender,
    async ensureLoaded() {
      if (st.loaded || st.loading) return;
      st.loading = true;
      const soft = (p, fb) => p.catch(() => fb);
      const [prot, schedules] = await Promise.all([soft(api.protectedPathsGet(), null), soft(api.schedulesList(), [])]);
      Object.assign(st, { prot, schedules, loaded: true, loading: false });
      rerender();
    },
    toggleOpen(key) { st.open[key] = !st.open[key]; rerender(); },

    /** Change settings with `mutate(copy)`; the engine validates and the error shows under `field`. */
    async commit(field, mutate) {
      const next = structuredClone(view.s.settings);
      mutate(next);
      try {
        const saved = await api.settingsSet({ settings: next });
        ok(field);
        view.s = { ...view.s, settings: saved };
        await actions.loadSettings();
        rerender();
      } catch (e) { fail(field, e); }
    },
    /** A whole number within bounds, checked here first so typing errors never hit the engine. */
    async commitNumber(field, raw, min, max, apply) {
      const n = Number(raw);
      if (raw === '' || !Number.isInteger(n) || n < min || n > max) { fail(field, { message: errors.range(min, max) }); return; }
      await ctl.commit(field, (s) => apply(s, n));
    },

    // ----------------------------------------------------- protected paths
    async setProtected(user) {
      try { st.prot = await api.protectedPathsSet({ user }); ok('protected'); actions.loadSettings(); rerender(); } catch (e) { fail('protected', e); }
    },
    // ----------------------------------------------------------- exclusions
    async addExclusion(text) {
      const v = text.trim();
      if (!v) { fail('exclusions', { message: errors.exclusionEmpty }); return; }
      await ctl.commit('exclusions', (s) => { if (!s.safety.exclusions.includes(v)) s.safety.exclusions.push(v); });
    },
    removeExclusion: (v) => ctl.commit('exclusions', (s) => { s.safety.exclusions = s.safety.exclusions.filter((x) => x !== v); }),
    // ---------------------------------------------------------------- tools
    async refreshTools() {
      try { await api.toolsRefresh(); ok('tools'); await actions.loadSettings(); rerender(); } catch (e) { fail('tools', e); }
    },
    async setToolPath(id, raw) {
      const v = raw.trim();
      if (!isAbsolutePath(v)) { fail(`tool:${id}`, { message: errors.absolute }); return; }
      await ctl.commit(`tool:${id}`, (s) => { s.tool_paths[id] = v; });
      await ctl.refreshTools();
    },
    async clearToolPath(id) {
      await ctl.commit(`tool:${id}`, (s) => { delete s.tool_paths[id]; });
      await ctl.refreshTools();
    },
    // ------------------------------------------------------------ schedules
    async saveSchedule(schedule, field) {
      try { st.schedules = await api.scheduleSave({ schedule }); ok(field); rerender(); } catch (e) { fail(field, e); }
    },
    toggleSchedule(id, enabled) {
      const s = (st.schedules ?? []).find((x) => x.id === id);
      return s ? ctl.saveSchedule({ ...s, enabled }, `schedule:${id}`) : undefined;
    },
    async runSchedule(id) {
      try { await api.scheduleRunNow({ id }); st.notes[`schedule:${id}`] = 'started'; ok(`schedule:${id}`); rerender(); } catch (e) { fail(`schedule:${id}`, e); }
    },
    async deleteSchedule(id) {
      if (st.confirmDelete !== id) { st.confirmDelete = id; rerender(); return; }
      st.confirmDelete = null;
      try { st.schedules = await api.scheduleDelete({ id }); ok(`schedule:${id}`); rerender(); } catch (e) { fail(`schedule:${id}`, e); }
    },
    // --------------------------------------------------------------- picker
    /** Open the inline folder picker for `purpose` ('protected' | 'temp' | 'backup'). */
    async openPicker(purpose) {
      st.picker = { purpose, path: '', parent: null, entries: [], places: [], loading: true };
      rerender();
      try {
        const first = await api.fsListDir({});
        st.picker = { ...st.picker, places: first.places };
        const start = first.places[0]?.path;
        if (start) await ctl.pickerTo(start); else { st.picker.loading = false; rerender(); }
      } catch (e) { st.picker = null; fail('picker', e); }
    },
    async pickerTo(path) {
      st.picker = { ...st.picker, loading: true };
      rerender();
      try {
        const r = await api.fsListDir({ path, show_hidden: false, include_files: false });
        st.picker = { ...st.picker, path: r.path, parent: r.parent ?? null, entries: r.entries.filter((e) => e.is_dir), loading: false };
        rerender();
      } catch (e) { st.picker = { ...st.picker, loading: false }; fail('picker', e); }
    },
    closePicker() { st.picker = null; rerender(); },
    async pickerChoose() {
      const { purpose, path } = st.picker ?? {};
      if (!path) return;
      st.picker = null;
      if (purpose === 'protected') await ctl.setProtected([...(st.prot?.user ?? []), path]);
      else if (purpose === 'temp') await ctl.commit('temp', (s) => { s.performance.temp_dir = path; });
      else if (purpose === 'backup') await ctl.commit('originals', (s) => { s.general.default_originals = { policy: 'backup_folder', folder: path }; });
    },
  };
  return ctl;
}
