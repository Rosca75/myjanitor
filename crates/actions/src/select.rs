//! Smart auto-select for groups (CLN-ACT-01): apply a `KeepRule` to every group
//! of a step and return the resulting selection. Owner: W2-E.

use mj_core::{Badges, Candidate, CandidateGroup, CandidateId, GroupId, GroupKind, KeepInfo, KeepRule};

/// For every candidate in a group: `(id, selected)` where the keeper is unselected and the
/// rest are selected, except a hard-linked member (removing it frees nothing), which is
/// never selected and, in a group of identical files, kept in preference. Candidates outside groups are not returned.
pub fn apply_keep_rule(
    groups: &[CandidateGroup],
    candidates: &[Candidate],
    rule: &KeepRule,
) -> Vec<(CandidateId, bool)> {
    // ACT-7: bucket the candidates by group once (candidate order kept in each bucket)
    // instead of scanning every candidate for every group.
    let mut by_group: std::collections::HashMap<GroupId, Vec<&Candidate>> = std::collections::HashMap::new();
    for c in candidates {
        if let Some(g) = c.group {
            by_group.entry(g).or_default().push(c);
        }
    }
    let mut out = Vec::new();
    for g in groups {
        let Some(members) = by_group.get(&g.id) else { continue };
        let infos: Vec<KeepInfo<'_>> = members
            .iter()
            .map(|c| KeepInfo { path: &c.path, modified: c.modified, size: c.size, pixels: c.pixels() })
            .collect();
        // DUP-4: a hard-linked name frees nothing when removed (its size is 0).
        let linked: Vec<bool> = members.iter().map(|c| c.badges.contains(Badges::HARDLINKED)).collect();
        let same_content = matches!(g.kind, GroupKind::Exact | GroupKind::Font);
        let k = if same_content { rule.pick_same_content(&infos, &linked) } else { rule.pick(&infos) };
        if let Some(k) = k {
            for (i, c) in members.iter().enumerate() {
                out.push((c.id, i != k && !linked[i]));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::{Confidence, DetectorId, FileStamp, Source, TargetKind, Timestamp};
    use std::path::PathBuf;

    fn cand(id: u64, group: u64, path: &str, modified: i64) -> Candidate {
        Candidate {
            id: CandidateId(id),
            path: PathBuf::from(path),
            target: TargetKind::File,
            source: Source::Detector(DetectorId::ExactDuplicates),
            size: 10,
            size_after: 0,
            confidence: Confidence::Exact,
            reason: String::new(),
            badges: Badges::empty(),
            default_selected: true,
            group: Some(GroupId(group)),
            sub_group: None,
            modified: Some(Timestamp(modified)),
            stamp: FileStamp { size: 10, mtime: Timestamp(modified) },
            file_count: None,
            columns: Default::default(),
        }
    }

    fn group(id: u64) -> CandidateGroup {
        CandidateGroup {
            id: GroupId(id),
            kind: GroupKind::Exact,
            label: String::new(),
            confidence: 100.0,
            keeper: None,
            members: Vec::new(),
        }
    }

    /// Groups in their order, members in candidate order; candidates outside groups and
    /// groups without candidates give nothing.
    #[test]
    fn keeps_one_per_group_in_a_stable_order() {
        let mut lone = cand(9, 0, "/z", 1);
        lone.group = None;
        let cands =
            vec![cand(1, 2, "/b/1", 5), cand(2, 1, "/a/1", 3), lone, cand(3, 2, "/b/2", 9), cand(4, 1, "/a/2", 1)];
        let groups = vec![group(1), group(2), group(3)];
        let out = apply_keep_rule(&groups, &cands, &KeepRule::Newest);
        let ids: Vec<(u64, bool)> = out.iter().map(|(c, s)| (c.0, *s)).collect();
        assert_eq!(ids, [(2, false), (4, true), (1, true), (3, false)]);
    }

    /// DUP-4: a hard-linked member frees nothing when removed: in a group of identical
    /// files it is kept in preference, and it is never selected as a copy to remove.
    #[test]
    fn hard_linked_members_are_kept_and_never_selected() {
        let mut linked = cand(1, 1, "/a/x.bin", 1);
        linked.badges |= Badges::HARDLINKED;
        linked.size = 0;
        let cands = vec![linked.clone(), cand(2, 1, "/a/x-copy.bin", 9)];
        let out = apply_keep_rule(&[group(1)], &cands, &KeepRule::Newest);
        assert_eq!(out, [(CandidateId(1), false), (CandidateId(2), true)]);
        // Similar images are different files: the rule decides, the linked one stays unselected.
        let mut g = group(1);
        g.kind = GroupKind::Similar;
        let cands = vec![linked.clone(), cand(2, 1, "/a/y.jpg", 9), cand(3, 1, "/a/z.jpg", 5)];
        let out = apply_keep_rule(&[g], &cands, &KeepRule::Newest);
        assert_eq!(out, [(CandidateId(1), false), (CandidateId(2), false), (CandidateId(3), true)]);
    }

    /// ACT-7: one pass over the candidates, not one per group (50 000 groups of 3 took
    /// 7.5e9 comparisons under the results write lock).
    #[test]
    fn large_steps_are_linear() {
        let n = 50_000u64;
        let groups: Vec<CandidateGroup> = (0..n).map(group).collect();
        let cands: Vec<Candidate> =
            (0..3 * n).map(|i| cand(i, i % n, &format!("/d/{i}"), (i * 7 % 13) as i64)).collect();
        let started = std::time::Instant::now();
        let out = apply_keep_rule(&groups, &cands, &KeepRule::Oldest);
        assert_eq!(out.len(), 3 * n as usize);
        assert_eq!(out.iter().filter(|(_, s)| !s).count(), n as usize);
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "{:?}", started.elapsed());
    }
}
