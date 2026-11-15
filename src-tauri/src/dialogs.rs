//! Native file dialogs: the shell commands of `mj_engine::api::SHELL_COMMANDS`
//! (`dialog_pick_folder`, `dialog_save_file`). Decision record: `docs/decisions/W6-A.md`.
//!
//! The dialogs are the operating system's own (Windows: `IFileOpenDialog` /
//! `IFileSaveDialog`; Linux: the GTK file chooser), shown through the `rfd` crate
//! from Rust. The webview gets no dialog or filesystem permission: it can only ask
//! for "a folder" or "a file name to save to" and receives a path (or `null` on
//! cancel). A dialog chooses a path and nothing else; whatever is done with the
//! path goes through an engine command with its own checks.
//!
//! Threading: the command hops to the main thread only to read the owner window's
//! handle and to start rfd's async dialog, which returns a future at once; the
//! command then awaits it, so neither the main thread nor the async runtime is
//! blocked while the dialog is open. On Windows rfd shows the dialog from the thread
//! that polls it. On Linux (rfd 0.17.2, `gtk3` feature) rfd does not use the calling
//! thread for GTK: it spawns its own thread that calls `gtk_init_check` and loops on
//! `gtk_main_iteration`, and the dialog is built and shown from an idle source on the
//! default GLib main context. Tauri's (tao's) main thread iterates that same context,
//! so whichever thread is iterating when the source is ready runs it; under Tauri that
//! is normally the main thread, but rfd does not guarantee it. See the decision record.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

use mj_core::ErrorInfo;
use mj_engine::api::{DialogFileFilter, DialogPath, DialogPickFolderRequest, DialogSaveFileRequest};
use tauri::{State, WebviewWindow};

/// Longest dialog title and filter label passed on (characters).
const MAX_LABEL: usize = 120;
/// Longest suggested file name passed on (characters).
const MAX_FILE_NAME: usize = 200;
/// Longest file extension accepted in a filter (characters).
const MAX_EXTENSION: usize = 16;

/// Managed state: at most one native dialog is open at a time. On Windows the
/// dialog disables its owner window, so a second request cannot happen; the GTK
/// dialog does not, and a second dialog on top of the first would be confusing.
#[derive(Default)]
pub struct DialogGate(AtomicBool);

/// Proof that this caller holds the gate; dropping it reopens the gate.
pub struct OpenDialog<'a>(&'a AtomicBool);

impl DialogGate {
    /// Take the gate, or fail with `conflict` while another dialog is open.
    pub fn enter(&self) -> Result<OpenDialog<'_>, ErrorInfo> {
        if self.0.swap(true, Ordering::AcqRel) {
            return Err(ErrorInfo {
                code: "conflict".into(),
                message: "A file dialog is already open".into(),
                hint: "Choose or cancel in the open dialog first.".into(),
                path: None,
            });
        }
        Ok(OpenDialog(&self.0))
    }
}

impl Drop for OpenDialog<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Managed state: the path the last "Save as" dialog returned and no export has
/// used yet. The export commands honour `overwrite: true` only for exactly this
/// path (see [`grant_overwrite`]), so "only the file the user confirmed in the OS
/// dialog is replaced" is enforced here and not left to the webview.
#[derive(Default)]
pub struct SavedDestination(Mutex<Option<PathBuf>>);

impl SavedDestination {
    /// Remember `path` (or forget the last one after a cancelled dialog).
    fn record(&self, path: Option<PathBuf>) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = path;
    }

    /// The `overwrite` flag an export to `dest` may run with.
    pub fn overwrite_for(&self, dest: &Path, requested: bool) -> bool {
        grant_overwrite(&mut self.0.lock().unwrap_or_else(PoisonError::into_inner), dest, requested)
    }
}

/// The decision: a requested overwrite is granted only when `dest` is the recorded
/// dialog path, and granting consumes the record (one confirmation, one replace).
/// Anything else is refused, and the export then writes a free sibling name.
fn grant_overwrite(recorded: &mut Option<PathBuf>, dest: &Path, requested: bool) -> bool {
    if requested && recorded.as_deref() == Some(dest) {
        *recorded = None;
        return true;
    }
    false
}

/// Which dialog to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Folder,
    Save,
}

/// A cleaned request: only values that are safe to hand to the OS dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Spec {
    kind: Kind,
    title: Option<String>,
    start_dir: Option<PathBuf>,
    file_name: Option<String>,
    /// `(label, extensions without the dot)`.
    filters: Vec<(String, Vec<String>)>,
}

impl Spec {
    fn folder(req: DialogPickFolderRequest) -> Self {
        Spec {
            kind: Kind::Folder,
            title: req.title.as_deref().and_then(clean_label),
            start_dir: clean_start_dir(req.start_dir),
            file_name: None,
            filters: Vec::new(),
        }
    }

    fn save(req: DialogSaveFileRequest) -> Self {
        Spec {
            kind: Kind::Save,
            title: req.title.as_deref().and_then(clean_label),
            start_dir: clean_start_dir(req.start_dir),
            file_name: req.file_name.as_deref().and_then(clean_file_name),
            filters: clean_filters(&req.filters),
        }
    }

    /// The dialog, without its owner window.
    fn dialog(&self) -> rfd::AsyncFileDialog {
        let mut d = rfd::AsyncFileDialog::new().set_can_create_directories(true);
        if let Some(title) = &self.title {
            d = d.set_title(title);
        }
        if let Some(dir) = &self.start_dir {
            d = d.set_directory(dir);
        }
        if let Some(name) = &self.file_name {
            d = d.set_file_name(name);
        }
        for (label, extensions) in &self.filters {
            d = d.add_filter(label, extensions);
        }
        d
    }
}

/// A title or label without control characters (a NUL would abort the GTK dialog),
/// trimmed and capped; `None` when nothing is left.
fn clean_label(text: &str) -> Option<String> {
    let cleaned: String = text.chars().filter(|c| !c.is_control()).collect();
    let cleaned: String = cleaned.trim().chars().take(MAX_LABEL).collect();
    (!cleaned.is_empty()).then_some(cleaned)
}

/// The starting folder is a hint: a relative path would be resolved against the
/// process's working directory, so it is dropped. A folder that does not exist is
/// ignored by the OS dialog itself.
fn clean_start_dir(dir: Option<PathBuf>) -> Option<PathBuf> {
    dir.filter(|d| d.is_absolute())
}

/// A suggested file name must be a single, portable name: no path separator, no
/// control character, nothing Windows refuses (`<>:"|?*`, reserved device names,
/// a trailing dot or space). Anything else is dropped, not repaired: the dialog
/// then opens with an empty name.
fn clean_file_name(name: &str) -> Option<String> {
    let name = name.trim();
    let bad = name.is_empty()
        || name == "."
        || name == ".."
        || name.chars().count() > MAX_FILE_NAME
        || name.chars().any(|c| c == '/' || c == '\\' || c.is_control())
        || mj_core::pathutil::has_invalid_windows_name(name);
    (!bad).then(|| name.to_string())
}

/// An extension as the dialogs expect it: no dot, letters and digits only. The
/// Windows dialog builds a `*.ext;*.ext` pattern from these, so nothing that could
/// change the pattern is let through.
fn clean_extension(ext: &str) -> Option<String> {
    let ext = ext.trim();
    let ext = ext.strip_prefix("*.").or_else(|| ext.strip_prefix('.')).unwrap_or(ext);
    let ok = !ext.is_empty() && ext.len() <= MAX_EXTENSION && ext.chars().all(|c| c.is_ascii_alphanumeric());
    ok.then(|| ext.to_string())
}

/// Filters with at least one usable extension; a missing label becomes the
/// upper-cased first extension.
fn clean_filters(filters: &[DialogFileFilter]) -> Vec<(String, Vec<String>)> {
    filters
        .iter()
        .filter_map(|f| {
            let extensions: Vec<String> = f.extensions.iter().filter_map(|e| clean_extension(e)).collect();
            let first = extensions.first()?;
            let label = clean_label(&f.name).unwrap_or_else(|| first.to_uppercase());
            Some((label, extensions))
        })
        .collect()
}

fn internal(message: String) -> ErrorInfo {
    mj_core::Error::Internal(message).info()
}

/// The pending answer of an open dialog.
type Answer = Pin<Box<dyn Future<Output = Option<rfd::FileHandle>> + Send>>;

/// Show the dialog owned by `window` and wait for the user's answer.
async fn show(window: WebviewWindow, spec: Spec) -> Result<DialogPath, ErrorInfo> {
    let (tx, mut rx) = tauri::async_runtime::channel::<Answer>(1);
    let owner = window.clone();
    window
        .run_on_main_thread(move || {
            // Owner window: the dialog is modal to it on Windows (the GTK backend of
            // rfd 0.17 does not use the handle; see the decision record).
            let dialog = spec.dialog().set_parent(&owner);
            let answer: Answer = match spec.kind {
                Kind::Folder => Box::pin(dialog.pick_folder()),
                Kind::Save => Box::pin(dialog.save_file()),
            };
            // The receiver is only gone when the command was dropped: nothing to do.
            let _ = tx.try_send(answer);
        })
        .map_err(|e| internal(format!("the file dialog could not be opened: {e}")))?;
    let answer =
        rx.recv().await.ok_or_else(|| internal("the window closed before the file dialog opened".to_string()))?;
    Ok(DialogPath(answer.await.map(|chosen| chosen.path().to_path_buf())))
}

/// The OS folder dialog. Resolves to the chosen folder, or `null` when cancelled.
#[tauri::command]
pub async fn dialog_pick_folder(
    window: WebviewWindow,
    gate: State<'_, DialogGate>,
    req: DialogPickFolderRequest,
) -> Result<DialogPath, ErrorInfo> {
    let _open = gate.inner().enter()?;
    show(window, Spec::folder(req)).await
}

/// The OS "Save as" dialog. Resolves to the chosen file path, or `null` when
/// cancelled. Nothing is created: the export command writes the file.
#[tauri::command]
pub async fn dialog_save_file(
    window: WebviewWindow,
    gate: State<'_, DialogGate>,
    saved: State<'_, SavedDestination>,
    req: DialogSaveFileRequest,
) -> Result<DialogPath, ErrorInfo> {
    let _open = gate.inner().enter()?;
    let answer = show(window, Spec::save(req)).await?;
    saved.record(answer.0.clone());
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(name: &str, extensions: &[&str]) -> DialogFileFilter {
        DialogFileFilter { name: name.into(), extensions: extensions.iter().map(|e| e.to_string()).collect() }
    }

    #[test]
    fn overwrite_is_granted_only_for_the_path_the_dialog_returned() {
        let dest = Path::new("/home/me/out.csv");
        let mut rec = Some(dest.to_path_buf());
        assert!(!grant_overwrite(&mut rec, dest, false), "not asked for: not granted");
        assert!(rec.is_some(), "and the confirmation is kept");
        assert!(!grant_overwrite(&mut rec, Path::new("/home/me/other.csv"), true), "another path");
        assert!(!grant_overwrite(&mut rec, Path::new("out.csv"), true), "a relative spelling");
        assert!(rec.is_some(), "a refusal keeps the confirmation");
        assert!(grant_overwrite(&mut rec, dest, true));
        assert!(rec.is_none(), "granting consumes the record");
        assert!(!grant_overwrite(&mut rec, dest, true), "one confirmation replaces once");
        assert!(!grant_overwrite(&mut None, dest, true), "no dialog, no overwrite");
    }

    #[test]
    fn the_state_records_and_forgets() {
        let s = SavedDestination::default();
        let dest = Path::new("/a/b.csv");
        assert!(!s.overwrite_for(dest, true));
        s.record(Some(dest.to_path_buf()));
        assert!(s.overwrite_for(dest, true));
        assert!(!s.overwrite_for(dest, true));
        s.record(Some(dest.to_path_buf()));
        s.record(None);
        assert!(!s.overwrite_for(dest, true), "a cancelled dialog forgets the last path");
    }

    #[test]
    fn only_one_dialog_at_a_time() {
        let gate = DialogGate::default();
        let open = gate.enter().expect("the first dialog opens");
        let second = gate.enter().err().expect("a second dialog is refused while one is open");
        assert_eq!(second.code, "conflict");
        assert!(!second.hint.is_empty());
        drop(open);
        assert!(gate.enter().is_ok(), "closing the dialog reopens the gate");
        assert!(gate.enter().is_ok(), "and a finished one never blocks the next");
    }

    #[test]
    fn labels_lose_control_characters_and_are_capped() {
        assert_eq!(clean_label("  Add a location  ").as_deref(), Some("Add a location"));
        // A NUL in the title would make the GTK backend panic inside a C callback.
        assert_eq!(clean_label("Sa\0ve\r\n as\u{7}").as_deref(), Some("Save as"));
        assert_eq!(clean_label(" \0\n "), None);
        assert_eq!(clean_label(&"x".repeat(500)).unwrap().chars().count(), MAX_LABEL);
        assert_eq!(clean_label("Résultats 結果").as_deref(), Some("Résultats 結果"));
    }

    #[test]
    fn a_relative_start_folder_is_dropped() {
        assert_eq!(clean_start_dir(None), None);
        assert_eq!(clean_start_dir(Some("Documents".into())), None);
        assert_eq!(clean_start_dir(Some("".into())), None);
        let abs = if cfg!(windows) { PathBuf::from(r"C:\Users\me") } else { PathBuf::from("/home/me") };
        assert_eq!(clean_start_dir(Some(abs.clone())), Some(abs));
    }

    #[test]
    fn a_suggested_file_name_is_one_portable_name() {
        assert_eq!(clean_file_name(" myjanitor-results.csv ").as_deref(), Some("myjanitor-results.csv"));
        assert_eq!(clean_file_name("report (1).html").as_deref(), Some("report (1).html"));
        for bad in [
            "",
            " ",
            ".",
            "..",
            "a/b.csv",
            r"a\b.csv",
            r"C:\x.csv",
            "/etc/passwd",
            "a\0b.csv",
            "a\nb.csv",
            "a:b.csv",
            "what?.csv",
            "CON.csv",
            "name.",
            "x<y>.csv",
        ] {
            assert_eq!(clean_file_name(bad), None, "{bad:?}");
        }
        assert_eq!(clean_file_name(&"n".repeat(MAX_FILE_NAME + 1)), None);
    }

    #[test]
    fn filters_keep_plain_extensions_only() {
        assert_eq!(clean_extension("csv").as_deref(), Some("csv"));
        assert_eq!(clean_extension(".html").as_deref(), Some("html"));
        assert_eq!(clean_extension("*.PDF").as_deref(), Some("PDF"));
        for bad in ["", ".", "*", "*.*", "csv;*.exe", "a b", "a/b", "tar.gz", &"e".repeat(MAX_EXTENSION + 1)] {
            assert_eq!(clean_extension(bad), None, "{bad:?}");
        }
        let cleaned = clean_filters(&[
            filter("CSV file", &["csv"]),
            filter("", &[".html", "htm"]),
            filter("Nothing usable", &["*", ""]),
            filter("Bad\0 label", &["pdf"]),
        ]);
        assert_eq!(
            cleaned,
            vec![
                ("CSV file".to_string(), vec!["csv".to_string()]),
                ("HTML".to_string(), vec!["html".to_string(), "htm".to_string()]),
                ("Bad label".to_string(), vec!["pdf".to_string()]),
            ]
        );
    }

    #[test]
    fn requests_become_clean_specs() {
        let spec =
            Spec::folder(DialogPickFolderRequest { title: Some("Pick\0".into()), start_dir: Some("rel".into()) });
        assert_eq!(
            spec,
            Spec { kind: Kind::Folder, title: Some("Pick".into()), start_dir: None, file_name: None, filters: vec![] }
        );
        let spec = Spec::save(DialogSaveFileRequest {
            title: None,
            start_dir: None,
            file_name: Some("../../evil.csv".into()),
            filters: vec![filter("CSV file", &["csv"])],
        });
        assert_eq!(spec.kind, Kind::Save);
        assert_eq!(spec.file_name, None, "a path is not a file name");
        assert_eq!(spec.filters.len(), 1);
        let empty = Spec::save(DialogSaveFileRequest::default());
        assert_eq!((empty.title, empty.start_dir, empty.file_name, empty.filters.len()), (None, None, None, 0));
    }
}
