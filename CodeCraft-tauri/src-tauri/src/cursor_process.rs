//! Process identities are checked against Windows handles, never a claimed PID
//! or an executable basename alone. No process termination is used by this adapter.
use crate::schema::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Identity {
    pub pid: u32,
    pub created: u64,
    pub executable: PathBuf,
}

pub fn limit_cpu() -> Result<()> {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::*;
        let (mut mask, mut system) = (0usize, 0usize);
        let process = GetCurrentProcess();
        if GetProcessAffinityMask(process, &mut mask, &mut system) == 0 {
            return Err("AFFINITY_QUERY_FAILED");
        }
        let first = mask & mask.wrapping_neg();
        let rest = mask & !first;
        let two = first | (rest & rest.wrapping_neg());
        if two == 0 || SetProcessAffinityMask(process, two) == 0 {
            return Err("AFFINITY_SET_FAILED");
        }
    }
    Ok(())
}

/// Validate the actual helper command line, rather than granting MCP authority
/// from a role string supplied in a bootstrap file.
#[cfg(windows)]
pub(crate) fn helper_role(identity: &Identity, role: crate::transport::Role) -> bool {
    use windows_sys::Win32::{Foundation::*, System::Threading::*, UI::Shell::CommandLineToArgvW};
    #[repr(C)]
    struct UnicodeString {
        length: u16,
        maximum: u16,
        buffer: *const u16,
    }
    #[link(name = "ntdll")]
    extern "system" {
        fn NtQueryInformationProcess(
            process: HANDLE,
            class: u32,
            buffer: *mut core::ffi::c_void,
            size: u32,
            returned: *mut u32,
        ) -> i32;
    }
    let result = (|| unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, identity.pid);
        if handle.is_null() {
            return None;
        }
        let mut needed = 0;
        NtQueryInformationProcess(handle, 60, std::ptr::null_mut(), 0, &mut needed);
        if !(16..=131072).contains(&needed) {
            CloseHandle(handle);
            return None;
        }
        let mut storage = vec![0usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
        let status =
            NtQueryInformationProcess(handle, 60, storage.as_mut_ptr().cast(), needed, &mut needed);
        CloseHandle(handle);
        if status < 0 {
            return None;
        }
        let value = &*storage.as_ptr().cast::<UnicodeString>();
        let start = storage.as_ptr() as usize;
        let end = start + std::mem::size_of_val(storage.as_slice());
        let ptr = value.buffer as usize;
        if value.length % 2 != 0
            || ptr < start
            || ptr
                .checked_add(value.length as usize)
                .is_none_or(|p| p > end)
        {
            return None;
        }
        let mut command =
            std::slice::from_raw_parts(value.buffer, value.length as usize / 2).to_vec();
        command.push(0);
        let mut count = 0;
        let argv = CommandLineToArgvW(command.as_ptr(), &mut count);
        if argv.is_null() {
            return None;
        }
        let mut args = Vec::new();
        for i in 0..count {
            let p = *argv.add(i as usize);
            let mut len = 0;
            while *p.add(len) != 0 {
                len += 1;
            }
            args.push(String::from_utf16_lossy(std::slice::from_raw_parts(p, len)));
        }
        LocalFree(argv.cast());
        Some(args)
    })();
    let Some(args) = result else { return false };
    if identify(identity.pid).as_ref() != Ok(identity) {
        return false;
    }
    let hook = args.iter().any(|s| s == "--codecraft-cursor-hook");
    let mcp = args.iter().any(|s| s == "--codecraft-cursor-mcp");
    #[cfg(test)]
    if args.iter().any(|s| {
        s == match role {
            crate::transport::Role::Hook => "runtime::tests::subprocess_observation_helper",
            crate::transport::Role::Mcp => "runtime::tests::subprocess_live_mcp_helper",
            _ => "not-a-helper",
        }
    }) {
        return true;
    }
    matches!(
        (role, hook, mcp),
        (crate::transport::Role::Hook, true, false) | (crate::transport::Role::Mcp, false, true)
    )
}
#[cfg(not(windows))]
pub(crate) fn helper_role(_identity: &Identity, _role: crate::transport::Role) -> bool {
    false
}

#[cfg(windows)]
pub fn identify(pid: u32) -> Result<Identity> {
    use windows_sys::Win32::{Foundation::*, System::Threading::*};
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return Err("PROCESS_UNAVAILABLE");
        }
        let result = (|| {
            let mut exit_code = 0;
            if GetExitCodeProcess(handle, &mut exit_code) == 0 || exit_code != 259 {
                return Err("PROCESS_NOT_RUNNING");
            }
            let mut created: FILETIME = std::mem::zeroed();
            let mut exit = created;
            let mut kernel = created;
            let mut user = created;
            if GetProcessTimes(handle, &mut created, &mut exit, &mut kernel, &mut user) == 0 {
                return Err("PROCESS_IDENTITY_UNAVAILABLE");
            }
            let mut buffer = vec![0u16; 32768];
            let mut size = buffer.len() as u32;
            if QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut size) == 0 {
                return Err("PROCESS_IDENTITY_UNAVAILABLE");
            }
            let path = PathBuf::from(String::from_utf16_lossy(&buffer[..size as usize]));
            Ok(Identity {
                pid,
                created: ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64,
                executable: path
                    .canonicalize()
                    .map_err(|_| "PROCESS_PATH_UNAVAILABLE")?,
            })
        })();
        CloseHandle(handle);
        result
    }
}

#[cfg(not(windows))]
pub fn identify(_: u32) -> Result<Identity> {
    Err("WINDOWS_REQUIRED")
}

#[cfg(windows)]
pub fn processes() -> Vec<(u32, u32)> {
    use windows_sys::Win32::{Foundation::*, System::Diagnostics::ToolHelp::*};
    unsafe {
        let handle = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if handle == INVALID_HANDLE_VALUE {
            return Vec::new();
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut result = Vec::new();
        if Process32FirstW(handle, &mut entry) != 0 {
            loop {
                result.push((entry.th32ProcessID, entry.th32ParentProcessID));
                if Process32NextW(handle, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(handle);
        result
    }
}
#[cfg(not(windows))]
pub fn processes() -> Vec<(u32, u32)> {
    Vec::new()
}

pub fn has_cursor_ancestor(child: &Identity, cursor: &Path) -> bool {
    let Ok(cursor) = cursor.canonicalize() else {
        return false;
    };
    let parents = processes();
    let mut current = child.clone();
    for _ in 0..12 {
        let Some((_, parent)) = parents.iter().find(|(pid, _)| *pid == current.pid) else {
            return false;
        };
        let Ok(identity) = identify(*parent) else {
            return false;
        };
        if identity.created > current.created || identity.pid == current.pid {
            return false;
        }
        if identity.executable == cursor {
            return true;
        }
        current = identity;
    }
    false
}

pub fn cursor_version(executable: &Path) -> Result<String> {
    if !executable
        .file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case("Cursor.exe"))
        || !executable.is_file()
    {
        return Err("CURSOR_EXECUTABLE_REQUIRED");
    }
    let package = executable
        .parent()
        .ok_or("CURSOR_EXECUTABLE_REQUIRED")?
        .join("resources/app/package.json");
    let bytes = crate::protocol::read_bytes(
        std::fs::File::open(package).map_err(|_| "CURSOR_PACKAGE_UNAVAILABLE")?,
    )
    .map_err(|_| "CURSOR_PACKAGE_INVALID")?;
    let value = crate::protocol::strict_json(&bytes).map_err(|_| "CURSOR_PACKAGE_INVALID")?;
    value["version"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() < 128)
        .map(str::to_owned)
        .ok_or("CURSOR_PACKAGE_INVALID")
}

fn cursor_from_candidates(candidates: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    let mut seen = std::collections::BTreeSet::new();
    candidates.into_iter().find_map(|path| {
        let path = path.canonicalize().ok()?;
        (seen.insert(path.clone()) && cursor_version(&path).is_ok()).then_some(path)
    })
}

fn cursor_path_candidates(path: &std::ffi::OsStr) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    for dir in std::env::split_paths(path) {
        if !dir.is_absolute() {
            continue;
        }
        candidates.push(dir.join("Cursor.exe"));
        // Cursor adds its CLI directory, not its installation root, to PATH.
        // Inspect the adjacent installation without executing the CLI wrapper.
        if dir.join("cursor.cmd").is_file() || dir.join("cursor").is_file() {
            candidates.push(dir.join("../Cursor.exe"));
            candidates.push(dir.join("../../../Cursor.exe"));
        }
    }
    candidates
}

#[cfg(windows)]
fn cursor_uninstall_candidates(
    name: &str,
    location: Option<&str>,
    icon: Option<&str>,
) -> Vec<PathBuf> {
    let name = name.to_ascii_lowercase();
    if name != "cursor" && !name.starts_with("cursor (") {
        return Vec::new();
    }
    let mut candidates = Vec::new();
    if let Some(location) = location.filter(|s| !s.trim().is_empty()) {
        candidates.push(PathBuf::from(location.trim().trim_matches('"')).join("Cursor.exe"));
    }
    if let Some(icon) = icon {
        // DisplayIcon may be a quoted path followed by a resource index.
        let path = icon
            .rsplit_once(',')
            .filter(|(_, index)| index.trim().parse::<i32>().is_ok())
            .map_or(icon, |(path, _)| path);
        candidates.push(PathBuf::from(path.trim().trim_matches('"')));
    }
    candidates
}

#[cfg(windows)]
fn cursor_registry_candidates() -> Vec<PathBuf> {
    use winreg::{enums::*, RegKey};
    let mut candidates = Vec::new();
    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let hive = RegKey::predef(hive);
        for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
            if let Ok(key) = hive.open_subkey_with_flags(
                "Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\Cursor.exe",
                KEY_READ | view,
            ) {
                if let Ok(path) = key.get_value::<String, _>("") {
                    candidates.push(PathBuf::from(path.trim().trim_matches('"')));
                }
            }
            if let Ok(key) = hive.open_subkey_with_flags(
                "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
                KEY_READ | view,
            ) {
                for name in key.enum_keys().flatten() {
                    let Ok(entry) = key.open_subkey(name) else {
                        continue;
                    };
                    let Ok(name) = entry.get_value::<String, _>("DisplayName") else {
                        continue;
                    };
                    candidates.extend(cursor_uninstall_candidates(
                        &name,
                        entry
                            .get_value::<String, _>("InstallLocation")
                            .ok()
                            .as_deref(),
                        entry.get_value::<String, _>("DisplayIcon").ok().as_deref(),
                    ));
                }
            }
        }
    }
    candidates
}

pub fn detect_cursor() -> Option<PathBuf> {
    // Prefer the exact running installation, then registered/default installs and PATH.
    for (pid, _) in processes() {
        if let Ok(identity) = identify(pid) {
            if identity
                .executable
                .file_name()
                .is_some_and(|n| n.eq_ignore_ascii_case("Cursor.exe"))
                && cursor_version(&identity.executable).is_ok()
            {
                return Some(identity.executable);
            }
        }
    }
    #[cfg(windows)]
    let mut candidates = cursor_registry_candidates();
    #[cfg(not(windows))]
    let mut candidates = Vec::new();
    for (env, suffix) in [
        ("LOCALAPPDATA", "Programs/cursor/Cursor.exe"),
        ("ProgramFiles", "Cursor/Cursor.exe"),
        ("ProgramFiles(x86)", "Cursor/Cursor.exe"),
    ] {
        if let Some(base) = std::env::var_os(env) {
            candidates.push(PathBuf::from(base).join(suffix));
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(cursor_path_candidates(&path));
    }
    cursor_from_candidates(candidates)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn installation() -> PathBuf {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("runs")
            .join(format!("detect-{}", uuid::Uuid::new_v4()))
            .join("自定义 Cursor");
        std::fs::create_dir_all(root.join("resources/app/bin")).unwrap();
        std::fs::write(root.join("Cursor.exe"), b"metadata-only-fixture").unwrap();
        std::fs::write(
            root.join("resources/app/package.json"),
            br#"{"version":"3.21.18"}"#,
        )
        .unwrap();
        std::fs::write(root.join("resources/app/bin/cursor.cmd"), b"not executed").unwrap();
        root
    }

    #[test]
    fn discovers_custom_installation_from_root_and_cli_path_without_running_cursor() {
        let root = installation();
        let expected = root.join("Cursor.exe").canonicalize().unwrap();
        for dir in [root.clone(), root.join("resources/app/bin")] {
            let path = std::env::join_paths([root.join("missing"), dir]).unwrap();
            let found = cursor_from_candidates(cursor_path_candidates(&path)).unwrap();
            assert_eq!(found, expected);
            assert_eq!(cursor_version(&found).unwrap(), "3.21.18");
        }
    }

    #[test]
    fn registered_custom_installation_uses_location_or_quoted_display_icon() {
        let root = installation();
        let expected = root.join("Cursor.exe").canonicalize().unwrap();
        for name in ["Cursor", "Cursor (User)"] {
            let candidates = cursor_uninstall_candidates(name, root.to_str(), None);
            assert_eq!(cursor_from_candidates(candidates), Some(expected.clone()));
        }
        let icon = format!("\"{}\",0", root.join("Cursor.exe").display());
        let candidates = cursor_uninstall_candidates("Cursor (User)", Some(""), Some(&icon));
        assert_eq!(cursor_from_candidates(candidates), Some(expected));
        assert!(cursor_uninstall_candidates("Other App", root.to_str(), Some(&icon)).is_empty());
    }

    #[test]
    fn stale_registry_paths_and_cli_wrappers_do_not_prove_an_installation() {
        let root = installation();
        let exe = root.join("Cursor.exe");
        let path = std::env::join_paths([root.join("resources/app/bin")]).unwrap();
        std::fs::remove_file(root.join("resources/app/package.json")).unwrap();
        assert!(cursor_from_candidates([exe.clone()]).is_none());
        assert!(cursor_from_candidates(cursor_path_candidates(&path)).is_none());
        std::fs::write(root.join("resources/app/package.json"), b"{}").unwrap();
        assert!(cursor_from_candidates([exe.clone()]).is_none());
        std::fs::remove_file(exe).unwrap();
        assert!(
            cursor_from_candidates(cursor_uninstall_candidates("Cursor", root.to_str(), None))
                .is_none()
        );
    }
}
