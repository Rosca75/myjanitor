//! Tolerant loading of the small JSON state files (settings, presets, schedules).
//! A file that exists but cannot be understood is **moved aside** (never deleted)
//! and defaults are used, so start-up never fails and nothing the user wrote is lost.
//! Owner: W3-E.

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;

/// What loading found.
#[derive(Debug)]
#[allow(dead_code)] // the fields are for diagnostics (Debug output, tests)
pub enum Loaded<T> {
    /// No file (first run) or it could not be read: use defaults, nothing to report.
    Missing,
    /// Parsed.
    Ok(T),
    /// Unparseable: the file was moved to `backup` (or could not be, `None`).
    Corrupt {
        backup: Option<PathBuf>,
        #[allow(dead_code)]
        reason: String,
    },
}

/// Name of the backup for a corrupt file: `<name>.bad-<unix seconds>`.
pub fn backup_path(path: &Path, now_secs: i64) -> PathBuf {
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(format!(".bad-{now_secs}"));
    path.with_file_name(name)
}

/// Read `path` as JSON of type `T`; back the file up when it is corrupt.
pub fn read_or_backup<T: DeserializeOwned>(path: &Path) -> Loaded<T> {
    // Fast path: a readable, valid file.
    if let Some(v) = crate::store::read_json::<T>(path) {
        return Loaded::Ok(v);
    }
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(_) => return Loaded::Missing,
    };
    match serde_json::from_slice::<T>(&bytes) {
        Ok(v) => Loaded::Ok(v),
        Err(e) => {
            let mut target = backup_path(path, mj_core::Timestamp::now().secs());
            // Two corruptions inside one second must not overwrite each other's backup.
            let mut n = 1;
            while target.exists() {
                target = backup_path(path, mj_core::Timestamp::now().secs() + n);
                n += 1;
            }
            let backup = std::fs::rename(path, &target).ok().map(|_| target);
            Loaded::Corrupt { backup, reason: e.to_string() }
        }
    }
}

#[cfg(test)]
mod tests {
    use mj_core::testutil::TempDir;

    use super::*;

    #[test]
    fn missing_ok_and_corrupt() {
        let t = TempDir::new("persist").unwrap();
        let p = t.join("x.json");
        assert!(matches!(read_or_backup::<Vec<u8>>(&p), Loaded::Missing));
        std::fs::write(&p, b"[1,2]").unwrap();
        assert!(matches!(read_or_backup::<Vec<u8>>(&p), Loaded::Ok(v) if v == vec![1, 2]));
        std::fs::write(&p, b"{nope").unwrap();
        match read_or_backup::<Vec<u8>>(&p) {
            Loaded::Corrupt { backup: Some(b), .. } => {
                assert!(!p.exists());
                assert_eq!(std::fs::read(&b).unwrap(), b"{nope");
                assert!(b.file_name().unwrap().to_string_lossy().starts_with("x.json.bad-"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn repeated_corruption_keeps_every_backup() {
        let t = TempDir::new("persist2").unwrap();
        let p = t.join("x.json");
        for _ in 0..3 {
            std::fs::write(&p, b"{nope").unwrap();
            let _ = read_or_backup::<Vec<u8>>(&p);
        }
        let n = std::fs::read_dir(t.path()).unwrap().count();
        assert_eq!(n, 3);
    }
}
