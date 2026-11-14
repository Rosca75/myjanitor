//! The Windows long-path (`\\?\`) form of a path, as UTF-16 code units. Pure, so it is
//! compiled and tested on every platform; `ffi::windows::wide_path` uses it.
//!
//! A verbatim (`\\?\`) path skips the Win32 path normaliser: `/` is not a separator
//! there, and `.` and `..` are ordinary names. So a prefix may only be added to a path
//! that is already in the normalised form, or the call fails with
//! `ERROR_INVALID_NAME` (123) at best, and at worst names another file. Before adding
//! the prefix, [`verbatim`] therefore does what the normaliser would have done to a
//! drive-absolute or UNC path: `/` becomes `\`, repeated separators collapse, `.`
//! components are dropped and `..` removes the previous component (never above the
//! drive or the share). Trailing dots and spaces are kept: the normaliser would strip
//! them, which is exactly the ambiguity the action guard refuses (F-19), and keeping
//! them names the file that really has that name.
//!
//! Paths that already carry a `\\?\` or `\\.\` prefix, and relative or drive-relative
//! paths (`a\b`, `C:a`, `\a`), are returned unchanged: the first are already verbatim,
//! the others are resolved by the OS against the current directory, which only works
//! without the prefix.

const BS: u16 = b'\\' as u16;
const FS: u16 = b'/' as u16;
const DOT: u16 = b'.' as u16;
const COLON: u16 = b':' as u16;

fn is_sep(c: u16) -> bool {
    c == BS || c == FS
}

fn is_drive_letter(c: u16) -> bool {
    (u16::from(b'a')..=u16::from(b'z')).contains(&c) || (u16::from(b'A')..=u16::from(b'Z')).contains(&c)
}

/// Push `parts` after `head`, separated by `\`, resolving `.` and `..`.
fn join_normalised(head: &[u16], rest: &[u16]) -> Vec<u16> {
    let mut parts: Vec<&[u16]> = Vec::new();
    for p in rest.split(|&c| is_sep(c)) {
        if p.is_empty() || p == [DOT] {
            continue;
        }
        if p == [DOT, DOT] {
            parts.pop();
            continue;
        }
        parts.push(p);
    }
    let mut out = head.to_vec();
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            out.push(BS);
        }
        out.extend_from_slice(p);
    }
    out
}

/// The verbatim form of `raw` (no NUL terminator), see the module docs.
pub fn verbatim(raw: &[u16]) -> Vec<u16> {
    // Already verbatim (`\\?\`) or a device path (`\\.\`): unchanged. Only the exact
    // backslash spelling is a prefix; `//?/` is normalised like any UNC path would be.
    if raw.len() >= 4 && raw[0] == BS && raw[1] == BS && (raw[2] == u16::from(b'?') || raw[2] == DOT) && raw[3] == BS {
        return raw.to_vec();
    }
    // `C:\x` or `C:/x`.
    if raw.len() >= 3 && is_drive_letter(raw[0]) && raw[1] == COLON && is_sep(raw[2]) {
        let mut head: Vec<u16> = r"\\?\".encode_utf16().collect();
        head.extend_from_slice(&raw[..2]);
        head.push(BS);
        return join_normalised(&head, &raw[3..]);
    }
    // `\\server\share\x` (either separator): server and share are never popped by `..`.
    if raw.len() >= 3 && is_sep(raw[0]) && is_sep(raw[1]) && !is_sep(raw[2]) {
        let rest = &raw[2..];
        let mut it = rest.splitn(3, |&c| is_sep(c));
        let server = it.next().unwrap_or(&[]);
        let share = it.next().unwrap_or(&[]);
        let tail = it.next().unwrap_or(&[]);
        if server.is_empty() || share.is_empty() {
            return raw.to_vec();
        }
        let mut head: Vec<u16> = r"\\?\UNC\".encode_utf16().collect();
        head.extend_from_slice(server);
        head.push(BS);
        head.extend_from_slice(share);
        head.push(BS);
        let out = join_normalised(&head, tail);
        return out;
    }
    raw.to_vec()
}

#[cfg(test)]
mod tests {
    use super::verbatim;

    fn v(s: &str) -> String {
        String::from_utf16(&verbatim(&s.encode_utf16().collect::<Vec<_>>())).unwrap()
    }

    #[test]
    fn drive_paths_get_the_prefix_with_backslashes_only() {
        assert_eq!(v(r"C:\Users\me\a.txt"), r"\\?\C:\Users\me\a.txt");
        assert_eq!(v(r"C:\Users\RUNNER~1\Temp\in/b.txt"), r"\\?\C:\Users\RUNNER~1\Temp\in\b.txt");
        assert_eq!(v("c:/x//y/"), r"\\?\c:\x\y");
        assert_eq!(v(r"C:\"), r"\\?\C:\");
        assert_eq!(v(r"C:\a\.\b\..\c"), r"\\?\C:\a\c");
        assert_eq!(v(r"C:\..\..\a"), r"\\?\C:\a", "never above the drive");
    }

    #[test]
    fn trailing_dots_and_spaces_are_kept() {
        assert_eq!(v(r"C:\scan\Windows."), r"\\?\C:\scan\Windows.");
        assert_eq!(v(r"C:\scan\a "), r"\\?\C:\scan\a ");
    }

    #[test]
    fn unc_paths_use_the_unc_prefix_and_keep_the_share() {
        assert_eq!(v(r"\\srv\share\x/y"), r"\\?\UNC\srv\share\x\y");
        assert_eq!(v("//srv/share/x"), r"\\?\UNC\srv\share\x");
        assert_eq!(v(r"\\srv\share\..\..\x"), r"\\?\UNC\srv\share\x", "never above the share");
        assert_eq!(v(r"\\srv"), r"\\srv", "no share: left to the OS");
    }

    #[test]
    fn verbatim_device_and_relative_paths_are_unchanged() {
        for s in [r"\\?\C:\a/b", r"\\?\UNC\s\h\x", r"\\.\PhysicalDrive0", r"a\b", "C:a", r"\a", ""] {
            assert_eq!(v(s), s);
        }
    }
}
