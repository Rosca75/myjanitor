//! Persisted user settings (Settings drawer) and scheduled profiles (AUT-02).
//! Stored as JSON by the engine (`settings.json`, `schedules.json`); unknown
//! fields are ignored and missing fields take defaults, so adding a field is
//! backward compatible. Bump [`SETTINGS_VERSION`] only for meaning changes.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::action::{Action, OriginalsPolicy};
use crate::opportunity::OpportunityId;
use crate::params::ParamValues;
use crate::scope::{ScanOptions, ScanScope};
use crate::time::Timestamp;

/// Current settings schema version.
pub const SETTINGS_VERSION: u32 = 1;

/// All settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub version: u32,
    pub general: GeneralSettings,
    /// Defaults for new scans.
    pub scan: ScanOptions,
    pub safety: SafetySettings,
    pub optimise: OptimiseSettings,
    pub performance: PerformanceSettings,
    /// Manual paths for external tools, by tool id (overrides PATH lookup).
    #[serde(with = "path_map")]
    pub tool_paths: BTreeMap<String, PathBuf>,
    pub integration: IntegrationSettings,
    /// Editable file-type groups (CLN-FLT-02): group id -> lower-case extensions.
    pub type_groups: BTreeMap<String, Vec<String>>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            version: SETTINGS_VERSION,
            general: GeneralSettings::default(),
            scan: ScanOptions::default(),
            safety: SafetySettings::default(),
            optimise: OptimiseSettings::default(),
            performance: PerformanceSettings::default(),
            tool_paths: BTreeMap::new(),
            integration: IntegrationSettings::default(),
            type_groups: default_type_groups(),
        }
    }
}

/// General behaviour.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralSettings {
    /// Days an applied run stays undoable from History (SAF-03).
    pub undo_days: u32,
    /// Days before quarantined items are purged (CLN-ACT-04).
    pub quarantine_days: u32,
    /// Default originals policy for optimisations (OPT-ENG-06).
    pub default_originals: OriginalsPolicy,
    /// Suffix of light copies (OPT-ENG-07).
    pub light_copy_suffix: String,
    /// Quarantine folder override (default: `<data dir>/quarantine` on the same volume when possible).
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::pathutil::serde_path::opt")]
    pub quarantine_dir: Option<PathBuf>,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        GeneralSettings {
            undo_days: 30,
            quarantine_days: 30,
            default_originals: OriginalsPolicy::RecycleBin,
            light_copy_suffix: "_light".into(),
            quarantine_dir: None,
        }
    }
}

/// Safety rules (SAF-*). The built-in protected system paths are not here: they are
/// fixed in `mj-actions` and shown read-only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SafetySettings {
    /// User-protected paths (SAF-01): never touched, shown with the `protected` badge.
    #[serde(with = "crate::pathutil::serde_path::vec")]
    pub protected_paths: Vec<PathBuf>,
    /// Global exclusions (CLN-ENG-03): glob patterns or paths never scanned.
    pub exclusions: Vec<String>,
    /// Skip digitally signed files (SAF-09).
    pub skip_signed: bool,
    /// Skip sensitivity-labelled files (SAF-07).
    pub skip_labelled: bool,
    /// Skip encrypted / password-protected files (SAF-07).
    pub skip_encrypted: bool,
    /// Warn before removing files in synced folders (SAF-10).
    pub synced_folder_warning: bool,
}

impl Default for SafetySettings {
    fn default() -> Self {
        SafetySettings {
            protected_paths: Vec::new(),
            exclusions: Vec::new(),
            skip_signed: true,
            skip_labelled: true,
            skip_encrypted: true,
            synced_folder_warning: true,
        }
    }
}

/// Optimisation defaults (OPT-ENG-*).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct OptimiseSettings {
    /// OPT-ENG-01 global switch: when true lossy presets are hidden unless a step opts in.
    pub lossless_only: bool,
    /// OPT-ENG-04 (on by default; the UI shows it locked on).
    pub keep_if_not_smaller: bool,
    /// OPT-ENG-05.
    pub preserve_timestamps: bool,
    pub preserve_permissions: bool,
    /// Windows alternate data streams.
    pub preserve_ads: bool,
}

impl Default for OptimiseSettings {
    fn default() -> Self {
        OptimiseSettings {
            lossless_only: true,
            keep_if_not_smaller: true,
            preserve_timestamps: true,
            preserve_permissions: true,
            preserve_ads: true,
        }
    }
}

/// CPU priority of background work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CpuPriority {
    #[default]
    Normal,
    Low,
}

/// Performance (AUT-05..07, OPT-ENG-11).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PerformanceSettings {
    /// Worker threads (0 = logical CPUs).
    pub threads: u16,
    /// Parallel optimisation jobs (0 = automatic).
    pub parallel_jobs: u16,
    /// Low-memory mode for very large drives (AUT-06).
    pub low_memory: bool,
    /// Temp folder (AUT-07); `None` = system temp.
    #[serde(skip_serializing_if = "Option::is_none", with = "crate::pathutil::serde_path::opt")]
    pub temp_dir: Option<PathBuf>,
    pub cpu_priority: CpuPriority,
}

/// OS integration (AUT-03, OPT-FSC-03).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct IntegrationSettings {
    /// Windows Explorer / Linux file-manager context menu entries (per user, no admin).
    pub context_menu: bool,
    /// Background recompression watcher (OPT-FSC-03, Windows).
    pub fs_compression_watcher: bool,
    #[serde(with = "crate::pathutil::serde_path::vec")]
    pub watched_folders: Vec<PathBuf>,
}

/// The shipped file-type groups (CLN-FLT-02).
pub fn default_type_groups() -> BTreeMap<String, Vec<String>> {
    let g = |exts: &[&str]| exts.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    BTreeMap::from([
        ("audio".into(), g(&["mp3", "m4a", "aac", "wav", "flac", "ogg", "opus", "wma", "aiff", "aif"])),
        ("video".into(), g(&["mp4", "mkv", "mov", "avi", "wmv", "webm", "m4v", "mpg", "mpeg", "flv"])),
        ("images".into(), g(&["jpg", "jpeg", "png", "gif", "bmp", "tif", "tiff", "webp", "heic", "heif", "avif"])),
        ("raw".into(), g(&["cr2", "cr3", "nef", "arw", "dng", "raf", "orf", "rw2"])),
        ("office".into(), g(&["docx", "xlsx", "pptx", "doc", "xls", "ppt", "odt", "ods", "odp", "rtf"])),
        ("pdf".into(), g(&["pdf"])),
        ("archives".into(), g(&["zip", "7z", "rar", "tar", "gz", "tgz", "bz2", "xz", "zst"])),
        ("installers".into(), g(&["msi", "exe", "msix", "appx", "iso", "img", "dmg", "deb", "rpm", "appimage"])),
        ("system".into(), g(&["dll", "sys", "so", "dmp", "log", "etl", "tmp"])),
        ("fonts".into(), g(&["ttf", "otf", "ttc", "woff", "woff2"])),
    ])
}

/// When a schedule runs (local time).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "every", rename_all = "snake_case")]
pub enum Cadence {
    Daily {
        hour: u8,
        minute: u8,
    },
    /// `weekday`: 1 = Monday ... 7 = Sunday (ISO 8601).
    Weekly {
        weekday: u8,
        hour: u8,
        minute: u8,
    },
    /// `day`: 1-28.
    Monthly {
        day: u8,
        hour: u8,
        minute: u8,
    },
}

/// One step of a schedule. Only steps whose action needs no confirmation may be
/// scheduled (SAF-11): the engine rejects lossy or permanent steps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScheduledStep {
    pub opportunity: OpportunityId,
    pub action: Action,
    #[serde(default)]
    pub params: ParamValues,
}

/// A scheduled profile ("Weekly Downloads cleanup").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Schedule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub cadence: Cadence,
    pub scope: ScanScope,
    pub steps: Vec<ScheduledStep>,
    #[serde(default)]
    pub last_run: Option<Timestamp>,
    #[serde(default)]
    pub next_run: Option<Timestamp>,
}

mod path_map {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use crate::pathutil::{decode_path, encode_path};

    pub fn serialize<S: Serializer>(m: &BTreeMap<String, PathBuf>, s: S) -> Result<S::Ok, S::Error> {
        let enc: BTreeMap<&String, String> = m.iter().map(|(k, v)| (k, encode_path(v))).collect();
        enc.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<BTreeMap<String, PathBuf>, D::Error> {
        BTreeMap::<String, String>::deserialize(d)?
            .into_iter()
            .map(|(k, v)| decode_path(&v).map(|p| (k, p)).map_err(serde::de::Error::custom))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_tolerate_missing_and_unknown_fields() {
        let s: Settings = serde_json::from_str(r#"{"general":{"undo_days":7},"future":1}"#).unwrap();
        assert_eq!(s.general.undo_days, 7);
        assert_eq!(s.general.quarantine_days, 30);
        assert!(s.type_groups.contains_key("installers"));
        let json = serde_json::to_string(&Settings::default()).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back, Settings::default());
    }
}
