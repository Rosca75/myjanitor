//! Small JSON state files (settings, presets, schedules): atomic writes
//! (`<file>.tmp` + `sync_all` + rename), tolerant reads (missing or corrupt =
//! `None`, the caller keeps defaults).

use std::io::Write;
use std::path::Path;

use mj_core::{Error, Result};
use serde::de::DeserializeOwned;
use serde::Serialize;

/// Read a JSON file; `None` when missing or unreadable.
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Write a JSON file atomically.
pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let data = serde_json::to_vec_pretty(value).map_err(|e| Error::Internal(e.to_string()))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    {
        let mut f = std::fs::File::create(&tmp).map_err(|e| Error::io(&tmp, e))?;
        f.write_all(&data).map_err(|e| Error::io(&tmp, e))?;
        f.sync_all().map_err(|e| Error::io(&tmp, e))?;
    }
    std::fs::rename(&tmp, path).map_err(|e| Error::io(path, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::testutil::TempDir;

    #[test]
    fn round_trip_and_tolerant_read() {
        let t = TempDir::new("store").unwrap();
        let p = t.join("x/s.json");
        write_json(&p, &vec![1, 2, 3]).unwrap();
        assert_eq!(read_json::<Vec<i32>>(&p), Some(vec![1, 2, 3]));
        std::fs::write(&p, b"{broken").unwrap();
        assert_eq!(read_json::<Vec<i32>>(&p), None);
        assert_eq!(read_json::<Vec<i32>>(&t.join("missing.json")), None);
    }
}
