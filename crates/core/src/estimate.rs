//! Savings estimates (estimate first, act second).

use serde::{Deserialize, Serialize};

/// How trustworthy a number is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// Measured (removal sizes; dry-run output sizes).
    Exact,
    /// Predicted from a model or a sample ("about 0.6 GB").
    #[default]
    Estimated,
    /// Cannot be predicted before running (shown as "?").
    Unknown,
}

/// Bytes before and after an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Estimate {
    pub files: u64,
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub confidence: Confidence,
    /// Items that will be skipped (protected, changed, confidential, not smaller...).
    pub skipped: u64,
}

impl Estimate {
    /// Saving in bytes (never negative).
    pub fn saving(&self) -> u64 {
        self.bytes_before.saturating_sub(self.bytes_after)
    }

    /// Sum two estimates; the confidence is the weaker of the two.
    pub fn add(&self, o: &Estimate) -> Estimate {
        Estimate {
            files: self.files + o.files,
            bytes_before: self.bytes_before + o.bytes_before,
            bytes_after: self.bytes_after + o.bytes_after,
            confidence: self.confidence.max(o.confidence),
            skipped: self.skipped + o.skipped,
        }
    }
}
