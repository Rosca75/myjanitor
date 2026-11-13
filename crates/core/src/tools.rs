//! Optional external command-line tools, detected at runtime, never downloaded.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Tool ids MyJanitor knows about.
pub const KNOWN_TOOLS: &[&str] = &["ffmpeg", "ffprobe", "avifenc", "cjxl", "cwebp", "7z", "zstd", "oxipng", "jpegtran"];

/// Detection result for one tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalTool {
    /// One of [`KNOWN_TOOLS`].
    pub id: String,
    pub found: bool,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::pathutil::serde_path::opt")]
    pub path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Features that need it (for the Settings page).
    pub used_for: Vec<String>,
}

/// Look up a found tool by id.
pub fn find<'a>(tools: &'a [ExternalTool], id: &str) -> Option<&'a ExternalTool> {
    tools.iter().find(|t| t.id == id && t.found)
}
