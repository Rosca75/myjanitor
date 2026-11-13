//! Cloud placeholder state (OneDrive/Dropbox Files On-Demand), SCAN-09. Owner: W2-A.
//! Windows: attributes (`RECALL_ON_*`, `PINNED`, `UNPINNED`) and `CfGetPlaceholderState`.
//! Linux: not applicable.

use std::path::Path;

use mj_core::Result;

use crate::traits::{CloudFileState, CloudInfo};

/// Cloud state of one path.
pub fn cloud_info(path: &Path) -> Result<CloudInfo> {
    #[cfg(windows)]
    {
        use mj_core::Attributes;
        let e = super::metadata::metadata(path)?;
        let state = if e.attrs.contains(Attributes::ONLINE_ONLY) {
            CloudFileState::Dehydrated
        } else if e.attrs.contains(Attributes::PINNED) {
            CloudFileState::Pinned
        } else {
            CloudFileState::NotCloud
        };
        Ok(CloudInfo { state, provider: None })
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Ok(CloudInfo { state: CloudFileState::NotCloud, provider: None })
    }
}
