//! Rule presets (CLN-ENG-02). Owner: W3-E. Stored in `<data>/presets.json` (user presets only).

use mj_core::{lock, Error, Result, RulePreset};

use crate::api::{IdRequest, PresetSaveRequest};
use crate::Engine;

const FILE: &str = "presets.json";

pub(crate) fn load(engine: &Engine) {
    if engine.cfg.persist {
        if let Some(v) = crate::store::read_json::<Vec<RulePreset>>(&engine.cfg.data_dir.join(FILE)) {
            *lock(&engine.presets) = v.into_iter().filter(|p| !p.builtin).collect();
        }
    }
}

impl Engine {
    fn all_presets(&self) -> Vec<RulePreset> {
        let mut v = mj_filters::presets::builtin();
        v.extend(lock(&self.presets).iter().cloned());
        v
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
        if mj_filters::presets::builtin().iter().any(|b| b.id == p.id) {
            return Err(Error::InvalidInput("built-in presets cannot be changed; save a copy under a new name".into()));
        }
        if p.name.trim().is_empty() {
            return Err(Error::InvalidInput("a preset needs a name".into()));
        }
        if p.id.is_empty() {
            p.id = format!("user_{}", self.next_id());
        }
        p.builtin = false;
        {
            let mut v = lock(&self.presets);
            match v.iter_mut().find(|x| x.id == p.id) {
                Some(slot) => *slot = p,
                None => v.push(p),
            }
        }
        self.save_presets()?;
        Ok(self.all_presets())
    }

    /// Delete a user preset.
    pub fn preset_delete(&self, req: IdRequest) -> Result<Vec<RulePreset>> {
        if mj_filters::presets::builtin().iter().any(|b| b.id == req.id) {
            return Err(Error::InvalidInput("built-in presets cannot be deleted".into()));
        }
        lock(&self.presets).retain(|p| p.id != req.id);
        self.save_presets()?;
        Ok(self.all_presets())
    }
}
