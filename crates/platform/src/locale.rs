//! The user's interface language (RF-9), for the language-pack detector.
//!
//! Windows: `GetUserDefaultLocaleName` (kernel32), because a program started from Explorer
//! inherits no `LANG`. Elsewhere: `LC_ALL`, `LC_MESSAGES`, then `LANG` (`C` and `POSIX` mean
//! "no language").

/// The locale name (`fr-FR`, `pt_BR.UTF-8`), or `None` when the OS does not say.
pub fn system_language() -> Option<String> {
    let name = os_locale()?;
    let name = name.trim();
    (!name.is_empty() && name != "C" && name != "POSIX" && !name.starts_with("C.")).then(|| name.to_string())
}

#[cfg(windows)]
fn os_locale() -> Option<String> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetUserDefaultLocaleName(lpLocaleName: *mut u16, cchLocaleName: i32) -> i32;
    }
    // LOCALE_NAME_MAX_LENGTH.
    let mut buf = [0u16; 85];
    // SAFETY: the buffer is valid for 85 UTF-16 units, which is the length passed.
    let n = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) };
    if n <= 1 {
        return None;
    }
    // `n` counts the terminating NUL.
    Some(String::from_utf16_lossy(&buf[..(n as usize - 1).min(buf.len())]))
}

#[cfg(not(windows))]
fn os_locale() -> Option<String> {
    ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn answers_without_failing() {
        // The value depends on the machine; it is never empty or "C".
        if let Some(l) = super::system_language() {
            assert!(!l.is_empty() && l != "C" && l != "POSIX");
        }
    }
}
