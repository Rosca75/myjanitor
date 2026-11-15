//! Version chains (CLN-DET-18), derived files (CLN-DET-19), attachment copies
//! (CLN-DET-20) and duplicate fonts (OPT-FNT-01) on generated trees.

mod common;

use common::*;
use mj_core::testutil::TempDir;
use mj_core::Badges;
use mj_dupes::attachments::AttachmentCopies;
use mj_dupes::derived::DerivedFiles;
use mj_dupes::fonts::DuplicateFonts;
use mj_dupes::version_chains::VersionChains;

fn files(t: &TempDir, names: &[(&str, u64)]) {
    for (i, (n, days)) in names.iter().enumerate() {
        let p = t.write(n, format!("content {i}").as_bytes()).unwrap();
        age(&p, *days);
    }
}

#[test]
fn version_chains_on_consulting_names() {
    let t = TempDir::new("chains").unwrap();
    files(
        &t,
        &[
            ("Acme/Proposal/Acme Proposal v1.docx", 1),
            ("Acme/Proposal/Acme Proposal v2.docx", 5),
            ("Acme/Proposal/Acme Proposal_final.docx", 9),
            ("Acme/Proposal/Acme Proposal_final_v2_JD.docx", 12),
            ("Acme/Proposal/Acme Propsal v3.doc", 7),
            ("Acme/Proposal/Acme Proposal v2.pdf", 6),
            ("Acme/Proposal/Acme Proposal v3.pdf", 8),
            ("Acme/Deck/2024-05-01 Steering Committee.pptx", 3),
            ("Acme/Deck/20240514 Steering Committee.pptx", 4),
            ("Acme/Deck/Steering Committee (1).pptx", 2),
            ("Acme/Deck/Steering Committee - Copy.pptx", 1),
            // Negative cases: these must never chain.
            ("Acme/Reports/Q1 Report.docx", 1),
            ("Acme/Reports/Q2 Report.docx", 2),
            ("Acme/Reports/Q3 Report_v2.docx", 3),
            ("Acme/Finance/Budget 2023.xlsx", 1),
            ("Acme/Finance/Budget 2024.xlsx", 2),
            ("Acme/Finance/Budget 2024 v2.xlsx", 3),
            ("Acme/Finance/Report_EU.xlsx", 1),
            ("Acme/Finance/Report_US.xlsx", 2),
            ("Acme/Finance/Workshop Part 1.pptx", 1),
            ("Acme/Finance/Workshop Part 2.pptx", 2),
            // Same name in another folder: not a chain.
            ("Beta/Proposal/Acme Proposal v4.docx", 30),
        ],
    );
    let h = Harness::new(index_of(t.path()));
    let out = h.run(&VersionChains).unwrap();
    assert_eq!(
        group_names(&out),
        vec![
            vec![
                "2024-05-01 Steering Committee.pptx",
                "20240514 Steering Committee.pptx",
                "Steering Committee (1).pptx",
                "Steering Committee - Copy.pptx",
            ],
            vec![
                "Acme Proposal v1.docx",
                "Acme Proposal v2.docx",
                "Acme Proposal_final.docx",
                "Acme Proposal_final_v2_JD.docx",
                "Acme Propsal v3.doc",
            ],
            vec!["Acme Proposal v2.pdf", "Acme Proposal v3.pdf"],
            vec!["Budget 2024 v2.xlsx", "Budget 2024.xlsx"],
        ]
    );
    assert_one_keeper_per_group(&out);
    // Latest modified is kept; nothing is pre-selected (review only).
    assert_eq!(keeper_of(&out, "Acme Proposal v1.docx"), "Acme Proposal_final_v2_JD.docx");
    assert_eq!(keeper_of(&out, "Steering Committee (1).pptx"), "20240514 Steering Committee.pptx");
    assert!(out.candidates.iter().all(|c| !c.default_selected));
    let v1 = out.candidates.iter().find(|c| c.path.ends_with("Acme Proposal_final_v2_JD.docx")).unwrap();
    assert_eq!(v1.columns.get("version").map(String::as_str), Some("v2, final, JD"));

    // Fuzzy matching off: the typo no longer chains.
    let out = Harness::new(index_of(t.path())).flag("fuzzy", false).run(&VersionChains).unwrap();
    assert!(group_names(&out).concat().iter().all(|n| !n.contains("Propsal")));
    assert_eq!(group_names(&out).len(), 4);
    // Keep rule "oldest".
    let out = Harness::new(index_of(t.path())).text("keep_rule", "oldest").run(&VersionChains).unwrap();
    assert_eq!(keeper_of(&out, "Acme Proposal v1.docx"), "Acme Proposal v1.docx");
}

#[test]
fn derived_pdfs_next_to_their_sources() {
    let t = TempDir::new("derived").unwrap();
    files(
        &t,
        &[
            ("Client/Deck.pptx", 10),
            ("Client/Deck.pdf", 11),
            ("Client/Memo.docx", 20),
            ("Client/Memo.pdf", 15), // older than its source: may be the only copy of an earlier state
            ("Client/Model.xlsx", 3),
            ("Client/Model.pdf", 3),
            ("Client/Scan.pdf", 1),
            ("Other/Deck.pdf", 30),
        ],
    );
    let out = Harness::new(index_of(t.path())).run(&DerivedFiles).unwrap();
    assert_eq!(group_names(&out), vec![vec!["Deck.pdf", "Deck.pptx"], vec!["Model.pdf", "Model.xlsx"]]);
    assert_one_keeper_per_group(&out);
    for c in &out.candidates {
        assert!(!c.default_selected);
        assert_eq!(c.badges.contains(Badges::KEEPER), c.path.extension().unwrap() != "pdf");
    }
}

#[test]
fn attachment_copies_in_downloads() {
    let t = TempDir::new("attach").unwrap();
    let contract = noise(1, 80_000);
    let deck = noise(2, 4_000);
    t.write("OneDrive - Contoso/Clients/Contract.pdf", &contract).unwrap();
    t.write("OneDrive - Contoso/Clients/Deck.pptx", &deck).unwrap();
    t.write("Downloads/Contract (1).pdf", &contract).unwrap();
    t.write("Downloads/Contract (2).pdf", &contract).unwrap();
    t.write("Downloads/unrelated.zip", &noise(3, 80_000)).unwrap();
    // A duplicate inside Downloads only is not an attachment copy.
    t.write("Downloads/a.txt", b"same").unwrap();
    t.write("Downloads/b.txt", b"same").unwrap();
    // Same name and size as the deck but different content.
    t.write("Downloads/Deck.pptx", &noise(4, 4_000)).unwrap();

    let h = Harness::new(index_of(t.path()));
    let out = h.run(&AttachmentCopies).unwrap();
    assert_eq!(group_names(&out), vec![vec!["Contract (1).pdf", "Contract (2).pdf", "Contract.pdf"]]);
    assert_one_keeper_per_group(&out);
    let selected: Vec<_> = out.candidates.iter().filter(|c| c.default_selected).collect();
    assert_eq!(selected.len(), 2);
    assert!(selected.iter().all(|c| c.path.starts_with(t.join("Downloads"))));
    assert_eq!(out.groups[0].keeper.as_ref(), Some(&t.join("OneDrive - Contoso/Clients/Contract.pdf")));

    // Explicit folders replace the heuristics.
    t.write("Work/Synced/Plan.docx", &noise(5, 3_000)).unwrap();
    t.write("Inbox/Plan.docx", &noise(5, 3_000)).unwrap();
    let h = Harness::new(index_of(t.path()))
        .text("downloads_folders", &t.join("Inbox").to_string_lossy())
        .text("synced_folders", &t.join("Work/Synced").to_string_lossy());
    let out = h.run(&AttachmentCopies).unwrap();
    assert_eq!(group_names(&out), vec![vec!["Plan.docx", "Plan.docx"]]);
}

#[test]
fn duplicate_fonts_are_review_only() {
    let t = TempDir::new("fonts").unwrap();
    let font = noise(7, 50_000);
    t.write("usr/share/fonts/truetype/Inter.ttf", &font).unwrap();
    t.write("home/me/.local/share/fonts/Inter-Regular.ttf", &font).unwrap();
    t.write("home/me/.fonts/inter.ttf", &font).unwrap();
    t.write("home/me/.fonts/Other.otf", &noise(8, 50_000)).unwrap();
    // Not in a font folder: ignored.
    t.write("home/me/Downloads/Inter.ttf", &font).unwrap();
    let h = Harness::new(index_of(t.path()));
    let out = h.run(&DuplicateFonts).unwrap();
    assert_eq!(group_names(&out), vec![vec!["Inter-Regular.ttf", "Inter.ttf", "inter.ttf"]]);
    assert_one_keeper_per_group(&out);
    assert!(out.candidates.iter().all(|c| !c.default_selected));
    // Explicit folders.
    let h = Harness::new(index_of(t.path())).text("font_folders", &t.join("home/me").to_string_lossy());
    let out = h.run(&DuplicateFonts).unwrap();
    assert_eq!(group_names(&out), vec![vec!["Inter-Regular.ttf", "Inter.ttf", "inter.ttf"]]);
}

#[test]
fn every_detector_runs_on_an_empty_index() {
    let t = TempDir::new("empty").unwrap();
    let h = Harness::new(index_of(t.path()));
    for d in mj_dupes::detectors() {
        let out = h.run(d.as_ref()).unwrap();
        assert!(out.candidates.is_empty() && out.groups.is_empty(), "{:?}", d.info().id);
    }
}
