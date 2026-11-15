//! Scope commands (scope chips, SCAN-01/02). Owner: W3-D.

use mj_core::{lock, Error, Result, ScanScope};

use crate::api::{ScopePathRequest, ScopeSetRequest};
use crate::Engine;

fn normalised(mut scope: ScanScope) -> Result<ScanScope> {
    for r in &scope.roots {
        if !r.is_absolute() {
            return Err(Error::InvalidInput(format!("scan locations must be absolute paths: {}", r.display())));
        }
    }
    scope.roots = mj_scan::effective_roots(&scope);
    Ok(scope)
}

impl Engine {
    /// The session scope.
    pub fn scope_get(&self) -> Result<ScanScope> {
        Ok(lock(&self.scope).clone())
    }

    /// Replace the scope.
    pub fn scope_set(&self, req: ScopeSetRequest) -> Result<ScanScope> {
        let scope = normalised(req.scope)?;
        *lock(&self.scope) = scope.clone();
        Ok(scope)
    }

    /// Add one root (must be an existing folder).
    pub fn scope_add_root(&self, req: ScopePathRequest) -> Result<ScanScope> {
        let md = self.platform().metadata(&req.path)?;
        if !md.is_dir() {
            return Err(Error::InvalidInput(format!("not a folder: {}", req.path.display())));
        }
        let mut scope = lock(&self.scope).clone();
        scope.roots.push(req.path);
        let scope = normalised(scope)?;
        *lock(&self.scope) = scope.clone();
        Ok(scope)
    }

    /// Remove one root.
    pub fn scope_remove_root(&self, req: ScopePathRequest) -> Result<ScanScope> {
        let key = mj_core::pathutil::path_key(&req.path);
        let mut s = lock(&self.scope);
        s.roots.retain(|r| mj_core::pathutil::path_key(r) != key);
        Ok(s.clone())
    }
}
