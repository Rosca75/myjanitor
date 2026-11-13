//! Exact duplicates (CLN-DET-01): staged size -> partial hash (head/middle/tail 64 KiB, D-005) -> full streaming hash -> SHA-256 or byte confirmation before deletion is offered.
//! Owner: W2-B.
//!
//! Every member of a group is a candidate; the keeper chosen by the keep rule
//! (CLN-ACT-01, default "oldest": the first copy is usually the original in
//! its intended place, later copies are downloads and re-saves) carries
//! `Badges::KEEPER` and is never selected. Groups are confirmed by equal size,
//! full xxh64 and SHA-256 (see [`crate::staged`]).

use std::collections::BTreeMap;

use mj_core::{
    ActionKind, Badges, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, Diagnostics,
    FileEntry, GroupId, GroupKind, KeepRule, PlatformSupport, Result, SafetyClass,
};

use crate::keep::{self, GroupSpec, Selection};
use crate::staged;

/// Exact duplicates: Files with identical content
#[derive(Debug, Default, Clone, Copy)]
pub struct ExactDuplicates;

/// Default keep rule of this detector.
pub const DEFAULT_KEEP: KeepRule = KeepRule::Oldest;

impl Detector for ExactDuplicates {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::ExactDuplicates,
            title: "Exact duplicates".into(),
            description: "Files with identical content".into(),
            category: Category::Clean,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Trash,
            available_actions: vec![
                ActionKind::Trash,
                ActionKind::Quarantine,
                ActionKind::PermanentDelete,
                ActionKind::Move,
                ActionKind::Hardlink,
                ActionKind::Reflink,
            ],
            settings: keep::settings("oldest", true),
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-01".into(), "CLN-ACT-01".into(), "CLN-ACT-06".into()],
            unit: "files".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let files: Vec<&FileEntry> = ctx.index.files().collect();
        let rule = keep::rule_from_params(ctx.params, DEFAULT_KEEP);
        let mut out = DetectorOutput::default();
        let mut diag = Diagnostics::default();
        let groups = staged::exact_groups(ctx, &files, &mut diag)?;
        for (n, g) in groups.iter().enumerate() {
            let members: Vec<&FileEntry> = g.members.iter().map(|&i| files[i]).collect();
            let pixels = pixels_of(ctx, &members);
            let keeper = keep::pick(&rule, &members, &pixels);
            emit_exact(GroupId(n as u64 + 1), DetectorId::ExactDuplicates, members, keeper, g, Selection::AllButKeeper, &mut out);
        }
        out.diagnostics = diag;
        Ok(out)
    }
}

/// Pixel counts from the fingerprint cache (for "highest resolution"); never reads files.
pub(crate) fn pixels_of(ctx: &DetectContext<'_>, members: &[&FileEntry]) -> Vec<Option<u64>> {
    members
        .iter()
        .map(|e| {
            let fp = ctx.fingerprints.get(&e.path, e.stamp())?;
            Some(fp.width? as u64 * fp.height? as u64)
        })
        .collect()
}

/// Emit one confirmed exact group.
pub(crate) fn emit_exact(
    id: GroupId,
    source: DetectorId,
    members: Vec<&FileEntry>,
    keeper: usize,
    g: &staged::ExactGroup,
    selection: Selection,
    out: &mut DetectorOutput,
) {
    let n = members.len();
    let name = members[keeper.min(n - 1)].name();
    let others = n - 1;
    let reason = format!("Same content as {others} other file{}", if others == 1 { "" } else { "s" });
    let hash = mj_core::hash::to_hex(&g.sha256[..8]);
    let columns = (0..n).map(|_| BTreeMap::from([("sha256".to_string(), hash.clone())])).collect();
    keep::emit(
        GroupSpec {
            id,
            kind: GroupKind::Exact,
            source,
            label: format!("{n} copies of {name}"),
            confidence: 100.0,
            members,
            keeper,
            selection,
            badges: Badges::DUPLICATE,
            reason,
            keeper_reason: "Copy kept (keep rule)".into(),
            columns,
        },
        out,
    );
}
