//! The searchable index (OPT-ARC-04): `INDEX.csv`, written as the first entry
//! of every archive and as a sidecar `<archive>.index.csv`.
//!
//! Columns: `path,size,mtime,sha256,note`. Directories have an empty size and
//! hash; links are rows with the note `symlink not archived -> <target>`.
//! Fields are quoted per RFC 4180 when needed.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use mj_core::hash::Sha256;
use mj_core::{CancelToken, Error, Result};

use super::walk::{Item, Kind};

/// Name of the index entry inside an archive.
pub const INDEX_NAME: &str = "INDEX.csv";
/// Read buffer for streamed hashing.
const CHUNK: usize = 1 << 20;

/// SHA-256 and length of a file, streamed in 1 MiB chunks (cancel checked between chunks).
pub fn hash_file(path: &Path, cancel: &CancelToken) -> Result<(String, u64)> {
    let mut f = File::open(path).map_err(|e| Error::io(path, e))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    let mut n = 0u64;
    loop {
        cancel.checkpoint()?;
        let k = f.read(&mut buf).map_err(|e| Error::io(path, e))?;
        if k == 0 {
            break;
        }
        h.update(&buf[..k]);
        n += k as u64;
    }
    Ok((h.finish_hex(), n))
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// One index row per item; `hashes` are aligned with `items` (`None` for non-files).
pub fn build(items: &[Item], hashes: &[Option<String>]) -> Vec<u8> {
    let mut out = String::from("path,size,mtime,sha256,note\n");
    for (it, h) in items.iter().zip(hashes) {
        let (size, note) = match &it.kind {
            Kind::File { size } => (size.to_string(), String::new()),
            Kind::Dir => (String::new(), "folder".to_string()),
            Kind::Link { target } => (String::new(), format!("symlink not archived -> {target}")),
        };
        let path = if it.kind == Kind::Dir { format!("{}/", it.rel) } else { it.rel.clone() };
        out.push_str(&format!(
            "{},{},{},{},{}\n",
            csv_field(&path),
            size,
            it.mtime.to_rfc3339(),
            h.as_deref().unwrap_or(""),
            csv_field(&note)
        ));
    }
    out.into_bytes()
}

/// Hash every file of `items` (in order).
pub fn hash_all(items: &[Item], cancel: &CancelToken) -> Result<Vec<Option<String>>> {
    items
        .iter()
        .map(|it| match it.kind {
            Kind::File { size } => {
                let (h, n) = hash_file(&it.abs, cancel)?;
                if n != size {
                    return Err(Error::Changed(it.abs.clone()));
                }
                Ok(Some(h))
            }
            _ => Ok(None),
        })
        .collect()
}

/// The index entry name, avoiding a clash with an archived item.
pub fn index_name(items: &[Item]) -> String {
    if items.iter().any(|i| i.rel == INDEX_NAME) {
        "INDEX (MyJanitor).csv".into()
    } else {
        INDEX_NAME.into()
    }
}

/// Path of the sidecar beside an archive.
pub fn sidecar_path(archive: &Path) -> std::path::PathBuf {
    let mut s = archive.as_os_str().to_os_string();
    s.push(".index.csv");
    s.into()
}

/// Links of `items` (reported in notes and the index).
pub fn links(items: &[Item]) -> Vec<&Item> {
    items.iter().filter(|i| matches!(i.kind, Kind::Link { .. })).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::Timestamp;
    use std::path::PathBuf;

    #[test]
    fn csv_rows_and_quoting() {
        let it = |rel: &str, kind| Item { rel: rel.into(), abs: PathBuf::new(), kind, mtime: Timestamp(0), mode: None };
        let items = vec![
            it("d", Kind::Dir),
            it("d/a,b.txt", Kind::File { size: 3 }),
            it("d/l", Kind::Link { target: "../x".into() }),
        ];
        let csv = String::from_utf8(build(&items, &[None, Some("ab".into()), None])).unwrap();
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines[0], "path,size,mtime,sha256,note");
        assert_eq!(lines[1], "d/,,1970-01-01T00:00:00Z,,folder");
        assert_eq!(lines[2], "\"d/a,b.txt\",3,1970-01-01T00:00:00Z,ab,");
        assert_eq!(lines[3], "d/l,,1970-01-01T00:00:00Z,,symlink not archived -> ../x");
        assert_eq!(index_name(&items), INDEX_NAME);
        assert_eq!(sidecar_path(Path::new("/a/b.zip")), PathBuf::from("/a/b.zip.index.csv"));
    }
}
