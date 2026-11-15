//! External formats: 7z (through `7z`) and tar.zst (in-house tar + `zstd`).
//!
//! Argument builders are pure functions (unit-tested without the tools).
//! 7z receives an explicit UTF-8 list file of the regular files to store, so
//! it never recurses into a folder and never sees a link (links are listed in
//! the index instead). Empty folders are therefore not stored in 7z archives;
//! the index lists them. Encryption and split volumes are not offered: the
//! `ArchiveJob` contract carries no password or volume size, and 7z only
//! takes a password on its command line, visible to other processes.

use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use mj_core::{CancelToken, Error, Result};

use crate::external::{path_arg, run, RunOptions};

use super::walk::{Item, Kind};

/// A 24-hour limit (archiving large folders is slow); cancel stops the tool at once.
fn opts<'a>(cancel: &'a CancelToken, cwd: Option<&'a Path>) -> RunOptions<'a> {
    RunOptions {
        timeout: Some(Duration::from_secs(24 * 3600)),
        max_stdout: 64 << 20,
        cancel: Some(cancel),
        cwd,
        ..RunOptions::default()
    }
}

/// `7z a` arguments: solid 7z at `-mx=level`, store links as links (never follow), UTF-8 list file.
pub fn seven_zip_add_args(dst: &Path, level: u8, list: Option<&Path>, names: &[&str]) -> Vec<OsString> {
    let mut a: Vec<OsString> =
        ["a", "-t7z", &format!("-mx={}", level.min(9)), "-ms=on", "-snl", "-bd", "-y", "-scsUTF-8"]
            .iter()
            .map(OsString::from)
            .collect();
    // The list file goes in as an include switch, before `--`: after the terminator newer
    // 7-Zip releases read `@file` as a literal file name and fail with "No such file".
    if let Some(l) = list {
        let mut s = OsString::from("-i@");
        s.push(l.as_os_str());
        a.push(s);
    }
    a.push("--".into());
    a.push(path_arg(dst));
    a.extend(names.iter().map(|n| path_arg(Path::new(n))));
    a
}

/// `7z t` (integrity test).
pub fn seven_zip_test_args(archive: &Path) -> Vec<OsString> {
    vec!["t".into(), "-bd".into(), "-y".into(), "--".into(), path_arg(archive)]
}

/// `7z l -slt` (technical listing with CRCs). On Windows 7-Zip writes its console output
/// in the OEM code page, which turns `résumé 日本.txt` into another name and the archive
/// into "file missing": `-sccUTF-8` asks for UTF-8 (elsewhere the output follows the
/// locale, and older p7zip builds do not know the switch).
pub fn seven_zip_list_args(archive: &Path) -> Vec<OsString> {
    let mut a: Vec<OsString> = vec!["l".into(), "-slt".into(), "-bd".into(), "-scsUTF-8".into()];
    if cfg!(windows) {
        a.push("-sccUTF-8".into());
    }
    a.push("--".into());
    a.push(path_arg(archive));
    a
}

/// `7z e -so` of one entry.
pub fn seven_zip_extract_args(archive: &Path, name: &str) -> Vec<OsString> {
    vec!["e".into(), "-so".into(), "-bd".into(), "-scsUTF-8".into(), "--".into(), path_arg(archive), name.into()]
}

/// zstd level for an `ArchiveJob` level (0-9 -> 1-19).
pub fn zstd_level(level: u8) -> u8 {
    1 + 2 * level.min(9)
}

/// `zstd` compression of `src` into `dst`.
pub fn zstd_compress_args(src: &Path, dst: &Path, level: u8) -> Vec<OsString> {
    vec![
        format!("-{}", zstd_level(level)).into(),
        "-q".into(),
        "-T0".into(),
        "-o".into(),
        dst.into(),
        "--".into(),
        path_arg(src),
    ]
}

/// `zstd -d` of `src` into `dst`.
pub fn zstd_decompress_args(src: &Path, dst: &Path) -> Vec<OsString> {
    vec!["-d".into(), "-q".into(), "-o".into(), dst.into(), "--".into(), path_arg(src)]
}

/// One file entry of a `7z l -slt` listing: (path with `/`, size, CRC-32, is folder).
pub fn parse_slt(text: &str) -> Vec<(String, u64, Option<u32>, bool)> {
    let mut out = Vec::new();
    // 7-Zip for Windows ends its lines with CRLF; the blocks are split on blank lines.
    let text = text.replace("\r\n", "\n");
    // Entries follow the "----------" separator; each is "Key = Value" lines separated by blank lines.
    let body = text.split_once("\n----------").map(|(_, b)| b).unwrap_or("");
    for block in body.split("\n\n") {
        let mut path = None;
        let mut size = 0;
        let mut crc = None;
        let mut folder = false;
        for line in block.lines() {
            let Some((k, v)) = line.split_once(" = ") else { continue };
            match k.trim() {
                "Path" => path = Some(v.replace('\\', "/")),
                "Size" => size = v.trim().parse().unwrap_or(0),
                "CRC" => crc = u32::from_str_radix(v.trim(), 16).ok(),
                "Folder" => folder = v.trim() == "+",
                "Attributes" => folder |= v.starts_with('D'),
                _ => {}
            }
        }
        if let Some(p) = path {
            out.push((p, size, crc, folder));
        }
    }
    out
}

/// CRC-32 of a file, streamed.
pub fn crc_file(path: &Path, cancel: &CancelToken) -> Result<u32> {
    let mut f = std::fs::File::open(path).map_err(|e| Error::io(path, e))?;
    let mut c = mj_formats::crc32::Crc32::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        cancel.checkpoint()?;
        let k = f.read(&mut buf).map_err(|e| Error::io(path, e))?;
        if k == 0 {
            return Ok(c.finish());
        }
        c.update(&buf[..k]);
    }
}

/// A scratch directory beside `dst` (removed by the caller).
pub fn scratch_dir(dst: &Path, tag: &str) -> Result<PathBuf> {
    let mut s = dst.as_os_str().to_os_string();
    s.push(format!(".{tag}.mjtmp"));
    let p = PathBuf::from(s);
    std::fs::create_dir(&p).map_err(|e| Error::io(&p, e))?;
    Ok(p)
}

/// Create a 7z archive: the index first, then the files of `items` from a list file.
#[allow(clippy::too_many_arguments)]
pub fn create_7z(
    tool: &Path,
    items: &[Item],
    index_bytes: &[u8],
    index_name: &str,
    base: &Path,
    dst: &Path,
    level: u8,
    cancel: &CancelToken,
) -> Result<()> {
    let tmp = scratch_dir(dst, "7z")?;
    let res = (|| {
        std::fs::write(tmp.join(index_name), index_bytes).map_err(|e| Error::io(&tmp, e))?;
        run(tool, &seven_zip_add_args(dst, level, None, &[index_name]), &opts(cancel, Some(&tmp)))?
            .ensure_success("7z")?;
        let list: String =
            items.iter().filter(|i| matches!(i.kind, Kind::File { .. })).map(|i| format!("{}\n", i.rel)).collect();
        let list_path = tmp.join("files.txt");
        std::fs::write(&list_path, list).map_err(|e| Error::io(&list_path, e))?;
        run(tool, &seven_zip_add_args(dst, level, Some(&list_path), &[]), &opts(cancel, Some(base)))?
            .ensure_success("7z")
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    res
}

/// Verify a 7z archive: `7z t`, the index content, every file present with the
/// same size and CRC-32 as the source now, nothing unexpected inside.
pub fn verify_7z(
    tool: &Path,
    archive: &Path,
    items: &[Item],
    index_bytes: &[u8],
    index_name: &str,
    cancel: &CancelToken,
) -> Result<()> {
    let bad = |m: String| Error::parse("archive integrity test", m);
    run(tool, &seven_zip_test_args(archive), &opts(cancel, None))?.ensure_success("7z")?;
    let idx = run(tool, &seven_zip_extract_args(archive, index_name), &opts(cancel, None))?;
    idx.ensure_success("7z")?;
    if idx.stdout != index_bytes {
        return Err(bad("index does not match the sources".into()));
    }
    let list = run(tool, &seven_zip_list_args(archive), &opts(cancel, None))?;
    list.ensure_success("7z")?;
    let entries = parse_slt(&String::from_utf8_lossy(&list.stdout));
    let mut files: std::collections::HashMap<&str, (u64, Option<u32>)> = std::collections::HashMap::new();
    for (p, s, c, folder) in &entries {
        if !folder {
            files.insert(p.as_str(), (*s, *c));
        }
    }
    let mut expected = 1;
    for it in items {
        if let Kind::File { size } = it.kind {
            expected += 1;
            let (s, c) = files.get(it.rel.as_str()).ok_or_else(|| bad(format!("{} missing", it.rel)))?;
            // 7z lists no CRC for an empty file; the CRC-32 of no bytes is 0.
            let listed = if *s == 0 { c.or(Some(0)) } else { *c };
            if *s != size || listed != Some(crc_file(&it.abs, cancel)?) {
                return Err(bad(format!("{}: content differs from the source", it.rel)));
            }
        }
    }
    if files.len() != expected {
        return Err(bad(format!("{} files in the archive, expected {expected}", files.len())));
    }
    Ok(())
}

/// Compress a finished tar with zstd into `dst`, removing the tar.
pub fn zstd_compress(tool: &Path, tar: &Path, dst: &Path, level: u8, cancel: &CancelToken) -> Result<()> {
    let r = run(tool, &zstd_compress_args(tar, dst, level), &opts(cancel, None)).and_then(|o| o.ensure_success("zstd"));
    let _ = std::fs::remove_file(tar);
    r
}

/// Decompress `archive` to a scratch tar and return its path (caller removes it).
/// The tar lives in a folder created exclusively for this verification (F-03): never a
/// fixed name beside the archive, which another run (or a file of the user's) could hold.
pub fn zstd_decompress(tool: &Path, archive: &Path, cancel: &CancelToken) -> Result<mj_core::scratch::ScratchDir> {
    let parent = archive.parent().unwrap_or(Path::new("."));
    let dir = mj_core::scratch::ScratchDir::new(parent, "verify")?;
    let out = dir.join("archive.tar");
    run(tool, &zstd_decompress_args(archive, &out), &opts(cancel, None)).and_then(|o| o.ensure_success("zstd"))?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[OsString]) -> Vec<String> {
        v.iter().map(|x| x.to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn builders() {
        let a = s(&seven_zip_add_args(Path::new("/o/x.7z"), 9, Some(Path::new("/t/l.txt")), &[]));
        assert_eq!(
            a,
            ["a", "-t7z", "-mx=9", "-ms=on", "-snl", "-bd", "-y", "-scsUTF-8", "-i@/t/l.txt", "--", "/o/x.7z"]
        );
        let a = s(&seven_zip_add_args(Path::new("x.7z"), 3, None, &["-INDEX.csv"]));
        assert_eq!(a.last().unwrap(), "./-INDEX.csv");
        assert_eq!(s(&seven_zip_test_args(Path::new("/o/x.7z")))[0], "t");
        assert_eq!(zstd_level(0), 1);
        assert_eq!(zstd_level(9), 19);
        let z = s(&zstd_compress_args(Path::new("/o/a.tar"), Path::new("/o/a.tar.zst"), 9));
        assert_eq!(z, ["-19", "-q", "-T0", "-o", "/o/a.tar.zst", "--", "/o/a.tar"]);
        assert_eq!(s(&zstd_decompress_args(Path::new("a"), Path::new("b")))[0], "-d");
    }

    #[test]
    fn slt_listing() {
        let text = "7-Zip 23.01\n\nListing archive: x.7z\n\n--\nPath = x.7z\nType = 7z\n\n----------\nPath = INDEX.csv\nSize = 10\nCRC = 0A0B0C0D\nAttributes = A\n\nPath = dir\\a.txt\nSize = 3\nCRC = FFFFFFFF\n\nPath = dir\nSize = 0\nFolder = +\n";
        let e = parse_slt(text);
        assert_eq!(e[0], ("INDEX.csv".into(), 10, Some(0x0A0B0C0D), false));
        assert_eq!(e[1], ("dir/a.txt".into(), 3, Some(0xFFFF_FFFF), false));
        assert!(e[2].3);
        // The Windows build writes CRLF line endings: the same entries.
        assert_eq!(parse_slt(&text.replace('\n', "\r\n")), e);
    }
}
