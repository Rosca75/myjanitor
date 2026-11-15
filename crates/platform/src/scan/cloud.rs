//! Cloud placeholder state (OneDrive/Dropbox Files On-Demand), SCAN-09. Owner: W2-A.
//!
//! Windows: from the attributes and reparse tag `metadata` already reads,
//! without opening the file (an open could recall an online-only file):
//! `RECALL_ON_DATA_ACCESS` / `RECALL_ON_OPEN` / `OFFLINE` = online-only,
//! `PINNED` = always keep on this device, a cloud-files reparse tag without
//! those = hydrated placeholder. Linux: not applicable (`NotCloud`).

use std::path::Path;

use mj_core::{Attributes, FileEntry, Result};

use crate::traits::{CloudFileState, CloudInfo};

/// Cloud state from an entry's attributes. Pure.
pub fn state_of(e: &FileEntry) -> CloudFileState {
    if e.is_online_only() {
        CloudFileState::Dehydrated
    } else if e.attrs.contains(Attributes::PINNED) {
        CloudFileState::Pinned
    } else if e.attrs.contains(Attributes::CLOUD_LOCAL) {
        CloudFileState::Hydrated
    } else {
        CloudFileState::NotCloud
    }
}

/// Cloud state of one path.
pub fn cloud_info(path: &Path) -> Result<CloudInfo> {
    if cfg!(windows) {
        let e = super::metadata::metadata(path)?;
        Ok(CloudInfo { state: state_of(&e), provider: None })
    } else {
        let _ = path;
        Ok(CloudInfo { state: CloudFileState::NotCloud, provider: None })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::EntryKind;

    #[test]
    fn states_from_attributes() {
        let mut e = FileEntry::new("/x", EntryKind::File, 1);
        assert_eq!(state_of(&e), CloudFileState::NotCloud);
        e.attrs = Attributes::CLOUD_LOCAL;
        assert_eq!(state_of(&e), CloudFileState::Hydrated);
        e.attrs |= Attributes::PINNED;
        assert_eq!(state_of(&e), CloudFileState::Pinned);
        e.attrs = Attributes::ONLINE_ONLY;
        assert_eq!(state_of(&e), CloudFileState::Dehydrated);
        e.attrs = Attributes::OFFLINE;
        assert_eq!(state_of(&e), CloudFileState::Dehydrated);
        assert_eq!(cloud_info(Path::new("/")).unwrap().state, CloudFileState::NotCloud);
    }
}
