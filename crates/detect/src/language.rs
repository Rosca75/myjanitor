//! Unused language packs and localisations (CLN-DET-16), review only.
//!
//! Recognised layouts: `<code>.lproj` folders, `locale/<code>` and `locales/<code>` folders, and
//! `<code>.pak` files inside a `locales` folder or named `lang-<code>.pak`. Everything whose
//! language is not in the kept list is listed; the kept list defaults to the system language
//! (from `LC_ALL`/`LC_MESSAGES`/`LANG`) plus English and can be set with `kept_languages`.
//! A folder name only counts when it parses as a real ISO 639-1 language code, so `locale/data`
//! or `locales/backup` are never mistaken for a localisation.
//! Owner: W2-D.

use mj_core::{
    ActionKind, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, EntryKind, FileEntry,
    PlatformSupport, Result, SafetyClass,
};

use crate::common::{self, Ticker};

/// ISO 639-1 codes (lower case).
const ISO_639_1: &[&str] = &[
    "aa", "ab", "af", "ak", "am", "an", "ar", "as", "av", "ay", "az", "ba", "be", "bg", "bh", "bi", "bm", "bn", "bo",
    "br", "bs", "ca", "ce", "ch", "co", "cr", "cs", "cu", "cv", "cy", "da", "de", "dv", "dz", "ee", "el", "en", "eo",
    "es", "et", "eu", "fa", "ff", "fi", "fj", "fo", "fr", "fy", "ga", "gd", "gl", "gn", "gu", "gv", "ha", "he", "hi",
    "ho", "hr", "ht", "hu", "hy", "hz", "ia", "id", "ie", "ig", "ii", "ik", "io", "is", "it", "iu", "ja", "jv", "ka",
    "kg", "ki", "kj", "kk", "kl", "km", "kn", "ko", "kr", "ks", "ku", "kv", "kw", "ky", "la", "lb", "lg", "li", "ln",
    "lo", "lt", "lu", "lv", "mg", "mh", "mi", "mk", "ml", "mn", "mr", "ms", "mt", "my", "na", "nb", "nd", "ne", "ng",
    "nl", "nn", "no", "nr", "nv", "ny", "oc", "oj", "om", "or", "os", "pa", "pi", "pl", "ps", "pt", "qu", "rm", "rn",
    "ro", "ru", "rw", "sa", "sc", "sd", "se", "sg", "si", "sk", "sl", "sm", "sn", "so", "sq", "sr", "ss", "st", "su",
    "sv", "sw", "ta", "te", "tg", "th", "ti", "tk", "tl", "tn", "to", "tr", "ts", "tt", "tw", "ty", "ug", "uk", "ur",
    "uz", "ve", "vi", "vo", "wa", "wo", "xh", "yi", "yo", "za", "zh", "zu",
];

/// Display names for the common languages (others show their code).
const NAMES: &[(&str, &str)] = &[
    ("ar", "Arabic"),
    ("bg", "Bulgarian"),
    ("ca", "Catalan"),
    ("cs", "Czech"),
    ("da", "Danish"),
    ("de", "German"),
    ("el", "Greek"),
    ("en", "English"),
    ("es", "Spanish"),
    ("et", "Estonian"),
    ("fi", "Finnish"),
    ("fr", "French"),
    ("he", "Hebrew"),
    ("hi", "Hindi"),
    ("hr", "Croatian"),
    ("hu", "Hungarian"),
    ("id", "Indonesian"),
    ("it", "Italian"),
    ("ja", "Japanese"),
    ("ko", "Korean"),
    ("lt", "Lithuanian"),
    ("lv", "Latvian"),
    ("nb", "Norwegian"),
    ("nl", "Dutch"),
    ("no", "Norwegian"),
    ("pl", "Polish"),
    ("pt", "Portuguese"),
    ("ro", "Romanian"),
    ("ru", "Russian"),
    ("sk", "Slovak"),
    ("sl", "Slovenian"),
    ("sr", "Serbian"),
    ("sv", "Swedish"),
    ("th", "Thai"),
    ("tr", "Turkish"),
    ("uk", "Ukrainian"),
    ("vi", "Vietnamese"),
    ("zh", "Chinese"),
];

/// Parse a locale folder/file stem (`fr`, `pt_BR`, `zh-Hans-CN`, `en_US.UTF-8`) into its language code.
pub fn language_code(raw: &str) -> Option<String> {
    let base = raw.split('.').next().unwrap_or(raw);
    let mut parts = base.split(['-', '_']);
    let lang = parts.next()?.to_ascii_lowercase();
    if !ISO_639_1.contains(&lang.as_str()) {
        return None;
    }
    // Remaining parts must look like a script (4 letters) or region (2 letters / 3 digits).
    let ok = parts.all(|p| {
        (p.len() == 2 && p.chars().all(|c| c.is_ascii_alphabetic()))
            || (p.len() == 3 && p.chars().all(|c| c.is_ascii_digit()))
            || (p.len() == 4 && p.chars().all(|c| c.is_ascii_alphabetic()))
    });
    ok.then_some(lang)
}

/// Codes to keep: the `kept_languages` setting, else the system language; English is always kept.
pub fn kept_languages(setting: &str) -> Vec<String> {
    let mut kept: Vec<String> = common::split_list(setting).iter().filter_map(|s| language_code(s)).collect();
    if kept.is_empty() {
        let sys =
            ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()));
        kept.extend(sys.as_deref().and_then(language_code));
    }
    kept.push("en".into());
    kept.sort();
    kept.dedup();
    kept
}

fn language_name(code: &str) -> String {
    NAMES.iter().find(|(c, _)| *c == code).map(|(_, n)| n.to_string()).unwrap_or_else(|| code.to_uppercase())
}

/// The language a locale entry belongs to, when it is one.
fn locale_of(e: &FileEntry) -> Option<String> {
    let name = e.path.file_name()?.to_string_lossy().into_owned();
    let parent =
        e.path.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let lower = name.to_lowercase();
    match e.kind {
        EntryKind::Dir => {
            if let Some(code) = lower.strip_suffix(".lproj") {
                return language_code(code);
            }
            matches!(parent.as_str(), "locale" | "locales").then(|| language_code(&name)).flatten()
        }
        EntryKind::File => {
            let stem = lower.strip_suffix(".pak")?;
            if let Some(code) = stem.strip_prefix("lang-").or_else(|| stem.strip_prefix("lang_")) {
                return language_code(code);
            }
            (parent == "locales").then(|| language_code(stem)).flatten()
        }
        _ => None,
    }
}

/// Unused language packs: Localisation files for languages you do not use
#[derive(Debug, Default, Clone, Copy)]
pub struct LanguagePacks;

impl Detector for LanguagePacks {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::LanguagePacks,
            title: "Language packs".into(),
            description: "Localisation files for languages you do not use".into(),
            category: Category::Clean,
            safety: SafetyClass::ReviewOnly,
            default_action: ActionKind::None,
            available_actions: vec![ActionKind::None, ActionKind::Quarantine, ActionKind::Trash],
            settings: vec![common::text_setting(
                "kept_languages",
                "Languages to keep",
                "Comma-separated language codes (en, fr, pt-BR). Empty = your system language. English is always kept.",
                "",
                false,
            )],
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-16".into()],
            unit: "files".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let kept = kept_languages(ctx.params.text("kept_languages", ""));
        let ix = ctx.index;
        let mut out = DetectorOutput::default();
        let mut tick = Ticker::new(ctx, ix.len() as u64);
        let mut hits: Vec<(usize, String)> = Vec::new();
        for (pos, e) in ix.entries().iter().enumerate() {
            tick.tick()?;
            if !common::usable(e) {
                continue;
            }
            if let Some(code) = locale_of(e).filter(|c| !kept.contains(c)) {
                hits.push((pos, code));
            }
        }
        let positions: Vec<usize> = hits.iter().map(|h| h.0).collect();
        for pos in common::topmost(ix, &positions) {
            let code = hits.iter().find(|h| h.0 == pos).map(|h| h.1.clone()).unwrap_or_default();
            let e = ix.entry(pos);
            if e.kind == EntryKind::Dir && ix.totals(pos).files == 0 {
                continue;
            }
            let reason = format!("{} localisation ({code}); you keep {}", language_name(&code), kept.join(", "));
            let mut c = if e.kind == EntryKind::Dir {
                common::folder_candidate(ix, pos, DetectorId::LanguagePacks, reason, false)
            } else {
                common::file_candidate(e, DetectorId::LanguagePacks, reason, false)
            };
            c.default_selected = false;
            c.sub_group = Some(code.clone());
            c.columns.insert("language".into(), language_name(&code));
            tick.found(c.size);
            out.candidates.push(c);
        }
        tick.finish();
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::*;
    use mj_core::testutil::TempDir;

    #[test]
    fn language_code_parsing() {
        for (s, want) in [
            ("fr", Some("fr")),
            ("pt_BR", Some("pt")),
            ("zh-Hans-CN", Some("zh")),
            ("en_US.UTF-8", Some("en")),
            ("DE", Some("de")),
            ("data", None),
            ("backup", None),
            ("xx", None),
            ("fr-FRANCE", None),
            ("", None),
        ] {
            assert_eq!(language_code(s).as_deref(), want, "{s}");
        }
    }

    #[test]
    fn kept_list_always_contains_english() {
        assert_eq!(kept_languages("fr, de-AT"), vec!["de", "en", "fr"]);
        assert!(kept_languages("").contains(&"en".to_string()));
    }

    fn fixture() -> TempDir {
        let t = TempDir::new("lang").unwrap();
        for p in [
            "app/locale/en/LC_MESSAGES/app.mo",
            "app/locale/fr/LC_MESSAGES/app.mo",
            "app/locale/ja/LC_MESSAGES/app.mo",
            "app/locale/data/keep.txt",
            "App.app/Resources/en.lproj/Localizable.strings",
            "App.app/Resources/de.lproj/Localizable.strings",
            "App.app/Resources/Base.lproj/Main.nib",
            "chrome/locales/en-US.pak",
            "chrome/locales/fr.pak",
            "chrome/locales/pt-BR.pak",
            "chrome/lang-es.pak",
            "chrome/resources.pak",
            "docs/fr/page.md", // a folder named fr outside a locale folder
        ] {
            t.write(p, b"strings").unwrap();
        }
        t
    }

    #[test]
    fn lists_languages_outside_the_kept_set_review_only() {
        let t = fixture();
        let mut params = mj_core::ParamValues::new();
        params.set("kept_languages", mj_core::ParamValue::Text("fr".into()));
        let out = run(&LanguagePacks, &index_of(t.path()), &params);
        assert_eq!(
            rel(&out, t.path()),
            vec!["App.app/Resources/de.lproj", "app/locale/ja", "chrome/lang-es.pak", "chrome/locales/pt-BR.pak",]
        );
        assert!(out.candidates.iter().all(|c| !c.default_selected));
        let ja = out.candidates.iter().find(|c| c.path.ends_with("locale/ja")).unwrap();
        assert!(ja.reason.contains("Japanese") && ja.reason.contains("en, fr"));
        assert_eq!(ja.sub_group.as_deref(), Some("ja"));
    }

    #[test]
    fn english_only_by_default_when_nothing_is_kept() {
        let t = fixture();
        // Explicit single-language setting that is not a language falls back to system language + English.
        let mut params = mj_core::ParamValues::new();
        params.set("kept_languages", mj_core::ParamValue::Text("en".into()));
        let out = run(&LanguagePacks, &index_of(t.path()), &params);
        let r = rel(&out, t.path());
        assert!(r.contains(&"app/locale/fr".to_string()) && r.contains(&"chrome/locales/fr.pak".to_string()));
        assert!(!r.iter().any(|p| p.contains("en.lproj")
            || p.contains("en-US")
            || p.contains("Base.lproj")
            || p.contains("locale/data")));
        assert!(!r.contains(&"docs/fr".to_string()));
    }
}
