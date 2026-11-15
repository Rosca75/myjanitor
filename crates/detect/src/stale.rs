//! Stale files (CLN-DET-17): files neither opened nor changed for N months (default 12). The list
//! feeds archiving; the detector itself is review-only and never pre-selects anything.
//!
//! "Last used" is the later of the access time and the modification time. Access times are only
//! trusted when they carry information: a missing one, or one equal to the creation time (never
//! updated), falls back to the modification time. If almost every file's access time is at or
//! before its modification time, the volume evidently does not record access times and the run
//! says so in its diagnostics notes.
//! Owner: W2-D.

use mj_core::{
    ActionKind, Attributes, Category, Confidence, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput,
    FileEntry, PlatformSupport, Result, SafetyClass, Timestamp,
};

use crate::common::{self, Ticker};

/// Access times within this many seconds of another stamp count as "equal".
const SLACK_SECS: i64 = 2;

fn near(a: Timestamp, b: Timestamp) -> bool {
    (a.0 - b.0).abs() <= SLACK_SECS * mj_core::time::NANOS_PER_SEC
}

/// When the file was last used, or `None` when no date is known.
pub fn last_used(e: &FileEntry) -> Option<Timestamp> {
    let atime_usable = match (e.accessed, e.created) {
        (Some(_), Some(c)) => !near(e.accessed.unwrap_or_default(), c),
        (Some(_), None) => true,
        (None, _) => false,
    };
    match (e.modified, e.accessed.filter(|_| atime_usable)) {
        (Some(m), Some(a)) => Some(m.max(a)),
        (Some(m), None) => Some(m),
        (None, a) => a,
    }
}

/// Stale files: Files not opened for a long time
#[derive(Debug, Default, Clone, Copy)]
pub struct StaleFiles;

impl Detector for StaleFiles {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::StaleFiles,
            title: "Stale files".into(),
            description: "Files not opened for a long time".into(),
            category: Category::Clean,
            safety: SafetyClass::ReviewOnly,
            default_action: ActionKind::None,
            available_actions: vec![ActionKind::None, ActionKind::Archive, ActionKind::Move, ActionKind::Trash],
            settings: vec![
                common::number_setting(
                    "months",
                    "Not used for",
                    "Files neither opened nor changed for this many months.",
                    12.0,
                    1.0,
                    240.0,
                    "months",
                ),
                common::number_setting(
                    "min_size_mb",
                    "Smallest file",
                    "Small files are not worth archiving.",
                    1.0,
                    0.0,
                    1_048_576.0,
                    "MB",
                ),
            ],
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-17".into()],
            unit: "files".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let months = ctx.params.number("months", 12.0).max(1.0) as i64;
        let min_size = (ctx.params.number("min_size_mb", 1.0).max(0.0) * 1_048_576.0) as u64;
        let cutoff = ctx.now.add_months(-months);
        let mut out = DetectorOutput::default();
        let mut tick = Ticker::new(ctx, ctx.index.len() as u64);
        let (mut files, mut atime_flat) = (0u64, 0u64);
        for e in ctx.index.entries() {
            tick.tick()?;
            if !e.is_file() || !common::usable(e) {
                continue;
            }
            files += 1;
            if e.accessed
                .is_none_or(|a| e.modified.is_some_and(|m| a.0 <= m.0 + SLACK_SECS * mj_core::time::NANOS_PER_SEC))
            {
                atime_flat += 1;
            }
            if e.size < min_size || e.attrs.contains(Attributes::SYSTEM) {
                continue;
            }
            let Some(used) = last_used(e) else {
                out.diagnostics.skip("no_dates");
                continue;
            };
            if used >= cutoff {
                continue;
            }
            let (y, m, d) = used.ymd();
            let reason =
                format!("Not opened or changed for {} (last {y:04}-{m:02}-{d:02})", common::human_age(used, ctx.now));
            tick.found(e.reclaimable());
            let mut c = common::file_candidate(e, DetectorId::StaleFiles, reason, false);
            // Archiving savings depend on the format; claim none.
            c.size_after = c.size;
            c.confidence = Confidence::Unknown;
            c.columns.insert("last_used".into(), format!("{y:04}-{m:02}-{d:02}"));
            out.candidates.push(c);
        }
        if files >= 20 && atime_flat * 100 >= files * 95 {
            out.diagnostics
                .notes
                .push("This volume does not seem to record access times; modification dates were used instead.".into());
        }
        tick.finish();
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::*;
    use mj_core::testutil::TempDir;
    use mj_core::EntryKind;

    fn entry(mod_days: i64, acc_days: Option<i64>, created_days: Option<i64>, now: Timestamp) -> FileEntry {
        let mut e = FileEntry::new("/r/f", EntryKind::File, 10);
        e.modified = Some(now.add_days(-mod_days));
        e.accessed = acc_days.map(|d| now.add_days(-d));
        e.created = created_days.map(|d| now.add_days(-d));
        e
    }

    #[test]
    fn last_used_prefers_real_access_times_and_falls_back() {
        let now = Timestamp::from_ymd_hms(2026, 6, 1, 0, 0, 0);
        // Opened recently although modified long ago: not stale.
        assert_eq!(last_used(&entry(900, Some(5), Some(1000), now)), Some(now.add_days(-5)));
        // Access time equal to creation: never updated, use modification.
        assert_eq!(last_used(&entry(300, Some(1000), Some(1000), now)), Some(now.add_days(-300)));
        // Access time older than modification: modification wins.
        assert_eq!(last_used(&entry(100, Some(800), Some(1000), now)), Some(now.add_days(-100)));
        // No access time at all.
        assert_eq!(last_used(&entry(400, None, None, now)), Some(now.add_days(-400)));
        let mut none = entry(0, None, None, now);
        none.modified = None;
        assert_eq!(last_used(&none), None);
    }

    #[test]
    fn old_files_are_listed_review_only() {
        let t = TempDir::new("stale").unwrap();
        for (f, days) in [("old/a.dat", 500), ("old/b.dat", 800), ("recent/c.dat", 30), ("edge/d.dat", 300)] {
            t.write(f, b"0123456789").unwrap();
            age(&t.join(f), days);
        }
        let ix = index_of(t.path());
        let out = run(&StaleFiles, &ix, &p(&[("min_size_mb", 0.0)]));
        assert_eq!(rel(&out, t.path()), vec!["old/a.dat", "old/b.dat"]);
        assert!(out.candidates.iter().all(|c| !c.default_selected && c.confidence == Confidence::Unknown));
        assert!(out.candidates[0].reason.starts_with("Not opened or changed for"));
        // 6 months threshold also catches the 300-day file; the default 1 MB minimum hides tiny files.
        assert_eq!(run(&StaleFiles, &ix, &p(&[("min_size_mb", 0.0), ("months", 6.0)])).candidates.len(), 3);
        assert!(run(&StaleFiles, &ix, &Default::default()).candidates.is_empty());
    }

    #[test]
    fn disabled_access_times_are_called_out() {
        let t = TempDir::new("stale_atime").unwrap();
        for i in 0..25 {
            let f = format!("f{i}.dat");
            t.write(&f, b"x").unwrap();
            age(&t.join(&f), 3); // atime == mtime for every file
        }
        let out = run(&StaleFiles, &index_of(t.path()), &p(&[("min_size_mb", 0.0)]));
        assert!(out.diagnostics.notes.iter().any(|n| n.contains("access times")));
    }
}
