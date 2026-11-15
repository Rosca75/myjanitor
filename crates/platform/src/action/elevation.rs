//! Elevation state (SAF-05). Owner: W2-E (Windows: `OpenProcessToken` +
//! `GetTokenInformation(TokenElevation)`; Linux: effective uid 0).
//!
//! MyJanitor never requests elevation. This only tells the engine whether it is
//! *already* elevated, so it can warn, and lets `mj-actions` say "needs
//! elevation" for permission-denied system paths instead of retrying elevated.

/// True when running as root (Linux) or elevated (Windows).
pub fn is_elevated() -> bool {
    #[cfg(target_os = "linux")]
    {
        crate::ffi::linux::effective_uid() == 0
    }
    #[cfg(windows)]
    {
        win::is_elevated()
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        // Unknown platform: assume not elevated (the safe answer: never skip a prompt).
        false
    }
}

#[cfg(windows)]
#[allow(non_camel_case_types, non_snake_case, clippy::upper_case_acronyms)]
mod win {
    use std::ffi::c_void;

    use crate::ffi::windows::{OwnedHandle, BOOL, DWORD, HANDLE};

    const TOKEN_QUERY: DWORD = 0x0008;
    /// `TOKEN_INFORMATION_CLASS::TokenElevation`.
    const TOKEN_ELEVATION: u32 = 20;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> HANDLE;
    }

    #[link(name = "advapi32")]
    extern "system" {
        fn OpenProcessToken(ProcessHandle: HANDLE, DesiredAccess: DWORD, TokenHandle: *mut HANDLE) -> BOOL;
        fn GetTokenInformation(
            TokenHandle: HANDLE,
            TokenInformationClass: u32,
            TokenInformation: *mut c_void,
            TokenInformationLength: DWORD,
            ReturnLength: *mut DWORD,
        ) -> BOOL;
    }

    pub fn is_elevated() -> bool {
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: GetCurrentProcess returns a pseudo-handle that needs no closing;
        // `token` receives a real handle on success.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return false;
        }
        let Ok(token) = OwnedHandle::new(token) else { return false };
        // TOKEN_ELEVATION is a single DWORD (`TokenIsElevated`).
        let mut elevated: DWORD = 0;
        let mut ret: DWORD = 0;
        // SAFETY: the buffer is a DWORD of the size passed; `token` is a valid token handle.
        let ok = unsafe {
            GetTokenInformation(
                token.0,
                TOKEN_ELEVATION,
                &mut elevated as *mut DWORD as *mut c_void,
                std::mem::size_of::<DWORD>() as DWORD,
                &mut ret,
            )
        };
        ok != 0 && elevated != 0
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn answers_without_panicking() {
        let _ = super::is_elevated();
    }
}
