//! Candidates: individual files or folders a detector or optimiser proposes to act on.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::badge::Badges;
use crate::detector::DetectorId;
use crate::entry::FileStamp;
use crate::estimate::Confidence;
use crate::optimiser::OptimiserId;
use crate::time::Timestamp;

/// Candidate id, unique within one scan generation. Assigned by the engine;
/// detectors leave it at `CandidateId(0)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CandidateId(pub u64);

/// Group id (duplicate group, version chain...). Detector-local in `DetectorOutput`;
/// the engine re-keys groups to be unique per scan generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GroupId(pub u64);

/// What a candidate points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    File,
    /// A whole folder (dev artefacts, caches, inactive projects); acted on as a unit.
    Folder,
}

/// Where a candidate or opportunity comes from.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Detector(DetectorId),
    Optimiser(OptimiserId),
    /// A custom search; the string is the custom-search id assigned by the engine.
    Custom(String),
}

/// One proposed file or folder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub id: CandidateId,
    #[serde(with = "crate::pathutil::serde_path")]
    pub path: PathBuf,
    pub target: TargetKind,
    pub source: Source,
    /// Bytes this candidate occupies now (reclaimable size; folder = subtree total).
    pub size: u64,
    /// Estimated bytes after the default action (0 for removals).
    pub size_after: u64,
    /// How the `size_after` was obtained.
    pub confidence: Confidence,
    /// Short human reason ("Same content as 2 other files", "Not modified for 3 years").
    pub reason: String,
    pub badges: Badges,
    /// Selected unless the user changes it. Forced false by `Badges::AUTO_UNCHECK`.
    pub default_selected: bool,
    /// Duplicate group / version chain this belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<GroupId>,
    /// Sub-group key for the step-review tabs (e.g. `"same_meeting"`), see `Opportunity::sub_groups`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_group: Option<String>,
    pub modified: Option<Timestamp>,
    /// Stamp at scan time; the action engine skips the item when it no longer matches.
    pub stamp: FileStamp,
    /// Files inside, for folder candidates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_count: Option<u64>,
    /// Extra per-source table columns (`"length": "1:02:14"`, `"source": "Teams"`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub columns: BTreeMap<String, String>,
}

impl Candidate {
    /// Estimated saving.
    pub fn saving(&self) -> u64 {
        self.size.saturating_sub(self.size_after)
    }

    /// Pixel count (width x height) when the source recorded it in
    /// `columns[`[`columns::RESOLUTION`]`]` as `"WxH"` or `columns[`[`columns::PIXELS`]`]`
    /// as an integer. Used by `KeepRule::HighestResolution` when a rule is re-applied.
    pub fn pixels(&self) -> Option<u64> {
        if let Some(p) = self.columns.get(columns::PIXELS).and_then(|v| v.trim().parse::<u64>().ok()) {
            return Some(p);
        }
        let r = self.columns.get(columns::RESOLUTION)?;
        let (w, h) = r.trim().split_once(['x', 'X', '×'])?;
        w.trim().parse::<u64>().ok()?.checked_mul(h.trim().parse::<u64>().ok()?)
    }

    /// Per-candidate safety class when a card mixes classes (`columns[`[`columns::SAFETY`]`]`
    /// holding a snake_case [`SafetyClass`] name). `None` = the card's class applies.
    pub fn safety_override(&self) -> Option<crate::safety::SafetyClass> {
        use crate::safety::SafetyClass as S;
        Some(match self.columns.get(columns::SAFETY)?.as_str() {
            "reversible" => S::Reversible,
            "rebuildable" => S::Rebuildable,
            "light_copy" => S::LightCopy,
            "lossy_opt_in" => S::LossyOptIn,
            "review_only" => S::ReviewOnly,
            "permanent" => S::Permanent,
            _ => return None,
        })
    }

    /// Apply the auto-uncheck rule to `default_selected`.
    pub fn normalise_selection(&mut self) {
        if self.badges.intersects(Badges::AUTO_UNCHECK) {
            self.default_selected = false;
        }
    }
}

/// Reserved `Candidate::columns` keys with a meaning to the engine (Wave 3 contract revision).
/// They are additive conventions so `Candidate` keeps its shape.
pub mod columns {
    /// `"WxH"` image or video resolution (shown as a column; feeds `Candidate::pixels`).
    pub const RESOLUTION: &str = "resolution";
    /// Pixel count as a decimal integer (alternative to `RESOLUTION`).
    pub const PIXELS: &str = "pixels";
    /// Per-candidate `SafetyClass` (snake_case) when a card mixes classes. The engine
    /// buckets the hero summary per candidate and requires confirmation for a step whose
    /// selection holds a candidate whose class requires it.
    pub const SAFETY: &str = "safety";
}

/// Kind of a group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupKind {
    /// Byte-identical files (full hash + confirmation).
    Exact,
    /// Perceptually similar images/videos/audio.
    Similar,
    /// v1..vN / "final" naming chain.
    VersionChain,
    /// An export (PDF) and its source (pptx/docx).
    Derived,
    /// The same font installed twice.
    Font,
    /// RAW + JPEG pair, sidecar and parent.
    Pair,
}

/// A group of related candidates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateGroup {
    pub id: GroupId,
    pub kind: GroupKind,
    /// Display label ("12 copies of IMG_0001.JPG").
    pub label: String,
    /// 100 for exact groups; `(1 - diameter/64) * 100` for perceptual groups.
    pub confidence: f32,
    /// Recommended copy to keep (also a candidate with `Badges::KEEPER`, never default-selected).
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::pathutil::serde_path::opt")]
    pub keeper: Option<PathBuf>,
    /// Member paths, keeper included.
    #[serde(with = "crate::pathutil::serde_path::vec")]
    pub members: Vec<PathBuf>,
}

/// Rule that picks which copy of a group to keep (CLN-ACT-01).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "rule", rename_all = "snake_case")]
pub enum KeepRule {
    Newest,
    Oldest,
    ShortestPath,
    HighestResolution,
    /// Keep the copy under this folder; falls back to `Newest`.
    PreferredFolder {
        #[serde(with = "crate::pathutil::serde_path")]
        folder: PathBuf,
    },
}

/// The facts a [`KeepRule`] looks at for one group member.
#[derive(Debug, Clone, Copy)]
pub struct KeepInfo<'a> {
    pub path: &'a std::path::Path,
    pub modified: Option<Timestamp>,
    pub size: u64,
    /// Width x height for images, when known.
    pub pixels: Option<u64>,
}

/// Comparator: `Less` means the first member is preferred.
type KeepCmp<'r> = dyn Fn(&KeepInfo<'_>, &KeepInfo<'_>) -> std::cmp::Ordering + 'r;

impl KeepRule {
    /// Index of the member to keep. Deterministic: ties go to the larger file, then the
    /// shorter path, then the lexically smaller path. `None` only for an empty slice.
    pub fn pick(&self, members: &[KeepInfo<'_>]) -> Option<usize> {
        if members.is_empty() {
            return None;
        }
        let depth = |m: &KeepInfo<'_>| m.path.components().count();
        let tie = |a: &KeepInfo<'_>, b: &KeepInfo<'_>| {
            b.size
                .cmp(&a.size)
                .then(depth(a).cmp(&depth(b)))
                .then(a.path.as_os_str().len().cmp(&b.path.as_os_str().len()))
                .then(a.path.cmp(b.path))
        };
        // `better(a, b) == Less` means a is preferred.
        let better: Box<KeepCmp<'_>> = match self {
            KeepRule::Newest => Box::new(|a, b| b.modified.cmp(&a.modified).then_with(|| tie(a, b))),
            KeepRule::Oldest => Box::new(|a, b| {
                // Unknown dates sort last.
                let key = |m: &KeepInfo<'_>| m.modified.map(|t| (0, t)).unwrap_or((1, Timestamp(0)));
                key(a).cmp(&key(b)).then_with(|| tie(a, b))
            }),
            KeepRule::ShortestPath => Box::new(|a, b| {
                depth(a)
                    .cmp(&depth(b))
                    .then(a.path.as_os_str().len().cmp(&b.path.as_os_str().len()))
                    .then_with(|| tie(a, b))
            }),
            KeepRule::HighestResolution => {
                Box::new(|a, b| b.pixels.unwrap_or(0).cmp(&a.pixels.unwrap_or(0)).then_with(|| tie(a, b)))
            }
            KeepRule::PreferredFolder { folder } => {
                let folder = folder.clone();
                Box::new(move |a, b| {
                    let ina = crate::pathutil::is_within(&folder, a.path);
                    let inb = crate::pathutil::is_within(&folder, b.path);
                    inb.cmp(&ina).then(b.modified.cmp(&a.modified)).then_with(|| tie(a, b))
                })
            }
        };
        (0..members.len()).min_by(|&i, &j| better(&members[i], &members[j]))
    }

    /// [`pick`](Self::pick) for members holding the same bytes, where `linked[i]` says
    /// member `i` shares its data with another file name (a hard link whose other names
    /// are not in the group). Removing such a member frees nothing, so one of them is
    /// kept in preference and the rule chooses among them; the rule over every member
    /// still wins when it is a preferred folder that holds its choice (an explicit wish).
    pub fn pick_same_content(&self, members: &[KeepInfo<'_>], linked: &[bool]) -> Option<usize> {
        let k = self.pick(members)?;
        let is_linked = |i: usize| linked.get(i).copied().unwrap_or(false);
        if is_linked(k) {
            return Some(k);
        }
        if let KeepRule::PreferredFolder { folder } = self {
            if crate::pathutil::is_within(folder, members[k].path) {
                return Some(k);
            }
        }
        let idx: Vec<usize> = (0..members.len()).filter(|&i| is_linked(i)).collect();
        let sub: Vec<KeepInfo<'_>> = idx.iter().map(|&i| members[i]).collect();
        Some(self.pick(&sub).map_or(k, |j| idx[j]))
    }
}

/// What a detector or optimiser analysis could *not* cover. Never silently empty.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Diagnostics {
    /// Files that could not be read.
    pub unreadable: u64,
    #[serde(with = "crate::pathutil::serde_path::vec")]
    pub unreadable_sample: Vec<PathBuf>,
    /// Files deliberately skipped (e.g. no embedded thumbnail: exact-match only), by reason.
    pub skipped: BTreeMap<String, u64>,
    /// Free-form notes for the report ("hash budget reached: 12 files not fully verified").
    pub notes: Vec<String>,
}

impl Diagnostics {
    /// Record an unreadable file (bounded sample of 10).
    pub fn unreadable(&mut self, path: &std::path::Path) {
        self.unreadable += 1;
        if self.unreadable_sample.len() < 10 {
            self.unreadable_sample.push(path.to_path_buf());
        }
    }

    /// Count a skipped file under `reason`.
    pub fn skip(&mut self, reason: &str) {
        *self.skipped.entry(reason.to_string()).or_default() += 1;
    }

    /// Merge another diagnostics into this one.
    pub fn merge(&mut self, other: Diagnostics) {
        self.unreadable += other.unreadable;
        for p in other.unreadable_sample {
            if self.unreadable_sample.len() < 10 {
                self.unreadable_sample.push(p);
            }
        }
        for (k, v) in other.skipped {
            *self.skipped.entry(k).or_default() += v;
        }
        self.notes.extend(other.notes);
    }
}

/// What one detector / optimiser analysis returns.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DetectorOutput {
    pub candidates: Vec<Candidate>,
    #[serde(default)]
    pub groups: Vec<CandidateGroup>,
    #[serde(default)]
    pub diagnostics: Diagnostics,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn keep_rules_are_deterministic() {
        let m = [
            KeepInfo { path: Path::new("/a/b/c/x.jpg"), modified: Some(Timestamp(5)), size: 10, pixels: Some(100) },
            KeepInfo { path: Path::new("/a/x.jpg"), modified: Some(Timestamp(1)), size: 10, pixels: Some(400) },
            KeepInfo { path: Path::new("/pref/x.jpg"), modified: Some(Timestamp(5)), size: 12, pixels: None },
        ];
        assert_eq!(KeepRule::Newest.pick(&m), Some(2)); // tie on date -> larger size
        assert_eq!(KeepRule::Oldest.pick(&m), Some(1));
        assert_eq!(KeepRule::ShortestPath.pick(&m), Some(1)); // depth 2 vs 2 -> shorter string
        assert_eq!(KeepRule::HighestResolution.pick(&m), Some(1));
        assert_eq!(KeepRule::PreferredFolder { folder: "/a/b".into() }.pick(&m), Some(0));
        assert_eq!(KeepRule::Newest.pick(&[]), None);
    }

    #[test]
    fn reserved_columns() {
        let mut c = Candidate {
            id: CandidateId(1),
            path: "/a.jpg".into(),
            target: TargetKind::File,
            source: Source::Custom("x".into()),
            size: 1,
            size_after: 0,
            confidence: Confidence::Exact,
            reason: String::new(),
            badges: Badges::empty(),
            default_selected: true,
            group: None,
            sub_group: None,
            modified: None,
            stamp: FileStamp { size: 1, mtime: Timestamp(0) },
            file_count: None,
            columns: BTreeMap::new(),
        };
        assert_eq!(c.pixels(), None);
        c.columns.insert(columns::RESOLUTION.into(), "4000x3000".into());
        assert_eq!(c.pixels(), Some(12_000_000));
        c.columns.insert(columns::SAFETY.into(), "permanent".into());
        assert_eq!(c.safety_override(), Some(crate::safety::SafetyClass::Permanent));
    }
}
