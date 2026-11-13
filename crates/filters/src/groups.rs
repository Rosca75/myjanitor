//! File-type groups (CLN-FLT-02): lookup of an extension in `Settings::type_groups`. Owner: W2-C.

use std::collections::BTreeMap;

/// Group ids containing `ext` (lower case, no dot).
pub fn groups_of<'a>(groups: &'a BTreeMap<String, Vec<String>>, ext: &str) -> Vec<&'a str> {
    groups.iter().filter(|(_, exts)| exts.iter().any(|e| e == ext)).map(|(g, _)| g.as_str()).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn finds_groups() {
        let g = mj_core::settings::default_type_groups();
        assert_eq!(super::groups_of(&g, "mp4"), vec!["video"]);
        assert!(super::groups_of(&g, "zzz").is_empty());
    }
}
