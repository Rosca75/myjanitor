//! `fs_list_dir`: the in-app folder browser (D-104). Read-only. Owner: W3-D.

use std::path::{Path, PathBuf};

use mj_core::{Attributes, EntryKind, Result};

use crate::api::{FsDirEntry, FsListDirRequest, FsListDirResponse, FsPlace};
use crate::Engine;

fn places() -> Vec<FsPlace> {
    let mut out = Vec::new();
    if let Some(home) = crate::paths::home_dir() {
        for (label, rel) in [
            ("Desktop", "Desktop"),
            ("Documents", "Documents"),
            ("Downloads", "Downloads"),
            ("Pictures", "Pictures"),
            ("Videos", "Videos"),
            ("Music", "Music"),
        ] {
            let p = home.join(rel);
            if p.is_dir() {
                out.push(FsPlace { label: label.into(), path: p, kind: "folder".into() });
            }
        }
        out.insert(0, FsPlace { label: "Home".into(), path: home, kind: "home".into() });
    }
    if cfg!(windows) {
        for letter in b'A'..=b'Z' {
            let p = PathBuf::from(format!("{}:\\", letter as char));
            if p.is_dir() {
                out.push(FsPlace { label: format!("{}:", letter as char), path: p, kind: "drive".into() });
            }
        }
    } else {
        out.push(FsPlace { label: "Computer".into(), path: PathBuf::from("/"), kind: "drive".into() });
        for base in ["/media", "/mnt", "/run/media"] {
            if let Ok(rd) = std::fs::read_dir(base) {
                for e in rd.flatten() {
                    if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        out.push(FsPlace {
                            label: e.file_name().to_string_lossy().into_owned(),
                            path: e.path(),
                            kind: "drive".into(),
                        });
                    }
                }
            }
        }
    }
    out
}

impl Engine {
    /// List a folder (folders first), or the places when no path is given.
    pub fn fs_list_dir(&self, req: FsListDirRequest) -> Result<FsListDirResponse> {
        let places = places();
        let Some(path) = req.path else {
            return Ok(FsListDirResponse { path: None, parent: None, entries: Vec::new(), places });
        };
        let mut entries = Vec::new();
        for item in self.platform().read_dir(&path)? {
            let name = item.name.to_string_lossy().into_owned();
            let is_link = item.kind == EntryKind::Link;
            let md = self.platform().metadata(&item.path).ok();
            let hidden = name.starts_with('.') || md.as_ref().is_some_and(|m| m.attrs.contains(Attributes::HIDDEN));
            if hidden && !req.show_hidden {
                continue;
            }
            let is_dir = item.kind == EntryKind::Dir;
            if !is_dir && !req.include_files {
                continue;
            }
            entries.push(FsDirEntry { name, path: item.path, is_dir, is_link, hidden });
        }
        entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        Ok(FsListDirResponse { parent: path.parent().map(Path::to_path_buf), path: Some(path), entries, places })
    }
}
