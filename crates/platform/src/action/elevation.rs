//! Elevation state (SAF-05). Owner: W2-E (Windows: `OpenProcessToken` +
//! `GetTokenInformation(TokenElevation)`).

/// True when running as root (Linux) or elevated (Windows).
pub fn is_elevated() -> bool {
    #[cfg(target_os = "linux")]
    {
        crate::ffi::linux::effective_uid() == 0
    }
    #[cfg(not(target_os = "linux"))]
    {
        // Wave 1 placeholder: assume not elevated (the safe answer: never skip a prompt).
        false
    }
}
