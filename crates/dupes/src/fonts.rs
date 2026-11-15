//! Duplicate installed fonts (OPT-FNT-01) in user and system font folders.
//! Owner: W2-B.
//!
//! Font files ([`EXTENSIONS`]) under the font folders are compared by content
//! (staged exact lane). The folders come from the [`FONT_FOLDERS`] parameter
//! (one per line); when it is empty every folder named `Fonts` or `.fonts`
//! counts (Windows `C:\Windows\Fonts` and the per-user
//! `AppData\Local\Microsoft\Windows\Fonts`, Linux `/usr/share/fonts`,
//! `~/.local/share/fonts`, `~/.fonts`). Results are review only: the copy in
//! a system font folder is kept when there is one, else the shortest path.
//! Fonts that are the same family but different bytes are not reported.

use mj_core::{
    ActionKind, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, Diagnostics, FileEntry,
    GroupId, GroupKind, KeepRule, PlatformSupport, Result, SafetyClass, SettingKind, SettingSpec,
};

use crate::keep::{self, GroupSpec, Selection};
use crate::staged;

/// Font file extensions.
pub const EXTENSIONS: &[&str] = &["ttf", "otf", "ttc", "woff", "woff2"];
/// Parameter key: font folders.
pub const FONT_FOLDERS: &str = "font_folders";

/// Duplicate fonts: The same font installed more than once
#[derive(Debug, Default, Clone, Copy)]
pub struct DuplicateFonts;

impl Detector for DuplicateFonts {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::DuplicateFonts,
            title: "Duplicate fonts".into(),
            description: "The same font installed more than once".into(),
            category: Category::Optimise,
            safety: SafetyClass::ReviewOnly,
            default_action: ActionKind::None,
            available_actions: vec![ActionKind::None, ActionKind::Trash],
            settings: vec![SettingSpec {
                key: FONT_FOLDERS.into(),
                label: "Font folders".into(),
                help: Some("Empty: every folder named Fonts or .fonts".into()),
                kind: SettingKind::Text { default: String::new(), placeholder: Some("One folder per line".into()) },
                advanced: true,
                lossy: false,
                disabled_reason: None,
            }],
            platform: PlatformSupport::Both,
            feature_ids: vec!["OPT-FNT-01".into()],
            unit: "fonts".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let folders = crate::path_list(ctx.params.text(FONT_FOLDERS, ""));
        let fonts: Vec<&FileEntry> = ctx
            .index
            .files()
            .filter(|e| EXTENSIONS.contains(&e.extension().as_str()))
            .filter(|e| {
                if folders.is_empty() {
                    e.path.parent().is_some_and(|p| {
                        p.components().any(|c| {
                            let n = c.as_os_str().to_string_lossy();
                            n.eq_ignore_ascii_case("fonts") || n == ".fonts"
                        })
                    })
                } else {
                    folders.iter().any(|f| mj_core::pathutil::is_within(f, &e.path))
                }
            })
            .collect();
        let mut diag = Diagnostics::default();
        let groups = staged::exact_groups(ctx, &fonts, &mut diag)?;
        let mut out = DetectorOutput::default();
        for (n, g) in groups.iter().enumerate() {
            let members: Vec<&FileEntry> = g.members.iter().map(|&i| fonts[i]).collect();
            let keeper = members
                .iter()
                .position(|e| is_system_font_path(&e.path))
                .unwrap_or_else(|| keep::pick(&KeepRule::ShortestPath, &members, &[]));
            let count = members.len();
            let label = format!("{count} copies of the font {}", members[keeper].name());
            keep::emit(
                GroupSpec {
                    id: GroupId(n as u64 + 1),
                    kind: GroupKind::Font,
                    source: DetectorId::DuplicateFonts,
                    label,
                    confidence: 100.0,
                    members,
                    keeper,
                    selection: Selection::None,
                    badges: mj_core::Badges::DUPLICATE,
                    reason: "Same font file installed in another place".into(),
                    keeper_reason: "Installed copy kept".into(),
                    columns: Vec::new(),
                },
                &mut out,
            );
        }
        out.diagnostics = diag;
        Ok(out)
    }
}

/// True for the system-wide font folders (Windows `\Windows\Fonts`, `/usr/share/fonts`, `/usr/local/share/fonts`).
pub fn is_system_font_path(path: &std::path::Path) -> bool {
    let key = mj_core::pathutil::path_key(path).to_lowercase();
    key.contains("/windows/fonts/") && !key.contains("/appdata/")
        || key.starts_with("/usr/share/fonts/")
        || key.starts_with("/usr/local/share/fonts/")
}
