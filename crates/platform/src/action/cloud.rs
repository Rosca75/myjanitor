//! Cloud placeholders: dehydrate / hydrate (`CfDehydratePlaceholder`,
//! `CfHydratePlaceholder` or the pin attributes) and synced-folder roots
//! (OneDrive registry keys, Dropbox `info.json`, Nextcloud config). Owner: W2-E.

use std::path::Path;

use mj_core::{Error, Result};

use crate::traits::SyncRoot;

/// "Free up space" (CLN-ACT-07). Windows only.
pub fn dehydrate(path: &Path) -> Result<()> {
    let _ = path;
    if cfg!(windows) {
        Err(Error::not_implemented("dehydrate placeholder (W2-E)"))
    } else {
        Err(Error::unsupported("Free up space", "cloud placeholders are Windows only"))
    }
}

/// Bring content back locally (undo of dehydrate). Windows only.
pub fn hydrate(path: &Path) -> Result<()> {
    let _ = path;
    if cfg!(windows) {
        Err(Error::not_implemented("hydrate placeholder (W2-E)"))
    } else {
        Err(Error::unsupported("Hydrate", "cloud placeholders are Windows only"))
    }
}

/// Synced folder roots of the current user. Until implemented this returns
/// `NotImplemented`, and callers must treat sync status as *unknown*, not "not synced".
pub fn sync_roots() -> Result<Vec<SyncRoot>> {
    Err(Error::not_implemented("synced folder detection (W2-E)"))
}
