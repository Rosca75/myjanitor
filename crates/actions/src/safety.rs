//! Safety inspection (SAF-07/09/10). **Owner: W2-E.**
//!
//! - path-only, cheap: synced-folder membership (`sync_root_of`), protected;
//! - content, bounded reads: signed (OOXML `_xmlsignatures/`, PDF `/ByteRange`
//!   and `/Sig`), sensitivity-labelled (OOXML `docProps/custom.xml` `MSIP_Label_*`),
//!   encrypted (CFB `EncryptionInfo`, PDF `/Encrypt`, ZIP flag bit 0).
//!
//! When sync roots are unknown (detection not available), `sync_status_known`
//! is false and the engine shows a checklist warning instead of claiming
//! "nothing is synced".

use std::path::Path;

use mj_core::{Badges, Error, FileEntry, Result};
use mj_platform::{ActionPlatform, SyncRoot};

use crate::protected::ProtectedPaths;

/// Inspects candidates before they are offered.
#[derive(Debug, Clone)]
pub struct SafetyInspector {
    sync_roots: Option<Vec<SyncRoot>>,
    protected: ProtectedPaths,
}

impl SafetyInspector {
    /// Build from the platform's sync roots (unknown when not detectable).
    pub fn new(platform: &dyn ActionPlatform, protected: ProtectedPaths) -> Self {
        SafetyInspector { sync_roots: platform.sync_roots().ok(), protected }
    }

    /// True when synced-folder detection worked.
    pub fn sync_status_known(&self) -> bool {
        self.sync_roots.is_some()
    }

    /// The sync root containing `path`.
    pub fn sync_root_of(&self, path: &Path) -> Option<&SyncRoot> {
        self.sync_roots.as_ref()?.iter().find(|r| mj_core::pathutil::is_within(&r.path, path))
    }

    /// Badges from metadata and path only (no I/O).
    pub fn path_badges(&self, entry: &FileEntry) -> Badges {
        let mut b = Badges::empty();
        if self.protected.is_protected(&entry.path) {
            b |= Badges::PROTECTED;
        }
        if self.sync_root_of(&entry.path).is_some() {
            b |= Badges::SYNCED;
        }
        if entry.is_online_only() {
            b |= Badges::ONLINE_ONLY;
        }
        if entry.attrs.contains(mj_core::Attributes::ENCRYPTED) {
            b |= Badges::ENCRYPTED;
        }
        b
    }

    /// Badges that need a bounded read of the file (signed, labelled, encrypted).
    pub fn content_badges(&self, path: &Path) -> Result<Badges> {
        let _ = path;
        Err(Error::not_implemented("signed/labelled/encrypted detection (W2-E)"))
    }
}
