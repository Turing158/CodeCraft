//! Best-effort local navigation to the terminal that emitted a Gemini Hook.
//!
//! The command accepts only a server-side session id.  Window identifiers are
//! never accepted from the renderer or LAN API.

use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum GeminiFocusResult {
    FocusedExactWindow,
    FocusedSharedTerminal,
    SessionEnded,
    StaleTarget,
    AccessDenied,
    NotFound,
    Unsupported,
}

impl GeminiFocusResult {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::FocusedExactWindow => "focusedExactWindow",
            Self::FocusedSharedTerminal => "focusedSharedTerminal",
            Self::SessionEnded => "sessionEnded",
            Self::StaleTarget => "staleTarget",
            Self::AccessDenied => "accessDenied",
            Self::NotFound => "notFound",
            Self::Unsupported => "unsupported",
        }
    }
}

#[cfg(windows)]
pub(crate) fn focus_session(
    session: &crate::gemini::GeminiSession,
) -> GeminiFocusResult {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        IsWindow, ShowWindow, SetForegroundWindow, SW_RESTORE,
    };

    if !session.is_active() {
        return GeminiFocusResult::SessionEnded;
    }
    let Some(binding) = session.terminal_binding.as_ref() else {
        return GeminiFocusResult::NotFound;
    };
    let Some(raw) = binding.console_window.as_deref().and_then(|value| value.parse::<isize>().ok()) else {
        return GeminiFocusResult::FocusedSharedTerminal;
    };
    let hwnd = HWND(raw as *mut core::ffi::c_void);
    let valid = unsafe { IsWindow(Some(hwnd)).as_bool() };
    if !valid {
        return GeminiFocusResult::StaleTarget;
    }
    unsafe {
        let _ = ShowWindow(hwnd, SW_RESTORE);
        if SetForegroundWindow(hwnd).as_bool() {
            GeminiFocusResult::FocusedExactWindow
        } else {
            GeminiFocusResult::AccessDenied
        }
    }
}

#[cfg(not(windows))]
pub(crate) fn focus_session(_session: &crate::gemini::GeminiSession) -> GeminiFocusResult {
    GeminiFocusResult::Unsupported
}
