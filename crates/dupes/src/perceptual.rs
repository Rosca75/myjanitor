//! Perceptual hashing and grouping (W2-B): `gray_grid` box averaging, dHash
//! (9x8), pHash (32x32 DCT, 63 coefficients, 0 clamped to 1), BK-tree,
//! Union-Find with rank, leader refinement (scan limit 256), diameter
//! confidence. Thresholds: `ThresholdPercent` (UI, capped at 18) converted once
//! to `HammingBits`. Owner: W2-B.
//!
//! These follow research §4-§7 exactly; the invariants that matter:
//! - downsample by **box average**, never point sampling (§4.1, invariant 2);
//! - both fingerprints from the same grid pass, mode chosen at match time (invariant 4);
//! - Union-Find output is only a **candidate** set; leader refinement splits it
//!   and the confidence comes from the measured diameter (invariant 3);
//! - no aspect-ratio bucketing (invariant 7); deterministic order (invariant 11).

use std::sync::OnceLock;

/// dHash grid: 9 columns x 8 rows.
pub const DHASH_COLS: usize = 9;
pub const DHASH_ROWS: usize = 8;
/// pHash grid: 32 x 32.
pub const PHASH_SIZE: usize = 32;
/// Candidate sets larger than this take their first member as leader (avoids O(n^3)).
pub const LEADER_SCAN_LIMIT: usize = 256;

/// Box-averages greyscale rows into the 9x8 and 32x32 grids in one pass.
/// Feed `height` rows of `width` luma bytes with [`GridAccumulator::push_row`].
pub struct GridAccumulator {
    width: usize,
    height: usize,
    row: usize,
    col9: Vec<u16>,
    col32: Vec<u16>,
    sum9: [u64; DHASH_COLS * DHASH_ROWS],
    cnt9: [u64; DHASH_COLS * DHASH_ROWS],
    sum32: Vec<u64>,
    cnt32: Vec<u64>,
}

/// The two averaged grids of one image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grids {
    pub g9: [u32; DHASH_COLS * DHASH_ROWS],
    pub g32: Vec<u32>,
}

impl GridAccumulator {
    /// Accumulator for a `width` x `height` image (both > 0).
    pub fn new(width: usize, height: usize) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        // Column index computed once per image (the hoisted division of research §4.1).
        let col9 = (0..width).map(|x| (x * DHASH_COLS / width) as u16).collect();
        let col32 = (0..width).map(|x| (x * PHASH_SIZE / width) as u16).collect();
        GridAccumulator {
            width,
            height,
            row: 0,
            col9,
            col32,
            sum9: [0; DHASH_COLS * DHASH_ROWS],
            cnt9: [0; DHASH_COLS * DHASH_ROWS],
            sum32: vec![0; PHASH_SIZE * PHASH_SIZE],
            cnt32: vec![0; PHASH_SIZE * PHASH_SIZE],
        }
    }

    /// Add the next row (extra bytes are ignored, missing ones count as absent; rows past
    /// `height` are ignored).
    pub fn push_row(&mut self, row: &[u8]) {
        if self.row >= self.height {
            return;
        }
        let r9 = (self.row * DHASH_ROWS / self.height) * DHASH_COLS;
        let r32 = (self.row * PHASH_SIZE / self.height) * PHASH_SIZE;
        for (x, &v) in row.iter().take(self.width).enumerate() {
            let v = v as u64;
            let a = r9 + self.col9[x] as usize;
            self.sum9[a] += v;
            self.cnt9[a] += 1;
            let b = r32 + self.col32[x] as usize;
            self.sum32[b] += v;
            self.cnt32[b] += 1;
        }
        self.row += 1;
    }

    /// The averaged grids (a cell with no pixel is 0).
    pub fn finish(&self) -> Grids {
        let avg = |s: u64, c: u64| if c == 0 { 0 } else { (s / c) as u32 };
        let mut g9 = [0u32; DHASH_COLS * DHASH_ROWS];
        for (i, g) in g9.iter_mut().enumerate() {
            *g = avg(self.sum9[i], self.cnt9[i]);
        }
        let g32 = self.sum32.iter().zip(&self.cnt32).map(|(&s, &c)| avg(s, c)).collect();
        Grids { g9, g32 }
    }
}

/// Grids of a whole greyscale buffer (`width * height` bytes, row-major).
pub fn grids(width: usize, height: usize, gray: &[u8]) -> Grids {
    let mut acc = GridAccumulator::new(width, height);
    if width > 0 {
        for row in gray.chunks(width).take(height) {
            acc.push_row(row);
        }
    }
    acc.finish()
}

/// dHash: bit `y*8 + x` (LSB first) = `grid[y][x] > grid[y][x+1]`.
pub fn dhash(g9: &[u32; DHASH_COLS * DHASH_ROWS]) -> u64 {
    let mut h = 0u64;
    for y in 0..DHASH_ROWS {
        for x in 0..8 {
            if g9[y * DHASH_COLS + x] > g9[y * DHASH_COLS + x + 1] {
                h |= 1 << (y * 8 + x);
            }
        }
    }
    h
}

fn basis() -> &'static [[f64; PHASH_SIZE]; 8] {
    static B: OnceLock<[[f64; PHASH_SIZE]; 8]> = OnceLock::new();
    B.get_or_init(|| {
        let mut b = [[0.0; PHASH_SIZE]; 8];
        for (k, row) in b.iter_mut().enumerate() {
            for (i, v) in row.iter_mut().enumerate() {
                *v = (std::f64::consts::PI * (i as f64 + 0.5) * k as f64 / PHASH_SIZE as f64).cos();
            }
        }
        b
    })
}

/// pHash: separable DCT-II (8 lowest frequencies per axis, unnormalised basis), 63 non-DC
/// coefficients compared with their upper median, bit 63 always 0, 0 clamped to 1.
pub fn phash(g32: &[u32]) -> u64 {
    let b = basis();
    let mut rows = [[0.0f64; 8]; PHASH_SIZE];
    for (y, r) in rows.iter_mut().enumerate() {
        for (k, out) in r.iter_mut().enumerate() {
            let mut s = 0.0;
            for i in 0..PHASH_SIZE {
                s += *g32.get(y * PHASH_SIZE + i).unwrap_or(&0) as f64 * b[k][i];
            }
            *out = s;
        }
    }
    let mut block = [[0.0f64; 8]; 8];
    for (k, brow) in block.iter_mut().enumerate() {
        for (x, out) in brow.iter_mut().enumerate() {
            let mut s = 0.0;
            for (y, r) in rows.iter().enumerate() {
                s += r[x] * b[k][y];
            }
            *out = s;
        }
    }
    let mut vals = Vec::with_capacity(63);
    for (y, brow) in block.iter().enumerate() {
        for (x, v) in brow.iter().enumerate() {
            if x == 0 && y == 0 {
                continue;
            }
            vals.push(*v);
        }
    }
    let mut sorted = vals.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let median = sorted[sorted.len() / 2];
    let mut h = 0u64;
    for (i, v) in vals.iter().enumerate() {
        if *v > median {
            h |= 1 << i;
        }
    }
    if h == 0 {
        1
    } else {
        h
    }
}

/// Both fingerprints of one greyscale image.
pub fn fingerprints(width: usize, height: usize, gray: &[u8]) -> (u64, u64) {
    let g = grids(width, height, gray);
    (dhash(&g.g9), phash(&g.g32))
}

/// Hamming distance.
pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Upper bound of the user threshold, in percent of the 64-bit hash (= 11 bits).
pub const MAX_THRESHOLD_PERCENT: u32 = 18;
/// Default threshold (10 % = 6 bits).
pub const DEFAULT_THRESHOLD_PERCENT: u32 = 10;

/// The UI threshold: a percentage of the hash width, capped at [`MAX_THRESHOLD_PERCENT`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ThresholdPercent(u32);

/// A Hamming radius in bits: what the grouper works in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct HammingBits(pub u32);

impl ThresholdPercent {
    /// Clamp `percent` to `0..=18` (non-finite values give the default).
    pub fn new(percent: f64) -> Self {
        if !percent.is_finite() {
            return ThresholdPercent(DEFAULT_THRESHOLD_PERCENT);
        }
        ThresholdPercent(percent.clamp(0.0, MAX_THRESHOLD_PERCENT as f64).floor() as u32)
    }

    /// The percentage after clamping.
    pub fn get(self) -> u32 {
        self.0
    }

    /// The one conversion to bits: `percent * 64 / 100` (integer division).
    pub fn to_bits(self) -> HammingBits {
        HammingBits(self.0 * 64 / 100)
    }
}

/// Which fingerprint(s) decide similarity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchMode {
    DHash,
    PHash,
    /// Both must agree: the distance is the worse of the two.
    Both,
}

impl MatchMode {
    /// Case-insensitive; unknown values give `DHash`.
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "phash" => MatchMode::PHash,
            "both" => MatchMode::Both,
            _ => MatchMode::DHash,
        }
    }

    /// A fingerprint pair this mode can use (0 means "none" for either hash).
    pub fn usable(self, fp: Fp) -> bool {
        match self {
            MatchMode::DHash => fp.dhash != 0,
            MatchMode::PHash => fp.phash != 0,
            MatchMode::Both => fp.dhash != 0 && fp.phash != 0,
        }
    }

    fn key(self, fp: Fp) -> u64 {
        match self {
            MatchMode::PHash => fp.phash,
            MatchMode::DHash | MatchMode::Both => fp.dhash,
        }
    }

    /// Pair distance under this mode.
    pub fn distance(self, a: Fp, b: Fp) -> u32 {
        match self {
            MatchMode::DHash => hamming(a.dhash, b.dhash),
            MatchMode::PHash => hamming(a.phash, b.phash),
            MatchMode::Both => hamming(a.dhash, b.dhash).max(hamming(a.phash, b.phash)),
        }
    }
}

/// The fingerprints of one image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fp {
    pub dhash: u64,
    pub phash: u64,
}

/// BK-tree over 64-bit hashes (arena of nodes, children keyed by edge distance).
#[derive(Debug, Default)]
pub struct BkTree {
    nodes: Vec<BkNode>,
}

#[derive(Debug)]
struct BkNode {
    hash: u64,
    item: usize,
    children: Vec<(u32, usize)>,
}

impl BkTree {
    /// Empty tree.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert `item` with `hash`.
    pub fn insert(&mut self, hash: u64, item: usize) {
        let new = self.nodes.len();
        self.nodes.push(BkNode { hash, item, children: Vec::new() });
        if new == 0 {
            return;
        }
        let mut cur = 0;
        loop {
            let d = hamming(self.nodes[cur].hash, hash);
            match self.nodes[cur].children.iter().find(|(cd, _)| *cd == d) {
                Some(&(_, child)) => cur = child,
                None => {
                    self.nodes[cur].children.push((d, new));
                    return;
                }
            }
        }
    }

    /// Items within `radius` of `hash` (explicit stack, triangle-inequality pruning).
    pub fn search(&self, hash: u64, radius: u32, out: &mut Vec<usize>) {
        out.clear();
        if self.nodes.is_empty() {
            return;
        }
        let mut stack = vec![0usize];
        while let Some(n) = stack.pop() {
            let node = &self.nodes[n];
            let d = hamming(node.hash, hash);
            if d <= radius {
                out.push(node.item);
            }
            let lo = d.saturating_sub(radius);
            let hi = d + radius;
            for &(cd, child) in &node.children {
                if cd >= lo && cd <= hi {
                    stack.push(child);
                }
            }
        }
    }
}

/// Union-Find with union by rank and path compression (iterative).
#[derive(Debug)]
pub struct UnionFind {
    parent: Vec<usize>,
    rank: Vec<u8>,
}

impl UnionFind {
    /// `n` singletons.
    pub fn new(n: usize) -> Self {
        UnionFind { parent: (0..n).collect(), rank: vec![0; n] }
    }

    /// Root of `x`.
    pub fn find(&mut self, x: usize) -> usize {
        let mut root = x;
        while self.parent[root] != root {
            root = self.parent[root];
        }
        let mut cur = x;
        while self.parent[cur] != root {
            let next = self.parent[cur];
            self.parent[cur] = root;
            cur = next;
        }
        root
    }

    /// Merge the sets of `a` and `b`.
    pub fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return;
        }
        match self.rank[ra].cmp(&self.rank[rb]) {
            std::cmp::Ordering::Less => self.parent[ra] = rb,
            std::cmp::Ordering::Greater => self.parent[rb] = ra,
            std::cmp::Ordering::Equal => {
                self.parent[rb] = ra;
                self.rank[ra] = self.rank[ra].saturating_add(1);
            }
        }
    }
}

/// One refined group: indexes into the input (ascending) and its diameter in bits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cluster {
    pub members: Vec<usize>,
    /// Largest pairwise distance among members.
    pub diameter: u32,
}

impl Cluster {
    /// `(1 - diameter / 64) * 100`.
    pub fn confidence(&self) -> f32 {
        (1.0 - self.diameter as f32 / 64.0) * 100.0
    }
}

/// Group `items` (given in sorted path order, so index order is deterministic).
/// Unusable items (a zero hash for the mode) never participate.
pub fn group(items: &[Fp], mode: MatchMode, radius: HammingBits) -> Vec<Cluster> {
    let t = radius.0;
    let usable: Vec<usize> = (0..items.len()).filter(|&i| mode.usable(items[i])).collect();
    let mut tree = BkTree::new();
    for &i in &usable {
        tree.insert(mode.key(items[i]), i);
    }
    let mut uf = UnionFind::new(items.len());
    let mut found = Vec::new();
    for &i in &usable {
        tree.search(mode.key(items[i]), t, &mut found);
        for &j in &found {
            if j != i && mode.distance(items[i], items[j]) <= t {
                uf.union(i, j);
            }
        }
    }
    let mut sets: std::collections::BTreeMap<usize, Vec<usize>> = std::collections::BTreeMap::new();
    for &i in &usable {
        let r = uf.find(i);
        sets.entry(r).or_default().push(i);
    }
    let mut out = Vec::new();
    for set in sets.into_values().filter(|s| s.len() >= 2) {
        refine(items, mode, t, set, &mut out);
    }
    out.sort_by(|a, b| b.members.len().cmp(&a.members.len()).then(a.members[0].cmp(&b.members[0])));
    out
}

/// Leader refinement of one candidate set (research §5.4).
fn refine(items: &[Fp], mode: MatchMode, t: u32, mut remaining: Vec<usize>, out: &mut Vec<Cluster>) {
    remaining.sort_unstable();
    let dist = |a: usize, b: usize| mode.distance(items[a], items[b]);
    while remaining.len() >= 2 {
        let leader = if remaining.len() > LEADER_SCAN_LIMIT {
            remaining[0]
        } else {
            // Most neighbours within t; ties go to the earliest (sorted) member.
            let mut best = (0usize, remaining[0]);
            for &a in &remaining {
                let deg = remaining.iter().filter(|&&b| b != a && dist(a, b) <= t).count();
                if deg > best.0 {
                    best = (deg, a);
                }
            }
            best.1
        };
        let mut members = vec![leader];
        members.extend(remaining.iter().copied().filter(|&p| p != leader && dist(leader, p) <= t));
        members.sort_unstable();
        remaining.retain(|p| !members.contains(p));
        if members.len() >= 2 {
            let mut diameter = 0;
            for (i, &a) in members.iter().enumerate() {
                for &b in &members[i + 1..] {
                    diameter = diameter.max(dist(a, b));
                }
            }
            out.push(Cluster { members, diameter });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp(d: u64) -> Fp {
        Fp { dhash: d, phash: d | 1 }
    }

    #[test]
    fn threshold_conversion_and_cap() {
        assert_eq!(ThresholdPercent::new(10.0).to_bits(), HammingBits(6));
        assert_eq!(ThresholdPercent::new(18.0).to_bits(), HammingBits(11));
        assert_eq!(ThresholdPercent::new(25.0).to_bits(), HammingBits(11), "capped at 18 %");
        assert_eq!(ThresholdPercent::new(100.0).get(), 18);
        assert_eq!(ThresholdPercent::new(-3.0).to_bits(), HammingBits(0));
        assert_eq!(ThresholdPercent::new(f64::NAN).to_bits(), HammingBits(6));
        assert_eq!(ThresholdPercent::new(1.0).to_bits(), HammingBits(0));
    }

    #[test]
    fn dhash_and_phash_basics() {
        // Horizontal ramp, bright on the left: every dHash bit is 1.
        let w = 90;
        let gray: Vec<u8> = (0..w * 80).map(|i| 255 - (i % w) as u8 * 2).collect();
        let (d, p) = fingerprints(w, 80, &gray);
        assert_eq!(d, u64::MAX);
        assert_ne!(p, 0);
        assert_eq!(p >> 63, 0, "bit 63 is never set");
        // Flat image: dHash 0 ("none"); pHash is never 0 (an all-zero result is clamped to 1).
        let flat = vec![128u8; 64 * 64];
        let (d, p) = fingerprints(64, 64, &flat);
        assert_eq!(d, 0);
        assert_ne!(p, 0);
        let mut zero_block = vec![0u32; PHASH_SIZE * PHASH_SIZE];
        zero_block[0] = 0;
        assert_eq!(phash(&zero_block), 1);
    }

    #[test]
    fn box_average_uses_every_pixel() {
        // A single bright pixel per 9x8 cell changes the average, not just one sampled pixel.
        let (w, h) = (90, 80);
        let mut gray = vec![0u8; w * h];
        gray[5 * w + 5] = 255; // inside cell (0,0)
        let g = grids(w, h, &gray);
        assert_eq!(g.g9[0], 255 / 100);
        let mut gray2 = vec![10u8; w * h];
        for y in 0..10 {
            for x in 0..10 {
                gray2[y * w + x] = 110;
            }
        }
        assert_eq!(grids(w, h, &gray2).g9[0], 110);
    }

    #[test]
    fn modes_parse_and_measure() {
        assert_eq!(MatchMode::parse("PHASH"), MatchMode::PHash);
        assert_eq!(MatchMode::parse("Both"), MatchMode::Both);
        assert_eq!(MatchMode::parse("bogus"), MatchMode::DHash);
        let a = Fp { dhash: 0b1111, phash: 0b1 };
        let b = Fp { dhash: 0b0000, phash: 0b1 };
        assert_eq!(MatchMode::DHash.distance(a, b), 4);
        assert_eq!(MatchMode::PHash.distance(a, b), 0);
        assert_eq!(MatchMode::Both.distance(a, b), 4);
        assert!(!MatchMode::DHash.usable(b) && MatchMode::PHash.usable(b) && !MatchMode::Both.usable(b));
    }

    #[test]
    fn bk_tree_matches_brute_force() {
        let mut s: u64 = 0x1234_5678;
        let mut hashes = Vec::new();
        for _ in 0..500 {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            hashes.push(s);
        }
        // Add near neighbours.
        for i in 0..50 {
            hashes.push(hashes[i] ^ (1 << (i % 64)) ^ (1 << ((i * 7) % 64)));
        }
        let mut t = BkTree::new();
        for (i, &h) in hashes.iter().enumerate() {
            t.insert(h, i);
        }
        let mut found = Vec::new();
        for r in [0, 3, 11] {
            for (i, &h) in hashes.iter().enumerate().take(80) {
                t.search(h, r, &mut found);
                found.sort_unstable();
                let brute: Vec<usize> = (0..hashes.len()).filter(|&j| hamming(hashes[j], h) <= r).collect();
                assert_eq!(found, brute, "item {i} radius {r}");
            }
        }
    }

    #[test]
    fn refinement_prevents_transitive_chaining() {
        // A chain 0 -> 1 -> ... -> 9 where neighbours differ by 4 bits and the ends by 36.
        let mut items = Vec::new();
        let mut h = 0u64;
        for i in 0..10 {
            items.push(fp(h | (1 << 63)));
            h |= 0xF << (i * 4);
        }
        let groups = group(&items, MatchMode::DHash, HammingBits(6));
        assert!(groups.len() >= 3, "the chain is split: {groups:?}");
        for g in &groups {
            assert!(g.diameter <= 12, "members are within t of a leader, so diameter <= 2t");
            for (i, &a) in g.members.iter().enumerate() {
                for &b in &g.members[i + 1..] {
                    assert!(MatchMode::DHash.distance(items[a], items[b]) <= g.diameter);
                }
            }
        }
        // Without refinement Union-Find would give one 10-member set.
        assert!(groups.iter().all(|g| g.members.len() < 10));
    }

    #[test]
    fn confidence_comes_from_diameter_and_order_is_deterministic() {
        let items = vec![fp(0b1 << 40), fp(0b11 << 40), fp(0b111 << 40), fp(0xFFFF_0000), fp(0xFFFF_0001)];
        let g = group(&items, MatchMode::DHash, HammingBits(6));
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].members, vec![0, 1, 2]);
        assert_eq!(g[0].diameter, 2);
        assert!((g[0].confidence() - (1.0 - 2.0 / 64.0) * 100.0).abs() < 1e-4);
        assert_eq!(g[1].members, vec![3, 4]);
        assert_eq!(group(&items, MatchMode::DHash, HammingBits(6)), g);
        // Zero hashes never match (flat images).
        let flat = vec![Fp { dhash: 0, phash: 1 }, Fp { dhash: 0, phash: 1 }];
        assert!(group(&flat, MatchMode::DHash, HammingBits(6)).is_empty());
        assert!(group(&flat, MatchMode::Both, HammingBits(6)).is_empty());
        assert_eq!(group(&flat, MatchMode::PHash, HammingBits(6)).len(), 1);
    }

    #[test]
    fn both_mode_requires_agreement() {
        let a = Fp { dhash: 0, phash: 0xFF };
        let b = Fp { dhash: 0, phash: 0xFF00_0000_0000 };
        let a = Fp { dhash: a.dhash | 1 << 50, ..a };
        let b = Fp { dhash: b.dhash | 1 << 50, ..b };
        assert_eq!(group(&[a, b], MatchMode::DHash, HammingBits(6)).len(), 1);
        assert!(group(&[a, b], MatchMode::Both, HammingBits(6)).is_empty());
    }

    #[test]
    fn large_sets_use_first_member_as_leader() {
        let items: Vec<Fp> = (0..300).map(|i| fp((1 << 62) | (i as u64 & 0x3))).collect();
        let g = group(&items, MatchMode::DHash, HammingBits(2));
        assert_eq!(g.iter().map(|c| c.members.len()).sum::<usize>(), 300);
        assert_eq!(g[0].members.len(), 300);
    }
}
