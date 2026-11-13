//! Which copy to keep (CLN-ACT-01): `KeepRule` evaluation and the quality
//! score for images (research §6), with deterministic tie-breaks (larger size,
//! then older mtime, then path). Owner: W2-B.
//!
//! The rule itself ([`KeepRule::pick`]) is part of the frozen contract in
//! `mj-core`; this module maps detector parameters to a rule, provides the
//! shared settings, and turns a group plus its keeper into candidates: exactly
//! one member per group is the keeper (`Badges::KEEPER`, never selected).

use std::collections::BTreeMap;
use std::path::PathBuf;

use mj_core::fingerprint::ExifSummary;
use mj_core::params::ChoiceOption;
use mj_core::{
    Badges, Candidate, CandidateGroup, CandidateId, Confidence, DetectorId, FileEntry, GroupId, GroupKind, KeepInfo,
    KeepRule, ParamValues, SettingKind, SettingSpec, Source, TargetKind,
};

/// Parameter key of the keep-rule choice.
pub const KEEP_RULE: &str = "keep_rule";
/// Parameter key of the preferred folder (used by `preferred_folder`).
pub const PREFERRED_FOLDER: &str = "preferred_folder";

/// Wire value of a rule (the `rule` tag of `KeepRule`).
pub fn rule_name(rule: &KeepRule) -> &'static str {
    match rule {
        KeepRule::Newest => "newest",
        KeepRule::Oldest => "oldest",
        KeepRule::ShortestPath => "shortest_path",
        KeepRule::HighestResolution => "highest_resolution",
        KeepRule::PreferredFolder { .. } => "preferred_folder",
    }
}

/// The rule chosen in `params` (unknown values and an empty preferred folder fall back to `default`).
pub fn rule_from_params(params: &ParamValues, default: KeepRule) -> KeepRule {
    let fallback = rule_name(&default);
    match params.text(KEEP_RULE, fallback) {
        "newest" => KeepRule::Newest,
        "oldest" => KeepRule::Oldest,
        "shortest_path" => KeepRule::ShortestPath,
        "highest_resolution" => KeepRule::HighestResolution,
        "preferred_folder" => {
            let f = params.text(PREFERRED_FOLDER, "").trim();
            if f.is_empty() {
                default
            } else {
                KeepRule::PreferredFolder { folder: PathBuf::from(f) }
            }
        }
        _ => default,
    }
}

/// Keep-rule settings (choice + preferred folder). `images` adds "highest resolution".
pub fn settings(default: &str, images: bool) -> Vec<SettingSpec> {
    let opt = |v: &str, l: &str| ChoiceOption { value: v.into(), label: l.into(), help: None };
    let mut options = vec![opt("newest", "Keep newest"), opt("oldest", "Keep oldest"), opt("shortest_path", "Keep shortest path")];
    if images {
        options.push(opt("highest_resolution", "Keep highest resolution"));
    }
    options.push(opt("preferred_folder", "Keep the copy in a preferred folder"));
    vec![
        SettingSpec {
            key: KEEP_RULE.into(),
            label: "Copy to keep".into(),
            help: Some("Exactly one file of every group is always kept".into()),
            kind: SettingKind::Choice { default: default.into(), options },
            advanced: false,
            lossy: false,
            disabled_reason: None,
        },
        SettingSpec {
            key: PREFERRED_FOLDER.into(),
            label: "Preferred folder".into(),
            help: Some("Used by \"preferred folder\"; falls back to the newest copy".into()),
            kind: SettingKind::Folder { default: String::new() },
            advanced: true,
            lossy: false,
            disabled_reason: None,
        },
    ]
}

/// Index of the keeper among `members` (never `None` for a non-empty group).
pub fn pick(rule: &KeepRule, members: &[&FileEntry], pixels: &[Option<u64>]) -> usize {
    let infos: Vec<KeepInfo<'_>> = members
        .iter()
        .enumerate()
        .map(|(i, e)| KeepInfo {
            path: &e.path,
            modified: e.modified,
            size: e.size,
            pixels: pixels.get(i).copied().flatten(),
        })
        .collect();
    rule.pick(&infos).unwrap_or(0)
}

/// Image quality score 0..=100 (research §6), computed at group time, never cached.
pub fn quality_score(width: Option<u32>, height: Option<u32>, size: u64, exif: Option<&ExifSummary>) -> u8 {
    let mut score = 0.0f64;
    if let (Some(w), Some(h)) = (width, height) {
        let mp = w as f64 * h as f64 / 1e6;
        if mp > 0.0 {
            score += (mp.log2() * 7.5).clamp(0.0, 30.0);
        }
    }
    let mib = size as f64 / (1024.0 * 1024.0);
    if mib > 0.0 {
        score += (mib.log2() * 3.0).clamp(0.0, 15.0);
    }
    if let Some(x) = exif {
        if x.date_taken.is_some() {
            score += 20.0;
        }
        if x.has_gps {
            score += 15.0;
        }
        if x.camera.as_deref().is_some_and(|c| !c.is_empty()) {
            score += 5.0;
        }
        if x.lens.as_deref().is_some_and(|c| !c.is_empty()) {
            score += 5.0;
        }
        if x.has_description {
            score += 10.0;
        }
    }
    score.clamp(0.0, 100.0) as u8
}

/// One candidate row for `entry` (ids are assigned by the engine).
pub fn candidate(entry: &FileEntry, source: DetectorId, reason: String) -> Candidate {
    let mut badges = Badges::empty();
    if entry.links.is_some_and(|l| l > 1) {
        badges |= Badges::HARDLINKED;
    }
    Candidate {
        id: CandidateId(0),
        path: entry.path.clone(),
        target: TargetKind::File,
        source: Source::Detector(source),
        size: entry.reclaimable(),
        size_after: 0,
        confidence: Confidence::Exact,
        reason,
        badges,
        default_selected: false,
        group: None,
        sub_group: None,
        modified: entry.modified,
        stamp: entry.stamp(),
        file_count: None,
        columns: BTreeMap::new(),
    }
}

/// How the non-keeper members of a group are pre-selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    /// Every member except the keeper is selected.
    AllButKeeper,
    /// Nothing is selected (review-only results).
    None,
}

/// Facts needed to emit one group.
pub struct GroupSpec<'a> {
    pub id: GroupId,
    pub kind: GroupKind,
    pub source: DetectorId,
    pub label: String,
    pub confidence: f32,
    pub members: Vec<&'a FileEntry>,
    pub keeper: usize,
    pub selection: Selection,
    /// Badges added to every member.
    pub badges: Badges,
    /// Reason text for a non-keeper member.
    pub reason: String,
    /// Reason text for the keeper.
    pub keeper_reason: String,
    /// Extra columns per member (same order as `members`).
    pub columns: Vec<BTreeMap<String, String>>,
}

/// Turn a group into its candidates (keeper included) and the group record.
pub fn emit(spec: GroupSpec<'_>, out: &mut mj_core::DetectorOutput) {
    let keeper = spec.keeper.min(spec.members.len().saturating_sub(1));
    for (i, e) in spec.members.iter().enumerate() {
        let is_keeper = i == keeper;
        let reason = if is_keeper { spec.keeper_reason.clone() } else { spec.reason.clone() };
        let mut c = candidate(e, spec.source, reason);
        c.badges |= spec.badges;
        c.group = Some(spec.id);
        if is_keeper {
            c.badges |= Badges::KEEPER;
            c.default_selected = false;
        } else {
            c.default_selected = spec.selection == Selection::AllButKeeper;
        }
        if let Some(cols) = spec.columns.get(i) {
            c.columns.extend(cols.clone());
        }
        c.normalise_selection();
        out.candidates.push(c);
    }
    out.groups.push(CandidateGroup {
        id: spec.id,
        kind: spec.kind,
        label: spec.label,
        confidence: spec.confidence,
        keeper: spec.members.get(keeper).map(|e| e.path.clone()),
        members: spec.members.iter().map(|e| e.path.clone()).collect(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::{EntryKind, ParamValue, Timestamp};

    fn e(p: &str, t: i64, size: u64) -> FileEntry {
        let mut f = FileEntry::new(p, EntryKind::File, size);
        f.modified = Some(Timestamp(t));
        f
    }

    #[test]
    fn params_map_to_rules() {
        let p = ParamValues::new().with(KEEP_RULE, ParamValue::Text("newest".into()));
        assert_eq!(rule_from_params(&p, KeepRule::Oldest), KeepRule::Newest);
        let p = ParamValues::new().with(KEEP_RULE, ParamValue::Text("preferred_folder".into()));
        assert_eq!(rule_from_params(&p, KeepRule::Oldest), KeepRule::Oldest, "empty folder falls back");
        let p = p.with(PREFERRED_FOLDER, ParamValue::Text("/keep".into()));
        assert_eq!(rule_from_params(&p, KeepRule::Oldest), KeepRule::PreferredFolder { folder: "/keep".into() });
        let p = ParamValues::new().with(KEEP_RULE, ParamValue::Text("bogus".into()));
        assert_eq!(rule_from_params(&p, KeepRule::ShortestPath), KeepRule::ShortestPath);
    }

    #[test]
    fn every_rule_keeps_exactly_one() {
        let a = e("/r/deep/er/a.jpg", 10, 100);
        let b = e("/r/b.jpg", 30, 100);
        let c = e("/r/keep/c.jpg", 20, 100);
        let m = [&a, &b, &c];
        let px = [Some(10), Some(5), Some(20)];
        assert_eq!(pick(&KeepRule::Newest, &m, &px), 1);
        assert_eq!(pick(&KeepRule::Oldest, &m, &px), 0);
        assert_eq!(pick(&KeepRule::ShortestPath, &m, &px), 1);
        assert_eq!(pick(&KeepRule::HighestResolution, &m, &px), 2);
        assert_eq!(pick(&KeepRule::PreferredFolder { folder: "/r/keep".into() }, &m, &px), 2);
        for rule in [KeepRule::Newest, KeepRule::Oldest, KeepRule::ShortestPath, KeepRule::HighestResolution] {
            let mut out = mj_core::DetectorOutput::default();
            emit(
                GroupSpec {
                    id: GroupId(1),
                    kind: GroupKind::Exact,
                    source: DetectorId::ExactDuplicates,
                    label: "x".into(),
                    confidence: 100.0,
                    members: m.to_vec(),
                    keeper: pick(&rule, &m, &px),
                    selection: Selection::AllButKeeper,
                    badges: Badges::DUPLICATE,
                    reason: "dup".into(),
                    keeper_reason: "keep".into(),
                    columns: vec![],
                },
                &mut out,
            );
            let keepers: Vec<_> = out.candidates.iter().filter(|c| c.badges.contains(Badges::KEEPER)).collect();
            assert_eq!(keepers.len(), 1);
            assert!(!keepers[0].default_selected);
            assert_eq!(out.candidates.iter().filter(|c| c.default_selected).count(), 2);
            assert_eq!(out.groups[0].keeper.as_ref(), Some(&keepers[0].path));
        }
    }

    #[test]
    fn quality_score_follows_research_table() {
        assert_eq!(quality_score(None, None, 0, None), 0);
        let x = ExifSummary {
            date_taken: Some(Timestamp(1)),
            camera: Some("Canon".into()),
            lens: Some("L".into()),
            has_gps: true,
            has_description: true,
        };
        // 16 MP -> 30, 32 MiB -> 15, exif 55.
        assert_eq!(quality_score(Some(4000), Some(4000), 32 << 20, Some(&x)), 100);
        assert_eq!(quality_score(Some(2000), Some(1000), 1 << 20, None), 7);
    }
}
