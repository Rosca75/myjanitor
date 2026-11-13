//! Derived files (CLN-DET-19): a PDF exported from a .pptx/.docx still present in the same folder.
//! Owner: W2-B.
//!
//! A PDF is "derived" when a file with the **same stem** (compared
//! case-insensitively, exact otherwise) and a source extension
//! ([`SOURCE_EXTENSIONS`]) is in the same folder, and the PDF is **not older**
//! than that source (an older PDF may be the only copy of an earlier state).
//! Files with an unknown modification time are not matched. The PDF is a
//! review candidate (never pre-selected); the source is shown as the keeper.

use std::collections::{BTreeMap, HashMap};

use mj_core::{
    ActionKind, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, FileEntry, GroupId,
    GroupKind, PlatformSupport, Result, SafetyClass,
};

use crate::keep::{self, GroupSpec, Selection};

/// Source document extensions a PDF is commonly exported from.
pub const SOURCE_EXTENSIONS: &[&str] =
    &["pptx", "ppt", "pptm", "docx", "doc", "docm", "xlsx", "xls", "xlsm", "odt", "odp", "ods", "rtf"];

/// Derived files: PDF exports next to their source documents
#[derive(Debug, Default, Clone, Copy)]
pub struct DerivedFiles;

impl Detector for DerivedFiles {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::DerivedFiles,
            title: "Derived files".into(),
            description: "PDF exports next to their source documents".into(),
            category: Category::Clean,
            safety: SafetyClass::ReviewOnly,
            default_action: ActionKind::None,
            available_actions: vec![ActionKind::None, ActionKind::Trash, ActionKind::Quarantine, ActionKind::Move],
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-19".into()],
            unit: "files".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let files: Vec<&FileEntry> = ctx.index.files().collect();
        let mut out = DetectorOutput::default();
        for (n, (pdf, src)) in find_derived(&files).into_iter().enumerate() {
            if n % 256 == 0 {
                ctx.cancel.checkpoint()?;
            }
            let (p, s) = (files[pdf], files[src]);
            let label = format!("{} exported from {}", p.name(), s.name());
            let reason = format!("PDF export of {} in the same folder", s.name());
            keep::emit(
                GroupSpec {
                    id: GroupId(n as u64 + 1),
                    kind: GroupKind::Derived,
                    source: DetectorId::DerivedFiles,
                    label,
                    confidence: 100.0,
                    members: vec![s, p],
                    keeper: 0,
                    selection: Selection::None,
                    badges: mj_core::Badges::empty(),
                    reason,
                    keeper_reason: "Source document (kept)".into(),
                    columns: vec![BTreeMap::new(), BTreeMap::new()],
                },
                &mut out,
            );
        }
        Ok(out)
    }
}

/// `(pdf index, source index)` pairs. When several sources share the stem, the newest
/// one that is not newer than the PDF is used.
pub fn find_derived(files: &[&FileEntry]) -> Vec<(usize, usize)> {
    let key = |e: &FileEntry| {
        let dir = e.path.parent().map(mj_core::pathutil::path_key).unwrap_or_default();
        let stem = e.path.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
        (dir, stem)
    };
    let mut sources: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for (i, e) in files.iter().enumerate() {
        if e.is_file() && SOURCE_EXTENSIONS.contains(&e.extension().as_str()) {
            sources.entry(key(e)).or_default().push(i);
        }
    }
    let mut out = Vec::new();
    for (i, e) in files.iter().enumerate() {
        if !e.is_file() || e.extension() != "pdf" {
            continue;
        }
        let Some(pdf_t) = e.modified else { continue };
        let Some(cands) = sources.get(&key(e)) else { continue };
        let best = cands
            .iter()
            .copied()
            .filter(|&s| files[s].modified.is_some_and(|t| t <= pdf_t))
            .max_by(|&a, &b| files[a].modified.cmp(&files[b].modified).then(files[b].path.cmp(&files[a].path)));
        if let Some(s) = best {
            out.push((i, s));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::{EntryKind, Timestamp};

    fn f(p: &str, t: Option<i64>) -> FileEntry {
        let mut e = FileEntry::new(p, EntryKind::File, 10);
        e.modified = t.map(Timestamp);
        e
    }

    #[test]
    fn pdf_next_to_newer_or_equal_source_only() {
        let v = [
            f("/r/Deck.pptx", Some(10)),
            f("/r/Deck.pdf", Some(20)),
            f("/r/Report.docx", Some(30)),
            f("/r/Report.pdf", Some(20)), // older than its source: kept out
            f("/r/other/Deck.pdf", Some(50)), // different folder
            f("/r/Notes.PDF", Some(5)),
            f("/r/notes.DOCX", Some(5)), // same time, case-insensitive stem
            f("/r/Lonely.pdf", Some(5)),
            f("/r/Unknown.pdf", None),
            f("/r/Unknown.docx", Some(1)),
        ];
        let refs: Vec<&FileEntry> = v.iter().collect();
        let mut got = find_derived(&refs);
        got.sort_unstable();
        assert_eq!(got, vec![(1, 0), (5, 6)]);
    }
}
