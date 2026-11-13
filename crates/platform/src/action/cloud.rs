//! Cloud placeholders: dehydrate / hydrate (`CfDehydratePlaceholder`,
//! `CfHydratePlaceholder` and the pin attributes) and synced-folder roots
//! (OneDrive environment variables and registry keys, Dropbox `info.json`,
//! Nextcloud config, well-known folder names and marker files). Owner: W2-E.
//!
//! **Sync detection never claims "not synced" when it could not look.** When the
//! user's home folder is unknown, [`detect_sync_roots`] fails (`Unsupported`), and
//! callers report the sync status as *unknown*. An empty list means "no root of a
//! provider we know was found"; providers outside the list (or a renamed folder
//! without a marker) are a known gap, documented in docs/decisions/W2-E.md, and
//! the caller may supply extra roots.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use mj_core::{Error, Result};

use crate::traits::{SyncProvider, SyncRoot};

/// "Free up space" (CLN-ACT-07). Windows only.
pub fn dehydrate(path: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        win::dehydrate(path)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err(Error::unsupported("Free up space", "cloud placeholders are Windows only"))
    }
}

/// Bring content back locally (undo of dehydrate). Windows only.
pub fn hydrate(path: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        win::hydrate(path)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err(Error::unsupported("Hydrate", "cloud placeholders are Windows only"))
    }
}

/// Synced folder roots of the current user (environment, registry on Windows, folders).
pub fn sync_roots() -> Result<Vec<SyncRoot>> {
    let env = SyncEnv::from_process();
    #[allow(unused_mut)]
    let mut roots = detect_sync_roots(&env)?;
    #[cfg(windows)]
    {
        for r in win::registry_roots() {
            push_unique(&mut roots, r);
        }
    }
    Ok(roots)
}

// ---------------------------------------------------------------- pure helpers (tested everywhere)

/// `FILE_ATTRIBUTE_PINNED`.
pub const ATTR_PINNED: u32 = 0x0008_0000;
/// `FILE_ATTRIBUTE_UNPINNED`.
pub const ATTR_UNPINNED: u32 = 0x0010_0000;
/// Attributes `SetFileAttributesW` accepts (others are ignored or rejected).
pub const ATTR_SETTABLE: u32 = 0x1 | 0x2 | 0x4 | 0x20 | 0x100 | 0x1000 | 0x2000 | ATTR_PINNED | ATTR_UNPINNED;

/// True for the Cloud Files reparse tags (`IO_REPARSE_TAG_CLOUD`, `_CLOUD_1` ... `_CLOUD_F`).
pub fn is_cloud_tag(tag: u32) -> bool {
    tag & 0xFFFF_0FFF == 0x9000_001A
}

/// Attributes to set for "free up space": unpinned, not pinned.
pub fn dehydrate_attributes(current: u32) -> u32 {
    (current & ATTR_SETTABLE & !ATTR_PINNED) | ATTR_UNPINNED
}

/// Attributes to set when bringing a file back: neither unpinned nor (newly) pinned.
pub fn hydrate_attributes(current: u32) -> u32 {
    current & ATTR_SETTABLE & !ATTR_UNPINNED
}

/// Inputs of sync-root detection, explicit so tests never read the real environment.
#[derive(Debug, Clone, Default)]
pub struct SyncEnv {
    /// The user's home / profile folder.
    pub home: Option<PathBuf>,
    /// Relevant environment variables (`OneDrive`, `OneDriveCommercial`, `OneDriveConsumer`,
    /// `APPDATA`, `LOCALAPPDATA`, `XDG_CONFIG_HOME`).
    pub vars: Vec<(String, OsString)>,
}

impl SyncEnv {
    /// From the running process.
    pub fn from_process() -> SyncEnv {
        let home = if cfg!(windows) { std::env::var_os("USERPROFILE") } else { std::env::var_os("HOME") }
            .map(PathBuf::from)
            .filter(|p| p.is_absolute());
        let vars = ["OneDrive", "OneDriveCommercial", "OneDriveConsumer", "APPDATA", "LOCALAPPDATA", "XDG_CONFIG_HOME"]
            .iter()
            .filter_map(|k| std::env::var_os(k).map(|v| (k.to_string(), v)))
            .collect();
        SyncEnv { home, vars }
    }

    fn var(&self, k: &str) -> Option<PathBuf> {
        self.vars
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(k))
            .map(|(_, v)| PathBuf::from(v))
            .filter(|p| p.is_absolute())
    }
}

fn push_unique(v: &mut Vec<SyncRoot>, r: SyncRoot) {
    let k = mj_core::pathutil::path_key(&r.path);
    if !v.iter().any(|x| mj_core::pathutil::path_key(&x.path) == k) {
        v.push(r);
    }
}

fn root(provider: SyncProvider, name: &str, path: PathBuf) -> SyncRoot {
    let can_dehydrate = cfg!(windows)
        && matches!(provider, SyncProvider::OneDrive | SyncProvider::OneDriveBusiness | SyncProvider::SharePoint);
    SyncRoot { provider, name: name.to_string(), path, can_dehydrate }
}

/// Provider for a well-known folder name directly under the home folder.
pub fn provider_for_folder_name(name: &str) -> Option<(SyncProvider, String)> {
    let n = name.trim();
    if n.eq_ignore_ascii_case("OneDrive") || n.eq_ignore_ascii_case("OneDrive - Personal") {
        return Some((SyncProvider::OneDrive, "OneDrive".into()));
    }
    if n.len() > 11 && n[..11].eq_ignore_ascii_case("OneDrive - ") {
        return Some((SyncProvider::OneDriveBusiness, n.to_string()));
    }
    if n.eq_ignore_ascii_case("Dropbox") || (n.starts_with("Dropbox (") && n.ends_with(')')) {
        return Some((SyncProvider::Dropbox, n.to_string()));
    }
    if n.eq_ignore_ascii_case("Google Drive")
        || n.eq_ignore_ascii_case("My Drive")
        || n.eq_ignore_ascii_case("GoogleDrive")
    {
        return Some((SyncProvider::GoogleDrive, "Google Drive".into()));
    }
    if n.eq_ignore_ascii_case("Nextcloud") || n.eq_ignore_ascii_case("ownCloud") {
        return Some((SyncProvider::Nextcloud, n.to_string()));
    }
    for other in
        ["iCloudDrive", "iCloud Drive", "Box", "Box Sync", "pCloud Drive", "MEGA", "MEGAsync", "Sync", "Tresorit"]
    {
        if n.eq_ignore_ascii_case(other) {
            return Some((SyncProvider::Other, n.to_string()));
        }
    }
    None
}

/// Marker files that identify a sync root whatever its name.
pub fn provider_for_markers(dir: &Path) -> Option<SyncProvider> {
    let has = |n: &str| std::fs::symlink_metadata(dir.join(n)).is_ok();
    if has(".dropbox") || has(".dropbox.cache") {
        Some(SyncProvider::Dropbox)
    } else if has(".sync_root_marker") || has(".owncloudsync.log") || has(".sync_journal.db") {
        Some(SyncProvider::Nextcloud)
    } else if has(".tmp.drivedownload") || has(".shortcut-targets-by-id") {
        Some(SyncProvider::GoogleDrive)
    } else {
        None
    }
}

/// Every `"path": "..."` string value in a JSON text (Dropbox `info.json`), unescaped.
pub fn json_path_values(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let b = text.as_bytes();
    let key = b"\"path\"";
    let mut i = 0;
    while let Some(p) = find(b, key, i) {
        let mut j = p + key.len();
        while j < b.len() && (b[j] as char).is_ascii_whitespace() {
            j += 1;
        }
        if b.get(j) != Some(&b':') {
            i = p + 1;
            continue;
        }
        j += 1;
        while j < b.len() && (b[j] as char).is_ascii_whitespace() {
            j += 1;
        }
        if b.get(j) != Some(&b'"') {
            i = p + 1;
            continue;
        }
        j += 1;
        let mut s = String::new();
        let mut chars = text[j..].chars();
        let mut ok = false;
        while let Some(c) = chars.next() {
            match c {
                '"' => {
                    ok = true;
                    break;
                }
                '\\' => match chars.next() {
                    Some('n') => s.push('\n'),
                    Some('t') => s.push('\t'),
                    Some('u') => {
                        let hex: String = chars.by_ref().take(4).collect();
                        if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                            s.push(ch);
                        }
                    }
                    Some(other) => s.push(other),
                    None => break,
                },
                c => s.push(c),
            }
        }
        if ok && !s.is_empty() {
            out.push(s);
        }
        i = p + 1;
    }
    out
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from >= hay.len() {
        return None;
    }
    hay[from..].windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

/// `key = value` or `key=value` lines of an INI-like config, value unquoted.
pub fn config_values(text: &str, key: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            let (k, v) = l.split_once('=')?;
            let k = k.trim();
            // Nextcloud stores `0\Folders\1\localPath=/home/me/Nextcloud/`.
            let k = k.rsplit(['\\', '/']).next().unwrap_or(k);
            if k.eq_ignore_ascii_case(key) {
                Some(v.trim().trim_matches('"').to_string())
            } else {
                None
            }
        })
        .filter(|v| !v.is_empty())
        .collect()
}

fn read_small(p: &Path) -> Option<String> {
    use std::io::Read;
    let f = std::fs::File::open(p).ok()?;
    let mut s = String::new();
    f.take(256 * 1024).read_to_string(&mut s).ok()?;
    Some(s)
}

fn is_dir(p: &Path) -> bool {
    std::fs::symlink_metadata(p).map(|m| m.is_dir()).unwrap_or(false)
}

/// Detect sync roots from `env` (folders must exist to be reported).
pub fn detect_sync_roots(env: &SyncEnv) -> Result<Vec<SyncRoot>> {
    let home =
        env.home.clone().ok_or_else(|| Error::unsupported("Synced folder detection", "the home folder is unknown"))?;
    let mut roots = Vec::new();
    // 1. OneDrive environment variables (set by the Windows client for each account).
    for (var, provider) in [
        ("OneDriveCommercial", SyncProvider::OneDriveBusiness),
        ("OneDriveConsumer", SyncProvider::OneDrive),
        ("OneDrive", SyncProvider::OneDrive),
    ] {
        if let Some(p) = env.var(var).filter(|p| is_dir(p)) {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "OneDrive".into());
            push_unique(&mut roots, root(provider, &name, p));
        }
    }
    // 2. Client configuration files.
    let mut configs: Vec<(PathBuf, SyncProvider, &str)> = Vec::new();
    for base in [env.var("APPDATA"), env.var("LOCALAPPDATA"), Some(home.clone())].into_iter().flatten() {
        configs.push((base.join("Dropbox").join("info.json"), SyncProvider::Dropbox, "json"));
    }
    configs.push((home.join(".dropbox").join("info.json"), SyncProvider::Dropbox, "json"));
    let xdg_config = env.var("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));
    configs.push((xdg_config.join("Nextcloud").join("nextcloud.cfg"), SyncProvider::Nextcloud, "localPath"));
    if let Some(appdata) = env.var("APPDATA") {
        configs.push((appdata.join("Nextcloud").join("nextcloud.cfg"), SyncProvider::Nextcloud, "localPath"));
    }
    configs.push((xdg_config.join("onedrive").join("config"), SyncProvider::OneDrive, "sync_dir"));
    for (file, provider, how) in configs {
        let Some(text) = read_small(&file) else { continue };
        let values = if how == "json" { json_path_values(&text) } else { config_values(&text, how) };
        for v in values {
            let v = v.strip_prefix("~/").map(|r| home.join(r)).unwrap_or_else(|| PathBuf::from(&v));
            if v.is_absolute() && is_dir(&v) {
                let name = v.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                push_unique(&mut roots, root(provider, &name, v));
            }
        }
    }
    // 3. Well-known folder names and marker files directly under the home folder.
    if let Ok(rd) = std::fs::read_dir(&home) {
        for e in rd.flatten() {
            let p = e.path();
            if !e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            if let Some((provider, display)) = provider_for_folder_name(&name) {
                push_unique(&mut roots, root(provider, &display, p));
            } else if let Some(provider) = provider_for_markers(&p) {
                push_unique(&mut roots, root(provider, &name, p));
            }
        }
    }
    Ok(roots)
}

#[cfg(windows)]
#[allow(non_camel_case_types, non_snake_case, clippy::upper_case_acronyms)]
mod win {
    use std::ffi::c_void;
    use std::path::{Path, PathBuf};

    use mj_core::{Error, Result};

    use super::{dehydrate_attributes, hydrate_attributes, is_cloud_tag, root, SyncProvider, SyncRoot};
    use crate::ffi::windows::{open_no_follow, wide, wide_path, BOOL, DWORD, HANDLE, LPCWSTR, OVERLAPPED};

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    const FILE_READ_ATTRIBUTES: DWORD = 0x80;
    const FILE_WRITE_ATTRIBUTES: DWORD = 0x100;
    /// `FILE_INFO_BY_HANDLE_CLASS::FileAttributeTagInfo`.
    const FILE_ATTRIBUTE_TAG_INFO_CLASS: u32 = 9;

    #[repr(C)]
    #[allow(non_snake_case)]
    struct FILE_ATTRIBUTE_TAG_INFO {
        FileAttributes: DWORD,
        ReparseTag: DWORD,
    }

    const _: () = assert!(std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() == 8);

    #[link(name = "kernel32")]
    extern "system" {
        fn GetFileInformationByHandleEx(
            hFile: HANDLE,
            FileInformationClass: u32,
            lpFileInformation: *mut c_void,
            dwBufferSize: DWORD,
        ) -> BOOL;
        fn SetFileAttributesW(lpFileName: LPCWSTR, dwFileAttributes: DWORD) -> BOOL;
    }

    #[link(name = "cldapi")]
    extern "system" {
        // HRESULT CfDehydratePlaceholder(HANDLE, LARGE_INTEGER StartingOffset,
        //     LARGE_INTEGER Length, CF_DEHYDRATE_FLAGS, LPOVERLAPPED)
        fn CfDehydratePlaceholder(
            FileHandle: HANDLE,
            StartingOffset: i64,
            Length: i64,
            DehydrateFlags: u32,
            Overlapped: *mut OVERLAPPED,
        ) -> i32;
        // HRESULT CfHydratePlaceholder(HANDLE, LARGE_INTEGER StartingOffset,
        //     LARGE_INTEGER Length, CF_HYDRATE_FLAGS, LPOVERLAPPED)
        fn CfHydratePlaceholder(
            FileHandle: HANDLE,
            StartingOffset: i64,
            Length: i64,
            HydrateFlags: u32,
            Overlapped: *mut OVERLAPPED,
        ) -> i32;
    }

    /// Attributes and reparse tag through an attribute-only handle (no data access, so
    /// no hydration is triggered).
    fn tag_info(path: &Path) -> Result<(u32, u32)> {
        let h = open_no_follow(path, FILE_READ_ATTRIBUTES).map_err(|e| Error::io(path, e))?;
        let mut info = FILE_ATTRIBUTE_TAG_INFO { FileAttributes: 0, ReparseTag: 0 };
        // SAFETY: `h` is a valid handle; `info` is a correctly sized FILE_ATTRIBUTE_TAG_INFO.
        let ok = unsafe {
            GetFileInformationByHandleEx(
                h.0,
                FILE_ATTRIBUTE_TAG_INFO_CLASS,
                &mut info as *mut _ as *mut c_void,
                std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as DWORD,
            )
        };
        if ok == 0 {
            return Err(Error::io(path, std::io::Error::last_os_error()));
        }
        Ok((info.FileAttributes, info.ReparseTag))
    }

    fn require_placeholder(path: &Path) -> Result<u32> {
        let (attrs, tag) = tag_info(path)?;
        if attrs & FILE_ATTRIBUTE_REPARSE_POINT == 0 || !is_cloud_tag(tag) {
            return Err(Error::unsupported("Free up space", format!("{} is not a cloud file", path.display())));
        }
        Ok(attrs)
    }

    fn set_attrs(path: &Path, attrs: u32) -> Result<()> {
        let w = wide_path(path);
        // SAFETY: `w` is NUL-terminated and outlives the call.
        if unsafe { SetFileAttributesW(w.as_ptr(), attrs) } == 0 {
            return Err(Error::io(path, std::io::Error::last_os_error()));
        }
        Ok(())
    }

    pub fn dehydrate(path: &Path) -> Result<()> {
        let attrs = require_placeholder(path)?;
        // The documented user-level mechanism: unpinned files are dehydrated by the sync engine.
        set_attrs(path, dehydrate_attributes(attrs))?;
        if let Ok(h) = open_no_follow(path, FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES) {
            // SAFETY: valid handle; offset 0 and length -1 (to the end) as documented; synchronous.
            let _hr = unsafe { CfDehydratePlaceholder(h.0, 0, -1, 0, std::ptr::null_mut()) };
            // A failure here is not fatal: the unpinned attribute makes the provider dehydrate.
        }
        Ok(())
    }

    pub fn hydrate(path: &Path) -> Result<()> {
        let attrs = require_placeholder(path)?;
        set_attrs(path, hydrate_attributes(attrs))?;
        let h = open_no_follow(path, FILE_READ_ATTRIBUTES).map_err(|e| Error::io(path, e))?;
        // SAFETY: valid handle; whole file; synchronous call.
        let hr = unsafe { CfHydratePlaceholder(h.0, 0, -1, 0, std::ptr::null_mut()) };
        if hr < 0 {
            return Err(Error::Io {
                path: Some(path.to_path_buf()),
                kind: "other",
                message: format!("CfHydratePlaceholder failed with HRESULT {hr:#x}"),
            });
        }
        Ok(())
    }

    // ---- registry (advapi32)

    type HKEY = *mut c_void;
    const HKEY_CURRENT_USER: HKEY = 0x8000_0001u32 as i32 as isize as HKEY;
    const KEY_READ: DWORD = 0x20019;
    const REG_SZ: DWORD = 1;
    const REG_EXPAND_SZ: DWORD = 2;

    #[link(name = "advapi32")]
    extern "system" {
        fn RegOpenKeyExW(
            hKey: HKEY,
            lpSubKey: LPCWSTR,
            ulOptions: DWORD,
            samDesired: DWORD,
            phkResult: *mut HKEY,
        ) -> i32;
        fn RegEnumKeyExW(
            hKey: HKEY,
            dwIndex: DWORD,
            lpName: *mut u16,
            lpcchName: *mut DWORD,
            lpReserved: *mut DWORD,
            lpClass: *mut u16,
            lpcchClass: *mut DWORD,
            lpftLastWriteTime: *mut c_void,
        ) -> i32;
        fn RegQueryValueExW(
            hKey: HKEY,
            lpValueName: LPCWSTR,
            lpReserved: *mut DWORD,
            lpType: *mut DWORD,
            lpData: *mut u8,
            lpcbData: *mut DWORD,
        ) -> i32;
        fn RegCloseKey(hKey: HKEY) -> i32;
    }

    struct Key(HKEY);

    impl Drop for Key {
        fn drop(&mut self) {
            // SAFETY: the key was opened by RegOpenKeyExW and is closed exactly once.
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }

    fn open(parent: HKEY, sub: &str) -> Option<Key> {
        let w = wide(std::ffi::OsStr::new(sub));
        let mut h: HKEY = std::ptr::null_mut();
        // SAFETY: `w` is NUL-terminated; `h` receives the opened key.
        let rc = unsafe { RegOpenKeyExW(parent, w.as_ptr(), 0, KEY_READ, &mut h) };
        (rc == 0).then_some(Key(h))
    }

    fn subkeys(k: &Key) -> Vec<String> {
        let mut out = Vec::new();
        for i in 0..1024u32 {
            let mut buf = [0u16; 256];
            let mut len: DWORD = buf.len() as DWORD;
            // SAFETY: `buf` holds `len` characters; optional outputs are null.
            let rc = unsafe {
                RegEnumKeyExW(
                    k.0,
                    i,
                    buf.as_mut_ptr(),
                    &mut len,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if rc != 0 {
                break;
            }
            out.push(String::from_utf16_lossy(&buf[..(len as usize).min(buf.len())]));
        }
        out
    }

    fn string_value(k: &Key, name: &str) -> Option<String> {
        let w = wide(std::ffi::OsStr::new(name));
        let mut ty: DWORD = 0;
        let mut buf = vec![0u8; 4096];
        let mut len: DWORD = buf.len() as DWORD;
        // SAFETY: `buf` holds `len` bytes; `w` is NUL-terminated.
        let rc =
            unsafe { RegQueryValueExW(k.0, w.as_ptr(), std::ptr::null_mut(), &mut ty, buf.as_mut_ptr(), &mut len) };
        if rc != 0 || !(ty == REG_SZ || ty == REG_EXPAND_SZ) {
            return None;
        }
        let units: Vec<u16> = buf[..(len as usize).min(buf.len())]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|&c| c != 0)
            .collect();
        let s = String::from_utf16_lossy(&units);
        (!s.is_empty()).then_some(s)
    }

    /// OneDrive accounts (`Software\Microsoft\OneDrive\Accounts\*\UserFolder`) and every
    /// synced library (`Software\SyncEngines\Providers\OneDrive\*\MountPoint`, which
    /// includes SharePoint / Teams libraries).
    pub fn registry_roots() -> Vec<SyncRoot> {
        let mut out = Vec::new();
        if let Some(acc) = open(HKEY_CURRENT_USER, "Software\\Microsoft\\OneDrive\\Accounts") {
            for name in subkeys(&acc) {
                if let Some(k) = open(acc.0, &name) {
                    if let Some(folder) = string_value(&k, "UserFolder") {
                        let provider = if name.eq_ignore_ascii_case("Personal") {
                            SyncProvider::OneDrive
                        } else {
                            SyncProvider::OneDriveBusiness
                        };
                        let p = PathBuf::from(&folder);
                        let disp = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(folder);
                        out.push(root(provider, &disp, p));
                    }
                }
            }
        }
        if let Some(prov) = open(HKEY_CURRENT_USER, "Software\\SyncEngines\\Providers\\OneDrive") {
            for name in subkeys(&prov) {
                if let Some(k) = open(prov.0, &name) {
                    if let Some(mount) = string_value(&k, "MountPoint") {
                        let p = PathBuf::from(&mount);
                        let known = out.iter().any(|r: &SyncRoot| mj_core::pathutil::is_within(&r.path, &p));
                        if !known {
                            let disp = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(mount);
                            out.push(root(SyncProvider::SharePoint, &disp, p));
                        }
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::testutil::TempDir;

    #[test]
    fn attribute_maths() {
        assert!(is_cloud_tag(0x9000_001A));
        assert!(is_cloud_tag(0x9000_301A));
        assert!(!is_cloud_tag(0xA000_000C)); // symlink
        assert!(!is_cloud_tag(0xA000_0003)); // junction
        let a = dehydrate_attributes(0x20 | ATTR_PINNED | 0x400);
        assert_eq!(a, 0x20 | ATTR_UNPINNED);
        assert_eq!(hydrate_attributes(0x20 | ATTR_UNPINNED | 0x40_0000), 0x20);
    }

    #[test]
    fn non_windows_dehydrate_is_unsupported() {
        if !cfg!(windows) {
            assert_eq!(dehydrate(Path::new("/x")).unwrap_err().code(), "unsupported");
            assert_eq!(hydrate(Path::new("/x")).unwrap_err().code(), "unsupported");
        }
    }

    #[test]
    fn folder_names_and_parsers() {
        assert_eq!(provider_for_folder_name("OneDrive").unwrap().0, SyncProvider::OneDrive);
        assert_eq!(
            provider_for_folder_name("OneDrive - Acme").unwrap(),
            (SyncProvider::OneDriveBusiness, "OneDrive - Acme".into())
        );
        assert_eq!(provider_for_folder_name("Dropbox (Acme)").unwrap().0, SyncProvider::Dropbox);
        assert_eq!(provider_for_folder_name("Google Drive").unwrap().0, SyncProvider::GoogleDrive);
        assert!(provider_for_folder_name("Documents").is_none());
        let json = r#"{"personal": {"path": "C:\\Users\\me\\Dropbox", "host": 1}, "business": {"path" : "/home/me/Dropbox \"Acme\"", "x":"\u00e9"}}"#;
        assert_eq!(json_path_values(json), ["C:\\Users\\me\\Dropbox", "/home/me/Dropbox \"Acme\""]);
        let cfg = "[Accounts]\n0\\Folders\\1\\localPath=/home/me/Nextcloud/\nother=1\n";
        assert_eq!(config_values(cfg, "localPath"), ["/home/me/Nextcloud/"]);
        assert_eq!(config_values("sync_dir = \"~/OneDrive\"\n", "sync_dir"), ["~/OneDrive"]);
    }

    #[test]
    fn detects_roots_in_a_fixture_home() {
        assert_eq!(detect_sync_roots(&SyncEnv::default()).unwrap_err().code(), "unsupported", "unknown, not empty");
        let t = TempDir::new("sync").unwrap();
        let home = t.mkdir("home").unwrap();
        t.mkdir("home/OneDrive - Acme").unwrap();
        t.mkdir("home/Documents").unwrap();
        t.write("home/Renamed Box/.dropbox", b"").unwrap();
        t.mkdir("elsewhere/Dropbox Work").unwrap();
        t.write(
            "home/.dropbox/info.json",
            format!(r#"{{"business":{{"path":"{}"}}}}"#, t.join("elsewhere/Dropbox Work").display()).as_bytes(),
        )
        .unwrap();
        t.mkdir("od").unwrap();
        let env = SyncEnv {
            home: Some(home.clone()),
            vars: vec![("OneDriveConsumer".into(), t.join("od").into_os_string())],
        };
        let roots = detect_sync_roots(&env).unwrap();
        let has = |p: PathBuf, prov: SyncProvider| roots.iter().any(|r| r.path == p && r.provider == prov);
        assert!(has(t.join("od"), SyncProvider::OneDrive), "{roots:?}");
        assert!(has(home.join("OneDrive - Acme"), SyncProvider::OneDriveBusiness), "{roots:?}");
        assert!(has(home.join("Renamed Box"), SyncProvider::Dropbox), "{roots:?}");
        assert!(has(t.join("elsewhere/Dropbox Work"), SyncProvider::Dropbox), "{roots:?}");
        assert!(!roots.iter().any(|r| r.path == home.join("Documents")));
        let empty = TempDir::new("sync-empty").unwrap();
        let none = detect_sync_roots(&SyncEnv { home: Some(empty.path().to_path_buf()), vars: vec![] }).unwrap();
        assert!(none.is_empty());
    }
}
