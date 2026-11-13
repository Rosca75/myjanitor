//! Recycle Bin (`SHFileOperationW` + `FOF_ALLOWUNDO`, or `IFileOperation`) and the
//! freedesktop.org Trash spec (`$XDG_DATA_HOME/Trash` + per-volume `.Trash-$UID`).
//! Owner: W2-E.

use std::path::Path;

use mj_core::{Error, Result, TrashTicket};

/// Move `path` to the trash and say where it went.
pub fn trash(path: &Path) -> Result<TrashTicket> {
    let _ = path;
    Err(Error::not_implemented("trash (W2-E)"))
}

/// Restore a trashed item to `original`.
pub fn restore(ticket: &TrashTicket, original: &Path) -> Result<()> {
    let _ = (ticket, original);
    Err(Error::not_implemented("restore from trash (W2-E)"))
}
