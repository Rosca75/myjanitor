//! External tools (W3-C).

use mj_core::{lock, ExternalTool, Result};

use crate::Engine;

pub(crate) fn load(engine: &Engine) {
    let overrides = lock(&engine.settings).tool_paths.clone();
    *lock(&engine.tools) = mj_optimise::tools::detect(&overrides);
}

impl Engine {
    /// Detected tools.
    pub fn tools_list(&self) -> Result<Vec<ExternalTool>> {
        Ok(lock(&self.tools).clone())
    }

    /// Detect again.
    pub fn tools_refresh(&self) -> Result<Vec<ExternalTool>> {
        load(self);
        self.tools_list()
    }
}
