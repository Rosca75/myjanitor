//! Font caches (OPT-FNT-02, review only).
//!
//! **Limitation, by design:** whether a font *family* is unused cannot be determined reliably
//! offline (applications load fonts by name at run time and nothing records which ones), so this
//! detector does not guess. It lists the font cache files and folders the operating system and
//! Adobe applications rebuild on demand, and says so in its diagnostics notes. Duplicate installed
//! fonts are the duplicates engine's job (OPT-FNT-01).
//! Owner: W2-D.

use mj_core::{
    ActionKind, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, PlatformSupport, Result,
    SafetyClass,
};

use crate::rules::{self, rule, RuleDef, RuleSet, Target};

/// Font cache locations.
pub fn font_cache_rules() -> Vec<RuleDef> {
    let d = Target::Dir;
    let f = Target::File;
    vec![
        rule(
            "Windows",
            "Font cache",
            "rebuilt by the Font Cache service",
            "**/Windows/ServiceProfiles/LocalService/AppData/Local/FontCache",
            d,
        ),
        rule("Windows", "Font cache file", "rebuilt by the Font Cache service", "**/Windows/System32/FNTCACHE.DAT", f),
        rule(
            "Windows",
            "Font cache",
            "rebuilt by the Font Cache service",
            "**/AppData/Local/Microsoft/Windows/FontCache",
            d,
        ),
        rule("Fontconfig", "Font cache", "rebuilt by fc-cache", "**/.cache/fontconfig", d),
        rule("Fontconfig", "Font cache", "rebuilt by fc-cache", "**/var/cache/fontconfig", d),
        rule("Fontconfig", "Font cache", "rebuilt by fc-cache", "**/.fontconfig", d),
        rule("Adobe", "Font cache", "rebuilt by Adobe applications", "**/AdobeFnt*.lst", f),
        rule("Adobe", "Font cache", "rebuilt by Adobe applications", "**/Adobe/TypeSupport/CT Font Cache", d),
    ]
}

/// Unused fonts: Font families and font caches you may not need
#[derive(Debug, Default, Clone, Copy)]
pub struct UnusedFonts;

impl Detector for UnusedFonts {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::UnusedFonts,
            title: "Unused fonts".into(),
            description: "Font families and font caches you may not need".into(),
            category: Category::Optimise,
            safety: SafetyClass::ReviewOnly,
            default_action: ActionKind::None,
            available_actions: vec![ActionKind::None],
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            feature_ids: vec!["OPT-FNT-02".into()],
            unit: "fonts".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let set = RuleSet::new(font_cache_rules());
        let mut out = rules::run_rules(ctx, &set, DetectorId::UnusedFonts, Vec::new(), true, 0.0)?;
        out.diagnostics
            .notes
            .push("Unused font families cannot be determined offline, so only font cache locations are listed.".into());
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::*;
    use mj_core::testutil::TempDir;

    #[test]
    fn lists_font_caches_review_only_and_documents_the_limitation() {
        let t = TempDir::new("fonts").unwrap();
        for p in [
            "u/.cache/fontconfig/abc-le64.cache-7",
            "u/.fonts/MyFont.ttf",
            "u/.local/share/fonts/Other.otf",
            "w/Windows/System32/FNTCACHE.DAT",
            "a/AdobeFnt20.lst",
        ] {
            t.write(p, b"cache").unwrap();
        }
        let out = run(&UnusedFonts, &index_of(t.path()), &Default::default());
        assert_eq!(
            rel(&out, t.path()),
            vec!["a/AdobeFnt20.lst", "u/.cache/fontconfig", "w/Windows/System32/FNTCACHE.DAT"]
        );
        assert!(out.candidates.iter().all(|c| !c.default_selected), "installed fonts are never listed");
        assert!(out.diagnostics.notes[0].contains("cannot be determined offline"));
    }
}
