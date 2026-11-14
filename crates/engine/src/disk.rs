//! Drives, disk-usage tree and breakdowns (SCAN-05/06/07). Owner: W3-D.

use std::path::Path;

use mj_core::{lock, EntryKind, Error, FileIndex, Result};

use crate::api::{
    Breakdown, BreakdownBy, BreakdownRequest, BreakdownRow, DiskNode, DiskOther, DiskUsageRequest, Drive,
};
use crate::Engine;

fn node(index: &FileIndex, i: usize, depth: u8, min_fraction: f32) -> DiskNode {
    let e = index.entry(i);
    let t = index.totals(i);
    let mut node = DiskNode {
        name: e.name(),
        path: e.path.clone(),
        is_dir: e.kind == EntryKind::Dir,
        size: t.bytes,
        allocated: t.reclaimable,
        files: t.files,
        children: Vec::new(),
        other: None,
    };
    if depth > 0 && node.is_dir {
        let mut kids: Vec<DiskNode> = index.children(i).into_iter().map(|c| node_shallow(index, c)).collect();
        kids.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
        let threshold = (node.size as f64 * min_fraction as f64) as u64;
        let (big, small): (Vec<DiskNode>, Vec<DiskNode>) =
            kids.into_iter().partition(|k| k.size >= threshold && k.size > 0);
        if !small.is_empty() {
            node.other = Some(DiskOther { size: small.iter().map(|k| k.size).sum(), count: small.len() as u64 });
        }
        node.children = big
            .into_iter()
            .map(|k| match index.position(&k.path) {
                Some(j) => self::node(index, j, depth - 1, min_fraction),
                None => k,
            })
            .collect();
    }
    node
}

fn node_shallow(index: &FileIndex, i: usize) -> DiskNode {
    let e = index.entry(i);
    let t = index.totals(i);
    DiskNode {
        name: e.name(),
        path: e.path.clone(),
        is_dir: e.kind == EntryKind::Dir,
        size: t.bytes,
        allocated: t.reclaimable,
        files: t.files,
        children: Vec::new(),
        other: None,
    }
}

/// IPC age bucket keys (docs/IPC.md `breakdown`).
fn ipc_age_key(k: &str) -> &str {
    match k {
        "lt_1m" => "<1m",
        "1m_6m" => "1-6m",
        "6m_1y" => "6-12m",
        "1y_2y" => "1-2y",
        "2y_5y" => "2-5y",
        "gt_5y" => ">5y",
        other => other,
    }
}

impl Engine {
    /// Volumes with the plan's effect: each plan step's saving attributed to the volume
    /// holding each selected item (longest mount point wins), scaled to the step estimate.
    pub fn drives_list(&self) -> Result<Vec<Drive>> {
        let vols = self.platform().volumes()?;
        let plan = lock(&self.plan).plan.clone();
        let mut saving = vec![0u64; vols.len()];
        let skip = crate::results::skip_badges(&lock(&self.settings));
        {
            let results = self.results.read().unwrap_or_else(|e| e.into_inner());
            for step in &plan.steps {
                let Some(rec) = results.get(&step.opportunity) else { continue };
                let rows: Vec<(&std::path::Path, u64)> = rec
                    .candidates
                    .iter()
                    .filter(|c| crate::results::is_selected(rec, c, skip))
                    .map(|c| (c.path.as_path(), c.saving()))
                    .collect();
                let sum: u64 = rows.iter().map(|r| r.1).sum();
                let target = step.estimate.saving();
                for (path, s) in rows {
                    let share = if sum == 0 { 0 } else { (s as u128 * target as u128 / sum as u128) as u64 };
                    if let Some((i, _)) = vols
                        .iter()
                        .enumerate()
                        .filter(|(_, v)| mj_core::pathutil::is_within(&v.mount, path))
                        .max_by_key(|(_, v)| v.mount.components().count())
                    {
                        saving[i] += share;
                    }
                }
            }
        }
        Ok(vols
            .into_iter()
            .zip(saving)
            .map(|(v, s)| Drive { free_after_plan: v.free.saturating_add(s), plan_saving: s, volume: v })
            .collect())
    }

    /// Folder tree by size.
    pub fn disk_usage_tree(&self, req: DiskUsageRequest) -> Result<DiskNode> {
        let results = self.results.read().unwrap_or_else(|e| e.into_inner());
        let index = results.index.as_ref().ok_or_else(|| Error::InvalidInput("scan first".into()))?;
        let depth = req.depth.clamp(1, 4);
        let min_fraction = req.min_fraction.unwrap_or(0.01).clamp(0.0, 1.0);
        match &req.path {
            Some(p) => {
                let i = index.position(p).ok_or_else(|| Error::NotFound(p.clone()))?;
                Ok(node(index, i, depth, min_fraction))
            }
            None => {
                let mut root = DiskNode {
                    name: "All locations".into(),
                    path: Path::new("").to_path_buf(),
                    is_dir: true,
                    size: 0,
                    allocated: 0,
                    files: 0,
                    children: Vec::new(),
                    other: None,
                };
                for r in &index.roots {
                    if let Some(i) = index.position(r) {
                        let n = node(index, i, depth - 1, min_fraction);
                        root.size += n.size;
                        root.allocated += n.allocated;
                        root.files += n.files;
                        root.children.push(n);
                    }
                }
                Ok(root)
            }
        }
    }

    /// Size by type, extension, age or owner.
    pub fn breakdown(&self, req: BreakdownRequest) -> Result<Breakdown> {
        let results = self.results.read().unwrap_or_else(|e| e.into_inner());
        let index = results.index.as_ref().ok_or_else(|| Error::InvalidInput("scan first".into()))?;
        let tree = mj_scan::FolderTree::build(index);
        let at = match &req.path {
            Some(p) => Some(index.position(p).ok_or_else(|| Error::NotFound(p.clone()))?),
            None => None,
        };
        let rows = match req.by {
            BreakdownBy::Type => {
                let groups = mj_filters::groups::effective(&lock(&self.settings).type_groups);
                tree.by_group(at, &groups)
            }
            BreakdownBy::Extension => tree.by_extension(at),
            BreakdownBy::Age => {
                let mut rows = tree.by_age(at, mj_core::Timestamp::now());
                for r in &mut rows {
                    r.key = ipc_age_key(&r.key).to_string();
                }
                rows
            }
            BreakdownBy::Owner => tree.by_owner(at),
        };
        Ok(Breakdown {
            by: req.by,
            rows: rows
                .into_iter()
                .map(|r| BreakdownRow { key: r.key, label: r.label, bytes: r.bytes, files: r.files })
                .collect(),
        })
    }
}
