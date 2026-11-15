//! CLN-FLT-06: user names for numeric owners on Linux, read in-house from `/etc/passwd`
//! (parsed once per process, cached; at most 1 MiB and 100 000 lines are read). An owner the
//! file does not list stays `uid:N`. Windows owners are not collected (no SID lookup).

use std::collections::HashMap;
use std::sync::OnceLock;

/// `uid → name` from the text of a passwd file (`name:x:uid:gid:...`). Malformed lines are
/// skipped; the first entry of a uid wins (as `getpwuid` does).
pub fn parse_passwd(text: &str) -> HashMap<u32, String> {
    let mut m = HashMap::new();
    for line in text.lines().take(100_000) {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut f = line.split(':');
        let (Some(name), Some(_), Some(uid)) = (f.next(), f.next(), f.next()) else { continue };
        if name.is_empty() || name.starts_with(['+', '-']) {
            continue; // NIS compatibility entries
        }
        if let Ok(uid) = uid.parse::<u32>() {
            m.entry(uid).or_insert_with(|| name.to_string());
        }
    }
    m
}

fn table() -> &'static HashMap<u32, String> {
    static T: OnceLock<HashMap<u32, String>> = OnceLock::new();
    T.get_or_init(|| {
        if !cfg!(target_os = "linux") {
            return HashMap::new();
        }
        use std::io::Read;
        let mut s = String::new();
        match std::fs::File::open("/etc/passwd") {
            Ok(f) => {
                let _ = f.take(1 << 20).read_to_string(&mut s);
                parse_passwd(&s)
            }
            Err(_) => HashMap::new(),
        }
    })
}

/// The user name of `uid`, when the system's passwd file lists it.
pub fn name_of_uid(uid: u32) -> Option<&'static str> {
    table().get(&uid).map(String::as_str)
}

/// The user name behind an owner string of the form `uid:N`, when known.
pub fn name_of_owner(owner: &str) -> Option<&'static str> {
    owner.strip_prefix("uid:").and_then(|n| n.parse::<u32>().ok()).and_then(name_of_uid)
}

#[cfg(test)]
mod tests {
    #[test]
    fn passwd_lines_map_uids_to_names() {
        let m = super::parse_passwd(
            "root:x:0:0:root:/root:/bin/bash\n# comment\n\nalice:x:1000:1000::/home/alice:/bin/sh\n\
             broken line\n+nis::::\nbob:x:notanumber:1\ndup:x:1000:1\n",
        );
        assert_eq!(m.get(&0).map(String::as_str), Some("root"));
        assert_eq!(m.get(&1000).map(String::as_str), Some("alice"), "first entry wins");
        assert_eq!(m.len(), 2);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn root_is_known_on_linux() {
        if std::path::Path::new("/etc/passwd").exists() {
            assert_eq!(super::name_of_owner("uid:0"), Some("root"));
        }
        assert_eq!(super::name_of_owner("someone"), None);
    }
}
