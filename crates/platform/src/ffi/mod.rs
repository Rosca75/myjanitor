//! Hand-written FFI declarations shared by the scan and action modules
//! (Wave 1, frozen). Workstream-specific declarations live next to the code
//! that uses them (`scan/*`, `action/*`), not here, so owners never edit a
//! shared file.

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(windows)]
pub mod windows;
