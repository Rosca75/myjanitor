// @ts-check
/**
 * Custom search (CLN-ENG-01/02/05): quick query with autocomplete and parse
 * errors, the rule builder, presets, and results shown in the shared step table.
 */
import { createStepOps, freshStep } from './ctl-step.js';
import { addTo, removeAt, updateAt, newCond, newGroup, fromRule, toRule, validate, initialModel } from './lib/rulebuilder.js';
import { applySuggestion, byteOffset } from './lib/query.js';
import { initialCustom } from './state.js';
import { toErrorInfo } from './errors.js';

/** Action choices of the search panel. */
const ACTIONS = { trash: { kind: 'trash' }, quarantine: { kind: 'quarantine', purge_after_days: 7 }, permanent_delete: { kind: 'permanent_delete' } };

/** @param {any} c */
export function searchActions(c) {
  const { api, set, get, gens, guard } = c;
  const cu = () => get().custom;
  const setCu = (p) => set({ custom: { ...get().custom, ...p } });
  const ops = createStepOps(c, { key: 'cstep', read: () => cu().step, write: (step) => setCu({ step }) });

  /** Parse and suggest for the current text; stale answers are dropped. */
  async function inspect(query, caret) {
    if (!query.trim()) { setCu({ errors: [], parsedRule: null, suggestions: [], suggestIndex: -1 }); return; }
    const gp = gens.next('parse');
    const gs = gens.next('suggest');
    const parse = api.searchParse({ query }).then((r) => { if (gens.current('parse', gp)) setCu({ errors: r.errors, parsedRule: r.rule ?? null }); });
    const suggest = api.searchSuggest({ query, cursor: byteOffset(query, caret) }).then((list) => { if (gens.current('suggest', gs)) setCu({ suggestions: list, suggestIndex: list.length ? 0 : -1 }); });
    await Promise.all([parse, suggest]);
  }

  /** The rule to run for the active mode, or null (errors are then shown). */
  async function currentRule() {
    const s = cu();
    if (s.mode === 'builder') {
      const problems = validate(s.builder);
      setCu({ builderErrors: problems });
      return problems.length ? null : toRule(s.builder);
    }
    if (!s.query.trim()) return null;
    const parsed = await api.searchParse({ query: s.query });
    setCu({ errors: parsed.errors, parsedRule: parsed.rule ?? null });
    return parsed.errors.length || !parsed.rule ? null : parsed.rule;
  }

  const rebuild = (builder) => setCu({ builder, builderErrors: [] });

  return {
    ...Object.fromEntries(Object.entries(ops).map(([k, v]) => [`custom_${k}`, v])),
    /** Typing in the quick-query box. @param {string} query @param {number} [caret] */
    setQuery: guard(async (query, caret) => {
      const at = caret ?? query.length;
      setCu({ query, caret: at }); // synchronous, so typing never loses a character
      await inspect(query, at);
    }),
    /** Arrow keys in the suggestion list. @param {number} delta */
    moveSuggestion(delta) {
      const n = cu().suggestions.length;
      if (n) setCu({ suggestIndex: (cu().suggestIndex + delta + n) % n });
    },
    closeSuggestions: () => setCu({ suggestions: [], suggestIndex: -1 }),
    /** Insert a suggestion at the caret. @param {number} [i] */
    acceptSuggestion: guard(async (i) => {
      const s = cu();
      const pick = s.suggestions[i ?? s.suggestIndex];
      if (!pick) return;
      const { text, caret } = applySuggestion(s.query, s.caret, pick);
      setCu({ query: text, caret, suggestions: [], suggestIndex: -1 });
      await inspect(text, caret);
    }),
    setSearchMode: (mode) => setCu({ mode }),
    setSearchOption: (patch) => setCu(patch),
    toggleSearchAdvanced: () => setCu({ advancedOpen: !cu().advancedOpen }),

    // ------------------------------------------------------------ builder
    /** @param {number[]} path @param {'cond'|'group'} what @param {string} [field] */
    builderAdd: (path, what, field) => rebuild(addTo(cu().builder, path, what === 'group' ? newGroup('all') : newCond(field))),
    /** @param {number[]} path */
    builderRemove: (path) => rebuild(removeAt(cu().builder, path)),
    /** Edit fields of the node at `path`. @param {number[]} path @param {object} patch */
    builderSet: (path, patch) => rebuild(updateAt(cu().builder, path, (n) => ({ ...n, ...patch }))),
    /** Switch a condition to another field, keeping its negation. @param {number[]} path @param {string} field */
    builderField: (path, field) => rebuild(updateAt(cu().builder, path, (n) => ({ ...newCond(field), not: n.not }))),
    builderReset: () => rebuild(initialModel()),
    /** "Edit in the builder": carry the parsed quick query over. */
    importQueryToBuilder() {
      const rule = cu().parsedRule;
      if (rule) setCu({ builder: fromRule(rule), mode: 'builder', builderErrors: [] });
    },

    // ------------------------------------------------------------ presets
    loadPresets: async () => {
      setCu({ presetsLoading: true });
      try {
        const presets = await api.presetsList();
        setCu({ presets, presetsLoading: false });
      } catch (e) {
        setCu({ presetsLoading: false });
        c.fail(e);
      }
    },
    /** Load a preset into the builder. @param {string} id */
    applyPreset(id) {
      const p = cu().presets.find((x) => x.id === id);
      if (p) setCu({ builder: fromRule(p.rule), mode: 'builder', topmost: !!p.topmost_only, title: p.name, builderErrors: [] });
    },
    /** "Save current as preset". */
    savePreset: c.feature('presets', async () => {
      const s = cu();
      const problems = s.mode === 'builder' ? validate(s.builder) : [];
      if (problems.length) { setCu({ builderErrors: problems }); return; }
      const rule = await currentRule();
      if (!rule || !s.presetName.trim()) return;
      const presets = await api.presetSave({ preset: { id: '', name: s.presetName.trim(), ...(s.presetDesc.trim() ? { description: s.presetDesc.trim() } : {}), rule, topmost_only: s.topmost, builtin: false } });
      setCu({ presets, presetName: '', presetDesc: '' });
    }),
    /** @param {string} id */
    deletePreset: c.feature('presets', async (id) => { setCu({ presets: await api.presetDelete({ id }) }); }),

    // --------------------------------------------------------------- run
    /** "Search": create the custom opportunity, then show its rows. */
    runSearch: guard(async () => {
      const rule = await currentRule();
      if (!rule) return;
      const s = cu();
      setCu({ running: true });
      try {
        const opp = await api.searchRun({ rule, title: s.title.trim() || 'Custom search', topmost_only: s.topmost, action: ACTIONS[s.action] ?? ACTIONS.trash });
        setCu({ errors: [], result: opp, running: false, step: opp.count > 0 ? freshStep(opp.id) : null });
        await c.actions.refreshOpps();
        if (opp.count > 0) await ops.reloadStep();
      } catch (e) {
        setCu({ running: false });
        c.fail(toErrorInfo(e));
      }
    }),
    /** "Add to plan" for the result, with the chosen action. */
    addSearchToPlan: guard(async () => {
      const s = cu();
      if (!s.result) return;
      set({ plan: await api.planAddStep({ opportunity_id: s.result.id, action: ACTIONS[s.action] ?? ACTIONS.trash }) });
      await c.actions.refreshOpps();
      await ops.loadDetail();
    }),
    clearSearchResult: () => setCu({ result: null, step: null }),
    /** Reset everything (kept for tests and "Clear"). */
    resetSearch: () => set({ custom: initialCustom() }),
  };
}
