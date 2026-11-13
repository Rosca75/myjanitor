//! Drives, disk-usage tree and breakdowns (SCAN-05/06/07). Owner: W3-D.

use std::path::Path;

use mj_core::{EntryKind, Error, FileIndex, Result};

use crate::api::{Breakdown, BreakdownRequest, DiskNode, DiskOther, DiskUsageRequest, Drive};
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

impl Engine {
    /// Volumes with the plan's effect.
    pub fn drives_list(&self) -> Result<Vec<Drive>> {
        let vols = self.platform().volumes()?;
        Ok(vols.into_iter().map(|v| Drive { free_after_plan: v.free, plan_saving: 0, volume: v }).collect())
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
        let _ = req;
        Err(Error::not_implemented("breakdown (W3-D)"))
    }
}
