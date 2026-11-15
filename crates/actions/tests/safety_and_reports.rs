//! Safety detectors on generated fixtures (encrypted, signed, labelled, synced) and the
//! report / audit / results writers (escaping, formula-injection neutralisation).

mod common;

use std::path::PathBuf;

use common::*;
use mj_actions::report::{self, csv_cell, html_escape};
use mj_actions::safety::{content_badges, SafetyInspector};
use mj_actions::ProtectedPaths;
use mj_core::{
    Action, Badges, Candidate, CandidateId, Confidence, DetectorId, EntryKind, FileEntry, FileStamp, ItemOutcome,
    ItemStatus, Report, ReportSection, ReportTotals, RunId, Source, TargetKind, Timestamp,
};
use mj_formats::cfb::build_test_file;
use mj_platform::{SyncProvider, SyncRoot};

#[test]
fn detects_signed_labelled_and_encrypted_documents() {
    let f = Fixture::new("detect");
    let custom = br#"<?xml version="1.0"?><Properties><property name="MSIP_Label_1234_Enabled"><vt:lpwstr>true</vt:lpwstr></property></Properties>"#;
    let cases: Vec<(&str, Vec<u8>, Badges)> = vec![
        ("plain.docx", zip(&[("[Content_Types].xml", b"<Types/>", 0), ("word/document.xml", b"<w/>", 0)]), Badges::empty()),
        ("signed.docx", zip(&[("word/document.xml", b"<w/>", 0), ("_xmlsignatures/sig1.xml", b"<Signature/>", 0)]), Badges::SIGNED),
        ("signed.odt", zip(&[("content.xml", b"<x/>", 0), ("META-INF/documentsignatures.xml", b"<s/>", 0)]), Badges::SIGNED),
        (
            "labelled.xlsx",
            zip(&[("xl/workbook.xml", b"<w/>", 0), ("docProps/custom.xml", custom, 0)]),
            Badges::LABELLED | Badges::CONFIDENTIAL,
        ),
        ("encrypted.zip", zip(&[("a.txt", b"ciphertext", 1)]), Badges::ENCRYPTED),
        (
            "encrypted.docx",
            build_test_file(&[("EncryptionInfo", b"\x04\x00\x04\x00"), ("EncryptedPackage", b"....")], &["\u{6}DataSpaces"]),
            Badges::ENCRYPTED,
        ),
        ("legacy-signed.doc", build_test_file(&[("WordDocument", &[0u8; 64])], &["_signatures"]), Badges::SIGNED),
        (
            "legacy-labelled.xls",
            build_test_file(
                &[
                    ("Workbook", &[0x09, 0x08, 0, 0, 0x0A, 0, 0, 0]),
                    ("\u{5}DocumentSummaryInformation", &"MSIP_Label_x_Enabled".encode_utf16().flat_map(|u| u.to_le_bytes()).collect::<Vec<u8>>()),
                ],
                &[],
            ),
            Badges::LABELLED | Badges::CONFIDENTIAL,
        ),
        ("encrypted.pdf", b"%PDF-1.7\n1 0 obj<<>>endobj\ntrailer\n<< /Size 3 /Encrypt 2 0 R >>\n%%EOF\n".to_vec(), Badges::ENCRYPTED),
        (
            "signed.pdf",
            b"%PDF-1.7\n5 0 obj\n<</Type/Sig/Filter/Adobe.PPKLite/ByteRange [0 100 200 300]/Contents <00>>>\nendobj\n%%EOF".to_vec(),
            Badges::SIGNED,
        ),
        ("sigfield-only.pdf", b"%PDF-1.7\n<< /Type /SigFieldLock >>\n%%EOF".to_vec(), Badges::empty()),
        ("text.txt", b"/Encrypt /ByteRange but not a PDF".to_vec(), Badges::empty()),
        ("broken.docx", b"PK\x03\x04garbage".to_vec(), Badges::empty()),
    ];
    for (name, data, want) in cases {
        let p = f.write(name, &data);
        assert_eq!(content_badges(&p).unwrap(), want, "{name}");
    }
    assert_eq!(content_badges(&f.root().join("missing")).unwrap_err().code(), "not_found");
    // A signature split across the scanner's chunk boundary is still found.
    let mut big = b"%PDF-1.7\n".to_vec();
    big.resize((1 << 20) - 4, b' ');
    big.extend_from_slice(b"/ByteRange [0 1 2 3]");
    let p = f.write("boundary.pdf", &big);
    assert_eq!(content_badges(&p).unwrap(), Badges::SIGNED);
}

#[test]
fn sync_status_is_unknown_unless_detected() {
    let unknown = SafetyInspector::with_sync_roots(None, ProtectedPaths::empty());
    assert!(!unknown.sync_status_known());
    let root = SyncRoot {
        provider: SyncProvider::OneDriveBusiness,
        name: "OneDrive - Acme".into(),
        path: PathBuf::from("/home/me/OneDrive - Acme"),
        can_dehydrate: false,
    };
    let mut known = SafetyInspector::with_sync_roots(Some(vec![root.clone()]), ProtectedPaths::empty());
    assert!(known.sync_status_known());
    let mut e = FileEntry::new("/home/me/OneDrive - Acme/x.docx", EntryKind::File, 10);
    assert!(known.path_badges(&e).contains(Badges::SYNCED));
    e.path = "/home/me/Documents/x.docx".into();
    assert!(!known.path_badges(&e).contains(Badges::SYNCED));
    known.add_sync_roots(&[SyncRoot { path: "/home/me/Documents".into(), ..root }]);
    assert!(known.path_badges(&e).contains(Badges::SYNCED));
    e.attrs |= mj_core::Attributes::ENCRYPTED | mj_core::Attributes::ONLINE_ONLY;
    let b = known.path_badges(&e);
    assert!(b.contains(Badges::ENCRYPTED | Badges::ONLINE_ONLY));
}

#[test]
fn csv_cells_are_escaped_and_formula_safe() {
    assert_eq!(csv_cell("plain"), "plain");
    assert_eq!(csv_cell("=HYPERLINK(\"x\")"), "\"'=HYPERLINK(\"\"x\"\")\"");
    assert_eq!(csv_cell("+1"), "'+1");
    assert_eq!(csv_cell("-2"), "'-2");
    assert_eq!(csv_cell("@SUM(A1)"), "'@SUM(A1)");
    assert_eq!(csv_cell("\tx"), "'\tx");
    assert_eq!(csv_cell("a,b"), "\"a,b\"");
    assert_eq!(csv_cell("line\nbreak"), "\"line\nbreak\"");
    assert_eq!(csv_cell("say \"hi\""), "\"say \"\"hi\"\"\"");
    assert_eq!(csv_cell(" padded "), "\" padded \"");
    assert_eq!(html_escape("<script>&\"'</script>"), "&lt;script&gt;&amp;&quot;&#39;&lt;/script&gt;");
}

fn sample_report() -> Report {
    let items = vec![
        ItemOutcome {
            path: "/r/=cmd.exe".into(),
            status: ItemStatus::Done,
            bytes_before: 100,
            bytes_after: 0,
            error: None,
        },
        ItemOutcome {
            path: "/r/<b>.txt".into(),
            status: ItemStatus::Skipped,
            bytes_before: 50,
            bytes_after: 50,
            error: Some(mj_core::Error::Protected("/r/<b>.txt".into()).info()),
        },
    ];
    Report {
        run_id: RunId(7),
        title: "Plan · 1 step".into(),
        started: Timestamp::from_ymd_hms(2026, 11, 13, 9, 0, 0),
        finished: None,
        totals: ReportTotals {
            files: 2,
            succeeded: 1,
            skipped: 1,
            failed: 0,
            bytes_before: 150,
            bytes_after: 50,
            bytes_saved: 100,
        },
        sections: vec![ReportSection {
            title: "Temp, files".into(),
            action: "To Recycle Bin".into(),
            totals: ReportTotals::default(),
            items,
        }],
        undo_until: None,
    }
}

#[test]
fn report_and_results_writers() {
    let mut out = Vec::new();
    report::report_csv(&sample_report(), &mut out).unwrap();
    let s = String::from_utf8(out).unwrap();
    assert!(s.starts_with('\u{feff}'));
    let lines: Vec<&str> = s.trim_start_matches('\u{feff}').split("\r\n").collect();
    assert_eq!(lines[0], "step,path,status,bytes_before,bytes_after,saved,error");
    assert_eq!(lines[1], "\"Temp, files\",/r/=cmd.exe,done,100,0,100,");
    assert!(lines[2].starts_with("\"Temp, files\",/r/<b>.txt,skipped,50,50,0,Protected path"));
    let mut html = Vec::new();
    report::report_html(&sample_report(), &mut html).unwrap();
    let h = String::from_utf8(html).unwrap();
    assert!(h.contains("/r/&lt;b&gt;.txt") && !h.contains("<b>.txt") && !h.contains("<script"));

    let c = Candidate {
        id: CandidateId(1),
        path: "/r/-rm.txt".into(),
        target: TargetKind::File,
        source: Source::Detector(DetectorId::ExactDuplicates),
        size: 1000,
        size_after: 0,
        confidence: Confidence::Exact,
        reason: "=Same content".into(),
        badges: Badges::DUPLICATE | Badges::SIGNED,
        default_selected: false,
        group: Some(mj_core::GroupId(3)),
        sub_group: None,
        modified: Some(Timestamp::from_ymd_hms(2026, 1, 2, 3, 4, 5)),
        stamp: FileStamp { size: 1000, mtime: Timestamp(0) },
        file_count: None,
        columns: Default::default(),
    };
    let mut out = Vec::new();
    report::candidates_csv(std::slice::from_ref(&c), &mut out).unwrap();
    let s = String::from_utf8(out).unwrap();
    assert!(s.contains("det:exact_duplicates,'-rm.txt") || s.contains("det:exact_duplicates,/r/-rm.txt"), "{s}");
    assert!(s.contains(",'=Same content,duplicate signed,false,3,2026-01-02T03:04:05Z"), "{s}");
    let mut html = Vec::new();
    report::candidates_html("Results <all>", &[c], &mut html).unwrap();
    assert!(String::from_utf8(html).unwrap().contains("Results &lt;all&gt;"));
}

#[test]
fn audit_and_report_from_a_real_run_and_exports_never_overwrite() {
    let f = Fixture::new("audit");
    let p = f.write("a.txt", b"audit me");
    f.write("b.txt", b"b");
    let mut changed = f.item(&f.root().join("b.txt"), Action::Trash);
    changed.expected.size = 99;
    let (run, _) = f.apply(&[f.item(&p, Action::Quarantine { purge_after_days: 30 }), changed], &opts());
    let entries = f.journal.entries_of(run);
    let mut out = Vec::new();
    report::audit_csv(&entries, &mut out).unwrap();
    let s = String::from_utf8(out).unwrap();
    let sha = mj_core::hash::to_hex(&mj_core::hash::sha256(b"audit me"));
    assert!(s.contains(&sha), "{s}");
    assert!(s.contains(",quarantine,done,"));
    assert!(s.contains(",trash,skipped,"));
    assert!(s
        .lines()
        .next()
        .unwrap()
        .ends_with("time,run_id,entry_id,action,state,path,retained_at,bytes_before,bytes_after,sha256,lossy,error"));

    let rec = f.journal.run(run).unwrap();
    let r = report::from_journal(&rec, &entries, &[]);
    assert_eq!((r.totals.files, r.totals.succeeded, r.totals.skipped), (2, 1, 1));
    assert_eq!(r.totals.bytes_saved, 8);
    assert!(r.undo_until.is_some());

    let dest = f.t.join("exports/audit.csv");
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    std::fs::write(&dest, b"existing").unwrap();
    let (written, bytes, rows) = report::write_export(&dest, |w| {
        report::audit_csv(&entries, w)?;
        Ok(entries.len() as u64)
    })
    .unwrap();
    assert_eq!(written, f.t.join("exports/audit (1).csv"));
    assert_eq!(std::fs::read(&dest).unwrap(), b"existing");
    assert!(bytes > 0 && rows == 2);
    assert!(report::write_export(std::path::Path::new("rel.csv"), |_| Ok(0)).is_err());
    // A failed export leaves no file behind.
    let failed = f.t.join("exports/failed.csv");
    assert!(report::write_export(&failed, |w| {
        w.write_all(b"half")?;
        Err(mj_core::Error::Internal("boom".into()))
    })
    .is_err());
    assert!(!failed.exists());
    let denied = vec![ItemOutcome {
        path: "/x".into(),
        status: ItemStatus::Failed,
        bytes_before: 1,
        bytes_after: 1,
        error: Some(mj_core::Error::PermissionDenied("/x".into()).info()),
    }];
    assert_eq!(report::permission_denied_paths(&denied), vec![PathBuf::from("/x")]);
}

/// W6-A D-6A4: the replacing export is for a target the user confirmed in the OS save
/// dialog. It replaces a regular file atomically and nothing else.
/// A replace the user confirmed must also work over a read-only file (Windows refuses a
/// plain rename over one; the Windows CI job runs this for real).
#[test]
fn replacing_export_replaces_a_read_only_file() {
    let t = mj_core::testutil::TempDir::new("export-replace-ro").unwrap();
    let dest = t.join("report.csv");
    std::fs::write(&dest, b"old").unwrap();
    let mut perms = std::fs::metadata(&dest).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&dest, perms).unwrap();
    let r = report::write_export_replacing(&dest, |w| {
        w.write_all(b"new export")?;
        Ok(1)
    });
    let (written, bytes, _) = r.expect("a confirmed replace of a read-only export succeeds");
    assert_eq!((written.as_path(), bytes), (dest.as_path(), 10));
    assert_eq!(std::fs::read(&dest).unwrap(), b"new export");
    let entries = std::fs::read_dir(t.path()).unwrap().count();
    assert_eq!(entries, 1, "no temporary file stays behind");
    // Let the temp dir clean up on every platform.
    let mut perms = std::fs::metadata(&dest).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    let _ = std::fs::set_permissions(&dest, perms);
}

#[test]
fn replacing_export_swaps_a_regular_file_and_nothing_else() {
    let t = mj_core::testutil::TempDir::new("export-replace").unwrap();
    let names = |t: &mj_core::testutil::TempDir| {
        let mut v: Vec<String> = std::fs::read_dir(t.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    };
    let dest = t.join("report.csv");

    // Nothing there yet: written at the exact name, like `write_export`.
    let (written, bytes, rows) = report::write_export_replacing(&dest, |w| {
        w.write_all(b"first")?;
        Ok(1)
    })
    .unwrap();
    assert_eq!((written.as_path(), bytes, rows), (dest.as_path(), 5, 1));

    // An existing regular file is replaced; no temporary file stays behind.
    let (written, bytes, rows) = report::write_export_replacing(&dest, |w| {
        w.write_all(b"second export")?;
        Ok(2)
    })
    .unwrap();
    assert_eq!((written.as_path(), bytes, rows), (dest.as_path(), 13, 2));
    assert_eq!(std::fs::read(&dest).unwrap(), b"second export");
    assert_eq!(names(&t), ["report.csv"]);

    // A failing writer leaves the old file exactly as it was, and no temporary file.
    let r = report::write_export_replacing(&dest, |w| {
        w.write_all(b"half written")?;
        Err(mj_core::Error::Internal("boom".into()))
    });
    assert!(r.is_err());
    assert_eq!(std::fs::read(&dest).unwrap(), b"second export");
    assert_eq!(names(&t), ["report.csv"]);

    // A folder with the target's name is never replaced: the export gets a free name.
    let dir = t.join("folder.csv");
    std::fs::create_dir(&dir).unwrap();
    let (written, ..) = report::write_export_replacing(&dir, |w| {
        w.write_all(b"x")?;
        Ok(1)
    })
    .unwrap();
    assert_eq!(written, t.join("folder (1).csv"));
    assert!(dir.is_dir());

    // Neither is a link, nor the file behind it.
    #[cfg(unix)]
    {
        let target = t.join("precious.txt");
        std::fs::write(&target, b"precious").unwrap();
        let link = t.join("link.csv");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let (written, ..) = report::write_export_replacing(&link, |w| {
            w.write_all(b"x")?;
            Ok(1)
        })
        .unwrap();
        assert_eq!(written, t.join("link (1).csv"));
        assert_eq!(std::fs::read(&target).unwrap(), b"precious");
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
    }

    assert!(report::write_export_replacing(std::path::Path::new("rel.csv"), |_| Ok(0)).is_err());
}
