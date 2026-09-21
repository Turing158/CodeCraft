use crate::{protocol::*, wire};
use fs2::FileExt;
use serde_json::{json, Value};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub fn root() -> PathBuf {
    crate::approval_policy::base_data_dir().join("trae-hook")
}
pub fn read(path: &Path, limit: usize) -> Result<Value> {
    wire::strict_json(
        &wire::read_bounded(File::open(path)?, limit).map_err(invalid)?,
        limit,
    )
    .map_err(invalid)
}
pub fn lock(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    file.try_lock_exclusive()
        .map_err(|e| ApiError::new(ErrorCode::StateUnavailable, format!("File is locked: {e}")))?;
    Ok(file)
}
pub fn backup(path: &Path) -> Result<()> {
    if path.exists() {
        let name = path
            .file_name()
            .ok_or_else(|| invalid("Invalid backup path"))?
            .to_string_lossy();
        let backup = path.with_file_name(format!("{name}.{}.bak", id()));
        let mut target = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(backup)?;
        std::io::copy(&mut File::open(path)?, &mut target)?;
        target.sync_all()?;
    }
    Ok(())
}
pub fn atomic(path: &Path, value: &Value, replace: bool) -> Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|e| invalid(e.to_string()))?;
    let parent = path.parent().ok_or_else(|| invalid("Missing parent"))?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!("{}.tmp", id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        if !replace {
            fs::hard_link(&temp, path)?;
        } else {
            #[cfg(windows)]
            unsafe {
                use std::os::windows::ffi::OsStrExt;
                use windows_sys::Win32::Storage::FileSystem::{
                    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
                };
                let from = temp
                    .as_os_str()
                    .encode_wide()
                    .chain(Some(0))
                    .collect::<Vec<_>>();
                let to = path
                    .as_os_str()
                    .encode_wide()
                    .chain(Some(0))
                    .collect::<Vec<_>>();
                if MoveFileExW(
                    from.as_ptr(),
                    to.as_ptr(),
                    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                ) == 0
                {
                    return Err(std::io::Error::last_os_error().into());
                }
            }
            #[cfg(not(windows))]
            {
                fs::rename(&temp, path)?;
            }
        }
        Ok(())
    })();
    let _ = fs::remove_file(temp);
    result
}
pub fn initialize(root: &Path) -> Result<()> {
    fs::create_dir_all(root)?;
    secure_data_directory(root)?;
    for directory in ["inbox", "processing", "replies", "journal", "tasks"] {
        fs::create_dir_all(root.join(directory))?;
    }
    Ok(())
}
#[cfg(all(test, windows))]
#[test]
fn data_directory_has_a_protected_single_user_acl_inherited_by_new_files() {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::{
            Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT},
            GetSecurityDescriptorControl, DACL_SECURITY_INFORMATION, SE_DACL_PROTECTED,
        },
    };
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/trae-acl-tests")
        .join(id());
    initialize(&root).unwrap();
    let file = root.join("inbox/message.json");
    atomic(&file, &json!({"test":true}), false).unwrap();
    for path in [&root, &file] {
        let wide = path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        unsafe {
            let mut acl = std::ptr::null_mut();
            let mut descriptor = std::ptr::null_mut();
            assert_eq!(
                GetNamedSecurityInfoW(
                    wide.as_ptr(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &mut acl,
                    std::ptr::null_mut(),
                    &mut descriptor
                ),
                0
            );
            assert!(!acl.is_null());
            assert_eq!((*acl).AceCount, 1);
            if path == &root {
                let mut control = 0;
                let mut revision = 0;
                assert_ne!(
                    GetSecurityDescriptorControl(descriptor, &mut control, &mut revision),
                    0
                );
                assert_ne!(control & SE_DACL_PROTECTED, 0);
            }
            LocalFree(descriptor);
        }
    }
    fs::remove_dir_all(root).unwrap();
}
#[cfg(windows)]
fn secure_data_directory(root: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, LocalFree},
        Security::{
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
                SetNamedSecurityInfoW, SE_FILE_OBJECT,
            },
            GetSecurityDescriptorDacl, GetTokenInformation, TokenUser, DACL_SECURITY_INFORMATION,
            PROTECTED_DACL_SECURITY_INFORMATION, TOKEN_QUERY, TOKEN_USER,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    struct Local(*mut std::ffi::c_void);
    impl Drop for Local {
        fn drop(&mut self) {
            unsafe {
                LocalFree(self.0);
            }
        }
    }
    // Use the process user's SID, never an elevated process's owner group.
    let sddl = unsafe {
        let mut token = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut needed = 0;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed);
        let mut buffer = vec![0usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
        let ok = GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            needed,
            &mut needed,
        );
        let error = std::io::Error::last_os_error();
        CloseHandle(token);
        if ok == 0 {
            return Err(error.into());
        }
        let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
        let mut sid = std::ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut sid) == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let _sid = Local(sid.cast());
        let mut len = 0;
        while *sid.add(len) != 0 {
            len += 1;
        }
        let sid = String::from_utf16_lossy(std::slice::from_raw_parts(sid, len));
        format!("D:P(A;OICI;FA;;;{sid})")
            .encode_utf16()
            .chain(Some(0))
            .collect::<Vec<_>>()
    };
    unsafe {
        let mut descriptor = std::ptr::null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            std::ptr::null_mut(),
        ) == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        let _descriptor = Local(descriptor);
        let mut acl = std::ptr::null_mut();
        let mut present = 0;
        let mut defaulted = 0;
        if GetSecurityDescriptorDacl(descriptor, &mut present, &mut acl, &mut defaulted) == 0
            || present == 0
            || acl.is_null()
        {
            return Err(ApiError::new(
                ErrorCode::StateUnavailable,
                "Cannot restrict Trae data access",
            ));
        }
        // Also remove explicit grants from existing data; never traverse reparse points.
        let mut paths = vec![root.to_path_buf()];
        while let Some(path) = paths.pop() {
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(invalid("Trae data directory contains a reparse point"));
            }
            let wide = path
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>();
            let error = SetNamedSecurityInfoW(
                wide.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                acl,
                std::ptr::null_mut(),
            );
            if error != 0 {
                return Err(std::io::Error::from_raw_os_error(error as i32).into());
            }
            if metadata.is_dir() {
                for entry in fs::read_dir(path)? {
                    paths.push(entry?.path());
                }
            }
        }
    }
    Ok(())
}
#[cfg(not(windows))]
fn secure_data_directory(root: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
    Ok(())
}
pub fn canonical_directory(value: &str) -> Result<PathBuf> {
    let path = Path::new(value);
    if !path.is_absolute() || !path.is_dir() {
        return Err(invalid(
            "Workspace roots must be existing absolute directories",
        ));
    }
    Ok(path.canonicalize()?)
}
pub fn roots(primary: &str, values: &[String]) -> Result<Vec<String>> {
    if values.is_empty() || values.len() > 16 {
        return Err(invalid("Expected 1–16 workspace roots"));
    }
    let primary = canonical_directory(primary)?;
    let mut roots = values
        .iter()
        .map(|r| canonical_directory(r).map(|p| p.to_string_lossy().into_owned()))
        .collect::<Result<Vec<_>>>()?;
    roots.sort();
    roots.dedup();
    if !roots.iter().any(|r| Path::new(r) == primary) {
        return Err(invalid("Primary root must be included"));
    }
    Ok(roots)
}
pub fn plan(path: &str, primary: &str) -> Result<(String, String)> {
    let path = Path::new(path).canonicalize()?;
    let root = canonical_directory(primary)?;
    if !path.starts_with(root) || !path.is_file() {
        return Err(invalid("Plan path escapes its workspace"));
    }
    let bytes = wire::read_bounded(File::open(&path)?, PLAN_LIMIT)
        .map_err(|_| ApiError::new(ErrorCode::PayloadTooLarge, "Plan file exceeds 128 KiB"))?;
    let text = std::str::from_utf8(&bytes).map_err(|_| invalid("Plan must be UTF-8"))?;
    let text = normalize(text);
    if text.len() > PLAN_LIMIT {
        return Err(ApiError::new(
            ErrorCode::PayloadTooLarge,
            "Normalized plan is too large",
        ));
    }
    use sha2::{Digest, Sha256};
    let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
    Ok((text, hash))
}
pub fn normalize(text: &str) -> String {
    text.strip_prefix('\u{feff}')
        .unwrap_or(text)
        .replace("\r\n", "\n")
        .replace('\r', "\n")
}
pub fn plan_identity(path: &str) -> Result<String> {
    let file = File::open(path)?;
    #[cfg(windows)]
    unsafe {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        };
        let mut info: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
        if GetFileInformationByHandle(file.as_raw_handle(), &mut info) == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(format!(
            "{}:{}:{}",
            info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow
        ))
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::MetadataExt;
        let m = file.metadata()?;
        Ok(format!("{}:{}", m.dev(), m.ino()))
    }
}

#[derive(Clone)]
pub struct Client {
    pub root: PathBuf,
    pub epoch: String,
}
impl Client {
    pub fn connect(root: &Path) -> Result<Self> {
        let h = read(&root.join("heartbeat.json"), 4096).map_err(|_| {
            ApiError::new(ErrorCode::BridgeUnavailable, "CodeCraft bridge is offline")
        })?;
        let updated = h["updatedAt"]
            .as_i64()
            .ok_or_else(|| invalid("Heartbeat timestamp missing"))?;
        let age = chrono::Utc::now().timestamp_millis() - updated;
        if !(0..=10000).contains(&age) || h["healthy"] != true {
            return Err(ApiError::new(
                ErrorCode::BridgeUnavailable,
                "CodeCraft heartbeat is stale",
            ));
        }
        let epoch = h["appEpoch"]
            .as_str()
            .ok_or_else(|| invalid("Heartbeat epoch missing"))?
            .to_string();
        check_id(&epoch)?;
        Ok(Self {
            root: root.into(),
            epoch,
        })
    }
    pub fn call(&self, command: Value) -> Result<Value> {
        self.call_id(command, &id())
    }
    pub fn call_id(&self, command: Value, command_id: &str) -> Result<Value> {
        check_id(command_id)?;
        if Self::connect(&self.root)?.epoch != self.epoch {
            return Err(ApiError::new(
                ErrorCode::RequestExpired,
                "CodeCraft restarted",
            ));
        }
        let control = control_message(&command);
        let body_hash = wire::hash(&command).map_err(invalid)?;
        let message = json!({"schemaVersion":1,"appEpoch":self.epoch,"messageId":id(),"commandId":command_id,"command":command});
        let size = serde_json::to_vec(&message)
            .map_err(|e| invalid(e.to_string()))?
            .len();
        if size > wire::MAX_IPC {
            return Err(ApiError::new(
                ErrorCode::PayloadTooLarge,
                "IPC message exceeds 2 MiB",
            ));
        }
        let started = Instant::now();
        let guard = loop {
            match lock(&self.root.join("queue.lock")) {
                Ok(l) => break l,
                Err(e) => {
                    if started.elapsed() > Duration::from_secs(3) {
                        return Err(e);
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        };
        let mut count = 0;
        let mut bytes = size as u64;
        for dir in ["inbox", "processing"] {
            for entry in fs::read_dir(self.root.join(dir))? {
                let entry = entry?;
                count += 1;
                if let Ok(m) = entry.metadata() {
                    bytes += m.len();
                }
            }
        }
        if count >= if control { 1024 } else { 896 }
            || bytes
                > if control {
                    64 * 1024 * 1024
                } else {
                    56 * 1024 * 1024
                }
        {
            return Err(ApiError::new(ErrorCode::QueueFull, "Trae inbox is full"));
        }
        atomic(
            &self.root.join("inbox").join(format!("{}.json", id())),
            &message,
            false,
        )?;
        drop(guard);
        let path = self.root.join("replies").join(format!("{command_id}.json"));
        let started = Instant::now();
        loop {
            if path.exists() {
                let reply = read(&path, wire::MAX_IPC)?;
                if reply["appEpoch"] != self.epoch {
                    return Err(ApiError::new(
                        ErrorCode::RequestExpired,
                        "Reply belongs to a previous CodeCraft process",
                    ));
                }
                if reply["bodyHash"] != body_hash {
                    return Err(ApiError::new(
                        ErrorCode::IdempotencyConflict,
                        "Command ID was reused with different content",
                    ));
                }
                if !reply["error"].is_null() {
                    return Err(serde_json::from_value(reply["error"].clone())
                        .map_err(|e| invalid(e.to_string()))?);
                }
                if command["kind"] == "poll" {
                    let _ = fs::remove_file(&path);
                }
                return Ok(reply["result"].clone());
            }
            if started.elapsed() > Duration::from_secs(3) {
                return Err(ApiError::new(
                    ErrorCode::BridgeUnavailable,
                    "CodeCraft response timed out",
                ));
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

pub fn control_message(command: &Value) -> bool {
    matches!(
        command["kind"].as_str(),
        Some("cancel" | "ack" | "task_action" | "resolve_grant" | "refresh_capabilities")
    ) || (command["kind"] == "hook"
        && matches!(
            command["input"]["hook_event_name"].as_str(),
            Some("Stop" | "PostToolUse" | "UserPromptSubmit" | "SessionStart")
        ))
        || (command["kind"] == "decision" && command["body"]["action"]["kind"] == "cancel")
}
