//! Performance check (ignored by default): builds a synthetic tree of 200 000
//! files under a temp dir and prints cold and warm scan timings.
//!
//! `cargo test -p mj-scan --release --test perf -- --ignored --nocapture`
//! (`MJ_PERF_FILES` overrides the file count.)

use std::time::{Duration, Instant};

use mj_core::testutil::TempDir;
use mj_core::{CancelToken, NullProgress, ScanScope};
use mj_platform::NativePlatform;
use mj_scan::{scan, scan_with_cache, FolderTree, ScanCache, ScanRequest};

#[test]
#[ignore]
fn perf_200k_files_cold_and_warm() {
    let total: usize = std::env::var("MJ_PERF_FILES").ok().and_then(|v| v.parse().ok()).unwrap_or(200_000);
    let t = TempDir::new("perf-tree").unwrap();
    let data = TempDir::new("perf-cache").unwrap();
    let (tops, subs) = (20usize, 20usize);
    let per_dir = total.div_ceil(tops * subs);
    let t0 = Instant::now();
    let mut n = 0;
    'outer: for a in 0..tops {
        for b in 0..subs {
            let dir = t.mkdir(format!("top{a:02}/sub{b:02}")).unwrap();
            for c in 0..per_dir {
                if n == total {
                    break 'outer;
                }
                std::fs::write(dir.join(format!("file{c:04}.dat")), [(c % 251) as u8; 3]).unwrap();
                n += 1;
            }
        }
    }
    let built = t0.elapsed();
    let cache_file = data.join("scan-cache.bin");
    let scope = ScanScope { roots: vec![t.path().to_path_buf()], ..Default::default() };
    let p = NativePlatform::new();
    let cancel = CancelToken::new();
    let req = ScanRequest { scope: &scope, exclusions: &[], platform: &p, cancel: &cancel, progress: &NullProgress };

    let t1 = Instant::now();
    let plain = scan(&req).unwrap();
    let walk_only = t1.elapsed();
    assert_eq!(plain.stats.files as usize, n);

    let cache = ScanCache::open(&cache_file);
    let t2 = Instant::now();
    let cold = scan_with_cache(&req, &cache).unwrap();
    let cold_t = t2.elapsed();
    let t3 = Instant::now();
    cache.prune(&cold);
    cache.save().unwrap();
    let save_t = t3.elapsed();
    let bytes = std::fs::metadata(&cache_file).unwrap().len();

    // Files written less than 2 s before a scan are never trusted as cache hits.
    let wait = Duration::from_millis(2100).saturating_sub(t0.elapsed().saturating_sub(built));
    std::thread::sleep(wait);
    let t4 = Instant::now();
    let cache = ScanCache::open(&cache_file);
    let load_t = t4.elapsed();
    let t5 = Instant::now();
    let warm = scan_with_cache(&req, &cache).unwrap();
    let warm_t = t5.elapsed();
    let t6 = Instant::now();
    let tree = FolderTree::build(&warm);
    let tree_t = t6.elapsed();
    let root = warm.position(t.path()).unwrap();

    println!("[perf] files {n}, dirs {}, workers {}", warm.stats.dirs, mj_core::pool::default_workers());
    println!("[perf] build tree        {:>8.2?}", built);
    println!("[perf] walk only         {:>8.2?}", walk_only);
    println!("[perf] cold scan + cache {:>8.2?}  (misses {})", cold_t, cold.stats.cache_misses);
    println!("[perf] prune + save      {:>8.2?}  ({} bytes)", save_t, bytes);
    println!("[perf] cache load        {:>8.2?}  ({} entries)", load_t, cache.len());
    println!(
        "[perf] warm scan + cache {:>8.2?}  (hits {}, misses {})",
        warm_t, warm.stats.cache_hits, warm.stats.cache_misses
    );
    println!("[perf] folder tree       {:>8.2?}  (root {} files)", tree_t, tree.totals(root).files);
    assert_eq!(warm.stats.cache_hits as usize, n);
    assert_eq!(tree.totals(root).files as usize, n);
}
