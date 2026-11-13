//! Smart auto-select for groups (CLN-ACT-01): apply a `KeepRule` to every group
//! of a step and return the resulting selection. Owner: W2-E.

use mj_core::{Candidate, CandidateGroup, CandidateId, KeepInfo, KeepRule};

/// For every candidate in a group: `(id, selected)` where the keeper is unselected and the
/// rest are selected. Candidates outside groups are not returned.
pub fn apply_keep_rule(
    groups: &[CandidateGroup],
    candidates: &[Candidate],
    rule: &KeepRule,
) -> Vec<(CandidateId, bool)> {
    let mut out = Vec::new();
    for g in groups {
        let members: Vec<&Candidate> = candidates.iter().filter(|c| c.group == Some(g.id)).collect();
        let infos: Vec<KeepInfo<'_>> = members
            .iter()
            .map(|c| KeepInfo { path: &c.path, modified: c.modified, size: c.size, pixels: None })
            .collect();
        if let Some(k) = rule.pick(&infos) {
            for (i, c) in members.iter().enumerate() {
                out.push((c.id, i != k));
            }
        }
    }
    out
}
