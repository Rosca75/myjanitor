//! App caches and junk (CLN-DET-11): a built-in cross-platform rule table (browser caches, Electron
//! app caches, thumbnail caches, shader caches, crash dumps, rotated logs, Windows Update
//! leftovers, temp folders, Linux `~/.cache` entries) resolved against the paths of the scan index
//! and grouped per application, plus optional user-supplied `winapp2.ini` rules (`winapp2`).
//!
//! The detector is offline and read-only: it never ships or downloads third-party rule files, and
//! the `winapp2_path` setting only names a file the *user* already has.
//! Owner: W2-D.

use mj_core::{
    ActionKind, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, PlatformSupport, Result,
    SafetyClass,
};

use crate::common;
use crate::rules::{self, rule, RuleDef, RuleSet, Target};
use crate::winapp2;

/// Largest winapp2.ini the detector will read.
const MAX_WINAPP2_BYTES: u64 = 16 << 20;

/// Chromium-family browsers: (name, Windows folder under `AppData/Local`, Linux folder name).
const CHROMIUM: &[(&str, &str, &str)] = &[
    ("Chrome", "Google/Chrome", "google-chrome"),
    ("Chromium", "Chromium", "chromium"),
    ("Edge", "Microsoft/Edge", "microsoft-edge"),
    ("Brave", "BraveSoftware/Brave-Browser", "BraveSoftware/Brave-Browser"),
    ("Vivaldi", "Vivaldi", "vivaldi"),
];
/// Cache folders inside a Chromium profile.
const PROFILE_CACHES: &[&str] = &["Cache", "Code Cache", "GPUCache", "DawnCache", "Service Worker/CacheStorage"];
/// Electron apps: (name, folder under `AppData/Roaming` and `~/.config`).
const ELECTRON: &[(&str, &str)] = &[
    ("Slack", "Slack"),
    ("Discord", "discord"),
    ("Visual Studio Code", "Code"),
    ("Microsoft Teams (classic)", "Microsoft/Teams"),
];
const ELECTRON_CACHES: &[&str] = &["Cache", "Code Cache", "GPUCache", "CachedData", "Service Worker/CacheStorage"];

/// The built-in table. Order is priority: specific rules first, the generic `~/.cache/*` last.
pub fn builtin_rules() -> Vec<RuleDef> {
    let d = Target::Dir;
    let mut v: Vec<RuleDef> = Vec::new();
    for (app, win, lin) in CHROMIUM {
        for sub in PROFILE_CACHES {
            v.push(rule(
                app,
                "Browser cache",
                "rebuilt as you browse",
                format!("**/AppData/Local/{win}/User Data/*/{sub}"),
                d,
            ));
            v.push(rule(app, "Browser cache", "rebuilt as you browse", format!("**/.config/{lin}/*/{sub}"), d));
            v.push(rule(app, "Browser cache", "rebuilt as you browse", format!("**/.cache/{lin}/*/{sub}"), d));
        }
        v.push(rule(
            app,
            "Shader cache",
            "rebuilt on demand",
            format!("**/AppData/Local/{win}/User Data/ShaderCache"),
            d,
        ));
        v.push(rule(
            app,
            "Shader cache",
            "rebuilt on demand",
            format!("**/AppData/Local/{win}/User Data/GrShaderCache"),
            d,
        ));
        v.push(rule(app, "Browser cache", "rebuilt as you browse", format!("**/.cache/{lin}"), d));
    }
    for base in ["AppData/Local/Mozilla/Firefox/Profiles", ".cache/mozilla/firefox", ".mozilla/firefox"] {
        for sub in ["cache2", "startupCache", "thumbnails", "jumpListCache"] {
            v.push(rule("Firefox", "Browser cache", "rebuilt as you browse", format!("**/{base}/*/{sub}"), d));
        }
    }
    v.push(rule(
        "Internet cache",
        "Legacy browser cache",
        "rebuilt on demand",
        "**/AppData/Local/Microsoft/Windows/INetCache",
        d,
    ));
    for (app, dir) in ELECTRON {
        for sub in ELECTRON_CACHES {
            v.push(rule(app, "App cache", "rebuilt on next start", format!("**/AppData/Roaming/{dir}/{sub}"), d));
            v.push(rule(app, "App cache", "rebuilt on next start", format!("**/.config/{dir}/{sub}"), d));
        }
    }
    // Thumbnails and icon caches.
    for pat in ["thumbcache_*.db", "iconcache_*.db"] {
        v.push(rule(
            "Windows Explorer",
            "Thumbnail cache",
            "rebuilt when folders are opened",
            format!("**/AppData/Local/Microsoft/Windows/Explorer/{pat}"),
            Target::File,
        ));
    }
    v.push(rule(
        "Windows Explorer",
        "Icon cache",
        "rebuilt automatically",
        "**/AppData/Local/IconCache.db",
        Target::File,
    ));
    v.push(rule("Thumbnails", "Thumbnail cache", "rebuilt when folders are opened", "**/.cache/thumbnails", d));
    v.push(rule("Thumbnails", "Thumbnail cache", "rebuilt when folders are opened", "**/.thumbnails", d));
    // GPU shader caches.
    for (app, pat) in [
        ("NVIDIA", "**/AppData/Local/NVIDIA/DXCache"),
        ("NVIDIA", "**/AppData/Local/NVIDIA/GLCache"),
        ("NVIDIA", "**/.nv/GLCache"),
        ("AMD", "**/AppData/Local/AMD/DxCache"),
        ("DirectX", "**/AppData/Local/D3DSCache"),
        ("Mesa", "**/.cache/mesa_shader_cache"),
    ] {
        v.push(rule(app, "Shader cache", "rebuilt on demand", pat, d));
    }
    // Crash dumps and error reports.
    v.push(rule("Windows", "Crash dumps", "only useful for debugging", "**/AppData/Local/CrashDumps", d));
    v.push(rule("Windows", "Crash dumps", "only useful for debugging", "**/Windows/Minidump", d).review());
    v.push(rule("Windows", "Memory dump", "only useful for debugging", "**/Windows/MEMORY.DMP", Target::File).review());
    for sub in ["ReportArchive", "ReportQueue"] {
        v.push(rule(
            "Windows Error Reporting",
            "Error reports",
            "only useful for debugging",
            format!("**/Microsoft/Windows/WER/{sub}"),
            d,
        ));
    }
    v.push(rule("System", "Crash reports", "only useful for debugging", "**/var/crash", d).review());
    v.push(rule("System", "Core dumps", "only useful for debugging", "**/var/lib/systemd/coredump", d).review());
    // Logs (rotated and compressed only; live logs are left alone).
    for pat in
        ["**/var/log/*.gz", "**/var/log/**/*.gz", "**/var/log/*.[0-9]", "**/var/log/*.old", "**/var/log/**/*.[0-9]"]
    {
        v.push(rule("System logs", "Rotated logs", "already archived by logrotate", pat, Target::File).review());
    }
    v.push(rule("Desktop session", "Old logs", "replaced at every login", "**/.xsession-errors.old", Target::File));
    v.push(rule("Xorg", "Old logs", "replaced at every start", "**/.local/share/xorg/*.log.old", Target::File));
    // Windows Update leftovers (system folders: never pre-selected).
    v.push(
        rule(
            "Windows Update",
            "Update download cache",
            "Windows downloads it again if needed",
            "**/Windows/SoftwareDistribution/Download",
            d,
        )
        .review(),
    );
    v.push(
        rule("Windows Update", "Patch cache", "Windows rebuilds it if needed", "**/Windows/Installer/$PatchCache$", d)
            .review(),
    );
    for pat in ["**/$WINDOWS.~BT", "**/$WINDOWS.~WS", "**/$GetCurrent"] {
        v.push(rule("Windows Update", "Setup leftovers", "left by an upgrade", pat, d).review());
    }
    v.push(
        rule(
            "Windows Update",
            "Previous Windows installation",
            "removing it disables rolling back",
            "**/Windows.old",
            d,
        )
        .review(),
    );
    // Temp folders: children only, never the folder itself, and never anything touched recently.
    v.push(
        rule(
            "Temp files",
            "Temporary file",
            "programs recreate what they need",
            "**/AppData/Local/Temp/*",
            Target::Any,
        )
        .aged(),
    );
    v.push(
        rule("Windows temp", "Temporary file", "programs recreate what they need", "**/Windows/Temp/*", Target::Any)
            .aged()
            .review(),
    );
    v.push(
        rule("Temp files", "Temporary file", "programs recreate what they need", "/tmp/*", Target::Any)
            .aged()
            .review()
            .runtime_tmp(),
    );
    v.push(
        rule("Temp files", "Temporary file", "programs recreate what they need", "/var/tmp/*", Target::Any)
            .aged()
            .review()
            .runtime_tmp(),
    );
    // Known per-user package caches.
    v.push(rule("pip", "Package cache", "downloaded again when needed", "**/.cache/pip", d));
    v.push(rule("Yarn", "Package cache", "downloaded again when needed", "**/.cache/yarn", d));
    v.push(rule("npm", "Package cache", "downloaded again when needed", "**/.npm/_cacache", d));
    v.push(rule("npm", "Package cache", "downloaded again when needed", "**/AppData/Local/npm-cache", d));
    // Every other direct child of `~/.cache` is some program's cache; unknown programs stay opt-in.
    v.push(rule("", "Application cache", "the app recreates it", "**/.cache/*", d).review());
    v
}

/// App caches & junk: Browsers, Teams, crash dumps, logs
#[derive(Debug, Default, Clone, Copy)]
pub struct AppCaches;

impl Detector for AppCaches {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::AppCaches,
            title: "App caches & junk".into(),
            description: "Browsers, Teams, crash dumps, logs".into(),
            category: Category::Clean,
            safety: SafetyClass::Rebuildable,
            default_action: ActionKind::Quarantine,
            available_actions: vec![ActionKind::Quarantine, ActionKind::Trash, ActionKind::PermanentDelete],
            settings: vec![
                common::number_setting(
                    "min_age_hours",
                    "Ignore temp files changed within",
                    "Programs may still be using recently changed temporary files.",
                    24.0,
                    0.0,
                    8760.0,
                    "hours",
                ),
                common::text_setting(
                    "winapp2_path",
                    "winapp2.ini rules",
                    "Optional path of your own winapp2.ini; its file rules are added to the built-in list.",
                    "",
                    true,
                ),
            ],
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-11".into()],
            unit: "apps".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let mut diag = mj_core::Diagnostics::default();
        let extra = load_winapp2(ctx, &mut diag);
        let set = RuleSet::new(builtin_rules());
        let hours = ctx.params.number("min_age_hours", 24.0);
        let mut out = rules::run_rules(ctx, &set, DetectorId::AppCaches, extra, false, hours)?;
        out.diagnostics.merge(diag);
        Ok(out)
    }
}

/// Hits from the user's winapp2.ini, when the setting names a readable file.
fn load_winapp2(ctx: &DetectContext<'_>, diag: &mut mj_core::Diagnostics) -> Vec<rules::Hit> {
    let path = ctx.params.text("winapp2_path", "").trim();
    if path.is_empty() {
        return Vec::new();
    }
    let text = match std::fs::metadata(path) {
        Ok(m) if m.len() <= MAX_WINAPP2_BYTES => std::fs::read(path).map(|b| String::from_utf8_lossy(&b).into_owned()),
        Ok(_) => {
            diag.notes.push("winapp2.ini ignored: the file is larger than 16 MB".into());
            return Vec::new();
        }
        Err(e) => Err(e),
    };
    match text {
        Ok(t) => winapp2::resolve(ctx.index, &winapp2::parse(&t), &winapp2::process_env(), diag),
        Err(_) => {
            diag.notes.push(format!("winapp2.ini could not be read from {path}; its rules were not applied"));
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::*;
    use mj_core::testutil::TempDir;

    fn fixture() -> TempDir {
        let t = TempDir::new("caches").unwrap();
        let w = |p: &str| {
            t.write(p, b"cachedata").unwrap();
        };
        w("u/AppData/Local/Google/Chrome/User Data/Default/Cache/f_000001");
        w("u/AppData/Local/Google/Chrome/User Data/Default/Cache/f_000002");
        w("u/AppData/Local/Google/Chrome/User Data/Default/Cookies"); // never touched
        w("u/AppData/Local/Microsoft/Edge/User Data/Profile 1/Code Cache/js/a");
        w("u/AppData/Local/Microsoft/Windows/Explorer/thumbcache_256.db");
        w("u/AppData/Local/Microsoft/Windows/Explorer/other.db");
        w("u/AppData/Local/CrashDumps/app.dmp");
        w("u/AppData/Local/Temp/old.bin");
        w("u/AppData/Local/Temp/fresh.bin");
        w("u/.cache/google-chrome/Default/Cache/x");
        w("u/.cache/mozilla/firefox/abc.default/cache2/entries/e1");
        w("u/.cache/somecustomtool/data");
        w("u/.cache/thumbnails/large/t.png");
        w("var/log/syslog");
        w("var/log/syslog.1");
        w("var/log/auth.log.2.gz");
        w("u/Documents/Cache/mine.txt"); // a user folder merely named Cache
        age(&t.join("u/AppData/Local/Temp/old.bin"), 3);
        t
    }

    #[test]
    fn built_in_rules_group_per_application_and_leave_user_data_alone() {
        let t = fixture();
        let out = run(&AppCaches, &index_of(t.path()), &Default::default());
        let r = rel(&out, t.path());
        for want in [
            "u/AppData/Local/Google/Chrome/User Data/Default/Cache",
            "u/AppData/Local/Microsoft/Edge/User Data/Profile 1/Code Cache",
            "u/AppData/Local/Microsoft/Windows/Explorer/thumbcache_256.db",
            "u/AppData/Local/CrashDumps",
            "u/AppData/Local/Temp/old.bin",
            "u/.cache/google-chrome",
            "u/.cache/mozilla/firefox/abc.default/cache2",
            "u/.cache/somecustomtool",
            "u/.cache/thumbnails",
            "var/log/syslog.1",
            "var/log/auth.log.2.gz",
        ] {
            assert!(r.contains(&want.to_string()), "missing {want}: {r:?}");
        }
        for not in [
            "u/AppData/Local/Temp/fresh.bin",
            "var/log/syslog",
            "u/Documents/Cache",
            "u/AppData/Local/Microsoft/Windows/Explorer/other.db",
        ] {
            assert!(!r.contains(&not.to_string()), "unexpected {not}");
        }
        // Nested: the Chrome `.cache/google-chrome/Default/Cache` folder is inside the topmost rule.
        assert!(!r.iter().any(|p| p.starts_with("u/.cache/google-chrome/")));
        let chrome =
            out.candidates.iter().find(|c| c.path.ends_with("Default/Cache") && c.columns["app"] == "Chrome").unwrap();
        assert_eq!(chrome.sub_group.as_deref(), Some("chrome"));
        assert_eq!(chrome.file_count, Some(2));
        assert!(chrome.size >= 18);
        assert!(chrome.default_selected && chrome.badges.contains(mj_core::Badges::REBUILDABLE));
        // Unknown ~/.cache tools and system logs are opt-in.
        let custom = out.candidates.iter().find(|c| c.path.ends_with("somecustomtool")).unwrap();
        assert_eq!(custom.columns["app"], "somecustomtool");
        assert!(!custom.default_selected);
        assert_eq!(out.diagnostics.skipped["recently_modified"], 1);
    }

    #[test]
    fn user_supplied_winapp2_rules_are_added() {
        let t = TempDir::new("caches_w2").unwrap();
        t.write("app/cache/a.bin", b"12345").unwrap();
        t.write("app/keep/b.bin", b"12345").unwrap();
        let ini = t.write(
            "rules.ini",
            format!("[Demo App *]\nFileKey1={}|*.bin|RECURSE\n", t.join("app/cache").to_string_lossy()).as_bytes(),
        );
        let ix = index_of(t.path());
        let mut params = mj_core::ParamValues::new();
        params.set("winapp2_path", mj_core::ParamValue::Text(ini.unwrap().to_string_lossy().into_owned()));
        let out = run(&AppCaches, &ix, &params);
        assert_eq!(rel(&out, t.path()), vec!["app/cache/a.bin"]);
        assert_eq!(out.candidates[0].columns["app"], "Demo App");
        // Missing file: noted, not fatal.
        let mut bad = mj_core::ParamValues::new();
        bad.set("winapp2_path", mj_core::ParamValue::Text(t.join("nope.ini").to_string_lossy().into_owned()));
        let out = run(&AppCaches, &ix, &bad);
        assert!(out.candidates.is_empty() && out.diagnostics.notes.len() == 1);
    }

    #[test]
    fn linux_runtime_tmp_entries_are_skipped() {
        let set = RuleSet::new(builtin_rules());
        let e = mj_core::FileEntry::new("/tmp/systemd-private-123", mj_core::EntryKind::Dir, 0);
        assert!(set.find(&e).is_some_and(|r| r.runtime_tmp));
    }
}
