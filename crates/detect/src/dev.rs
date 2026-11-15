//! Developer artefacts (CLN-DET-13): `node_modules`, Python virtual environments and caches, Rust
//! `target/`, .NET `bin`/`obj`, Gradle output, JS build caches. Reported as folder candidates,
//! topmost only (an artefact inside another is never counted twice), `Rebuildable`, with the
//! command that recreates them.
//!
//! Generic folder names are only reported next to the file that proves they are build output:
//! `target/` needs a sibling `Cargo.toml`, `bin`/`obj` a project or solution file, `build` a
//! Gradle script, `dist` a package manifest, a virtual environment its own `pyvenv.cfg`.
//! Owner: W2-D.

use std::collections::HashMap;
use std::path::Path;

use mj_core::pathutil::path_key;
use mj_core::{
    ActionKind, Category, DetectContext, Detector, DetectorId, DetectorInfo, DetectorOutput, EntryKind, FileIndex,
    PlatformSupport, Result, SafetyClass,
};

use crate::common::{self, Ticker};

const CARGO: u32 = 1;
const DOTNET: u32 = 1 << 1;
const GRADLE: u32 = 1 << 2;
const NODE: u32 = 1 << 3;
const PYTHON: u32 = 1 << 4;
const PNPM: u32 = 1 << 5;
const YARN: u32 = 1 << 6;

/// Which manifest files sit in one folder.
fn marker_bits(name: &str) -> u32 {
    match name {
        "cargo.toml" => CARGO,
        "build.gradle" | "build.gradle.kts" | "settings.gradle" | "settings.gradle.kts" => GRADLE,
        "package.json" => NODE,
        "pyproject.toml" | "setup.py" | "setup.cfg" => PYTHON,
        "pnpm-lock.yaml" => NODE | PNPM,
        "yarn.lock" => NODE | YARN,
        n if [".csproj", ".fsproj", ".vbproj", ".sln", ".slnx"].iter().any(|e| n.ends_with(e)) => DOTNET,
        _ => 0,
    }
}

/// A recognised artefact folder name.
struct Artefact {
    name: &'static str,
    /// Any one of these markers must sit beside it (0 = none needed).
    needs: u32,
    /// Human label of what it is.
    what: &'static str,
    hint: &'static str,
}

const ARTEFACTS: &[Artefact] = &[
    Artefact { name: "node_modules", needs: 0, what: "Node.js packages", hint: "rebuild with npm install" },
    Artefact {
        name: ".venv",
        needs: 0,
        what: "Python virtual environment",
        hint: "recreate with python -m venv and pip install -r requirements.txt",
    },
    Artefact {
        name: "venv",
        needs: 0,
        what: "Python virtual environment",
        hint: "recreate with python -m venv and pip install -r requirements.txt",
    },
    Artefact { name: "target", needs: CARGO, what: "Rust build output", hint: "rebuild with cargo build" },
    Artefact { name: "bin", needs: DOTNET, what: ".NET build output", hint: "rebuild with dotnet build" },
    Artefact { name: "obj", needs: DOTNET, what: ".NET build output", hint: "rebuild with dotnet build" },
    Artefact {
        name: "__pycache__",
        needs: 0,
        what: "Python bytecode cache",
        hint: "Python recreates it on the next run",
    },
    Artefact { name: ".gradle", needs: GRADLE, what: "Gradle project cache", hint: "rebuild with gradle build" },
    Artefact { name: "build", needs: GRADLE, what: "Gradle build output", hint: "rebuild with gradle build" },
    Artefact { name: ".pytest_cache", needs: 0, what: "pytest cache", hint: "pytest recreates it" },
    Artefact { name: ".mypy_cache", needs: 0, what: "mypy cache", hint: "mypy recreates it" },
    Artefact { name: ".ruff_cache", needs: 0, what: "Ruff cache", hint: "Ruff recreates it" },
    Artefact { name: ".tox", needs: 0, what: "tox environments", hint: "tox recreates them" },
    Artefact {
        name: "dist",
        needs: NODE | PYTHON,
        what: "Build output",
        hint: "rebuild with your build command (npm run build or python -m build)",
    },
    Artefact { name: ".next", needs: NODE, what: "Next.js build cache", hint: "rebuild with npm run build" },
    Artefact { name: ".nuxt", needs: NODE, what: "Nuxt build cache", hint: "rebuild with npm run build" },
    Artefact { name: ".parcel-cache", needs: NODE, what: "Parcel cache", hint: "Parcel recreates it" },
];

/// Developer artefacts: node_modules, .venv, target/ and other rebuildable folders
#[derive(Debug, Default, Clone, Copy)]
pub struct DevArtefacts;

impl Detector for DevArtefacts {
    fn info(&self) -> DetectorInfo {
        DetectorInfo {
            id: DetectorId::DevArtefacts,
            title: "Developer artefacts".into(),
            description: "node_modules, .venv, target/ and other rebuildable folders".into(),
            category: Category::Clean,
            safety: SafetyClass::Rebuildable,
            default_action: ActionKind::Trash,
            available_actions: vec![ActionKind::Trash, ActionKind::Quarantine, ActionKind::PermanentDelete],
            settings: Vec::new(),
            platform: PlatformSupport::Both,
            feature_ids: vec!["CLN-DET-13".into()],
            unit: "folders".into(),
        }
    }

    fn run(&self, ctx: &DetectContext<'_>) -> Result<DetectorOutput> {
        let ix = ctx.index;
        let mut out = DetectorOutput::default();
        let mut tick = Ticker::new(ctx, ix.len() as u64 * 2);
        let markers = collect_markers(ix, &mut tick)?;

        let mut found: Vec<(usize, &Artefact, u32)> = Vec::new();
        for (pos, e) in ix.entries().iter().enumerate() {
            tick.tick()?;
            if e.kind != EntryKind::Dir || !common::usable(e) {
                continue;
            }
            let name = common::name_lower(e);
            let Some(a) = ARTEFACTS.iter().find(|a| a.name == name) else { continue };
            let bits = e.path.parent().and_then(|p| markers.get(&path_key(p))).copied().unwrap_or(0);
            if a.needs != 0 && bits & a.needs == 0 {
                continue; // a generic name without its project file: not ours to propose
            }
            if matches!(a.name, ".venv" | "venv") && ix.get(&e.path.join("pyvenv.cfg")).is_none() {
                continue; // any folder can be called venv; a real one has pyvenv.cfg
            }
            found.push((pos, a, bits));
        }
        let positions: Vec<usize> = found.iter().map(|f| f.0).collect();
        for pos in common::topmost(ix, &positions) {
            let (_, a, bits) = *found.iter().find(|f| f.0 == pos).expect("position came from found");
            if ix.totals(pos).files == 0 {
                out.diagnostics.skip("empty_folder");
                continue;
            }
            let hint = if a.name == "node_modules" && bits & PNPM != 0 {
                "rebuild with pnpm install"
            } else if a.name == "node_modules" && bits & YARN != 0 {
                "rebuild with yarn install"
            } else {
                a.hint
            };
            let reason = format!("{}; {hint}", a.what);
            let mut c = common::folder_candidate(ix, pos, DetectorId::DevArtefacts, reason, true);
            c.badges |= mj_core::Badges::REBUILDABLE;
            c.sub_group = Some(common::slug(a.name));
            c.columns.insert("kind".into(), a.name.into());
            c.columns.insert("rebuild".into(), hint.into());
            tick.found(c.size);
            out.candidates.push(c);
        }
        docker_hints(ix, &mut out);
        tick.finish();
        Ok(out)
    }
}

/// One pass over the files: which manifests does each folder hold?
fn collect_markers(ix: &FileIndex, tick: &mut Ticker<'_, '_>) -> Result<HashMap<String, u32>> {
    let mut m: HashMap<String, u32> = HashMap::new();
    for e in ix.entries() {
        tick.tick()?;
        if !e.is_file() {
            continue;
        }
        let bits = marker_bits(&common::name_lower(e));
        if bits == 0 {
            continue;
        }
        if let Some(parent) = e.path.parent() {
            *m.entry(path_key(parent)).or_default() |= bits;
        }
    }
    Ok(m)
}

/// Docker's build cache lives inside Docker's own data store, which must be pruned with Docker
/// (`docker builder prune`), never by deleting files. It is therefore reported as a note only.
fn docker_hints(ix: &FileIndex, out: &mut DetectorOutput) {
    let is_docker = |p: &Path| common::has_component(p, &["docker", "dockerdesktop", ".docker"]);
    let mut noted = 0;
    for e in ix.entries() {
        let name = common::name_lower(e);
        let hit = (e.kind == EntryKind::Dir && name == "buildkit" && is_docker(&e.path))
            || (e.is_file() && matches!(name.as_str(), "docker_data.vhdx" | "ext4.vhdx") && is_docker(&e.path));
        if hit && noted < 3 {
            noted += 1;
            out.diagnostics.notes.push(format!(
                "Docker data found at {}: reclaim build cache with `docker builder prune` instead of deleting files.",
                e.path.display()
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::*;
    use mj_core::testutil::TempDir;

    fn fixture() -> TempDir {
        let t = TempDir::new("dev").unwrap();
        let w = |p: &str| {
            t.write(p, b"0123456789").unwrap();
        };
        // JS project, with nested artefacts inside node_modules that must not double count.
        w("web/package.json");
        w("web/node_modules/left-pad/index.js");
        w("web/node_modules/left-pad/node_modules/x/index.js");
        w("web/dist/bundle.js");
        w("web/.next/cache/a");
        // Rust project and a lookalike `target` without Cargo.toml.
        w("rs/Cargo.toml");
        w("rs/target/debug/app");
        w("notes/target/plan.txt");
        // .NET.
        w("dn/App.csproj");
        w("dn/bin/Debug/app.dll");
        w("dn/obj/project.assets.json");
        w("lonely/bin/tool.exe"); // bin without a project file
                                  // Python.
        w("py/.venv/pyvenv.cfg");
        w("py/.venv/lib/site.py");
        w("py/venv/notes.txt"); // folder called venv that is not one
        w("py/pkg/__pycache__/m.pyc");
        w("py/.pytest_cache/v");
        // Gradle.
        w("jv/build.gradle");
        w("jv/build/classes/A.class");
        w("jv/.gradle/x");
        w("other/build/out.txt"); // build without gradle files
                                  // pnpm project.
        w("pn/package.json");
        w("pn/pnpm-lock.yaml");
        w("pn/node_modules/a/b.js");
        // Docker.
        w("home/.docker/buildkit/blob");
        t
    }

    #[test]
    fn finds_artefacts_only_where_their_project_file_exists() {
        let t = fixture();
        let out = run(&DevArtefacts, &index_of(t.path()), &Default::default());
        assert_eq!(
            rel(&out, t.path()),
            vec![
                "dn/bin",
                "dn/obj",
                "jv/.gradle",
                "jv/build",
                "pn/node_modules",
                "py/.pytest_cache",
                "py/.venv",
                "py/pkg/__pycache__",
                "rs/target",
                "web/.next",
                "web/dist",
                "web/node_modules",
            ]
        );
        for c in &out.candidates {
            assert!(c.default_selected && c.badges.contains(mj_core::Badges::REBUILDABLE));
            assert_eq!(c.target, mj_core::TargetKind::Folder);
        }
    }

    #[test]
    fn negatives_target_without_cargo_toml_and_nested_folders() {
        let t = fixture();
        let out = run(&DevArtefacts, &index_of(t.path()), &Default::default());
        let r = rel(&out, t.path());
        for not in ["notes/target", "lonely/bin", "py/venv", "other/build", "web/node_modules/left-pad/node_modules"] {
            assert!(!r.contains(&not.to_string()), "{not} must not be reported");
        }
        // node_modules counts its nested package once: 2 files.
        let nm = out.candidates.iter().find(|c| c.path.ends_with("web/node_modules")).unwrap();
        assert_eq!(nm.file_count, Some(2));
        assert!(nm.size >= 20);
    }

    #[test]
    fn rebuild_hints_follow_the_package_manager() {
        let t = fixture();
        let out = run(&DevArtefacts, &index_of(t.path()), &Default::default());
        let hint =
            |suffix: &str| out.candidates.iter().find(|c| c.path.ends_with(suffix)).unwrap().columns["rebuild"].clone();
        assert_eq!(hint("web/node_modules"), "rebuild with npm install");
        assert_eq!(hint("pn/node_modules"), "rebuild with pnpm install");
        assert_eq!(hint("rs/target"), "rebuild with cargo build");
        assert_eq!(hint("dn/obj"), "rebuild with dotnet build");
        let t = out.candidates.iter().find(|c| c.path.ends_with("rs/target")).unwrap();
        assert!(t.reason.contains("rebuild with cargo build"));
        assert_eq!(t.sub_group.as_deref(), Some("target"));
    }

    #[test]
    fn docker_cache_is_a_note_not_a_candidate() {
        let t = fixture();
        let out = run(&DevArtefacts, &index_of(t.path()), &Default::default());
        assert!(out.diagnostics.notes.iter().any(|n| n.contains("docker builder prune")));
        assert!(!rel(&out, t.path()).iter().any(|p| p.contains("buildkit")));
    }
}
