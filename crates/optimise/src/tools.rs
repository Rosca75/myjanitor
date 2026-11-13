//! External tool detection (W3-C owns version probing and adapters).
//! Wave 1: PATH lookup only (no process is started).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use mj_core::tools::KNOWN_TOOLS;
use mj_core::ExternalTool;

/// Features that use each tool (Settings page text).
fn used_for(id: &str) -> Vec<String> {
    let v: &[&str] = match id {
        "ffmpeg" | "ffprobe" => &["Video", "Screen recordings", "Meeting recordings"],
        "avifenc" | "cjxl" | "cwebp" => &["Photos (lossy)"],
        "7z" => &["7z archives"],
        "zstd" => &["zstd archives"],
        "oxipng" => &["PNG images"],
        "jpegtran" => &["JPEG images"],
        _ => &[],
    };
    v.iter().map(|s| s.to_string()).collect()
}

/// Find `name` on `PATH` (with `.exe` on Windows).
pub fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exe = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    std::env::split_paths(&path).map(|d| d.join(&exe)).find(|p| p.is_file())
}

/// Detect every known tool; `overrides` (from settings) win over PATH.
pub fn detect(overrides: &BTreeMap<String, PathBuf>) -> Vec<ExternalTool> {
    KNOWN_TOOLS
        .iter()
        .map(|id| {
            let path = overrides.get(*id).filter(|p| Path::new(p).is_file()).cloned().or_else(|| which(id));
            ExternalTool { id: id.to_string(), found: path.is_some(), path, version: None, used_for: used_for(id) }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn detects_every_known_tool() {
        let t = super::detect(&Default::default());
        assert_eq!(t.len(), mj_core::tools::KNOWN_TOOLS.len());
        assert!(t.iter().all(|x| x.found == x.path.is_some()));
    }
}
