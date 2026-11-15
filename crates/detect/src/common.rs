//! Shared helpers for detectors (W2-D): progress/cancel ticking, candidate builders, badges from
//! entry flags, topmost-folder reduction (CLN-ENG-04), age tests against `DetectContext::now`,
//! settings builders and small text helpers.
//!
//! Everything here is read-only. Detectors never delete or modify anything; they only describe
//! what could be acted on.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use mj_core::{
    Attributes, Badges, Candidate, CandidateId, Confidence, DetectContext, DetectorId, EntryKind, FileEntry, FileIndex,
    Phase, Progress, Result, SettingKind, SettingSpec, Source, TargetKind, Timestamp,
};

/// How many examined entries pass between two cancel checkpoints.
const CHECKPOINT_EVERY: u64 = 64;
/// How many examined entries pass between two progress reports (the engine throttles on top).
const REPORT_EVERY: u64 = 512;

/// Cancel + progress bookkeeping for one detector run.
///
/// Call [`Ticker::tick`] once per examined entry and [`Ticker::found`] for every candidate: the
/// progress sink then sees the live running total of bytes found (CLN-ENG progress rule), and a
/// cancelled token ends the run within [`CHECKPOINT_EVERY`] entries.
pub struct Ticker<'a, 'b> {
    ctx: &'a DetectContext<'b>,
    done: u64,
    total: u64,
    bytes: u64,
}

impl<'a, 'b> Ticker<'a, 'b> {
    /// Start a run over `total` items.
    pub fn new(ctx: &'a DetectContext<'b>, total: u64) -> Self {
        Ticker { ctx, done: 0, total, bytes: 0 }
    }

    /// Count one examined item; honours pause/cancel.
    pub fn tick(&mut self) -> Result<()> {
        self.done += 1;
        if self.done % CHECKPOINT_EVERY == 1 {
            self.ctx.cancel.checkpoint()?;
        }
        if self.done.is_multiple_of(REPORT_EVERY) {
            self.report(None);
        }
        Ok(())
    }

    /// Count a candidate that saves `bytes`.
    pub fn found(&mut self, bytes: u64) {
        self.bytes = self.bytes.saturating_add(bytes);
    }

    /// Report the final totals (always passes the throttle because `done >= total`).
    pub fn finish(&mut self) {
        self.done = self.done.max(self.total);
        self.report(None);
    }

    fn report(&self, current: Option<&Path>) {
        let mut p = Progress::new(Phase::Detecting, self.done, Some(self.total.max(self.done)));
        p.bytes = self.bytes;
        p.current = current.map(Path::to_path_buf);
        self.ctx.progress.report(&p);
    }
}

/// True when an entry's content is local and the entry is not the scan root itself.
/// Online-only placeholders are never proposed (their removal frees nothing and may trigger downloads).
pub fn usable(e: &FileEntry) -> bool {
    !e.is_online_only() && e.depth > 0
}

/// Badges derivable from the entry flags alone (the engine adds protected/synced from the safety inspector).
pub fn badges_for(e: &FileEntry) -> Badges {
    let mut b = Badges::empty();
    if e.attrs.contains(Attributes::ENCRYPTED) {
        b |= Badges::ENCRYPTED;
    }
    if e.attrs.intersects(Attributes::CLOUD_LOCAL | Attributes::PINNED) {
        b |= Badges::SYNCED;
    }
    if e.is_online_only() {
        b |= Badges::ONLINE_ONLY;
    }
    if e.kind == EntryKind::File && e.links.is_some_and(|n| n > 1) {
        b |= Badges::HARDLINKED;
    }
    b
}

/// A removal candidate for one file or link (nothing remains after the action).
pub fn file_candidate(e: &FileEntry, id: DetectorId, reason: impl Into<String>, selected: bool) -> Candidate {
    let target = if e.kind == EntryKind::Dir { TargetKind::Folder } else { TargetKind::File };
    let mut c = Candidate {
        id: CandidateId(0),
        path: e.path.clone(),
        target,
        source: Source::Detector(id),
        size: e.reclaimable(),
        size_after: 0,
        confidence: Confidence::Exact,
        reason: reason.into(),
        badges: badges_for(e),
        default_selected: selected,
        group: None,
        sub_group: None,
        modified: e.modified,
        stamp: e.stamp(),
        file_count: None,
        columns: BTreeMap::new(),
    };
    c.normalise_selection();
    c
}

/// A folder candidate whose size and file count are the subtree totals.
pub fn folder_candidate(
    index: &FileIndex,
    pos: usize,
    id: DetectorId,
    reason: impl Into<String>,
    selected: bool,
) -> Candidate {
    let e = index.entry(pos);
    let t = index.totals(pos);
    let mut c = file_candidate(e, id, reason, selected);
    c.target = TargetKind::Folder;
    c.size = t.reclaimable;
    c.file_count = Some(t.files);
    c.modified = t.newest_modified.or(e.modified);
    c
}

/// Keep only the topmost of `positions` (ascending): drop every position that has an ancestor in the set.
/// Nested matches are therefore never double counted.
pub fn topmost(index: &FileIndex, positions: &[usize]) -> Vec<usize> {
    let set: HashSet<usize> = positions.iter().copied().collect();
    positions
        .iter()
        .copied()
        .filter(|&p| {
            let mut cur = index.parent(p);
            while let Some(a) = cur {
                if set.contains(&a) {
                    return false;
                }
                cur = index.parent(a);
            }
            true
        })
        .collect()
}

/// True when `ts` is known and strictly before `cutoff`.
pub fn before(ts: Option<Timestamp>, cutoff: Timestamp) -> bool {
    ts.is_some_and(|t| t < cutoff)
}

/// Cutoff `hours` before `now`.
pub fn hours_ago(now: Timestamp, hours: f64) -> Timestamp {
    let secs = (hours.max(0.0) * 3600.0) as i64;
    Timestamp(now.0.saturating_sub(secs.saturating_mul(mj_core::time::NANOS_PER_SEC)))
}

/// Human age such as "3 years", "5 months" or "12 days", from `then` to `now`.
pub fn human_age(then: Timestamp, now: Timestamp) -> String {
    let days = then.days_until(now).max(0);
    let plural = |n: i64, unit: &str| format!("{n} {unit}{}", if n == 1 { "" } else { "s" });
    if days >= 730 {
        plural(days / 365, "year")
    } else if days >= 60 {
        plural(days / 30, "month")
    } else {
        plural(days, "day")
    }
}

/// Lower-case file name.
pub fn name_lower(e: &FileEntry) -> String {
    e.path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// File name without its last extension (case preserved).
pub fn stem(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    }
}

/// Slug usable as a sub-group key: lower-case ASCII letters, digits and `_`.
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    out.trim_end_matches('_').to_string()
}

/// True when any component of `path` equals one of `names` (ASCII case-insensitive).
pub fn has_component(path: &Path, names: &[&str]) -> bool {
    path.components().any(|c| {
        let s = c.as_os_str().to_string_lossy();
        names.iter().any(|n| s.eq_ignore_ascii_case(n))
    })
}

/// Number settings builder.
pub fn number_setting(key: &str, label: &str, help: &str, default: f64, min: f64, max: f64, unit: &str) -> SettingSpec {
    SettingSpec {
        key: key.into(),
        label: label.into(),
        help: Some(help.into()),
        kind: SettingKind::Number { default, min, max, step: 1.0, unit: Some(unit.into()) },
        advanced: false,
        lossy: false,
        disabled_reason: None,
    }
}

/// Text setting builder (comma separated lists, file paths).
pub fn text_setting(key: &str, label: &str, help: &str, default: &str, advanced: bool) -> SettingSpec {
    SettingSpec {
        key: key.into(),
        label: label.into(),
        help: Some(help.into()),
        kind: SettingKind::Text { default: default.into(), placeholder: None },
        advanced,
        lossy: false,
        disabled_reason: None,
    }
}

/// Split a comma/semicolon separated list into trimmed, lower-case, non-empty items.
pub fn split_list(s: &str) -> Vec<String> {
    s.split([',', ';']).map(|x| x.trim().to_lowercase()).filter(|x| !x.is_empty()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn slug_and_stem() {
        assert_eq!(slug("Google Chrome (Cache)"), "google_chrome_cache");
        assert_eq!(stem("a.b.txt"), "a.b");
        assert_eq!(stem(".bashrc"), ".bashrc");
        assert_eq!(split_list(" En, fr;DE ,,"), vec!["en", "fr", "de"]);
    }

    #[test]
    fn human_ages() {
        let now = Timestamp::from_ymd_hms(2026, 6, 1, 0, 0, 0);
        assert_eq!(human_age(now.add_days(-3), now), "3 days");
        assert_eq!(human_age(now.add_days(-400), now), "13 months");
        assert_eq!(human_age(now.add_days(-1500), now), "4 years");
    }

    #[test]
    fn topmost_drops_nested_and_keeps_siblings() {
        let e = |p: &str| FileEntry::new(p, EntryKind::Dir, 0);
        let ix = FileIndex::new(
            vec![PathBuf::from("/r")],
            vec![e("/r"), e("/r/a"), e("/r/a/b"), e("/r/a-x"), e("/r/c")],
            Default::default(),
            Timestamp(0),
        );
        let pos = |p: &str| ix.position(Path::new(p)).unwrap();
        let mut all = vec![pos("/r/a"), pos("/r/a/b"), pos("/r/a-x"), pos("/r/c")];
        all.sort();
        let top = topmost(&ix, &all);
        let names: Vec<_> = top.iter().map(|&i| ix.entry(i).path.to_string_lossy().into_owned()).collect();
        assert_eq!(names, vec!["/r/a", "/r/a-x", "/r/c"]);
    }

    #[test]
    fn badges_follow_flags() {
        let mut e = FileEntry::new("/r/x", EntryKind::File, 5);
        e.attrs |= Attributes::CLOUD_LOCAL | Attributes::ENCRYPTED;
        e.links = Some(2);
        let b = badges_for(&e);
        assert!(b.contains(Badges::SYNCED | Badges::ENCRYPTED | Badges::HARDLINKED));
        // Encrypted files are auto-unchecked by the contract.
        e.depth = 1;
        assert!(!file_candidate(&e, DetectorId::EmptyFiles, "x", true).default_selected);
    }
}
