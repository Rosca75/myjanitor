//! Office (OOXML) optimisers (W3-A). Shared package handling (parts,
//! relationships, media dedup, picture recompression, label preservation
//! SAF-08) goes in `package`.

pub mod excel;
pub mod package;
pub mod powerpoint;
pub mod word;

#[cfg(test)]
mod tests {
    use mj_core::testutil::TempDir;
    use mj_core::{
        CancelToken, DetectContext, EntryKind, FileEntry, FileIndex, NoFingerprintStore, NullProgress, Optimiser,
        ParamValues, ScanStats, Timestamp,
    };

    /// Integration: analysis over a generated tree routes by content, not by name.
    #[test]
    fn analyse_routes_by_content() {
        let t = TempDir::new("analyse").unwrap();
        let deck = super::powerpoint::tests::deck(false, false);
        let book = super::excel::tests::workbook(false);
        let files = [
            ("a.pptx", deck.clone()),
            ("renamed-deck.docx", deck),
            ("b.xlsx", book.clone()),
            ("book-named.pptx", book),
            ("broken.pptx", b"not a zip".to_vec()),
        ];
        let mut entries = vec![FileEntry::new(t.path(), EntryKind::Dir, 0)];
        for (n, d) in &files {
            let p = t.write(n, d).unwrap();
            let mut e = FileEntry::new(p, EntryKind::File, d.len() as u64);
            e.depth = 1;
            entries.push(e);
        }
        let index = FileIndex::new(vec![t.path().to_path_buf()], entries, ScanStats::default(), Timestamp(0));
        let params = ParamValues::new();
        let cancel = CancelToken::new();
        let ctx = DetectContext {
            index: &index,
            params: &params,
            now: Timestamp(0),
            fingerprints: &NoFingerprintStore,
            cancel: &cancel,
            progress: &NullProgress,
            threads: 1,
        };
        let out = super::powerpoint::PowerPoint.analyse(&ctx).unwrap();
        let mut names: Vec<_> =
            out.candidates.iter().map(|c| c.path.file_name().unwrap().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["a.pptx", "renamed-deck.docx"]);
        assert!(out.candidates.iter().all(|c| c.size_after < c.size && c.columns.contains_key("unused_layouts")));
        assert_eq!(out.diagnostics.skipped.get("content_mismatch"), Some(&2), "{:?}", out.diagnostics);
        assert_eq!(out.diagnostics.unreadable, 0);
        let x = super::excel::Excel.analyse(&ctx).unwrap();
        let mut names: Vec<_> =
            x.candidates.iter().map(|c| c.path.file_name().unwrap().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["b.xlsx", "book-named.pptx"]);
        cancel.cancel();
        assert!(super::word::Word.analyse(&ctx).is_err());
    }
}
