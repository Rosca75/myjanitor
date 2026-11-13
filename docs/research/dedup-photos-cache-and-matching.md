# Research: dedup-photos cache design and duplicate matching

Source: `/home/user/dedup-photos` (Go + Wails; read-only reference, same author).
Purpose: let a Rust engineer re-implement the cache, staged hashing, perceptual hashing
and grouping for MyJanitor. Numbers below were measured by the reference author on real
libraries (8,159 images; 1,134 and 3,542 image SMB corpora) unless marked "my reading".
Items marked **FLAW** are defects I found in the reference that MyJanitor must not copy.

---

## 1. Scan cache

### 1.1 What is keyed, and the invalidation rule
- Key: the **absolute file path string** (exact, case-sensitive as given).
- Validity stamp stored per entry: `Size` (i64 bytes) and `ModTime` (i64, Unix **nanoseconds**).
- Hit rule (`LookupAll`): `entry.Size == stat.Size && entry.ModTime == stat.ModTime.UnixNano`.
  Any difference means stale, so recompute everything for that file. No content check, no
  inode/file id, no ctime. Equality, not "newer than".
- Cache lookup runs **sequentially, after a parallel stat phase**, never inside the parallel
  loop (removes mutex contention; workers write only into index-aligned slots).

### 1.2 Entry contents (`CachedEntry`, v5)
`Size, ModTime, XXHash u64, DHash u64, PHash u64, Width, Height, Exif{OK, DateTaken, Camera,
Lens, FocalLength, Description, GPSLat, GPSLon, ISO}` (~100 bytes per entry, about 1 MB per
10k files). Deliberately **not** stored: `QualityScore` (derived at group time, so scoring
rule changes need no cache wipe) and raw header buffers (128 KB x 8,000 files = 1 GB).
Zero is a sentinel: `XXHash=0` means "not computed", `DHash=0` means "no perceptual hash".

### 1.3 Versioning
`const cacheVersion = 5`, stored in the file. Mismatch, decode error or missing file all give
an empty cache (silently rebuilt, one log line). History, which is the rule for when to bump:
- v2 added Width/Height. v4 added PHash (store **both** fingerprints always; algorithm choice
  is applied at match time, so switching setting never needs a rescan).
- v3: dHash downsampling changed from point sampling to area average. Bit layout identical
  but values not comparable. **Bump the version whenever the meaning of a stored value
  changes, not only its shape.**
- v5 added Exif. A v4 file decodes fine into a v5 struct with zeroed fields; without a bump
  the optimisation silently disappears. Gob tolerates added fields, so shape-only changes
  would not even error. Hence an explicit version integer and an explicit `Exif.OK` flag.

### 1.4 Storage format and location
- Go `encoding/gob` of `{Version int, Entries map[path]*CachedEntry}`: **one whole-file
  snapshot per scan root**.
- Location: `~/.dedup-photos/cache_<first 8 bytes of sha256(scanPath) as 16 hex>.gob`.
  One file per scan root; roots do not collide.
- Thumbnail cache (separate): `~/.dedup-photos/thumbs/thumb_<16hex of sha256(path)>.jpg`.
  One file per source image, overwritten in place. A fixed **16-byte header** precedes the
  JPEG: `u64 LE size, u64 LE mtime_ns`. Stale detection happens at read; no orphans, no
  directory glob. (An earlier scheme put size and mtime in the filename, orphaned files on
  every change and needed a glob per write: 275 us at 317 files, linear growth.)

### 1.5 Load / save strategy
1. Load the whole file into a map at scan start (primary root's cache; entries from other
   roots' caches are merged in, primary wins on conflict).
2. Stat every path in parallel (progress tick every 500 files), then split sequentially into
   hits and misses.
3. Hash misses only. Hits carry hashes, dimensions and EXIF straight through: zero I/O.
4. After the scan, `StoreAll` for every non-error result, then **save the whole map**:
   write `<path>.tmp`, close, rename over the target (atomic; a kill mid-write cannot corrupt
   the previous cache). Thumbnails use `CreateTemp` in the same dir plus rename, so concurrent
   writers of the same key cannot collide.
5. Cache failures are never fatal: ignore and continue.

### 1.6 Measured rescan speed
- Warm re-scan, hash cache fully valid, 1,134-file SMB library: hash phase `0.00 s`,
  `Cache split: 1134 hits, 0 misses`. Total **16.35 s -> 0.79 s** once grouping stopped
  re-reading EXIF (the cache itself was never the cost).
- Cold scan of that library, 8 workers, cold page cache: 61.7-69.6 s originally, 23.9-25.7 s
  after fixes. 8,159-image library: 28 s cold scan quoted for ~8,000 files.
- Per-file costs: EXIF parse from an in-memory buffer 0.15 ms; metadata via a fresh file
  open over SMB 44-253 ms; header read 128-192 KB over SMB ~143 ms/file.

### 1.7 Pitfalls found in the reference (apply to MyJanitor)
- **FLAW: zero-hash sentinel plus miss-only bucketing.** Same-size bucketing is done over
  *cache misses only*, and files that never collided store `XXHash=0` into the cache. A new
  file that is byte-identical to an old, cached, unhashed file has no same-size peer among
  the misses, so it is treated as a singleton and the exact duplicate is **missed** (my
  reading of `runExactHashPhase`/`buildFileSizeBuckets`; not covered by the reference's docs).
  MyJanitor: bucket by size across **all** files (hits and misses); cache stage results with
  explicit `Option` fields (partial hash, full hash), never "0 = absent"; when a same-size
  peer is cached but lacks a full hash, compute it lazily (see section 9).
- **FLAW: stale entries never pruned.** `Entries` only grows; deleted or moved files stay
  forever. Prune entries whose path was not seen (or whose root vanished) at save time.
- **Single-file whole-map rewrite** costs O(total entries) per save and loads everything even
  for a sub-folder scan. Fine to ~10^5 entries; plan segmenting if MyJanitor targets 10^6+.
- Path-only key: a rename/move re-hashes the file; a file replaced by an old-mtime file of
  equal size is a false hit (mtime-preserving copy tools). Accept, and strengthen with file id.
- mtime granularity: FAT/exFAT is 2 s, some SMB/NAS round differently. Compare nanosecond
  values as stored, but do not trust a hit when `mtime` is in the future or equals "now"
  within granularity (my suggestion; the reference does not).
- A hit must also carry the *decode-failure* result ("no perceptual hash"), so un-hashable
  files are not retried on every scan (the reference does this implicitly via `DHash=0`).

---

## 2. Hash pipeline

Phases, in order (`HashAllImagesWithProgress`; workers = number of logical CPUs):

| # | Phase | Work | Parallel |
|---|---|---|---|
| 1 | load cache | gob decode, merge extra roots | no |
| 2 | stat all | `os.Stat` per path into index-aligned slice | yes |
| 3 | split | hits vs misses (sequential, no locks) | no |
| 3a | exact lane | size buckets -> 64 KB partial xxHash -> full read for partial collisions | yes |
| 3b | perceptual | fingerprints (+ dims + EXIF) for every miss | yes |
| 4 | save cache | stamp + store + atomic write | no |

### 2.1 Exact lane parameters
- **Size bucketing**: group by byte size. Unique size = singleton = cannot be an exact dup;
  skip it in the exact lane (~85% of a photo corpus; claimed ~85% I/O saved).
- **Partial hash**: xxHash64 of the **first 64 KB only** (`bufPool` buffer of 65,536 B; a
  single `Read`, fewer bytes for small files). No tail or middle sample. If open fails or 0
  bytes read the file silently drops out of the lane (see flaw note in 3.3).
- **Collision group**: same partial hash (note: grouped by partial hash alone, *not* by
  size+partial; same-size is already guaranteed because input is size buckets).
  Groups of >= 2 get a **full read** (`os.ReadFile`) and a full xxHash64. Everything else
  keeps `XXHash=0`.
- Retained buffers: the 64 KB prefix of partial-hash-unique files is kept for the perceptual
  phase; cap `headerBufRetentionLimit = 1000` buffers (~64 MB). Collision-group prefixes are
  dropped (full data is retained instead and reused for fingerprints).
- Perceptual phase data sources, by priority: full file bytes (collision files) > retained
  64 KB prefix > fresh open reading **128 KB** (`headerBufPool`). Counters `fullData /
  headerBuf / reopen` are logged; EXIF-from-header ratio is a health metric.

### 2.2 Worker pool design (`parallel.go`)
- `runParallelIndexed(ctx, n, workers, fn(i))`: an `atomic i64` counter hands out indexes;
  each worker loops `if ctx cancelled: return; i = idx.fetch_add(1); if i >= n: return;
  fn(i)`. Results go into **pre-allocated index-aligned slices** (distinct slot per worker, no
  lock), converted to maps sequentially afterwards. Chosen over a pre-filled channel for
  thousands of tiny jobs. Rust: `AtomicUsize` + scoped threads/rayon, `Vec<Option<T>>` slots.
- Cancellation checked per item (between files, not mid-read).
- Progress: stat phase every 500 files; fingerprint phase every 25 files, reported against
  total scan size (hits + misses) so the bar never jumps back. Exact lane has no progress.

### 2.3 Adaptive concurrency for flaky filesystems (`resilient_io.go`)
Problem: on a GVFS/SMB mount whole-file reads fail with `EINVAL` when more than one is in
flight. Measured, 200 files whole-file reads succeeded: 8 workers 25/200, 4 workers 24/96,
2 workers 48/96, 1 worker 200/200 (successes ~ files/workers). Bounded prefixes (64/128/512
KB) succeed 200/200 at 8 workers; only reading to EOF trips it. Tried and **rejected**: chunked
reads (12/96), streaming hash (12/96), `ReadAt` windows (14/120), reopen per 512 KB window
(65/120) or 1 MB (88/120), blind retries at same concurrency (29/96 at 2.3x time).

Algorithm (`runAdaptiveIO`), applied to the full-read lane only:
```
constants: PROBE=12, FAIL_RATIO=0.5, SERIAL_ATTEMPTS=2
pass1 (only if workers > 1): run in parallel; after each result
    done++ ; if err { failed[i]=true; failures++ }
    if done >= PROBE and failures > FAIL_RATIO*done: cancel pass1 once
pending = indexes not attempted or failed
pass2 (serial): for each pending i:
    for attempt in 0..SERIAL_ATTEMPTS: if work(i).ok break; sleep((attempt+1)*25ms)
    if still failing: unreadable.push(path)
return unreadable
```
Healthy FS: pass 1 completes, pass 2 never runs (zero cost). Bad FS: gives up after ~12 files.
The fallback is decided per invocation; it is not remembered across scans.

### 2.4 What a failed read reports (`scan_diagnostics.go`)
Mutex-guarded per-scan counters, reset at scan start: `unreadable: Vec<path>` (still failing
after serial retry; surfaced as `ScanStats.Unreadable` with a sample of 10 paths) and
`noPerceptualHash: count` (files with `DHash==0`, surfaced as `ScanStats.SkippedPerceptual`).
A real library had 19.2% (1,563 of 8,159) with no perceptual hash; roughly 16-19% for HEIC
libraries. The UI must say so. Principle: "never compared" must be distinguishable from
"no duplicates".
- **FLAW (my reading)**: the cheap lanes (open failure or zero-byte read in the partial hash
  phase, stat failures, perceptual-phase open errors) still end in a bare `return`/`ErrNoThumbnail`
  and are not counted as unreadable. MyJanitor should route every failed read through the
  diagnostics, not only the full-read lane.

---

## 3. Exact matching

### 3.1 Candidate narrowing and confirmation
1. Size bucket (unique size eliminated).
2. 64 KB prefix xxHash64 (same-prefix survivors are candidates).
3. Full-file xxHash64 over the survivors; group by equal full hash, size >= 2.
4. Output groups carry `match_type="exact"`, `confidence=100.0`. Files already in an exact
   group are excluded from perceptual matching (no double reporting).

### 3.2 Confirmation strength
The reference treats **equal full xxHash64 as identity**: no byte compare, no second hash.
64-bit non-cryptographic collision chance is ~n^2/2^65; negligible for tens of thousands of
files but a wrong delete is serious. For MyJanitor, which deletes, do a final streaming
byte-for-byte compare (or BLAKE3/SHA-256) on groups before offering deletion.

### 3.3 Notes
Files <= 64 KB: partial hash equals full hash, yet the reference still does a redundant full
read. Skip it when `size <= partial_window`.

---

## 4. Perceptual hashing

Both fingerprints are computed for **every** file from the **same decode**, stored in the
cache, and the algorithm setting is applied only at match time (section 7.4).
Both are 64 bits, `0` = "none". Hamming distance = `(a ^ b).count_ones()`.

### 4.1 Shared downsampler: `grayGrid(img, cols, rows) -> Vec<u32>`
Box (area) average of 8-bit luma into `cols x rows` cells:
```
colOf[x] = x * cols / w                       // computed once per image (integer division)
for y in 0..h: rowBase = (y * rows / h) * cols
  for x in 0..w: sum[rowBase+colOf[x]] += luma(x,y); cnt[...] += 1
cell = sum / cnt  (integer division; 0 when cnt==0)
```
- Luma: for YCbCr JPEG-derived images read the **Y plane directly**; otherwise
  `luma = ((299*r + 587*g + 114*b) / 1000) >> 8` on 16-bit RGBA values (so 0-255).
- Use box average, not Catmull-Rom (about +1 point recall but 4.6 ms vs 244 us per image).
- Cost after optimisation: **~244 us per image**; the hoisted `colOf` took it from 1.39 ms.
  Old point sampling: 3.4 us per image but read 0.055% of the pixels.

### 4.2 dHash
- Grid **9 wide x 8 tall** (`grayGrid(img, 9, 8)`).
- For each row y (0..7), for x (0..7): `bit = gray[y*9+x] > gray[y*9+x+1]`; bit index counts up
  `0..63` row-major, **LSB first** (`hash |= 1 << bit`).
- Not special-cased when 0 (a flat image gives 0 and is treated as "no hash"; accepted loss).

### 4.3 pHash (DCT, `phash.go`)
- Grid **32 x 32** via the same `grayGrid`. 2-D DCT-II, separable, **only the 8 lowest
  frequencies per axis are computed**.
- Basis precomputed once: `basis[k][i] = cos(pi * (i + 0.5) * k / 32)`, **no normalisation
  factors** (re-implement exactly that to stay bit-compatible; f64 arithmetic).
- Row pass: `rows[y][k] = sum_i grid[y*32+i] * basis[k][i]`, k in 0..8. Column pass:
  `block[k][x] = sum_y rows[y][x] * basis[k][y]`.
- Take the 8x8 block, **exclude (0,0)** (DC; makes the hash exposure-invariant) leaving **63**
  coefficients in row-major order (`for y in 0..8 for x in 0..8`, skipping (0,0)).
- `median = sorted(vals)[len/2]` (index 31 of 63, upper median, no averaging).
  Bit i (LSB first, i = 0..62) = `vals[i] > median`. Bit 63 is always 0.
- If the result is exactly 0, set it to **1** (keeps 0 reserved for "none").
- Cost ~240 us per image beyond the shared grid; the second fingerprint was judged cheap vs
  I/O of milliseconds per file.

### 4.4 Measured recall (386 photos x 7 edits: re-save q90/q75/q60, 50%/25% downscale, combos)
| Threshold | 6 bits | 7 bits | 9 bits |
|---|---|---|---|
| dHash (area average) | 97.8% | 99.0% | 99.8% |
| pHash (area average) | 98.4% | 98.4% | 99.6% |
dHash with point sampling at 6 bits: 64.3%. Default threshold (10% = 6 bits): 97.8% found with
largest group 0.6% of the library.

### 4.5 Fast paths: where the pixels come from
Hashing runs on a **small decoded image**, never the full photo, when avoidable:
1. **Embedded JPEG thumbnail** in the EXIF APP1 segment, found by scanning the first 64 KB
   (partial-hash buffer) or 128 KB (fresh read). Typical cost ~0.5 ms vs ~30-50 ms full decode.
2. HEIC/HEIF: first **128 KB** only; ladder = (1) embedded thumbnail from the 128 KB header,
   (2) primary image from it, (3) thumbnail after widening to a **1 MB** prefix. Rung 1 hits for
   ~100% of iPhone files at 128 KB (64 KB -> 42%, 96 KB -> 98%, 128 KB -> 100%). No fourth
   full-decode rung: 22 of 1,038 files hit it, cost 125 s (3-13 s each under WASM), i.e. 2% of
   files, ~30% of scan time. Those files get `DHash=0` and are exact-match only. Equivalent
   rule for JPEG/RAW without a thumbnail: **skip perceptual, do not full-decode**.
3. PNG/BMP/GIF/WebP/TIFF (formats with no thumbnails): full `ReadFile` + decode (the only
   place a full decode happens by design; ~30-50 ms each).
4. RAW: embedded JPEG preview only; no full decode.
5. Dimensions (`Width`,`Height`) come from a header-only `DecodeConfig` on the same buffer.
   A cached dimension of 0 means unknown, not zero-sized.
Quality note: using the embedded thumbnail means the fingerprint describes the thumbnail, which
an editor may not have regenerated. Reference accepts this for speed; MyJanitor should record
the source of the fingerprint (thumb vs full decode) in the cache if edits are a concern.

---

## 5. Grouping

### 5.1 Threshold: percentage versus bits
- UI slider is a **percentage of hash width (0-100)**; grouper takes **bits**.
- `bits = percent * 64 / 100` (integer division), clamped: `percent` limited to
  `[0, maxThresholdPercent=18]`, so max **11 bits**. Default 10% = **6 bits**. Conversion
  happens exactly once at the request boundary.
- Why cap at 18%: Union-Find fuses the whole library into one component at ~14 bits (22%) on
  real data. At a 25-bit radius 99.4% of a 6,596-image library fell into one group.

### 5.2 Algorithm
```
candidates: BK-tree over the index hash of every usable, non-exact-grouped image
for each image h: for each r in tree.search(h, T): if r != h (and verify, see both): union(h, r)
roots -> candidate sets (Union-Find); sets of size < 2 are dropped IMMEDIATELY
for each candidate set: splitChainedGroup (leader refinement, 5.4)
```
- **BK-tree**: node = {hash, path, children keyed by distance to the node}. Insert walks down
  `children[d]` until free. Search is an explicit stack; a child with edge distance `cd` is
  visited iff `d - T <= cd <= d + T` (triangle inequality). Build once, query once per image.
  Search cost measured: 189-390 ms for 2,939 hashes; refinement ~0 ms.
- **Union-Find**: parent map with path compression (recursive `find`), `union` attaches root
  of y under root of x. No union-by-rank (Rust: use rank/size to avoid deep chains).
- Only files with `mode.usable()` and not in an exact group participate.

### 5.3 Why candidates are not results
Union-Find is transitive (A~B, B~C => A,C grouped even if far apart). The reference's first
1,730-image group reported 100% confidence while 93.7% of its internal pairs exceeded the
threshold used to build it.

### 5.4 Cluster refinement with leaders (`splitChainedGroup`)
```
remaining = sorted(paths)                    // sorted => deterministic ties
while remaining.len >= 2:
    leader = pickLeader(remaining)           // most neighbours within T; if len > 256, remaining[0]
    members = [leader] + [p in remaining if dist(leader,p) <= T]
    if members.len >= 2: emit(members, maxDiameter(members))
    remaining = remaining minus members      // leader always removed, so it terminates
```
- `pickLeader`: O(n^2) degree count, only when `len <= leaderScanLimit = 256`; above that take
  the first to avoid an O(n^3) stall.
- Members are within T of the **leader**, not of each other; so true pairwise diameter can be
  up to 2T. Hence confidence uses the measured diameter, below.
- Output sorted by (size desc, first path asc) for deterministic order.

### 5.5 Confidence
`confidence = (1 - MaxDist / 64) * 100`, where `MaxDist` is the **largest pairwise distance**
among members (the diameter, O(k^2)). Exact groups = 100.0. Never derive it from the closest
pair. Display "exact" separately.

### 5.6 Match modes (`dhash` | `phash` | `both`, parsed case-insensitively; unknown -> dhash)
| Mode | usable if | tree index | pair distance |
|---|---|---|---|
| dhash | `DHash != 0` | DHash | popcount(dA^dB) |
| phash | `PHash != 0` | PHash | popcount(pA^pB) |
| both | both non-zero | DHash | **max(dDist, pDist)** (both must agree) |
In `both`, the tree proves only the dHash is close; the candidate link is accepted only if
`distance(a,b) <= T` using the max. The same `distance` is used in leader selection, member
claiming and diameter. `both` is the only strictly stronger setting; dHash and pHash fail
differently so requiring agreement suppresses chance links at a small recall cost.

### 5.7 Series (burst) handling, optional
- Relabel "perceptual" groups with confidence >= 95.0 as "series" if (A) every filename stem
  is `<prefix><digits>` with identical case-insensitive prefix and sorted numeric gaps all in
  `0..=5`, or (B) every image has the same non-empty camera make+model and EXIF
  DateTimeOriginal within 60 s.
- Pre-filter before expensive metadata when series are not wanted: drop groups with
  `MaxDist <= threshold/2` and a filename series (path-only check, no I/O).
- Singletons (97.6% of perceptual entries in a real corpus) must be dropped *before*
  metadata work: that took metadata from 17.9 s to 0.6 s.

---

## 6. Choosing which copy to keep (quality score)

Computed at group time from cached inputs, never cached. Score 0-100, int, clamped:
| Component | Points | Formula |
|---|---|---|
| Resolution | 0-30 | `min(log2(megapixels) * 7.5, 30)`, floor at 0 (MP = w*h/1e6) |
| File size | 0-15 | `min(log2(MiB) * 3.0, 15)`, floor at 0 |
| Date taken (EXIF DateTimeOriginal) | 20 | present -> +20 |
| GPS | 15 | lat != 0 or lon != 0 |
| Camera make+model | 5 | non-empty |
| Lens | 5 | non-empty |
| Title or description | 10 | either non-empty |

Group images are sorted by score descending; the first gets `is_best=true` (the recommended
keeper). Ties are left to the (unstable in Go) sort: **MyJanitor should break ties
deterministically** (e.g. larger size, then older mtime, then path). Resolution and size are
the only factors available for non-EXIF files. Lazy extras, computed only on expansion:
JPEG blockiness (gradient at 8-px boundaries vs overall, `max(0, ratio-1)*100`, on a <=512 px
grayscale) and blur (Laplacian variance, `100 - variance*0.2` clamped 0-100, 100 if variance <
0.01). EXIF must come from bytes already read in the hash phase (rule 7.6).

For deletion, the reference removes the file permanently (Live Photo `.MOV` too, confirmed
by shared Apple ContentIdentifier UUID; not relevant to MyJanitor except as an example of
"verify a companion file before touching it").

---

## 7. Invariants: do not break these again (as rules for MyJanitor)

1. **Percent versus bits.** Keep two distinct types (`ThresholdPercent`, `HammingBits`); convert
   once. Cap the user threshold at 18% (11 bits of 64). Test: group size at the cap on a
   real-like corpus must stay far from "one giant component".
2. **Average, never point-sample, when downsampling.** Use a box average into 9x8 and 32x32
   grids from the Y plane. Do not substitute nearest-neighbour or a wide-support kernel.
3. **Union-Find yields candidates only.** Always run leader refinement; compute confidence from
   the group's **diameter**, never from the closest pair. Cap the O(n^2) leader scan (256).
4. **Both fingerprints always; algorithm applied at match time.** The cache must not depend on
   the algorithm setting. `both` = both must agree (distance is the worse of the two). Zero is
   "none" for either hash; clamp a genuine pHash of 0 to 1.
5. **A failed read must never look like "no duplicates".** Route every failure (open, read,
   decode, hash) into diagnostics with counts and a path sample; surface "skipped for
   perceptual matching" and "unreadable" in the UI. Full-file reads use adaptive concurrency
   (parallel first, serial retry fallback). Bounded prefix reads are safe at full concurrency.
6. **No new file opens in the hash phase to gather other metadata.** Parse EXIF/dimensions
   from the bytes already read (first 64-128 KB) and store them in the cache with an explicit
   `captured: bool` flag; fall back to reading only for files where it is false. Do not cache
   derived scores. Bump `cacheVersion` on any stored-field change *or* meaning change.
7. **No aspect-ratio (or other lossy) partitioning of the BK-tree.** 99.1% of a phone library
   fell into one bucket (no pruning) and crops became unmatchable against their originals.
8. **No fallback to a full decode for files without an embedded thumbnail** in the scan fast
   path (2% of files, 30% of time). Mark them exact-only and say so.
9. **Cheap measurement discipline**: keep per-phase `[perf]` counters (hit/miss ratios,
   header-window hit rates, fallback counts); a collapse in the thumbnail hit-rate or EXIF-
   from-header ratio is the alarm that a fast path broke.
10. **Atomic persistence.** Write to a temp file in the same directory then rename; treat any
    decode error, version mismatch or truncation as "empty cache", never as a failure.
11. **Deterministic output.** Sort candidate paths before leader selection; order groups by
    (size desc, first path); break score ties explicitly.
12. **Do not mutate or drop on single-source evidence.** Exact groups need equal full hash
    (ideally plus byte compare) before any delete is offered.

---

## 8. Adaptation for MyJanitor (general files, not only photos)

### 8.1 Cache key and validation
- Key: `(canonical absolute path)`; stamp `{size: u64, mtime_ns: i64}`. Hit only on exact
  equality of both. Add a **file id** where it is cheap to obtain: Linux `st_dev` + `st_ino`
  (from the already-needed `metadata()` call), Windows volume serial + file index (only via
  an open handle / `GetFileInformationByHandle`; skip on bulk scans, use for move detection
  of confirmed duplicates only). Use the id as an optional second index to carry hashes across
  renames/moves: if the path misses but `(dev, ino, size, mtime)` matches, reuse and re-key.
- Treat entries with `mtime` within ~2 s of the scan start as unsafe (still being written).
- Also store `ctime`/change time where available (Linux) as an extra stamp; skip on Windows.
- Never use `fs::canonicalize` blindly on Windows (`\\?\` prefixes); normalise consistently and
  store the same form you look up. Case-fold keys on Windows.

### 8.2 Per-entry data (explicit options, never sentinels)
`size, mtime_ns, file_id?, partial: Option<(u32 window_len, u64 xxh64)>, full: Option<u64>,
phash_d: Option<u64>, phash_p: Option<u64>, width: Option<u32>, height: Option<u32>,
flags(unreadable/no_thumb), version-tagged`. Stages are filled lazily and independently so a
later scan can add a full hash to an entry that only had a partial.

### 8.3 On-disk format without external crates
One file per scan root (or a single global store), little-endian, hand-rolled:
```
header:  magic "MJCACHE\0" (8) | u32 format_version | u32 algo_version (bumps when hash
         meaning changes: window sizes, xxh seed, grid size) | u64 entry_count | u32 flags
record:  u32 path_len | path bytes (UTF-8, or WTF-8/lossy on Windows)
         u64 size | i64 mtime_ns | u8 presence_bits (partial,full,dhash,phash,dims,file_id,...)
         then only the present fields in fixed order (u32+u64, u64, u64, u64, u32+u32, ...)
trailer: u64 xxh64 of everything before it (detects truncation/torn writes)
```
Write: stream to `<name>.tmp` in the same directory, `sync_all`, `rename` (use `MoveFileExW`
`REPLACE_EXISTING` semantics on Windows; `std::fs::rename` already does this). On load: bad
magic, wrong version, wrong trailer hash, or any parse error -> empty cache, log, continue.
A `u32 algo_version` separate from `format_version` mirrors the reference's lesson that value
meaning changes need a bump too. Load into `HashMap<PathBuf, Entry>`; keep a sorted `Vec` if
memory matters. Location: platform cache dir (`%LOCALAPPDATA%\MyJanitor\cache` on Windows,
`$XDG_CACHE_HOME/myjanitor` or `~/.cache/myjanitor` on Linux), file name `scan_<16hex of
xxh64(root path)>.bin`. If scans of overlapping roots are common, a single global store keyed
by path avoids duplicated entries and the reference's per-root merge logic. Prune on save:
drop entries whose path was not seen this scan within a scanned root, and entries under roots
that no longer exist.

### 8.4 Staged hashing parameters for arbitrary large files
Generalising size -> partial -> full with xxHash64:
1. **Stat + size bucket** across *all* files (cache hits included). Singletons end here.
   Ignore zero-byte files (report separately; they are all "equal").
2. **Partial hash** for files in same-size buckets. The reference's first-64-KB-only prefix is
   weak for formats with identical headers (disk images, VM files, databases, logs, container
   formats). Recommended: hash three windows, **head 64 KB + middle 64 KB + tail 64 KB**
   (offset `size/2 - 32 KiB`, and `size - 64 KiB`), concatenated into one xxh64 (or three
   xxh64 combined) for files larger than 192 KB; for files <= 192 KB read once fully and
   store as *full* directly (skips stage 3). Record the window layout in `algo_version`.
   Keep partial reads bounded: they are safe on flaky mounts (section 2.3) and cheap.
3. **Full hash** only for partial-collision groups: **stream** in 1 MiB buffers (not
   `ReadFile`; large files must not be held in memory). Use `xxh64` for grouping, then
   confirm candidate groups with a size-checked streaming byte compare or a second
   independent hash before any delete. Reuse stage-2 reads: the head window is already hot.
4. **Order within a stage**: sort by device then by (path or inode) to improve locality on
   spinning disks; limit concurrency per device (HDD: 1-2 readers; SSD: logical CPUs; network:
   adaptive). Apply the adaptive-concurrency fallback of section 2.3 to every full read.
5. Large-file budget: cap total bytes fully read per scan if desired and report skipped
   candidates as "not fully verified" rather than dropping them silently.
6. Hardlinks/symlinks: files sharing `(dev, ino)` are the *same* file, not duplicates; detect
   via file id and exclude (or label "hard link"). Do not follow symlinked directories.
7. Skip rules mirroring the reference's walker: skip hidden dirs (`.`-prefix), tolerate
   unreadable subtrees (count them), sort the final path list for determinism; consider a
   per-top-level-directory parallel walk for network shares (2-4x faster on 1-5 ms stat
   latency, ~10% on SSD).

### 8.5 Similar images (perceptual)
Reuse section 4-6 unchanged for image files only. Suggested Rust pieces: `image`/`jpeg-decoder`
(or hand-rolled) for thumbnails, a hand-rolled box downsampler (no kernel crate), an EXIF
APP1 scanner for the embedded JPEG thumbnail (first 64-128 KB), BK-tree with `u64::count_ones`,
Union-Find with rank, leader refinement as in 5.4. Store both hashes and the source of the
pixels (embedded thumbnail vs full decode) in the cache. Keep the 18% / 11-bit ceiling and the
"both must agree" mode available as the strict setting. Where the reference marks files
"exact-only", MyJanitor should show a count rather than silently dropping them.

Per-phase `[perf]` logging (hits/misses, bucket sizes, collisions, bytes read, unreadable and
no-thumbnail counts) made every regression in the reference visible; copy the practice.
