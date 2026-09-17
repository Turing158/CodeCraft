//! OS process identity for observations. A PID alone is never a session key.
//! Lineage is not proof of a compatible protocol and never enables decisions.
#[cfg(windows)]
fn process_tree() -> Option<std::collections::HashMap<u32, (u32, String)>> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        },
    };
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut processes = std::collections::HashMap::new();
        let mut next = Process32FirstW(snapshot, &mut entry);
        while next != 0 {
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
            next = Process32NextW(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
        Some(processes)
    }
}

#[cfg(windows)]
pub(crate) fn host_instance(hook_pid: u32) -> Option<String> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, FILETIME},
        System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    };
    let processes = process_tree()?;
    unsafe {
        let mut pid = hook_pid;
        for _ in 0..16 {
            pid = processes.get(&pid)?.0;
            if !matches!(
                processes.get(&pid)?.1.as_str(),
                "node.exe" | "codebuddy.exe" | "codebuddy-code.exe" | "cbc.exe" | "workbuddy.exe"
            ) {
                continue;
            }
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                return None;
            }
            let mut created: FILETIME = std::mem::zeroed();
            let mut exited = created;
            let mut kernel = created;
            let mut user = created;
            let ok = GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user);
            CloseHandle(process);
            if ok == 0 {
                return None;
            }
            let ticks =
                (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
            return Some(format!("{pid}:{ticks}"));
        }
        None
    }
}

/// Resolve the native host's WorkBuddy ancestor, never a PID claimed by a hook.
#[cfg(windows)]
pub(crate) fn desktop_process_id(identity: &str) -> Option<u32> {
    if is_alive(identity) != Some(true) {
        return None;
    }
    let processes = process_tree()?;
    let mut pid = identity.split_once(':')?.0.parse::<u32>().ok()?;
    let mut desktop = None;
    for _ in 0..16 {
        let Some((parent, name)) = processes.get(&pid) else {
            break;
        };
        if name == "workbuddy.exe" {
            desktop = Some(pid);
        }
        if *parent == 0 || *parent == pid {
            break;
        }
        pid = *parent;
    }
    desktop
}

#[cfg(not(windows))]
pub(crate) fn desktop_process_id(_identity: &str) -> Option<u32> {
    None
}

#[cfg(not(windows))]
pub(crate) fn host_instance(_hook_pid: u32) -> Option<String> {
    None
}

#[cfg(windows)]
pub(crate) fn is_alive(identity: &str) -> Option<bool> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, ERROR_INVALID_PARAMETER, FILETIME},
        System::Threading::{
            GetExitCodeProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        },
    };
    let (pid, ticks) = identity.split_once(':')?;
    let pid = pid.parse::<u32>().ok()?;
    let ticks = ticks.parse::<u64>().ok()?;
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return if GetLastError() == ERROR_INVALID_PARAMETER {
                Some(false)
            } else {
                None
            };
        }
        let mut created: FILETIME = std::mem::zeroed();
        let mut exited = created;
        let mut kernel = created;
        let mut user = created;
        let mut code = 0;
        let ok = GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) != 0
            && GetExitCodeProcess(process, &mut code) != 0;
        CloseHandle(process);
        if !ok {
            return None;
        }
        Some(
            code == 259
                && ((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
                    == ticks,
        )
    }
}

#[cfg(not(windows))]
pub(crate) fn is_alive(_identity: &str) -> Option<bool> {
    None
}
