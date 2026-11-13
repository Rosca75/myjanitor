//! The one error model shared by every crate.
//!
//! Engines return [`Result<T>`]. Across the IPC boundary an [`Error`] travels as
//! an [`ErrorInfo`] (`{code, message, hint}`): `code` is a stable snake_case
//! identifier the UI may switch on, `message` says what failed, `hint` says what
//! the user can do next. Per-item failures inside a batch are *not* command
//! errors: they travel inside job outcomes as `ItemOutcome::error`.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Result alias used across the workspace.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Every failure MyJanitor can report. Variants are coarse on purpose: the
/// `code()` is the contract, the text is for humans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// An I/O error on a path. `kind` is a stable code (see [`io_kind_code`]).
    Io { path: Option<PathBuf>, kind: &'static str, message: String },
    /// The path does not exist (any longer).
    NotFound(PathBuf),
    /// The OS refused access to the path.
    PermissionDenied(PathBuf),
    /// The feature cannot work on this platform, filesystem or volume.
    Unsupported { feature: String, reason: String },
    /// The feature is planned but its owner workstream has not implemented it yet.
    NotImplemented(String),
    /// The operation was cancelled by the user.
    Cancelled,
    /// The caller passed an invalid argument (IPC request, CLI flag, rule tree).
    InvalidInput(String),
    /// The path is protected (system path or a user-protected path) and must not be touched.
    Protected(PathBuf),
    /// A destructive action targeted a path outside the caller-supplied root allowlist.
    OutsideAllowedRoots(PathBuf),
    /// The file changed since it was scanned (size or mtime differ); the action was skipped.
    Changed(PathBuf),
    /// The path is a symlink, junction or other reparse point and destructive ops never follow those.
    LinkNotFollowed(PathBuf),
    /// A file or structure could not be parsed.
    Parse { what: String, message: String },
    /// An optional external tool (ffmpeg, 7z, ...) is not available.
    ToolMissing(String),
    /// The request conflicts with current state (e.g. a scan is already running).
    Conflict(String),
    /// A lossy or irreversible step was applied without the required confirmation.
    ConfirmationRequired(String),
    /// Something that should not happen; a bug.
    Internal(String),
}

impl Error {
    /// Build an I/O error tagged with the path it happened on.
    pub fn io(path: impl AsRef<Path>, err: io::Error) -> Self {
        let path = path.as_ref().to_path_buf();
        match err.kind() {
            io::ErrorKind::NotFound => Error::NotFound(path),
            io::ErrorKind::PermissionDenied => Error::PermissionDenied(path),
            kind => Error::Io { path: Some(path), kind: io_kind_code(kind), message: err.to_string() },
        }
    }

    /// Shorthand for [`Error::Unsupported`].
    pub fn unsupported(feature: impl Into<String>, reason: impl Into<String>) -> Self {
        Error::Unsupported { feature: feature.into(), reason: reason.into() }
    }

    /// Shorthand for [`Error::NotImplemented`].
    pub fn not_implemented(what: impl Into<String>) -> Self {
        Error::NotImplemented(what.into())
    }

    /// Shorthand for [`Error::Parse`].
    pub fn parse(what: impl Into<String>, message: impl Into<String>) -> Self {
        Error::Parse { what: what.into(), message: message.into() }
    }

    /// Stable machine-readable code. Part of the IPC contract (docs/IPC.md).
    pub fn code(&self) -> &'static str {
        match self {
            Error::Io { .. } => "io",
            Error::NotFound(_) => "not_found",
            Error::PermissionDenied(_) => "permission_denied",
            Error::Unsupported { .. } => "unsupported",
            Error::NotImplemented(_) => "not_implemented",
            Error::Cancelled => "cancelled",
            Error::InvalidInput(_) => "invalid_input",
            Error::Protected(_) => "protected",
            Error::OutsideAllowedRoots(_) => "outside_allowed_roots",
            Error::Changed(_) => "changed",
            Error::LinkNotFollowed(_) => "link_not_followed",
            Error::Parse { .. } => "parse",
            Error::ToolMissing(_) => "tool_missing",
            Error::Conflict(_) => "conflict",
            Error::ConfirmationRequired(_) => "confirmation_required",
            Error::Internal(_) => "internal",
        }
    }

    /// An actionable next step for the user.
    pub fn hint(&self) -> String {
        match self {
            Error::Io { .. } => "Check that the drive is connected and try again.".into(),
            Error::NotFound(_) => "The file was moved or deleted since the scan. Scan again.".into(),
            Error::PermissionDenied(_) => {
                "MyJanitor has no access to this path. Skip it, or run with access to it.".into()
            }
            Error::Unsupported { .. } => "This feature is not available here.".into(),
            Error::NotImplemented(_) => "This feature is not available in this build yet.".into(),
            Error::Cancelled => "Nothing else was changed.".into(),
            Error::InvalidInput(_) => "Correct the value and try again.".into(),
            Error::Protected(_) => "Protected paths are never touched. Edit them in Settings.".into(),
            Error::OutsideAllowedRoots(_) => {
                "Only files inside the scanned locations can be changed. Add the location and scan again.".into()
            }
            Error::Changed(_) => "The file changed after the scan, so it was skipped. Scan again.".into(),
            Error::LinkNotFollowed(_) => "Links and junctions are never followed by destructive actions.".into(),
            Error::Parse { .. } => "The file may be damaged; it was left unchanged.".into(),
            Error::ToolMissing(t) => format!("Install {t} and make sure it is on PATH, then refresh tools."),
            Error::Conflict(_) => "Wait for the running task to finish, or stop it.".into(),
            Error::ConfirmationRequired(_) => "Review the step and confirm it in the dialog.".into(),
            Error::Internal(_) => "This is a bug. The operation was stopped safely.".into(),
        }
    }

    /// The path this error is about, if any.
    pub fn path(&self) -> Option<&Path> {
        match self {
            Error::Io { path, .. } => path.as_deref(),
            Error::NotFound(p)
            | Error::PermissionDenied(p)
            | Error::Protected(p)
            | Error::OutsideAllowedRoots(p)
            | Error::Changed(p)
            | Error::LinkNotFollowed(p) => Some(p),
            _ => None,
        }
    }

    /// Serializable form for IPC, reports and the journal.
    pub fn info(&self) -> ErrorInfo {
        ErrorInfo {
            code: self.code().to_string(),
            message: self.to_string(),
            hint: self.hint(),
            path: self.path().map(Path::to_path_buf),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io { path: Some(p), message, .. } => write!(f, "I/O error on {}: {message}", p.display()),
            Error::Io { path: None, message, .. } => write!(f, "I/O error: {message}"),
            Error::NotFound(p) => write!(f, "Not found: {}", p.display()),
            Error::PermissionDenied(p) => write!(f, "Permission denied: {}", p.display()),
            Error::Unsupported { feature, reason } => write!(f, "{feature} is not supported: {reason}"),
            Error::NotImplemented(w) => write!(f, "Not implemented yet: {w}"),
            Error::Cancelled => write!(f, "Cancelled"),
            Error::InvalidInput(m) => write!(f, "Invalid input: {m}"),
            Error::Protected(p) => write!(f, "Protected path: {}", p.display()),
            Error::OutsideAllowedRoots(p) => write!(f, "Outside the allowed locations: {}", p.display()),
            Error::Changed(p) => write!(f, "Changed since the scan: {}", p.display()),
            Error::LinkNotFollowed(p) => write!(f, "Link or junction not followed: {}", p.display()),
            Error::Parse { what, message } => write!(f, "Could not parse {what}: {message}"),
            Error::ToolMissing(t) => write!(f, "{t} was not found"),
            Error::Conflict(m) => write!(f, "{m}"),
            Error::ConfirmationRequired(m) => write!(f, "Confirmation required: {m}"),
            Error::Internal(m) => write!(f, "Internal error: {m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Self {
        match err.kind() {
            io::ErrorKind::NotFound => Error::Io { path: None, kind: "not_found", message: err.to_string() },
            kind => Error::Io { path: None, kind: io_kind_code(kind), message: err.to_string() },
        }
    }
}

/// Stable code for an [`io::ErrorKind`].
pub fn io_kind_code(kind: io::ErrorKind) -> &'static str {
    use io::ErrorKind as K;
    match kind {
        K::NotFound => "not_found",
        K::PermissionDenied => "permission_denied",
        K::AlreadyExists => "already_exists",
        K::InvalidInput => "invalid_input",
        K::InvalidData => "invalid_data",
        K::TimedOut => "timed_out",
        K::WriteZero => "write_zero",
        K::Interrupted => "interrupted",
        K::Unsupported => "unsupported",
        K::UnexpectedEof => "unexpected_eof",
        K::OutOfMemory => "out_of_memory",
        _ => "other",
    }
}

/// Serializable error: the shape every command error and per-item error has on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorInfo {
    /// Stable snake_case code, see [`Error::code`].
    pub code: String,
    /// What failed, in one sentence.
    pub message: String,
    /// What the user can do next.
    pub hint: String,
    /// The path concerned, if any.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::pathutil::serde_path::opt")]
    pub path: Option<PathBuf>,
}

impl From<Error> for ErrorInfo {
    fn from(e: Error) -> Self {
        e.info()
    }
}

impl From<&Error> for ErrorInfo {
    fn from(e: &Error) -> Self {
        e.info()
    }
}

impl fmt::Display for ErrorInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_errors_map_to_specific_variants() {
        let e = Error::io("/x", io::Error::from(io::ErrorKind::NotFound));
        assert_eq!(e.code(), "not_found");
        let e = Error::io("/x", io::Error::from(io::ErrorKind::PermissionDenied));
        assert_eq!(e.code(), "permission_denied");
        let e = Error::io("/x", io::Error::from(io::ErrorKind::TimedOut));
        assert_eq!(e.code(), "io");
        assert_eq!(e.path(), Some(Path::new("/x")));
    }

    #[test]
    fn info_round_trips_through_json() {
        let info = Error::Protected(PathBuf::from("/etc")).info();
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"code\":\"protected\""));
        let back: ErrorInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(back, info);
    }
}
