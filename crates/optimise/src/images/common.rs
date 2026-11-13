//! Helpers shared by the W3-C optimisers: bounded whole-file reads, writing the destination
//! without ever clobbering an existing file, the standard skipped/finished results, and the
//! `analyse` loop that turns index entries into candidates.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use mj_core::{
    Badges, Candidate, CandidateId, Confidence, DetectContext, DetectorOutput, Error, Estimate, FileEntry,
    OptimiseOutput, OptimiserId, ParamValues, Result, Source,
};

/// Largest image file read into memory by the in-house rewriters (512 MiB).
pub const MAX_IN_MEMORY: u64 = 512 * 1024 * 1024;

/// Read a whole file, refusing anything over `limit` bytes.
pub fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let len = std::fs::metadata(path).map_err(|e| Error::io(path, e))?.len();
    if len > limit {
        return Err(Error::unsupported("optimise", format!("file is larger than {} MiB", limit >> 20)));
    }
    std::fs::read(path).map_err(|e| Error::io(path, e))
}

/// Write `bytes` to `dst`, which must not exist (never overwrites). A failed write removes the partial file.
pub fn write_new(dst: &Path, bytes: &[u8]) -> Result<()> {
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(dst).map_err(|e| Error::io(dst, e))?;
    if let Err(e) = f.write_all(bytes).and_then(|()| f.sync_all()) {
        drop(f);
        let _ = std::fs::remove_file(dst);
        return Err(Error::io(dst, e));
    }
    Ok(())
}

/// Move a finished temp file to `dst` (rename, or copy across volumes). `dst` must not exist.
pub fn move_into_place(tmp: &Path, dst: &Path) -> Result<()> {
    if dst.exists() {
        return Err(Error::Conflict(format!("{} already exists", dst.display())));
    }
    if std::fs::rename(tmp, dst).is_ok() {
        return Ok(());
    }
    match std::fs::copy(tmp, dst) {
        Ok(_) => {
            let _ = std::fs::remove_file(tmp);
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(dst);
            Err(Error::io(dst, e))
        }
    }
}

/// Result for "nothing was written".
pub fn skipped(bytes_before: u64, reason: impl Into<String>) -> OptimiseOutput {
    OptimiseOutput {
        bytes_before,
        bytes_after: bytes_before,
        lossy: false,
        skipped: Some(reason.into()),
        notes: Vec::new(),
    }
}

/// Result for a written `dst`.
pub fn done(bytes_before: u64, bytes_after: u64, lossy: bool, notes: Vec<String>) -> OptimiseOutput {
    OptimiseOutput { bytes_before, bytes_after, lossy, skipped: None, notes }
}

/// How one candidate is described in the step review table.
pub struct Described {
    pub reason: String,
    pub columns: BTreeMap<String, String>,
    pub sub_group: Option<String>,
}

/// The `analyse` loop shared by every W3-C optimiser: every file `applies` to (and at least
/// `min_size` bytes, local) becomes a candidate sized by `estimate`; files that would not shrink are
/// counted in the diagnostics instead of listed.
pub fn analyse_entries(
    ctx: &DetectContext<'_>,
    optimiser: OptimiserId,
    min_size: u64,
    applies: &dyn Fn(&FileEntry) -> bool,
    estimate: &dyn Fn(&FileEntry, &ParamValues) -> Result<Estimate>,
    describe: &dyn Fn(&FileEntry, &Estimate) -> Described,
) -> Result<DetectorOutput> {
    let mut out = DetectorOutput::default();
    for (n, e) in ctx.index.files().enumerate() {
        if n % 64 == 0 {
            ctx.cancel.checkpoint()?;
        }
        if !applies(e) {
            continue;
        }
        if e.is_online_only() {
            out.diagnostics.skip("online_only");
            continue;
        }
        if e.size < min_size {
            out.diagnostics.skip("too_small");
            continue;
        }
        let est = match estimate(e, ctx.params) {
            Ok(x) => x,
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(Error::Unsupported { .. }) => {
                out.diagnostics.skip("not_applicable");
                continue;
            }
            Err(_) => {
                out.diagnostics.skip("estimate_failed");
                continue;
            }
        };
        if est.bytes_after >= est.bytes_before {
            out.diagnostics.skip("no_saving_expected");
            continue;
        }
        let d = describe(e, &est);
        out.candidates.push(Candidate {
            id: CandidateId(0),
            path: e.path.clone(),
            target: mj_core::candidate::TargetKind::File,
            source: Source::Optimiser(optimiser),
            size: est.bytes_before,
            size_after: est.bytes_after,
            confidence: est.confidence,
            reason: d.reason,
            badges: Badges::empty(),
            default_selected: true,
            group: None,
            sub_group: d.sub_group,
            modified: e.modified,
            stamp: e.stamp(),
            file_count: None,
            columns: d.columns,
        });
    }
    Ok(out)
}

/// One-file estimate from a size ratio (estimated confidence).
pub fn ratio_estimate(entry: &FileEntry, ratio: f64) -> Estimate {
    let after = ((entry.size as f64) * ratio.clamp(0.0, 1.0)).round() as u64;
    Estimate {
        files: 1,
        bytes_before: entry.size,
        bytes_after: after.min(entry.size),
        confidence: Confidence::Estimated,
        skipped: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mj_core::testutil::TempDir;

    #[test]
    fn write_new_never_overwrites_and_move_refuses_existing() {
        let d = TempDir::new("common").unwrap();
        let p = d.join("a");
        write_new(&p, b"x").unwrap();
        assert!(write_new(&p, b"y").is_err());
        assert_eq!(std::fs::read(&p).unwrap(), b"x");
        let t = d.write("t", b"tmp").unwrap();
        assert!(matches!(move_into_place(&t, &p), Err(Error::Conflict(_))));
        let q = d.join("q");
        move_into_place(&t, &q).unwrap();
        assert_eq!(std::fs::read(&q).unwrap(), b"tmp");
    }

    #[test]
    fn read_bounded_refuses_big_files() {
        let d = TempDir::new("common2").unwrap();
        let p = d.write("a", &[0u8; 100]).unwrap();
        assert!(read_bounded(&p, 50).is_err());
        assert_eq!(read_bounded(&p, 100).unwrap().len(), 100);
    }
}

/// Settings-panel builders shared by the optimisers.
pub mod spec {
    use mj_core::params::{ChoiceOption, SettingKind, SettingSpec};
    use mj_core::{ParamValue, ParamValues, StepPreset};

    /// A toggle.
    pub fn toggle(
        key: &str,
        label: &str,
        help: Option<&str>,
        default: bool,
        advanced: bool,
        lossy: bool,
    ) -> SettingSpec {
        SettingSpec {
            key: key.into(),
            label: label.into(),
            help: help.map(Into::into),
            kind: SettingKind::Toggle { default },
            advanced,
            lossy,
            disabled_reason: None,
        }
    }

    /// A number input.
    pub fn number(
        key: &str,
        label: &str,
        default: f64,
        min: f64,
        max: f64,
        step: f64,
        unit: Option<&str>,
        advanced: bool,
    ) -> SettingSpec {
        SettingSpec {
            key: key.into(),
            label: label.into(),
            help: None,
            kind: SettingKind::Number { default, min, max, step, unit: unit.map(Into::into) },
            advanced,
            lossy: false,
            disabled_reason: None,
        }
    }

    /// A choice from `(value, label, help)` options.
    pub fn choice(
        key: &str,
        label: &str,
        default: &str,
        options: &[(&str, &str, Option<&str>)],
        advanced: bool,
    ) -> SettingSpec {
        SettingSpec {
            key: key.into(),
            label: label.into(),
            help: None,
            kind: SettingKind::Choice {
                default: default.into(),
                options: options
                    .iter()
                    .map(|(v, l, h)| ChoiceOption { value: (*v).into(), label: (*l).into(), help: h.map(Into::into) })
                    .collect(),
            },
            advanced,
            lossy: false,
            disabled_reason: None,
        }
    }

    /// A preset tile; `values` are `(key, value)` pairs.
    pub fn preset(id: &str, label: &str, description: &str, lossy: bool, values: &[(&str, ParamValue)]) -> StepPreset {
        let mut v = ParamValues::new();
        for (k, x) in values {
            v.set(k, x.clone());
        }
        StepPreset { id: id.into(), label: label.into(), description: description.into(), values: v, lossy }
    }

    /// Shorthand for text and number values.
    pub fn t(s: &str) -> ParamValue {
        ParamValue::Text(s.into())
    }
    pub fn n(x: f64) -> ParamValue {
        ParamValue::Number(x)
    }
    pub fn b(x: bool) -> ParamValue {
        ParamValue::Bool(x)
    }
}
