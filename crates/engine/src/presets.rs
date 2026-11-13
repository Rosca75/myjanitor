//! Rule presets (CLN-ENG-02). Owner: W3-E. Stored in `<data>/presets.json`
//! (user presets only; the built-ins come from `mj_filters::presets::builtin`
//! and are never written to disk).
//!
//! Rules: a user preset may not reuse the name of another preset (built-in or
//! user, compared case-insensitively and trimmed), its rule must compile, an
//! empty id creates a fresh `user_<slug>` id, built-ins can be neither
//! overwritten nor deleted, and a corrupt file is moved aside (see
//! `settings::persist`) while the built-ins keep working.

use mj_core::{lock, Error, Result, RulePreset};
use mj_filters::presets as filters;

use crate::api::{IdRequest, PresetSaveRequest};
use crate::settings::persist::{read_or_backup, Loaded};
use crate::Engine;

const FILE: &str = "presets.json";

pub(crate) fn load(engine: &Engine) {
    if !engine.cfg.persist {
        return;
    }
    if let Loaded::Ok(v) = read_or_backup::<Vec<RulePreset>>(&engine.cfg.data_dir.join(FILE)) {
        *lock(&engine.presets) = v.into_iter().filter(|p| !p.builtin).collect();
    }
}

/// The same name (trimmed, case-insensitive) under a different id.
fn name_taken(all: &[RulePreset], p: &RulePreset) -> bool {
    let want = p.name.trim().to_lowercase();
    all.iter().any(|o| o.id != p.id && o.name.trim().to_lowercase() == want)
}

impl Engine {
    fn all_presets(&self) -> Vec<RulePreset> {
        filters::all(&lock(&self.presets))
    }

    fn save_presets(&self) -> Result<()> {
        if self.cfg.persist {
            crate::store::write_json(&self.cfg.data_dir.join(FILE), &*lock(&self.presets))?;
        }
        Ok(())
    }

    /// Built-in and user presets.
    pub fn presets_list(&self) -> Result<Vec<RulePreset>> {
        Ok(self.all_presets())
    }

    /// Insert or replace a user preset (an empty id gets a fresh one).
    pub fn preset_save(&self, req: PresetSaveRequest) -> Result<Vec<RulePreset>> {
        let mut p = req.preset;
        p.name = p.name.trim().to_string();
        p.builtin = false;
        if filters::builtin().iter().any(|b| b.id == p.id) {
            return Err(Error::InvalidInput("built-in presets cannot be changed; save a copy under a new name".into()));
        }
        let groups = lock(&self.settings).type_groups.clone();
        filters::validate(&p, &mj_filters::FilterContext { now: mj_core::Timestamp::now(), type_groups: &groups })?;
        if name_taken(&self.all_presets(), &p) {
            return Err(Error::InvalidInput(format!(
                "a preset named \"{}\" already exists; pick another name",
                p.name
            )));
        }
        {
            let mut user = lock(&self.presets);
            filters::upsert(&mut user, p)?;
        }
        self.save_presets()?;
        Ok(self.all_presets())
    }

    /// Delete a user preset (built-ins and unknown ids are `invalid_input`).
    pub fn preset_delete(&self, req: IdRequest) -> Result<Vec<RulePreset>> {
        filters::remove(&mut lock(&self.presets), &req.id)?;
        self.save_presets()?;
        Ok(self.all_presets())
    }
}

#[cfg(test)]
mod tests {
    use mj_core::testutil::TempDir;
    use mj_core::{Condition, Rule};

    use super::*;
    use crate::{Engine, EngineConfig};

    fn user_preset(name: &str) -> RulePreset {
        let all = filters::builtin();
        RulePreset {
            id: String::new(),
            name: name.into(),
            description: Some("mine".into()),
            rule: all[0].rule.clone(),
            topmost_only: false,
            builtin: false,
        }
    }

    fn engine_in(t: &TempDir) -> std::sync::Arc<Engine> {
        Engine::new(EngineConfig::new(t.join("data"), t.join("cache"))).unwrap()
    }

    #[test]
    fn built_ins_come_first_and_user_presets_persist() {
        let t = TempDir::new("presets-rt").unwrap();
        let e = engine_in(&t);
        let builtin_count = filters::builtin().len();
        let list = e.preset_save(PresetSaveRequest { preset: user_preset("Old screen recordings") }).unwrap();
        assert_eq!(list.len(), builtin_count + 1);
        assert!(list[..builtin_count].iter().all(|p| p.builtin));
        let mine = list.last().unwrap();
        assert!(!mine.builtin && mine.id.starts_with("user_"));
        let again = engine_in(&t).presets_list().unwrap();
        assert_eq!(again.last().unwrap().name, "Old screen recordings");
        assert_eq!(again.iter().filter(|p| p.builtin).count(), builtin_count);
    }

    #[test]
    fn name_collisions_are_refused_but_a_rename_in_place_is_fine() {
        let t = TempDir::new("presets-name").unwrap();
        let e = engine_in(&t);
        let list = e.preset_save(PresetSaveRequest { preset: user_preset("Mine") }).unwrap();
        let id = list.last().unwrap().id.clone();
        let dup = e.preset_save(PresetSaveRequest { preset: user_preset("  mine ") }).unwrap_err();
        assert_eq!(dup.code(), "invalid_input");
        let builtin_name = filters::builtin()[0].name.to_uppercase();
        assert!(e.preset_save(PresetSaveRequest { preset: user_preset(&builtin_name) }).is_err());
        // Same id, same name: a replace, not a collision.
        let mut same = user_preset("Mine");
        same.id = id.clone();
        same.description = Some("changed".into());
        let list = e.preset_save(PresetSaveRequest { preset: same }).unwrap();
        assert_eq!(list.iter().filter(|p| p.id == id).count(), 1);
        assert_eq!(list.iter().find(|p| p.id == id).unwrap().description.as_deref(), Some("changed"));
        // Two different names with colliding slugs get distinct ids.
        let a = e.preset_save(PresetSaveRequest { preset: user_preset("a b") }).unwrap();
        let b = e.preset_save(PresetSaveRequest { preset: user_preset("a-b") }).unwrap();
        let ids: Vec<_> = b.iter().filter(|p| !p.builtin).map(|p| p.id.clone()).collect();
        assert_eq!(ids.len(), a.iter().filter(|p| !p.builtin).count() + 1);
        assert_eq!(ids.iter().collect::<std::collections::BTreeSet<_>>().len(), ids.len());
    }

    #[test]
    fn only_user_presets_can_be_changed_or_deleted() {
        let t = TempDir::new("presets-del").unwrap();
        let e = engine_in(&t);
        let builtin_id = filters::builtin()[0].id.clone();
        let mut p = user_preset("x");
        p.id = builtin_id.clone();
        assert!(e.preset_save(PresetSaveRequest { preset: p }).is_err());
        assert!(e.preset_delete(IdRequest { id: builtin_id }).is_err());
        assert!(e.preset_delete(IdRequest { id: "user_nope".into() }).is_err());
        let list = e.preset_save(PresetSaveRequest { preset: user_preset("Gone soon") }).unwrap();
        let id = list.last().unwrap().id.clone();
        let list = e.preset_delete(IdRequest { id }).unwrap();
        assert!(list.iter().all(|p| p.builtin));
        assert!(engine_in(&t).presets_list().unwrap().iter().all(|p| p.builtin));
    }

    #[test]
    fn invalid_rules_and_empty_names_are_rejected() {
        let t = TempDir::new("presets-bad").unwrap();
        let e = engine_in(&t);
        assert!(e.preset_save(PresetSaveRequest { preset: user_preset("   ") }).is_err());
        let mut p = user_preset("Bad regex");
        p.rule = Rule::Cond(Condition::Path {
            mode: mj_core::rule::TextMode::Regex,
            pattern: "(unclosed".into(),
            case_sensitive: false,
        });
        assert!(e.preset_save(PresetSaveRequest { preset: p }).is_err());
    }

    #[test]
    fn a_corrupt_presets_file_is_backed_up_and_built_ins_still_list() {
        let t = TempDir::new("presets-corrupt").unwrap();
        std::fs::create_dir_all(t.join("data")).unwrap();
        std::fs::write(t.join("data/presets.json"), b"not json").unwrap();
        let e = engine_in(&t);
        assert!(e.presets_list().unwrap().iter().all(|p| p.builtin));
        assert!(std::fs::read_dir(t.join("data"))
            .unwrap()
            .filter_map(|d| d.ok())
            .any(|d| d.file_name().to_string_lossy().starts_with("presets.json.bad-")));
    }
}
