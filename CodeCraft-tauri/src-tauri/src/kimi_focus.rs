//! Best-effort local navigation to the terminal that emitted a Kimi Hook.
//!
//! The command accepts only a server-side session id.  Window identifiers are
//! never accepted from the renderer or LAN API.

use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum KimiFocusResult {
    FocusedExactWindow,
    FocusedSharedTerminal,
    SessionEnded,
    StaleTarget,
    AccessDenied,
    NotFound,
    Unsupported,
}

#[cfg(windows)]
pub(crate) struct TerminalWindow {
    pub handle: String,
    pub pid: u32,
    pub created_at: u64,
    pub shared: bool,
}

#[cfg(windows)]
fn process_created_at(pid: u32) -> Option<u64> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, FILETIME},
        System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut created: FILETIME = std::mem::zeroed();
        let mut exited = created;
        let mut kernel = created;
        let mut user = created;
        let ok = GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user);
        CloseHandle(handle);
        if ok == 0 || exited.dwHighDateTime != 0 || exited.dwLowDateTime != 0 {
            return None;
        }
        let ticks = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
        Some((ticks / 10_000).saturating_sub(11_644_473_600_000))
    }
}

#[cfg(windows)]
fn describe_window(hwnd: windows::Win32::Foundation::HWND) -> Option<TerminalWindow> {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetWindowThreadProcessId, IsWindowVisible,
    };
    unsafe {
        if hwnd.is_invalid() || !IsWindowVisible(hwnd).as_bool() {
            return None;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let created_at = process_created_at(pid)?;
        let mut class = [0u16; 128];
        let len = GetClassNameW(hwnd, &mut class) as usize;
        Some(TerminalWindow {
            handle: (hwnd.0 as isize).to_string(),
            pid,
            created_at,
            shared: String::from_utf16_lossy(&class[..len]) != "ConsoleWindowClass",
        })
    }
}

#[cfg(windows)]
fn ancestor_terminal_window(pid: u32) -> Option<TerminalWindow> {
    use windows::{
        core::BOOL,
        Win32::{
            Foundation::{HWND, LPARAM},
            UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, IsWindowVisible},
        },
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        },
    };
    struct Search {
        pid: u32,
        windows: Vec<HWND>,
    }
    unsafe extern "system" fn collect(hwnd: HWND, data: LPARAM) -> BOOL {
        let search = &mut *(data.0 as *mut Search);
        let mut owner = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut owner));
        if owner == search.pid && IsWindowVisible(hwnd).as_bool() {
            search.windows.push(hwnd);
        }
        true.into()
    }
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut processes = std::collections::HashMap::new();
        let mut valid = Process32FirstW(snapshot, &mut entry);
        while valid != 0 {
            let len = entry
                .szExeFile
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(entry.szExeFile.len());
            processes.insert(
                entry.th32ProcessID,
                (
                    entry.th32ParentProcessID,
                    String::from_utf16_lossy(&entry.szExeFile[..len]).to_ascii_lowercase(),
                ),
            );
            valid = Process32NextW(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
        let mut current = pid;
        let mut child_created = process_created_at(current)?;
        for _ in 0..24 {
            current = processes.get(&current)?.0;
            let created = process_created_at(current)?;
            if created > child_created {
                return None;
            }
            child_created = created;
            let name = &processes.get(&current)?.1;
            if !matches!(
                name.as_str(),
                "windowsterminal.exe"
                    | "code.exe"
                    | "code - insiders.exe"
                    | "vscodium.exe"
                    | "cursor.exe"
                    | "windsurf.exe"
                    | "wezterm-gui.exe"
                    | "alacritty.exe"
                    | "mintty.exe"
            ) {
                continue;
            }
            let mut search = Search {
                pid: current,
                windows: Vec::new(),
            };
            let _ = EnumWindows(Some(collect), LPARAM(&mut search as *mut Search as isize));
            if search.windows.len() > 1 {
                return None;
            }
            if let Some(hwnd) = search.windows.first() {
                return describe_window(*hwnd);
            }
        }
        None
    }
}

/// Called only by the short-lived hook, never by the desktop process.
#[cfg(windows)]
pub(crate) fn capture_terminal_window(pid: u32) -> Option<TerminalWindow> {
    use windows::Win32::{
        System::Console::{AttachConsole, FreeConsole, GetConsoleProcessList, GetConsoleWindow},
        UI::WindowsAndMessaging::{GetAncestor, GA_ROOTOWNER},
    };
    unsafe {
        let mut processes = [0u32; 256];
        let count = GetConsoleProcessList(&mut processes) as usize;
        let already_attached = count <= processes.len() && processes[..count].contains(&pid);
        let attached = if already_attached {
            false
        } else {
            let _ = FreeConsole();
            AttachConsole(pid).is_ok()
        };
        // ConPTY's console HWND may be hidden; its root owner can be the
        // visible Windows Terminal host. Never select a window by its title.
        let window = if already_attached || attached {
            let console = GetConsoleWindow();
            describe_window(GetAncestor(console, GA_ROOTOWNER)).or_else(|| describe_window(console))
        } else {
            None
        };
        if attached {
            let _ = FreeConsole();
        }
        window.or_else(|| ancestor_terminal_window(pid))
    }
}

#[cfg(windows)]
pub(crate) fn focus_session(session: &crate::kimi::KimiSession) -> KimiFocusResult {
    use windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE},
    };
    if !session.is_active() {
        return KimiFocusResult::SessionEnded;
    }
    let Some(binding) = session.terminal_binding.as_ref() else {
        return KimiFocusResult::NotFound;
    };
    let Some((pid, created)) = binding.pid.zip(binding.process_created_at) else {
        return KimiFocusResult::Unsupported;
    };
    if process_created_at(pid) != Some(created) {
        return KimiFocusResult::StaleTarget;
    }
    let Some(raw) = binding
        .console_window
        .as_deref()
        .and_then(|value| value.parse::<isize>().ok())
    else {
        return KimiFocusResult::Unsupported;
    };
    let hwnd = HWND(raw as *mut core::ffi::c_void);
    let Some(window) = describe_window(hwnd) else {
        return KimiFocusResult::StaleTarget;
    };
    if binding.window_process_id != Some(window.pid)
        || binding.window_process_created_at != Some(window.created_at)
    {
        return KimiFocusResult::StaleTarget;
    }
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        if !SetForegroundWindow(hwnd).as_bool() {
            return KimiFocusResult::AccessDenied;
        }
    }
    if window.shared {
        KimiFocusResult::FocusedSharedTerminal
    } else {
        KimiFocusResult::FocusedExactWindow
    }
}

#[cfg(not(windows))]
pub(crate) fn focus_session(_session: &crate::kimi::KimiSession) -> KimiFocusResult {
    KimiFocusResult::Unsupported
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stale_process_identity_cannot_focus_a_reused_window() {
        let pid = std::process::id();
        let created = process_created_at(pid).unwrap();
        let mut store = crate::kimi::KimiStore::default();
        store.apply(
            crate::kimi::event_from_payload(
                json!({
                    "hook_event_name":"SessionStart", "session_id":"focus-test",
                    "pid":pid, "process_created_at":created + 1, "console_window":"123",
                }),
                1,
            )
            .unwrap(),
        );
        let snapshot = store.snapshot();
        assert!(matches!(
            focus_session(&snapshot.sessions[0]),
            KimiFocusResult::StaleTarget
        ));
    }

    #[test]
    fn missing_window_binding_does_not_report_success() {
        let pid = std::process::id();
        let mut store = crate::kimi::KimiStore::default();
        store.apply(
            crate::kimi::event_from_payload(
                json!({
                    "hook_event_name":"SessionStart", "session_id":"focus-test",
                    "pid":pid, "process_created_at":process_created_at(pid).unwrap(),
                }),
                1,
            )
            .unwrap(),
        );
        assert!(matches!(
            focus_session(&store.snapshot().sessions[0]),
            KimiFocusResult::Unsupported
        ));
    }
}
