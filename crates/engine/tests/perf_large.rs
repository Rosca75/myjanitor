//! Large-tree performance through the engine facade (W4-INT item 9). Ignored by default:
//!
//! ```text
//! CARGO_INCREMENTAL=0 cargo test --profile ci -p mj-engine --test perf_large -- --ignored --nocapture
//! ```
//!
//! Generates `MJ_PERF_FILES` (default 200 000) files in a temp dir (developer artefact
//! folders, temp files, duplicate pairs, plain files), then times scan → detectors →
//! opportunities → step paging → plan → dry run and prints peak RSS (Linux). Nothing is
//! applied: the tree is only read and then deleted with its temp dir.

mod common;

use std::time::Instant;

use common::*;
use mj_core::testutil::TempDir;
use mj_core::OpportunityId;
use mj_engine::api::*;

fn peak_rss_mib() -> Option<f64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    let kb: f64 = s.lines().find(|l| l.starts_with("VmHWM:"))?.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024.0)
}

fn generate(n: usize) -> TempDir {
    let t = TempDir::new("engine-perf").unwrap();
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(86_400 * 10);
    let mut made = 0usize;
    let mut i = 0usize;
    while made < n {
        let p = i % 20;
        let base = format!("p{:03}", i / 1000);
        match p {
            // Developer artefacts: many small files below node_modules (60 %).
            0..=11 => {
                for k in 0..10 {
                    t.write(format!("{base}/proj{i}/node_modules/m{k}/index.js"), b"x").unwrap();
                }
                t.write(format!("{base}/proj{i}/package.json"), b"{}").unwrap();
                made += 11;
            }
            // Temp files (10 %).
            12 | 13 => {
                let f = t.write(format!("{base}/work{i}/cache.tmp"), b"t").unwrap();
                mj_core::testutil::set_mtime(&f, old).unwrap();
                made += 1;
            }
            // Duplicate pairs of distinct content (10 %).
            14 | 15 => {
                let data = format!("duplicate content number {i} ").repeat(8);
                t.write(format!("{base}/a{i}/doc.bin"), data.as_bytes()).unwrap();
                t.write(format!("{base}/b{i}/doc copy.bin"), data.as_bytes()).unwrap();
                made += 2;
            }
            // Plain unique files.
            _ => {
                t.write(format!("{base}/plain{i}.txt"), format!("unique {i}").as_bytes()).unwrap();
                made += 1;
            }
        }
        i += 1;
    }
    t
}

fn timed<T>(label: &str, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let v = f();
    println!("[perf] {label:<38} {:>9.1} ms", t.elapsed().as_secs_f64() * 1000.0);
    v
}

#[test]
#[ignore = "large-tree benchmark; run explicitly (see the module docs)"]
fn large_tree_scan_detect_page_and_dry_run() {
    let n: usize = std::env::var("MJ_PERF_FILES").ok().and_then(|v| v.parse().ok()).unwrap_or(200_000);
    let tree = timed("generate tree", || generate(n));
    let f = engine_for(tree);
    let e = &f.engine;
    let status = timed("scan + detectors (cold)", || scan(e));
    assert_eq!(status.state, ScanRunState::Done, "{:?}", status.error);
    let stats = status.stats.unwrap();
    println!("[perf] files {} dirs {}", stats.files, stats.dirs);
    assert!(stats.files as usize >= n);
    let list = timed("opportunities_list", || e.opportunities_list(OpportunitiesRequest::default()).unwrap());
    for o in &list.items {
        if o.candidates > 0 {
            println!("[perf]   {:<28} {:>7} candidates", o.id.0, o.candidates);
        }
    }
    for id in ["det:dev_artefacts", "det:temp_files", "det:exact_duplicates"] {
        let total = list.items.iter().find(|o| o.id.0 == id).map(|o| o.candidates).unwrap_or(0);
        assert!(total > 0, "{id} found nothing");
        for (label, offset, sort) in [
            ("first page by saving", 0, CandidateSortKey::Saving),
            ("last page by path", total.saturating_sub(100), CandidateSortKey::Path),
        ] {
            timed(&format!("{id} {label}"), || {
                e.candidates_page(CandidatesPageRequest {
                    opportunity_id: OpportunityId(id.into()),
                    sub_group: None,
                    offset,
                    limit: 100,
                    sort,
                    descending: false,
                    filter: None,
                })
                .unwrap()
            });
        }
        timed(&format!("{id} filtered page"), || {
            e.candidates_page(CandidatesPageRequest {
                opportunity_id: OpportunityId(id.into()),
                sub_group: None,
                offset: 0,
                limit: 100,
                sort: CandidateSortKey::Saving,
                descending: true,
                filter: Some("p01".into()),
            })
            .unwrap()
        });
        timed(&format!("{id} plan_add_step"), || {
            e.plan_add_step(PlanAddStepRequest { opportunity_id: OpportunityId(id.into()), action: None, params: None })
                .unwrap()
        });
    }
    let job = timed("plan_dry_run (queue)", || e.plan_dry_run().unwrap());
    let report = timed("dry run over every selected item", || {
        wait("dry run", || {
            f.events.events().into_iter().find_map(|ev| match ev {
                Event::DryRun(r) if r.job_id == job.id => Some(r),
                _ => None,
            })
        })
    });
    println!("[perf] dry run: {} files, saving {} bytes", report.total.files, report.total.saving());
    let again = timed("rescan + detectors (warm cache)", || scan(e));
    assert!(again.stats.unwrap().cache_hits > 0);
    if let Some(m) = peak_rss_mib() {
        println!("[perf] peak RSS {m:.0} MiB");
    }
}
