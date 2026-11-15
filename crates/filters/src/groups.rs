//! File-type groups (CLN-FLT-02): the built-in table, user overrides and the
//! lookup of an extension. Owner: W2-C.
//!
//! The *effective* table is whatever the caller passes in
//! `FilterContext::type_groups`; the engine normally builds it with
//! [`effective`] from `Settings::type_groups`. A group id is lower case; its
//! extensions are lower case without the dot.

use std::collections::BTreeMap;

/// Group ids containing `ext` (lower case, no dot).
pub fn groups_of<'a>(groups: &'a BTreeMap<String, Vec<String>>, ext: &str) -> Vec<&'a str> {
    groups.iter().filter(|(_, exts)| exts.iter().any(|e| e == ext)).map(|(g, _)| g.as_str()).collect()
}

/// Normalise one extension: trim, drop a leading `.` or `*.`, lower-case.
pub fn normalise_ext(ext: &str) -> String {
    let e = ext.trim();
    let e = e.strip_prefix("*.").or_else(|| e.strip_prefix('.')).unwrap_or(e);
    e.to_lowercase()
}

/// The built-in table: `mj_core::settings::default_type_groups()` plus the
/// groups it lacks (code, disk images, e-books, text and data files).
pub fn builtin() -> BTreeMap<String, Vec<String>> {
    let mut g = mj_core::settings::default_type_groups();
    let mut add = |id: &str, exts: &[&str]| {
        g.entry(id.to_string()).or_insert_with(|| exts.iter().map(|s| s.to_string()).collect());
    };
    add(
        "code",
        &[
            "rs", "py", "js", "ts", "jsx", "tsx", "c", "h", "cpp", "hpp", "cc", "cs", "java", "kt", "go", "rb", "php",
            "swift", "sh", "ps1", "bat", "cmd", "lua", "sql", "html", "css", "scss", "json", "yaml", "yml", "toml",
            "xml",
        ],
    );
    add("disk_images", &["iso", "img", "vhd", "vhdx", "vmdk", "vdi", "wim", "dmg", "qcow2"]);
    add("ebooks", &["epub", "mobi", "azw", "azw3", "fb2", "djvu"]);
    add("text", &["txt", "md", "rtf", "log", "csv", "tsv", "ini", "cfg", "conf"]);
    add("data", &["db", "sqlite", "sqlite3", "mdb", "accdb", "bak", "dat"]);
    g
}

/// Merge user overrides over the built-in table: a user group with the same
/// id replaces the built-in one, a new id is added. Entries are normalised,
/// sorted and de-duplicated; a user group left empty is removed.
pub fn effective(user: &BTreeMap<String, Vec<String>>) -> BTreeMap<String, Vec<String>> {
    let mut out = builtin();
    for (id, exts) in user {
        out.insert(id.trim().to_lowercase(), exts.clone());
    }
    for exts in out.values_mut() {
        let mut v: Vec<String> = exts.iter().map(|e| normalise_ext(e)).filter(|e| !e.is_empty()).collect();
        v.sort();
        v.dedup();
        *exts = v;
    }
    out.retain(|id, exts| !id.is_empty() && !exts.is_empty());
    out
}

/// Find a group by id, exact first then case-insensitively.
pub fn find<'a>(groups: &'a BTreeMap<String, Vec<String>>, id: &str) -> Option<(&'a str, &'a [String])> {
    if let Some((k, v)) = groups.get_key_value(id) {
        return Some((k.as_str(), v.as_slice()));
    }
    groups.iter().find(|(k, _)| k.eq_ignore_ascii_case(id.trim())).map(|(k, v)| (k.as_str(), v.as_slice()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_groups() {
        let g = mj_core::settings::default_type_groups();
        assert_eq!(groups_of(&g, "mp4"), vec!["video"]);
        assert!(groups_of(&g, "zzz").is_empty());
    }

    #[test]
    fn builtin_has_code_and_disk_images() {
        let g = builtin();
        assert!(groups_of(&g, "rs").contains(&"code"));
        assert!(groups_of(&g, "iso").contains(&"disk_images"));
        assert!(g.contains_key("video"));
    }

    #[test]
    fn user_overrides_replace_and_add() {
        let user = BTreeMap::from([
            ("Video".to_string(), vec![".MP4".to_string(), "*.ts".to_string(), "mp4".to_string()]),
            ("mine".to_string(), vec!["abc".to_string()]),
            ("empty".to_string(), vec![]),
        ]);
        let g = effective(&user);
        assert_eq!(g["video"], vec!["mp4", "ts"]);
        assert_eq!(g["mine"], vec!["abc"]);
        assert!(!g.contains_key("empty"));
        assert!(g.contains_key("audio"));
    }

    #[test]
    fn find_is_case_insensitive() {
        let g = builtin();
        assert_eq!(find(&g, "VIDEO").map(|x| x.0), Some("video"));
        assert!(find(&g, "nope").is_none());
    }
}
