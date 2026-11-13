//! Zeroed files (CLN-DET-07): non-empty files whose every byte is zero, usually the remains of a
//! failed copy or a disk fault.
//!
//! Strategy (cheap first, certain last): a bounded sample (head, middle, tail) rejects almost every
//! file after three small reads. Only files whose samples are all zero get a full streaming
//! confirmation read, capped by `max_size_mb`, with a cancel check between chunks. Sparse and
//! online-only files are never read. Reads run in parallel and failed ones are retried serially,
//! because some network filesystems reject concurrent whole-file reads.
//! Owner: W2-D.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use mj_core::pool::parallel_map;
use mj_core::{
    ActionKind, Attributes, CancelToken, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput,
    Error, FileEntry, PlatformSupport, Result, SafetyClass,
};

use crate::common::{self, Ticker};
use crate::fsio;

/// Bytes read at each of the three sample points.
const SAMPLE: usize = 4096;
/// Streaming buffer of the confirmation read.
const CHUNK: usize = 1 << 20;
/// Extensions of files that are legitimately preallocated as zeros (VM disks, swap, images).
const PREALLOCATED: &[&str] = &["vhd", "vhdx", "vdi", "vmdk", "qcow2", "img", "raw", "swap", "sys", "iso", "bin"];

/// Outcome of inspecting one file.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    NotZero,
    Zero,
    TooLarge,
    Failed,
}

/// Zeroed files: Files filled with zeros, usually corrupted
#[derive(Debug, Default, Clone, Copy)]
pub struct ZeroedFiles;

impl Detector for ZeroedFiles {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::ZeroedFiles,
            title: "Zeroed files".into(),
            description: "Files filled with zeros, usually corrupted".into(),
            category: Category::Clean,
            safety: SafetyClass::Reversible,
            default_action: ActionKind::Trash,
            available_actions: vec![ActionKind::Trash, ActionKind::Quarantine],
            settings: vec![
                common::number_setting(
                    "min_size_kb",
                    "Smallest file to check",
                    "Tiny all-zero files are often deliberate placeholders.",
                    1.0,
                    0.0,
                    1_048_576.0,
                    "KB",
                ),
                common::number_setting(
                    "max_size_mb",
                    "Largest file to confirm",
                    "Bigger files whose samples are all zero are skipped, not read in full.",
                    256.0,
                    1.0,
                    65_536.0,
                    "MB",
                ),
                common::number_setting(
                    "min_age_hours",
                    "Ignore files changed within",
                    "A file still being written or preallocated may look zeroed.",
                    24.0,
                    0.0,
                    8760.0,
                    "hours",
                ),
            ],
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-07".into()],
            unit: "files".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let min_size = (ctx.params.number("min_size_kb", 1.0).max(0.0) * 1024.0) as u64;
        let max_size = (ctx.params.number("max_size_mb", 256.0).max(1.0) * 1_048_576.0) as u64;
        let cutoff = common::hours_ago(ctx.now, ctx.params.number("min_age_hours", 24.0));
        let mut out = DetectorOutput::default();
        let mut tick = Ticker::new(ctx, ctx.index.len() as u64);

        let mut work: Vec<&FileEntry> = Vec::new();
        for e in ctx.index.entries() {
            tick.tick()?;
            if !e.is_file() || e.size == 0 || e.size < min_size || !common::usable(e) {
                continue;
            }
            if e.attrs.contains(Attributes::SPARSE) {
                out.diagnostics.skip("sparse");
                continue;
            }
            if PREALLOCATED.contains(&e.extension().as_str()) {
                out.diagnostics.skip("preallocated_type");
                continue;
            }
            // Unknown or recent modification: possibly still being written.
            if !common::before(e.modified, cutoff) {
                out.diagnostics.skip("recently_modified");
                continue;
            }
            work.push(e);
        }

        let cancel = ctx.cancel;
        let mut verdicts: Vec<Option<Verdict>> =
            parallel_map(&work, ctx.threads, cancel, |_, e| inspect(&e.path, e.size, max_size, cancel))
                .into_iter()
                .collect();
        cancel.checkpoint()?;
        // Serial retry of failures (some filesystems only fail under concurrency).
        for (i, v) in verdicts.iter_mut().enumerate() {
            if matches!(v, None | Some(Verdict::Failed)) {
                *v = Some(inspect(&work[i].path, work[i].size, max_size, cancel));
            }
        }
        cancel.checkpoint()?;

        for (e, v) in work.iter().zip(verdicts) {
            match v {
                Some(Verdict::Zero) => {
                    let reason =
                        format!("Every byte is zero ({}); the content is lost", mj_core::size::format_bytes(e.size));
                    tick.found(e.reclaimable());
                    out.candidates.push(common::file_candidate(e, DetectorId::ZeroedFiles, reason, true));
                }
                Some(Verdict::TooLarge) => out.diagnostics.skip("too_large_to_confirm"),
                Some(Verdict::NotZero) => {}
                _ => out.diagnostics.unreadable(&e.path),
            }
        }
        tick.finish();
        Ok(out)
    }
}

/// Sample first, then confirm in full. Cancellation surfaces as `Failed` and is re-checked by the caller.
fn inspect(path: &Path, size: u64, max_size: u64, cancel: &CancelToken) -> Verdict {
    match sample_is_zero(path, size) {
        Err(_) => return Verdict::Failed,
        Ok(false) => return Verdict::NotZero,
        Ok(true) => {}
    }
    if size > max_size {
        return Verdict::TooLarge;
    }
    match full_is_zero(path, cancel) {
        Ok(true) => Verdict::Zero,
        Ok(false) => Verdict::NotZero,
        Err(_) => Verdict::Failed,
    }
}

fn all_zero(b: &[u8]) -> bool {
    b.iter().all(|&x| x == 0)
}

/// Head, middle and tail samples all zero? A short file is read whole.
fn sample_is_zero(path: &Path, size: u64) -> std::io::Result<bool> {
    let mut f = File::open(path)?;
    if size <= (3 * SAMPLE) as u64 {
        return Ok(all_zero(&fsio::read_at(&mut f, 0, size as usize)?));
    }
    let mid = size / 2 - (SAMPLE as u64) / 2;
    let tail = size - SAMPLE as u64;
    for off in [0, mid, tail] {
        if !all_zero(&fsio::read_at(&mut f, off, SAMPLE)?) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Stream the whole file; stops at the first non-zero byte or when cancelled.
fn full_is_zero(path: &Path, cancel: &CancelToken) -> std::io::Result<bool> {
    let mut f = File::open(path)?;
    let mut buf = vec![0u8; CHUNK];
    loop {
        if cancel.is_cancelled() {
            return Err(std::io::Error::other(Error::Cancelled.to_string()));
        }
        let n = f.read(&mut buf)?;
        if n == 0 {
            return Ok(true);
        }
        if !all_zero(&buf[..n]) {
            return Ok(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::*;
    use mj_core::testutil::TempDir;

    fn fixture() -> TempDir {
        let t = TempDir::new("zeroed").unwrap();
        t.write("zero_small.bin.dat", &vec![0u8; 5000]).unwrap();
        t.write("zero_big.dat", &vec![0u8; 3 << 20]).unwrap();
        // Zero at head, middle and tail but a single non-zero byte elsewhere: only the full read finds it.
        let mut sneaky = vec![0u8; 200_000];
        sneaky[10_000] = 1;
        t.write("sneaky.dat", &sneaky).unwrap();
        t.write("real.dat", b"hello world, not zero").unwrap();
        t.write("vm.vhdx", &vec![0u8; 5000]).unwrap();
        t.write("empty.dat", b"").unwrap();
        for f in ["zero_small.bin.dat", "zero_big.dat", "sneaky.dat", "real.dat", "vm.vhdx"] {
            age(&t.join(f), 5);
        }
        t
    }

    #[test]
    fn finds_only_fully_zero_files() {
        let t = fixture();
        let out = run(&ZeroedFiles, &index_of(t.path()), &p(&[("min_size_kb", 0.0)]));
        assert_eq!(rel(&out, t.path()), vec!["zero_big.dat", "zero_small.bin.dat"]);
        assert!(out.candidates.iter().all(|c| c.default_selected && c.size_after == 0));
        assert_eq!(out.diagnostics.skipped["preallocated_type"], 1);
    }

    #[test]
    fn fresh_files_and_small_files_are_skipped() {
        let t = fixture();
        t.write("fresh.dat", &vec![0u8; 5000]).unwrap(); // modified just now
        let out = run(&ZeroedFiles, &index_of(t.path()), &p(&[("min_size_kb", 0.0)]));
        assert!(!rel(&out, t.path()).contains(&"fresh.dat".to_string()));
        assert_eq!(out.diagnostics.skipped["recently_modified"], 1);
        // Default minimum size is 1 KiB: a 100-byte zero file is ignored.
        let s = TempDir::new("zeroed_small").unwrap();
        s.write("tiny.dat", &[0u8; 100]).unwrap();
        age(&s.join("tiny.dat"), 5);
        assert!(run(&ZeroedFiles, &index_of(s.path()), &Default::default()).candidates.is_empty());
    }

    #[test]
    fn large_zero_files_are_not_confirmed_over_the_cap() {
        let t = TempDir::new("zeroed_cap").unwrap();
        t.write("big.dat", &vec![0u8; 2 << 20]).unwrap();
        age(&t.join("big.dat"), 5);
        let out = run(&ZeroedFiles, &index_of(t.path()), &p(&[("max_size_mb", 1.0)]));
        assert!(out.candidates.is_empty());
        assert_eq!(out.diagnostics.skipped["too_large_to_confirm"], 1);
    }

    #[test]
    fn online_only_files_are_never_read() {
        let t = fixture();
        let mut ix = index_of(t.path());
        // Rebuild the index with the zero file flagged as an online-only placeholder.
        let mut entries: Vec<FileEntry> = ix.entries().to_vec();
        for e in &mut entries {
            if e.path.ends_with("zero_big.dat") {
                e.attrs |= Attributes::ONLINE_ONLY;
            }
        }
        ix = mj_core::FileIndex::new(ix.roots.clone(), entries, Default::default(), ix.scanned_at);
        let out = run(&ZeroedFiles, &ix, &p(&[("min_size_kb", 0.0)]));
        assert_eq!(rel(&out, t.path()), vec!["zero_small.bin.dat"]);
    }

    #[test]
    fn unreadable_files_are_reported_not_dropped() {
        let t = fixture();
        let ix = index_of(t.path());
        std::fs::remove_file(t.join("zero_big.dat")).unwrap(); // vanished after the scan
        let out = run(&ZeroedFiles, &ix, &p(&[("min_size_kb", 0.0)]));
        assert_eq!(out.diagnostics.unreadable, 1);
    }

    #[test]
    fn cancel_is_honoured() {
        let t = fixture();
        let c = CancelToken::new();
        c.cancel();
        let r = run_at(&ZeroedFiles, &index_of(t.path()), &Default::default(), mj_core::Timestamp::now(), &c);
        assert_eq!(r.unwrap_err(), Error::Cancelled);
    }
}
