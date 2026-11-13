//! Path helpers: lexical normalisation, containment, comparison keys, and the
//! lossless path codec used on the wire and in persisted files.
//!
//! **Never follow links here.** Everything in this module is lexical (no
//! filesystem access), so it is safe to call on untrusted paths.
//!
//! ## Path codec (wire and persisted form)
//! A path that is valid Unicode is written as the plain string. A path that is
//! not (non-UTF-8 bytes on Linux, unpaired surrogates on Windows) is written as
//! `"\u0000"` followed by an escaped form: `%XX` per non-printable or non-ASCII
//! byte on Unix, `%uXXXX` per unpaired surrogate on Windows, `%25` for `%`.
//! NUL can never occur in a real path, so the two forms cannot collide and
//! every path round-trips exactly. The UI shows such paths with
//! [`display_lossy`]; it never needs to decode them because it refers to
//! candidates by id.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

/// Marker that prefixes an escaped (non-Unicode) path in the codec.
pub const ESCAPED_PREFIX: char = '\u{0}';

/// Encode a path losslessly into a string (see module docs).
pub fn encode_path(path: &Path) -> String {
    if let Some(s) = path.to_str() {
        return s.to_string();
    }
    let mut out = String::new();
    out.push(ESCAPED_PREFIX);
    encode_escaped(path.as_os_str(), &mut out);
    out
}

/// Decode a string produced by [`encode_path`]. Plain strings decode to themselves.
pub fn decode_path(s: &str) -> Result<PathBuf, String> {
    match s.strip_prefix(ESCAPED_PREFIX) {
        None => Ok(PathBuf::from(s)),
        Some(rest) => decode_escaped(rest).map(PathBuf::from),
    }
}

/// Human-readable form (replacement characters for undecodable parts).
pub fn display_lossy(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(unix)]
fn encode_escaped(os: &std::ffi::OsStr, out: &mut String) {
    use std::os::unix::ffi::OsStrExt;
    for &b in os.as_bytes() {
        if (0x20..0x7f).contains(&b) && b != b'%' {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
}

#[cfg(unix)]
fn decode_escaped(s: &str) -> Result<OsString, String> {
    use std::os::unix::ffi::OsStringExt;
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3).ok_or("truncated escape")?;
            out.push(u8::from_str_radix(hex, 16).map_err(|e| e.to_string())?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Ok(OsString::from_vec(out))
}

#[cfg(windows)]
fn encode_escaped(os: &std::ffi::OsStr, out: &mut String) {
    use std::os::windows::ffi::OsStrExt;
    let units: Vec<u16> = os.encode_wide().collect();
    for r in char::decode_utf16(units.iter().copied()) {
        match r {
            Ok('%') => out.push_str("%25"),
            Ok(c) => out.push(c),
            Err(e) => out.push_str(&format!("%u{:04X}", e.unpaired_surrogate())),
        }
    }
}

#[cfg(windows)]
fn decode_escaped(s: &str) -> Result<OsString, String> {
    use std::os::windows::ffi::OsStringExt;
    let mut units: Vec<u16> = Vec::with_capacity(s.len());
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c == '%' {
            if s[i + 1..].starts_with('u') {
                let hex = s.get(i + 2..i + 6).ok_or("truncated escape")?;
                units.push(u16::from_str_radix(hex, 16).map_err(|e| e.to_string())?);
                for _ in 0..5 {
                    chars.next();
                }
            } else {
                let hex = s.get(i + 1..i + 3).ok_or("truncated escape")?;
                units.push(u16::from_str_radix(hex, 16).map_err(|e| e.to_string())?);
                chars.next();
                chars.next();
            }
        } else {
            let mut buf = [0u16; 2];
            units.extend_from_slice(c.encode_utf16(&mut buf));
        }
    }
    Ok(OsString::from_wide(&units))
}

/// Serde adapters for `PathBuf` fields using the lossless codec.
///
/// Use `#[serde(with = "mj_core::pathutil::serde_path")]` on `PathBuf`,
/// `serde_path::opt` on `Option<PathBuf>` and `serde_path::vec` on `Vec<PathBuf>`.
pub mod serde_path {
    use super::{decode_path, encode_path};
    use serde::{Deserialize, Deserializer, Serializer};
    use std::path::{Path, PathBuf};

    pub fn serialize<S: Serializer>(p: &Path, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&encode_path(p))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<PathBuf, D::Error> {
        let s = String::deserialize(d)?;
        decode_path(&s).map_err(serde::de::Error::custom)
    }

    /// `Option<PathBuf>`.
    pub mod opt {
        use super::*;
        pub fn serialize<S: Serializer>(p: &Option<PathBuf>, s: S) -> Result<S::Ok, S::Error> {
            match p {
                Some(p) => s.serialize_some(&encode_path(p)),
                None => s.serialize_none(),
            }
        }
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<PathBuf>, D::Error> {
            match Option::<String>::deserialize(d)? {
                Some(s) => decode_path(&s).map(Some).map_err(serde::de::Error::custom),
                None => Ok(None),
            }
        }
    }

    /// `Vec<PathBuf>`.
    pub mod vec {
        use super::*;
        use serde::ser::SerializeSeq;
        pub fn serialize<S: Serializer>(v: &[PathBuf], s: S) -> Result<S::Ok, S::Error> {
            let mut seq = s.serialize_seq(Some(v.len()))?;
            for p in v {
                seq.serialize_element(&encode_path(p))?;
            }
            seq.end()
        }
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<PathBuf>, D::Error> {
            Vec::<String>::deserialize(d)?.iter().map(|s| decode_path(s).map_err(serde::de::Error::custom)).collect()
        }
    }
}

/// Resolve `.` and `..` components lexically (no filesystem access, no link
/// resolution). `..` above the root is dropped.
pub fn normalize_lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                let can_pop = matches!(out.components().next_back(), Some(Component::Normal(_)));
                if can_pop {
                    out.pop();
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// True when paths compare case-insensitively on this platform (Windows).
pub const CASE_INSENSITIVE: bool = cfg!(windows);

/// Comparison key for a path: lexically normalised, separators unified to `/`,
/// case-folded on Windows. Use it for cache keys, de-duplication and the
/// protected-path / allowlist checks. Not a displayable path.
pub fn path_key(path: &Path) -> String {
    let norm = normalize_lexical(path);
    let mut s = encode_path(&norm);
    if cfg!(windows) {
        s = s.replace('\\', "/");
        s = s.to_lowercase();
        // `\\?\C:\x` and `C:\x` are the same path.
        if let Some(rest) = s.strip_prefix("//?/") {
            if !rest.starts_with("unc/") {
                s = rest.to_string();
            }
        }
    }
    while s.len() > 1 && s.ends_with('/') && !s.ends_with(":/") {
        s.pop();
    }
    s
}

/// True when `path` is `root` or lies below it, compared lexically by
/// component after normalisation (case-insensitive on Windows).
/// `/home/a` is **not** within `/home/ab`.
pub fn is_within(root: &Path, path: &Path) -> bool {
    let r = path_key(root);
    let p = path_key(path);
    if p == r {
        return true;
    }
    let prefix = if r.ends_with('/') { r } else { format!("{r}/") };
    p.starts_with(&prefix)
}

/// Number of components of `path` below `root` (0 for the root itself), or
/// `None` when `path` is not within `root`.
pub fn depth_below(root: &Path, path: &Path) -> Option<u32> {
    if !is_within(root, path) {
        return None;
    }
    let r = normalize_lexical(root).components().count();
    let p = normalize_lexical(path).components().count();
    Some(p.saturating_sub(r) as u32)
}

/// Lower-case extension without the dot (`"JPG"` -> `"jpg"`), or `""`.
pub fn extension_lower(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// File name as a lossy string, or `""`.
pub fn file_name_lossy(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Characters that are invalid in Windows file names (CLN-FLT-08), checked on every platform
/// because files are often shared with Windows machines.
pub const WINDOWS_INVALID_CHARS: &[char] = &['<', '>', ':', '"', '|', '?', '*'];

/// Windows reserved device names (matched case-insensitively on the stem before the first dot).
pub const WINDOWS_RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2",
    "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// True when a single file name is not portable to Windows: invalid characters,
/// control characters, a reserved device name, or a trailing dot or space.
pub fn has_invalid_windows_name(name: &str) -> bool {
    if name.chars().any(|c| WINDOWS_INVALID_CHARS.contains(&c) || (c as u32) < 0x20) {
        return true;
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return name != "." && name != "..";
    }
    let stem = name.split('.').next().unwrap_or("");
    WINDOWS_RESERVED_NAMES.iter().any(|r| r.eq_ignore_ascii_case(stem))
}

/// Pick a non-existing sibling name: `name.ext`, `name (1).ext`, `name (2).ext`...
/// `exists` is injected so the caller decides how existence is checked.
pub fn unique_name(path: &Path, exists: impl Fn(&Path) -> bool) -> PathBuf {
    if !exists(path) {
        return path.to_path_buf();
    }
    let parent = path.parent().unwrap_or(Path::new(""));
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    for n in 1u32.. {
        let candidate = parent.join(format!("{stem} ({n}){ext}"));
        if !exists(&candidate) {
            return candidate;
        }
    }
    unreachable!("u32 exhausted")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_dots() {
        assert_eq!(normalize_lexical(Path::new("/a/./b/../c")), PathBuf::from("/a/c"));
        assert_eq!(normalize_lexical(Path::new("/../a")), PathBuf::from("/a"));
        assert_eq!(normalize_lexical(Path::new("a/../../b")), PathBuf::from("b"));
    }

    #[test]
    fn within_is_component_wise() {
        assert!(is_within(Path::new("/home/a"), Path::new("/home/a")));
        assert!(is_within(Path::new("/home/a"), Path::new("/home/a/x/y")));
        assert!(!is_within(Path::new("/home/a"), Path::new("/home/ab")));
        assert!(!is_within(Path::new("/home/a"), Path::new("/home/a/../b")));
        assert!(is_within(Path::new("/"), Path::new("/etc")));
        assert_eq!(depth_below(Path::new("/r"), Path::new("/r/a/b")), Some(2));
        assert_eq!(depth_below(Path::new("/r"), Path::new("/s")), None);
    }

    #[test]
    fn codec_round_trips_plain_paths() {
        for p in ["/a/b c/ü.txt", "100% done.txt", ""] {
            let e = encode_path(Path::new(p));
            assert_eq!(e, p);
            assert_eq!(decode_path(&e).unwrap(), PathBuf::from(p));
        }
    }

    #[cfg(unix)]
    #[test]
    fn codec_round_trips_non_utf8() {
        use std::os::unix::ffi::OsStringExt;
        let p = PathBuf::from(OsString::from_vec(b"/tmp/bad\xff%name".to_vec()));
        let e = encode_path(&p);
        assert!(e.starts_with(ESCAPED_PREFIX));
        assert_eq!(decode_path(&e).unwrap(), p);
        let json = serde_json_like(&e);
        assert!(json.contains("\\u0000"));
    }

    #[cfg(unix)]
    fn serde_json_like(s: &str) -> String {
        s.chars().map(|c| if c == '\0' { "\\u0000".to_string() } else { c.to_string() }).collect()
    }

    #[test]
    fn invalid_windows_names() {
        assert!(has_invalid_windows_name("a:b"));
        assert!(has_invalid_windows_name("CON"));
        assert!(has_invalid_windows_name("con.txt"));
        assert!(has_invalid_windows_name("trailing."));
        assert!(!has_invalid_windows_name("console.txt"));
        assert!(!has_invalid_windows_name("ok name.txt"));
    }

    #[test]
    fn unique_name_appends_counter() {
        let taken = [PathBuf::from("/d/a.txt"), PathBuf::from("/d/a (1).txt")];
        let got = unique_name(Path::new("/d/a.txt"), |p| taken.iter().any(|t| t == p));
        assert_eq!(got, PathBuf::from("/d/a (2).txt"));
    }

    #[test]
    fn extension_is_lowercased() {
        assert_eq!(extension_lower(Path::new("/x/IMG.JPG")), "jpg");
        assert_eq!(extension_lower(Path::new("/x/noext")), "");
    }
}
