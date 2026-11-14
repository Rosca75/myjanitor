//! Refusals for scan roots that are too dangerous to act on (SAF, session safety).
//! Pure, so the rules are unit-tested without touching the disk. Owner: W3-E.

use std::path::Path;

use mj_core::pathutil::{is_within, path_key};

/// Why `root` must not be scanned, or `Ok`. `protected` are folders the app
/// owns (data, cache): a root that equals or contains one is refused, because
/// the journal and quarantine would be scanned and "cleaned" like user files.
pub fn check_root(root: &Path, home: Option<&Path>, protected: &[&Path]) -> Result<(), String> {
    let shown = root.display();
    if !root.is_absolute() {
        return Err(format!("{shown} is not an absolute path"));
    }
    if root.parent().is_none() || is_bare_drive(root) {
        return Err(format!("{shown} is the root of a file system or drive; choose a folder inside it"));
    }
    if home.is_some_and(|h| path_key(h) == path_key(root)) {
        return Err(format!("{shown} is your home folder; choose a folder inside it (for example Downloads)"));
    }
    if let Some(p) = protected.iter().find(|p| is_within(root, p)) {
        return Err(format!(
            "{shown} contains MyJanitor's own data folder ({}); choose a folder that does not",
            p.display()
        ));
    }
    Ok(())
}

/// `C:` or `C:\` (also under the `\\?\` prefix) on any host.
fn is_bare_drive(p: &Path) -> bool {
    let s = p.to_string_lossy().replace('/', "\\");
    let s = s.strip_prefix("\\\\?\\").unwrap_or(&s);
    let b = s.as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && s[2..].trim_matches('\\').is_empty()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn data() -> PathBuf {
        PathBuf::from("/home/m/.local/share/myjanitor")
    }

    #[test]
    fn refuses_filesystem_and_drive_roots() {
        let home = PathBuf::from("/home/m");
        let d = data();
        for bad in ["/", "C:\\", "C:", "c:/", "\\\\?\\D:\\"] {
            let r = check_root(Path::new(bad), Some(&home), &[&d]);
            // On Linux "C:\\" is not absolute, which is refused too.
            assert!(r.is_err(), "{bad} must be refused");
        }
        assert!(check_root(Path::new("/"), None, &[]).unwrap_err().contains("root of a file system"));
        assert!(
            is_bare_drive(Path::new("C:\\")) && is_bare_drive(Path::new("D:")) && !is_bare_drive(Path::new("C:\\x"))
        );
    }

    #[test]
    fn refuses_the_home_folder_itself_but_not_below_it() {
        let home = PathBuf::from("/home/m");
        let d = data();
        assert!(check_root(Path::new("/home/m"), Some(&home), &[&d]).unwrap_err().contains("home folder"));
        assert!(check_root(Path::new("/home/m/"), Some(&home), &[&d]).is_err());
        assert!(check_root(Path::new("/home/m/./"), Some(&home), &[&d]).is_err());
        assert!(check_root(Path::new("/home/m/Downloads"), Some(&home), &[&d]).is_ok());
        assert!(check_root(Path::new("/home/mm"), Some(&home), &[&d]).is_ok());
    }

    #[test]
    fn refuses_roots_containing_the_data_or_cache_folder() {
        let home = PathBuf::from("/home/m");
        let d = data();
        let c = PathBuf::from("/home/m/.cache/myjanitor");
        assert!(check_root(Path::new("/home/m/.local"), Some(&home), &[&d, &c]).unwrap_err().contains("data folder"));
        assert!(check_root(&d, Some(&home), &[&d]).is_err());
        assert!(check_root(Path::new("/home/m/.cache"), Some(&home), &[&d, &c]).is_err());
        assert!(
            check_root(&d.join("quarantine"), Some(&home), &[&d]).is_ok(),
            "inside the data folder is a separate concern"
        );
        assert!(check_root(Path::new("relative/dir"), None, &[]).is_err());
    }
}
