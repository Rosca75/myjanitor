//! Order and paging of the step review table (W6-A).
//!
//! Two modes over the same filters:
//! - flat: every row sorted on its own (the historical behaviour);
//! - "keep groups together": the groups are ordered by an aggregate of the sort column,
//!   each group's rows stay contiguous (kept copy first), filters keep or drop whole
//!   groups, and pages are packed by groups so a page never splits a group that fits
//!   on one page.
//!
//! Group aggregates (the owner's rule for size; the others chosen so the order reads
//! the way the column does, see docs/decisions/W6-A.md):
//! - `size`: the group's largest file; `saving`: what the group frees (sum of savings);
//! - `name` / `path`: the group's lead row (kept copy, else the first path);
//! - `modified`: the group's most recently modified member; `group`: the group id.
//!
//! The order of one request shape is cached (one entry, see [`OrderCache`]) and checked
//! against a fingerprint of the candidates, so paging through a 1M-row step sorts once.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;

use mj_core::{Badges, Candidate, GroupId, OpportunityId};

use crate::api::{CandidateSortKey, GroupStats};

/// The request shape an order depends on (everything but the offset).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Query {
    pub sub_group: Option<String>,
    /// Lower-cased, non-empty.
    pub filter: Option<String>,
    pub min_size: Option<u64>,
    pub sort: CandidateSortKey,
    pub descending: bool,
    /// Effective: asked for and the step has groups.
    pub keep_groups: bool,
    pub limit: usize,
}

/// Rows in display order.
#[derive(Debug, Default)]
pub(crate) struct Order {
    /// Indices into the record's candidates.
    pub rows: Vec<u32>,
    /// Grouped mode: first row of every page (`[0]` when empty).
    pub page_starts: Option<Vec<u64>>,
    /// Grouped mode: totals of every shown group.
    pub stats: HashMap<GroupId, GroupStats>,
}

/// One page cut out of an [`Order`].
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PageSpan {
    pub start: u64,
    pub end: u64,
    pub page: u64,
    pub pages: u64,
    pub next: Option<u64>,
    pub prev: Option<u64>,
}

impl Order {
    pub fn total(&self) -> u64 {
        self.rows.len() as u64
    }

    /// The page holding `offset`. Flat mode keeps the historical arithmetic (any offset).
    pub fn span(&self, offset: u64, limit: usize) -> PageSpan {
        let total = self.total();
        let limit = limit.max(1) as u64;
        match &self.page_starts {
            None => PageSpan {
                start: offset,
                end: offset.saturating_add(limit).min(total).max(offset),
                page: offset / limit + 1,
                pages: total.div_ceil(limit).max(1),
                next: (offset.saturating_add(limit) < total).then(|| offset + limit),
                prev: (offset > 0).then(|| offset.saturating_sub(limit)),
            },
            Some(starts) => {
                let i = starts.partition_point(|&s| s <= offset).saturating_sub(1);
                let start = starts[i];
                let end = starts.get(i + 1).copied().unwrap_or(total);
                PageSpan {
                    start,
                    end,
                    page: i as u64 + 1,
                    pages: starts.len() as u64,
                    next: starts.get(i + 1).copied(),
                    prev: i.checked_sub(1).map(|j| starts[j]),
                }
            }
        }
    }
}

fn keeper(c: &Candidate) -> bool {
    c.badges.contains(Badges::KEEPER)
}

fn name_lower(c: &Candidate) -> Option<String> {
    c.path.file_name().map(|n| n.to_string_lossy().to_lowercase())
}

/// The flat order of two rows for a column, "natural" direction (biggest first for
/// `saving` and `size`), the path as total tie-breaker.
fn cmp_rows(sort: CandidateSortKey, a: &Candidate, b: &Candidate) -> Ordering {
    let by_path = || a.path.cmp(&b.path);
    match sort {
        CandidateSortKey::Saving => b.saving().cmp(&a.saving()).then_with(by_path),
        CandidateSortKey::Size => b.size.cmp(&a.size).then_with(by_path),
        CandidateSortKey::Name => name_lower(a).cmp(&name_lower(b)).then_with(by_path),
        CandidateSortKey::Path => by_path(),
        CandidateSortKey::Modified => a.modified.cmp(&b.modified).then_with(by_path),
        CandidateSortKey::Group => (a.group, !keeper(a)).cmp(&(b.group, !keeper(b))).then_with(by_path),
    }
}

/// Cheap O(n) fingerprint of what an order depends on, so a cached order is never
/// served after the candidates changed (apply, keep rule, re-analysis, rescan).
pub(crate) fn fingerprint(cands: &[Candidate]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ cands.len() as u64;
    for c in cands {
        let v = c.id.0
            ^ c.size.rotate_left(17)
            ^ c.size_after.rotate_left(29)
            ^ c.group.map_or(u64::MAX, |g| g.0).rotate_left(41)
            ^ (keeper(c) as u64) << 63
            ^ c.modified.map_or(0, |t| t.0 as u64).rotate_left(7)
            ^ (c.path.as_os_str().len() as u64).rotate_left(53);
        h = (h ^ v).wrapping_mul(0x0100_0000_01b3).rotate_left(5);
    }
    h
}

struct Block {
    group: Option<GroupId>,
    members: Vec<u32>,
    lead: u32,
    largest: u64,
    smallest: u64,
    saving: u64,
    newest: Option<mj_core::Timestamp>,
    matched: bool,
}

/// Compute the order of `cands` for `q`.
pub(crate) fn order(cands: &[Candidate], q: &Query) -> Order {
    let in_tab = |c: &Candidate| q.sub_group.is_none() || c.sub_group == q.sub_group;
    let matches = |c: &Candidate| q.filter.as_ref().is_none_or(|f| c.path.to_string_lossy().to_lowercase().contains(f));
    if !q.keep_groups {
        let mut rows: Vec<u32> = (0..cands.len() as u32)
            .filter(|&i| {
                let c = &cands[i as usize];
                in_tab(c) && q.min_size.is_none_or(|m| c.size >= m) && matches(c)
            })
            .collect();
        rows.sort_by(|&a, &b| cmp_rows(q.sort, &cands[a as usize], &cands[b as usize]));
        if q.descending {
            rows.reverse();
        }
        return Order { rows, page_starts: None, stats: HashMap::new() };
    }

    // Blocks: one per group, one per ungrouped row.
    let mut blocks: Vec<Block> = Vec::new();
    let mut by_group: HashMap<GroupId, usize> = HashMap::new();
    for (i, c) in cands.iter().enumerate() {
        if !in_tab(c) {
            continue;
        }
        let i = i as u32;
        let b = match c.group.and_then(|g| by_group.get(&g).copied()) {
            Some(b) => b,
            None => {
                blocks.push(Block {
                    group: c.group,
                    members: Vec::new(),
                    lead: i,
                    largest: 0,
                    smallest: u64::MAX,
                    saving: 0,
                    newest: None,
                    matched: false,
                });
                if let Some(g) = c.group {
                    by_group.insert(g, blocks.len() - 1);
                }
                blocks.len() - 1
            }
        };
        let blk = &mut blocks[b];
        blk.members.push(i);
        blk.largest = blk.largest.max(c.size);
        blk.smallest = blk.smallest.min(c.size);
        blk.saving = blk.saving.saturating_add(c.saving());
        blk.newest = blk.newest.max(c.modified);
        if !blk.matched && matches(c) {
            blk.matched = true;
        }
        let lead = &cands[blk.lead as usize];
        if (keeper(c), std::cmp::Reverse(&c.path)) > (keeper(lead), std::cmp::Reverse(&lead.path)) {
            blk.lead = i;
        }
    }
    blocks.retain(|b| b.matched && q.min_size.is_none_or(|m| b.largest >= m));

    let lead = |b: &Block| &cands[b.lead as usize];
    let names: Vec<Option<String>> = match q.sort {
        CandidateSortKey::Name => blocks.iter().map(|b| name_lower(lead(b))).collect(),
        _ => Vec::new(),
    };
    let mut idx: Vec<usize> = (0..blocks.len()).collect();
    idx.sort_by(|&x, &y| {
        let (a, b) = (&blocks[x], &blocks[y]);
        let by_lead = || lead(a).path.cmp(&lead(b).path).then(a.group.cmp(&b.group));
        match q.sort {
            CandidateSortKey::Saving => b.saving.cmp(&a.saving).then_with(by_lead),
            CandidateSortKey::Size => b.largest.cmp(&a.largest).then_with(by_lead),
            CandidateSortKey::Name => names[x].cmp(&names[y]).then_with(by_lead),
            CandidateSortKey::Path => by_lead(),
            CandidateSortKey::Modified => a.newest.cmp(&b.newest).then_with(by_lead),
            CandidateSortKey::Group => a.group.cmp(&b.group).then_with(by_lead),
        }
    });
    if q.descending {
        idx.reverse();
    }

    let limit = q.limit.max(1);
    let total: usize = blocks.iter().map(|b| b.members.len()).sum();
    let mut rows = Vec::with_capacity(total);
    let mut starts = vec![0u64];
    let mut stats = HashMap::new();
    let mut used = 0usize;
    for x in idx {
        let b = &mut blocks[x];
        // Inside a group: the kept copy first, then the column's natural order.
        b.members.sort_by(|&m, &n| {
            let (c, d) = (&cands[m as usize], &cands[n as usize]);
            keeper(d).cmp(&keeper(c)).then_with(|| cmp_rows(q.sort, c, d))
        });
        if let Some(g) = b.group {
            let kept = b.members.iter().filter(|&&m| keeper(&cands[m as usize])).count();
            let savings =
                b.members.iter().filter(|&&m| !keeper(&cands[m as usize])).map(|&m| cands[m as usize].saving());
            let freed: u64 = savings.sum();
            let reclaimable = if kept > 0 {
                freed
            } else {
                // No kept copy marked: apply still keeps one, the one freeing the most.
                let best = b.members.iter().map(|&m| cands[m as usize].saving()).max().unwrap_or(0);
                freed.saturating_sub(best)
            };
            stats.insert(
                g,
                GroupStats {
                    id: g,
                    files: b.members.len() as u64,
                    largest: b.largest,
                    smallest: b.smallest,
                    reclaimable,
                },
            );
        }
        // Page packing: a block that does not fit starts a new page; a block larger
        // than a page is split (unavoidable), its tail shares a page with what follows.
        let n = b.members.len();
        if used > 0 && used + n > limit {
            starts.push(rows.len() as u64);
            used = 0;
        }
        let mut rem = n;
        while used + rem > limit {
            let take = limit - used;
            rem -= take;
            starts.push((rows.len() + n - rem) as u64);
            used = 0;
        }
        used += rem;
        rows.extend_from_slice(&b.members);
    }
    if starts.len() > 1 && *starts.last().unwrap_or(&0) >= rows.len() as u64 {
        starts.pop();
    }
    Order { rows, page_starts: Some(starts), stats }
}

/// Cache key: opportunity, scan generation, the request shape and the fingerprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OrderKey {
    pub opportunity: OpportunityId,
    pub generation: u64,
    pub query: Query,
    pub fingerprint: u64,
}

/// The last computed order (paging re-uses it).
#[derive(Default)]
pub(crate) struct OrderCache {
    last: Option<(OrderKey, Arc<Order>)>,
}

impl OrderCache {
    pub fn get(&self, key: &OrderKey) -> Option<Arc<Order>> {
        self.last.as_ref().filter(|(k, _)| k == key).map(|(_, o)| Arc::clone(o))
    }

    pub fn put(&mut self, key: OrderKey, order: Arc<Order>) {
        self.last = Some((key, order));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::{CandidateId, Timestamp};

    fn cand(id: u64, path: &str, size: u64, group: Option<u64>, keep: bool) -> Candidate {
        Candidate {
            id: CandidateId(id),
            path: path.into(),
            target: mj_core::TargetKind::File,
            source: mj_core::Source::Custom("t".into()),
            size,
            size_after: 0,
            confidence: mj_core::Confidence::Exact,
            reason: String::new(),
            badges: if keep { Badges::KEEPER } else { Badges::empty() },
            default_selected: !keep,
            group: group.map(GroupId),
            sub_group: None,
            modified: Some(Timestamp(id as i64)),
            stamp: mj_core::FileStamp { size, mtime: Timestamp(0) },
            file_count: None,
            columns: Default::default(),
        }
    }

    fn q(sort: CandidateSortKey, descending: bool, keep_groups: bool) -> Query {
        Query { sub_group: None, filter: None, min_size: None, sort, descending, keep_groups, limit: 50 }
    }

    fn ids(cands: &[Candidate], o: &Order) -> Vec<u64> {
        o.rows.iter().map(|&i| cands[i as usize].id.0).collect()
    }

    /// Group 1: 10 + 10 (kept /b). Group 2: 50 (kept), 5. Ungrouped: 30.
    fn sample() -> Vec<Candidate> {
        vec![
            cand(1, "/a/one.jpg", 10, Some(1), false),
            cand(2, "/b/one.jpg", 10, Some(1), true),
            cand(3, "/c/two.jpg", 50, Some(2), true),
            cand(4, "/d/two-small.jpg", 5, Some(2), false),
            cand(5, "/e/alone.jpg", 30, None, false),
        ]
    }

    #[test]
    fn size_orders_groups_by_their_largest_file_keeper_first() {
        let c = sample();
        let o = order(&c, &q(CandidateSortKey::Size, false, true));
        assert_eq!(ids(&c, &o), vec![3, 4, 5, 2, 1]);
        let o = order(&c, &q(CandidateSortKey::Size, true, true));
        assert_eq!(ids(&c, &o), vec![2, 1, 5, 3, 4], "descending reverses the groups, not the rows inside");
        // Flat: rows scattered.
        let o = order(&c, &q(CandidateSortKey::Size, false, false));
        assert_eq!(ids(&c, &o), vec![3, 5, 1, 2, 4]);
    }

    #[test]
    fn other_columns_use_their_documented_aggregate() {
        let c = sample();
        // Saving: group 2 frees 55, ungrouped 30, group 1 20.
        assert_eq!(ids(&c, &order(&c, &q(CandidateSortKey::Saving, false, true))), vec![3, 4, 5, 2, 1]);
        // Path / name: the lead row (kept copy) of each group.
        assert_eq!(ids(&c, &order(&c, &q(CandidateSortKey::Path, false, true))), vec![2, 1, 3, 4, 5]);
        assert_eq!(ids(&c, &order(&c, &q(CandidateSortKey::Name, false, true))), vec![5, 2, 1, 3, 4]);
        // Modified: the newest member (ids double as timestamps): g1 = 2, g2 = 4, alone = 5.
        assert_eq!(ids(&c, &order(&c, &q(CandidateSortKey::Modified, false, true))), vec![2, 1, 3, 4, 5]);
        assert_eq!(ids(&c, &order(&c, &q(CandidateSortKey::Group, false, true))), vec![5, 2, 1, 3, 4]);
    }

    #[test]
    fn filters_keep_whole_groups() {
        let c = sample();
        let mut qq = q(CandidateSortKey::Size, false, true);
        qq.min_size = Some(40);
        assert_eq!(ids(&c, &order(&c, &qq)), vec![3, 4], "the 5-byte member stays with its 50-byte group");
        qq.keep_groups = false;
        assert_eq!(ids(&c, &order(&c, &qq)), vec![3]);
        let mut qq = q(CandidateSortKey::Size, false, true);
        qq.filter = Some("/a/".into());
        assert_eq!(ids(&c, &order(&c, &qq)), vec![2, 1], "one member matches: the group shows whole");
    }

    #[test]
    fn group_stats_count_reclaimable_without_the_kept_copy() {
        let c = sample();
        let o = order(&c, &q(CandidateSortKey::Size, false, true));
        let g2 = o.stats[&GroupId(2)];
        assert_eq!((g2.files, g2.largest, g2.smallest, g2.reclaimable), (2, 50, 5, 5));
        assert_eq!(o.stats[&GroupId(1)].reclaimable, 10);
        // Without a kept copy, one copy is still kept.
        let c2 = vec![cand(1, "/x", 7, Some(9), false), cand(2, "/y", 7, Some(9), false)];
        let o = order(&c2, &q(CandidateSortKey::Size, false, true));
        assert_eq!(o.stats[&GroupId(9)].reclaimable, 7);
    }

    #[test]
    fn pages_never_split_a_group_that_fits() {
        // Groups of 3 with a page of 4: every page holds one group.
        let c: Vec<Candidate> =
            (0..30u64).map(|i| cand(i + 1, &format!("/p/{:02}", i), 100 - i / 3, Some(i / 3), i % 3 == 0)).collect();
        let mut qq = q(CandidateSortKey::Size, false, true);
        qq.limit = 4;
        let o = order(&c, &qq);
        let starts = o.page_starts.clone().unwrap();
        assert_eq!(starts, (0..10).map(|p| p * 3).collect::<Vec<u64>>());
        let s = o.span(0, 4);
        assert_eq!((s.start, s.end, s.page, s.pages, s.next, s.prev), (0, 3, 1, 10, Some(3), None));
        let s = o.span(4, 4);
        assert_eq!((s.start, s.end, s.page, s.next, s.prev), (3, 6, 2, Some(6), Some(0)));
        let s = o.span(29, 4);
        assert_eq!((s.start, s.end, s.page, s.next), (27, 30, 10, None));
        // Page of 7: two groups per page (6 rows), never 7.
        qq.limit = 7;
        assert_eq!(order(&c, &qq).page_starts.unwrap(), vec![0, 6, 12, 18, 24]);
    }

    #[test]
    fn a_group_larger_than_a_page_is_split_and_the_rest_follows() {
        let mut c: Vec<Candidate> =
            (0..5u64).map(|i| cand(i + 1, &format!("/big/{i}"), 100, Some(1), i == 0)).collect();
        c.push(cand(10, "/small/a", 1, Some(2), true));
        c.push(cand(11, "/small/b", 1, Some(2), false));
        let mut qq = q(CandidateSortKey::Size, false, true);
        qq.limit = 2;
        let o = order(&c, &qq);
        assert_eq!(o.page_starts.clone().unwrap(), vec![0, 2, 4, 5]);
        assert_eq!(o.span(6, 2).end, 7);
        let empty = order(&[], &qq);
        let s = empty.span(0, 2);
        assert_eq!((s.start, s.end, s.page, s.pages, s.next, s.prev), (0, 0, 1, 1, None, None));
    }

    #[test]
    fn flat_span_keeps_the_historical_arithmetic() {
        let c = sample();
        let o = order(&c, &q(CandidateSortKey::Size, false, false));
        let s = o.span(3, 2);
        assert_eq!((s.start, s.end, s.page, s.pages, s.next, s.prev), (3, 5, 2, 3, None, Some(1)));
        let s = o.span(10, 2);
        assert_eq!((s.start, s.end), (10, 10), "past the end: an empty page");
    }

    #[test]
    fn fingerprint_changes_with_the_keeper() {
        let mut c = sample();
        let a = fingerprint(&c);
        c[0].badges |= Badges::KEEPER;
        assert_ne!(a, fingerprint(&c));
        assert_eq!(c[0].id, CandidateId(1));
    }

    #[test]
    #[ignore = "timing; run with --ignored --nocapture"]
    fn million_rows_order_once() {
        let c: Vec<Candidate> = (0..1_000_000u64)
            .map(|i| cand(i, &format!("/r/{:07}/f.bin", i), 1000 + (i * 7919) % 100_000, Some(i / 2), i % 2 == 0))
            .collect();
        let t = std::time::Instant::now();
        let o = order(&c, &q(CandidateSortKey::Size, false, true));
        println!("grouped order: {:?}", t.elapsed());
        let t = std::time::Instant::now();
        let _ = order(&c, &q(CandidateSortKey::Size, false, false));
        println!("flat order: {:?}", t.elapsed());
        let t = std::time::Instant::now();
        let _ = fingerprint(&c);
        println!("fingerprint: {:?}", t.elapsed());
        assert_eq!(o.total(), 1_000_000);
    }
}
