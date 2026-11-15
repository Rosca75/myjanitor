//! W6-A (third task): ACT-10, originals kept in the quarantine by an archive or an
//! optimisation carry their retention, and stay undoable no longer than they are held.

mod common;

use std::path::{Path, PathBuf};

use common::*;
use mj_core::Action;

struct OkArchiver;

impl mj_core::Archiver for OkArchiver {
    fn formats(&self, _: &[mj_core::ExternalTool]) -> Vec<mj_core::ArchiveFormat> {
        vec![mj_core::ArchiveFormat::Zip]
    }
    fn create(&self, job: &mj_core::ArchiveJob<'_>) -> mj_core::Result<u64> {
        std::fs::write(job.dst, b"archive bytes").unwrap();
        Ok(13)
    }
    fn verify(&self, _: &Path, _: &[PathBuf], _: &Path, _: &mj_core::CancelToken) -> mj_core::Result<()> {
        Ok(())
    }
}

#[test]
fn act10_archived_originals_in_quarantine_carry_their_retention() {
    let mut f = Fixture::new("act10");
    f.settings.general.quarantine_days = 7;
    f.settings.general.undo_days = 30;
    let file = f.write("report.txt", b"the report");
    let archiver = OkArchiver;
    let services = mj_actions::Services { optimisers: &[], archiver: Some(&archiver), tools: &[] };
    let item = f.item(
        &file,
        Action::Archive {
            format: mj_core::ArchiveFormat::Zip,
            level: 6,
            dest: None,
            originals: mj_core::OriginalsPolicy::Quarantine,
            keep_sources: false,
        },
    );
    let run = f.new_run(mj_core::RunKind::Apply);
    let out = mj_actions::execute_with(&f.ctx(run, services), &[item], &confirmed()).unwrap();
    assert_eq!(out.succeeded, 1, "{out:?}");
    let e = f.journal.entries_of(run).into_iter().next().unwrap();
    assert!(e.retained_at.is_some(), "the original is held in the quarantine: {e:?}");
    assert_eq!(e.retention_days, Some(7));
    let deadline = e.undo_deadline.expect("undoable");
    assert!(deadline <= e.at.add_days(7), "undoable while held, not for the 30-day window");
    assert!(deadline > e.at.add_days(6));
}
