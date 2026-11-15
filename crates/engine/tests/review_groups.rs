//! W6-A: duplicate review with groups kept together (sort, filters, paging, selection)
//! and row thumbnails / the bounded preview image, through the engine facade.

mod common;

use std::collections::BTreeSet;

use common::*;
use mj_core::testutil::TempDir;
use mj_core::{CandidateId, KeepRule, OpportunityId};
use mj_engine::api::*;

fn dup() -> OpportunityId {
    OpportunityId("det:exact_duplicates".into())
}

fn req(offset: u64, limit: u32, sort: CandidateSortKey, descending: bool, keep_groups: bool) -> CandidatesPageRequest {
    CandidatesPageRequest {
        opportunity_id: dup(),
        sub_group: None,
        offset,
        limit,
        sort,
        descending,
        filter: None,
        min_size: None,
        keep_groups,
    }
}

/// 10 groups of 3 identical files; group g is (g + 1) x 8 KiB (whole allocation blocks). Copies live in
/// `a/`, `b/`, `c/`, so the same name sorts the groups apart from the rows.
fn tree() -> TempDir {
    let t = TempDir::new("engine-review-groups").unwrap();
    for g in 0..10u64 {
        let data = bytes(500 + g, (g as usize + 1) * 8192);
        for d in ["a", "b", "c"] {
            t.write(format!("{d}/f{g}.bin"), &data).unwrap();
        }
    }
    t
}

fn keeper(r: &CandidateRow) -> bool {
    r.candidate.badges.contains(mj_core::Badges::KEEPER)
}

/// Rows of a page form whole runs per group, kept copy first.
fn assert_contiguous_groups(rows: &[CandidateRow]) -> Vec<u64> {
    let mut order = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        let g = r.candidate.group.expect("grouped row").0;
        if i == 0 || rows[i - 1].candidate.group != r.candidate.group {
            assert!(!order.contains(&g), "group {g} appears twice");
            order.push(g);
            assert!(keeper(r), "the kept copy opens its group");
        } else {
            assert!(!keeper(r));
        }
    }
    order
}

#[test]
fn size_sort_keeps_groups_together_and_flat_mode_does_not() {
    let f = engine_for(tree());
    let e = &f.engine;
    scan(e);
    let p = e.candidates_page(req(0, 100, CandidateSortKey::Size, false, true)).unwrap();
    assert!(p.has_groups && p.keep_groups);
    assert_eq!((p.total, p.rows.len()), (30, 30));
    let order = assert_contiguous_groups(&p.rows);
    let sizes: Vec<u64> = order
        .iter()
        .map(|g| p.group_stats.iter().find(|s| s.id.0 == *g).expect("stats for every group").largest)
        .collect();
    assert!(sizes.windows(2).all(|w| w[0] > w[1]), "biggest group first: {sizes:?}");
    let s = &p.group_stats[0];
    assert_eq!((s.files, s.largest, s.smallest, s.reclaimable), (3, 81_920, 81_920, 163_840));
    // Descending reverses the groups, not the rows inside.
    let d = e.candidates_page(req(0, 100, CandidateSortKey::Size, true, true)).unwrap();
    let rev = assert_contiguous_groups(&d.rows);
    assert_eq!(rev, order.iter().rev().copied().collect::<Vec<_>>());
    // Name sort: every group stays whole too (the rows of one name are the same group here,
    // so check a column that scatters them in flat mode: path).
    let flat = e.candidates_page(req(0, 100, CandidateSortKey::Path, false, false)).unwrap();
    assert!(!flat.keep_groups);
    assert_ne!(flat.rows[0].candidate.group, flat.rows[1].candidate.group, "flat path sort scatters groups");
    let grouped = e.candidates_page(req(0, 100, CandidateSortKey::Path, false, true)).unwrap();
    assert_contiguous_groups(&grouped.rows);
    // Turning the mode back on restores the same group view.
    assert_eq!(
        e.candidates_page(req(0, 100, CandidateSortKey::Size, false, true)).unwrap().rows,
        p.rows,
        "grouped view is restored"
    );
}

#[test]
fn filters_keep_or_drop_whole_groups() {
    let f = engine_for(tree());
    let e = &f.engine;
    scan(e);
    let mut r = req(0, 100, CandidateSortKey::Size, false, true);
    r.min_size = Some(65_536);
    let p = e.candidates_page(r.clone()).unwrap();
    assert_eq!(p.total, 9, "groups of 64, 72 and 80 KiB");
    assert_contiguous_groups(&p.rows);
    r.min_size = None;
    r.filter = Some("/b/f3".into());
    let p = e.candidates_page(r.clone()).unwrap();
    assert_eq!(p.total, 3, "one member matches: its group shows whole");
    assert!(p.rows.iter().any(|x| x.candidate.path.to_string_lossy().contains("/a/")));
    r.keep_groups = false;
    assert_eq!(e.candidates_page(r).unwrap().total, 1, "flat: only the matching row");
}

#[test]
fn pages_hold_whole_groups_and_cover_every_row_once() {
    let f = engine_for(tree());
    let e = &f.engine;
    scan(e);
    let mut seen = BTreeSet::new();
    let mut offset = Some(0);
    let mut pages = 0;
    while let Some(o) = offset {
        let p = e.candidates_page(req(o, 7, CandidateSortKey::Size, false, true)).unwrap();
        assert_eq!(p.total, 30);
        assert_eq!(p.offset, o);
        assert_eq!(p.rows.len(), 6, "two groups of 3 fit in 7 rows, a third does not");
        assert_eq!(p.page, pages + 1);
        assert_eq!(p.pages, 5);
        assert_contiguous_groups(&p.rows);
        for r in &p.rows {
            assert!(seen.insert(r.candidate.id), "row on two pages");
        }
        if let Some(prev) = p.prev_offset {
            assert!(prev < o);
        }
        offset = p.next_offset;
        pages += 1;
    }
    assert_eq!((pages, seen.len()), (5, 30));
    // An offset inside a page returns that page.
    let mid = e.candidates_page(req(8, 7, CandidateSortKey::Size, false, true)).unwrap();
    assert_eq!((mid.offset, mid.page), (6, 2));
}

#[test]
fn select_all_and_keep_rules_behave_the_same_in_both_modes() {
    let f = engine_for(tree());
    let e = &f.engine;
    scan(e);
    let all = |keep_groups| e.candidates_page(req(0, 100, CandidateSortKey::Size, false, keep_groups)).unwrap().rows;
    // Select all never selects a kept copy (F-02), whichever mode the table shows.
    e.selection_set(SelectionSetRequest { opportunity_id: dup(), ids: None, sub_group: None, selected: true }).unwrap();
    for mode in [true, false] {
        for r in all(mode) {
            assert_eq!(r.selected, !keeper(&r), "{}", r.candidate.path.display());
        }
    }
    // A keep rule moves the kept copy: grouped pages put the new one first, and the
    // cached order is not served stale.
    let before: Vec<CandidateId> = all(true).iter().filter(|r| keeper(r)).map(|r| r.candidate.id).collect();
    e.selection_apply_keep_rule(SelectionKeepRuleRequest {
        opportunity_id: dup(),
        rule: KeepRule::PreferredFolder { folder: f.tree.path().join("c") },
    })
    .unwrap();
    let rows = all(true);
    assert_contiguous_groups(&rows);
    let after: Vec<&CandidateRow> = rows.iter().filter(|r| keeper(r)).collect();
    assert_eq!(after.len(), 10);
    assert!(after.iter().all(|r| r.candidate.path.parent().unwrap().ends_with("c")));
    assert!(after.iter().all(|r| !r.selected), "the kept copy is unchecked");
    assert_ne!(before, after.iter().map(|r| r.candidate.id).collect::<Vec<_>>());
    // Explicitly checking a kept copy still works (and apply keeps one per group, F-02).
    let k = after[0].candidate.id;
    e.selection_set(SelectionSetRequest { opportunity_id: dup(), ids: Some(vec![k]), sub_group: None, selected: true })
        .unwrap();
    assert!(all(true).iter().find(|r| r.candidate.id == k).unwrap().selected);
    assert!(all(false).iter().find(|r| r.candidate.id == k).unwrap().selected);
}

fn png(w: u32, h: u32) -> Vec<u8> {
    mj_formats::png::build::rgb_png(w, h, &[], 1)
}

#[test]
fn thumbnails_are_bounded_shared_per_group_and_never_fail_the_batch() {
    let t = TempDir::new("engine-thumbs").unwrap();
    let big = png(1200, 900);
    t.write("a/photo.png", &big).unwrap();
    t.write("b/photo.png", &big).unwrap();
    let doc = bytes(7, 20_000);
    t.write("a/doc.bin", &doc).unwrap();
    t.write("b/doc.bin", &doc).unwrap();
    let f = engine_for(t);
    let e = &f.engine;
    scan(e);
    let rows = e.candidates_page(req(0, 100, CandidateSortKey::Path, false, true)).unwrap().rows;
    assert_eq!(rows.len(), 4);
    let mut ids: Vec<CandidateId> = rows.iter().map(|r| r.candidate.id).collect();
    ids.push(CandidateId(9_999_999));
    let th = e
        .candidate_thumbnails(CandidateThumbnailsRequest { opportunity_id: dup(), candidate_ids: ids.clone() })
        .unwrap();
    assert_eq!(th.thumbnails.iter().map(|x| x.candidate_id).collect::<Vec<_>>(), ids, "request order");
    let by = |name: &str| -> Vec<&Thumbnail> {
        rows.iter()
            .zip(&th.thumbnails)
            .filter(|(r, _)| r.candidate.path.to_string_lossy().ends_with(name))
            .map(|(_, x)| x)
            .collect()
    };
    let photos = by("photo.png");
    let url = photos[0].data_url.as_deref().expect("thumbnail");
    assert!(url.starts_with("data:image/png;base64,"));
    assert_eq!(photos[1].data_url.as_deref(), Some(url), "one thumbnail per exact group");
    assert!(url.len() < 64 * 1024, "small: {} bytes", url.len());
    assert!(by("doc.bin").iter().all(|x| x.data_url.is_none() && x.reason.as_deref() == Some("not_image")));
    assert_eq!(th.thumbnails[4].reason.as_deref(), Some("unknown"));
    // Bounded batch.
    let err = e
        .candidate_thumbnails(CandidateThumbnailsRequest {
            opportunity_id: dup(),
            candidate_ids: vec![CandidateId(1); 101],
        })
        .unwrap_err();
    assert_eq!(err.code(), "invalid_input");

    // The preview pane gets a real thumbnail of at most 512 px, not the 1200 px file.
    let id = rows.iter().find(|r| r.candidate.path.to_string_lossy().ends_with("photo.png")).unwrap().candidate.id;
    let p = e.candidate_preview(CandidatePreviewRequest { candidate_id: id }).unwrap();
    assert_eq!((p.kind.as_str(), p.mime.as_deref()), ("image", Some("image/png")));
    let data = decode_b64(p.data.as_deref().unwrap());
    let w = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
    let h = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
    assert_eq!((w, h), (512, 384));
}

fn decode_b64(s: &str) -> Vec<u8> {
    let val = |c: u8| match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        _ => 63,
    };
    let bytes: Vec<u8> = s.bytes().filter(|&c| c != b'=').collect();
    let mut out = Vec::new();
    for chunk in bytes.chunks(4) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, &c)| n | (val(c) as u32) << (18 - 6 * i));
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    out
}
