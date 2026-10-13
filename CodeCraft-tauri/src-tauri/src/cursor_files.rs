//! Local file primitives for the Cursor bridge. No default path, installation
//! or process is created by importing this module.
use crate::{protocol, schema::Result};
use fs2::FileExt;
use serde_json::{json, Value};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
};
use uuid::Uuid;

const OWNER_FILE: &str = "codecraft-cursor-owner.json";

pub fn reject_link(path: &Path) -> Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err("FILE_METADATA_FAILED"),
    };
    if metadata.file_type().is_symlink() {
        return Err("REPARSE_POINT_REJECTED");
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("REPARSE_POINT_REJECTED");
        }
    }
    Ok(())
}

/// Check the entire lexical path before canonicalization can hide a junction.
/// Files in the bridge directory never need to follow links. Project identity
/// resolution has a separate handle-based policy that deliberately follows them.
pub fn reject_link_path(path: &Path) -> Result<()> {
    for ancestor in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
        if !ancestor.as_os_str().is_empty() {
            reject_link(ancestor)?;
        }
    }
    Ok(())
}

pub fn child(root: &Path, relative: &str) -> Result<PathBuf> {
    if relative.is_empty() {
        return Err("INVALID_FILE_NAME");
    }
    let mut result = root.to_path_buf();
    reject_link_path(root)?;
    for component in Path::new(relative).components() {
        let Component::Normal(name) = component else {
            return Err("INVALID_FILE_NAME");
        };
        let text = name.to_str().ok_or("INVALID_FILE_NAME")?;
        let stem = text.split('.').next().unwrap_or("").to_ascii_uppercase();
        if !text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
            || text.ends_with('.')
            || text.len() > 160
            || [
                "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
                "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8",
                "LPT9",
            ]
            .contains(&stem.as_str())
        {
            return Err("INVALID_FILE_NAME");
        }
        result.push(name);
        reject_link(&result)?;
    }
    Ok(result)
}

pub fn read_json(path: &Path) -> Result<Value> {
    reject_link_path(path)?;
    protocol::read_input(retry_file_sharing(|| File::open(path)).map_err(|_| "FILE_UNREADABLE")?)
        .map_err(|_| "INVALID_FILE_JSON")
}

/// Windows readers/scanners can briefly prevent opening or replacing a file.
/// Retry only definite OS failures, before any protocol value is consumed or
/// returned. Missing files, invalid JSON and one-shot publication are not retried.
fn retry_file_sharing<T>(mut operation: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    #[cfg(windows)]
    {
        use std::{
            thread,
            time::{Duration, Instant},
        };
        use windows_sys::Win32::Foundation::{
            ERROR_ACCESS_DENIED, ERROR_LOCK_VIOLATION, ERROR_SHARING_VIOLATION,
        };
        let started = Instant::now();
        loop {
            match operation() {
                Err(error)
                    if matches!(
                        error.raw_os_error().map(|code| code as u32),
                        Some(ERROR_ACCESS_DENIED | ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION)
                    ) && started.elapsed() < Duration::from_millis(250) =>
                {
                    thread::sleep(Duration::from_millis(10));
                }
                result => return result,
            }
        }
    }
    #[cfg(not(windows))]
    operation()
}

pub fn lock(path: &Path) -> Result<File> {
    reject_link_path(path)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|_| "LOCK_UNAVAILABLE")?;
    FileExt::try_lock_exclusive(&file).map_err(|_| "FILE_ALREADY_LOCKED")?;
    Ok(file)
}

pub fn atomic_json(path: &Path, value: &Value, replace: bool) -> Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|_| "ENCODE_FAILED")?;
    atomic_write(path, &bytes, replace)
}

pub fn atomic_write(path: &Path, bytes: &[u8], replace: bool) -> Result<()> {
    if bytes.len() > protocol::MAX_MESSAGE_BYTES {
        return Err("FILE_TOO_LARGE");
    }
    let parent = path.parent().ok_or("FILE_PARENT_REQUIRED")?;
    reject_link_path(path)?;
    let temporary = parent.join(format!("{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|_| "FILE_WRITE_FAILED")?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "FILE_WRITE_FAILED")?;
        drop(file);
        if !replace {
            fs::hard_link(&temporary, path).map_err(|_| "FILE_PUBLISH_FAILED")?;
        } else {
            #[cfg(windows)]
            {
                use std::os::windows::ffi::OsStrExt;
                use windows_sys::Win32::Storage::FileSystem::{
                    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
                };
                let from = temporary
                    .as_os_str()
                    .encode_wide()
                    .chain(Some(0))
                    .collect::<Vec<_>>();
                let to = path
                    .as_os_str()
                    .encode_wide()
                    .chain(Some(0))
                    .collect::<Vec<_>>();
                retry_file_sharing(|| {
                    if unsafe {
                        MoveFileExW(
                            from.as_ptr(),
                            to.as_ptr(),
                            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                        )
                    } == 0
                    {
                        Err(std::io::Error::last_os_error())
                    } else {
                        Ok(())
                    }
                })
                .map_err(|_| "FILE_REPLACE_FAILED")?;
            }
            #[cfg(not(windows))]
            fs::rename(&temporary, path).map_err(|_| "FILE_REPLACE_FAILED")?;
        }
        #[cfg(not(windows))]
        File::open(parent)
            .and_then(|f| f.sync_all())
            .map_err(|_| "DIRECTORY_SYNC_FAILED")?;
        Ok(())
    })();
    let _ = fs::remove_file(&temporary);
    result
}

/// Only claims an empty directory or one already marked as our own. This
/// prevents accidental ACL changes to an arbitrary existing user directory.
pub fn private_directory(root: &Path) -> Result<PathBuf> {
    if !root.is_absolute() {
        return Err("ABSOLUTE_DIRECTORY_REQUIRED");
    }
    reject_link_path(root)?;
    if !root.exists() {
        fs::create_dir(root).map_err(|_| "DIRECTORY_CREATE_FAILED")?;
    }
    let root = root.canonicalize().map_err(|_| "DIRECTORY_UNAVAILABLE")?;
    let marker = child(&root, OWNER_FILE)?;
    if marker.exists() {
        if read_json(&marker)? != json!({"schemaVersion":1,"owner":"CodeCraft.Cursor"}) {
            return Err("DIRECTORY_NOT_OWNED");
        }
    } else if fs::read_dir(&root)
        .map_err(|_| "DIRECTORY_UNAVAILABLE")?
        .next()
        .is_some()
    {
        return Err("DIRECTORY_NOT_OWNED");
    }
    // Inventory before changing any ACL; never walk through a junction.
    let mut queue = vec![root.clone()];
    let mut paths = Vec::new();
    while let Some(path) = queue.pop() {
        reject_link(&path)?;
        if paths.len() >= 20_000 {
            return Err("DIRECTORY_ENTRY_LIMIT");
        }
        if path.is_dir() {
            for entry in fs::read_dir(&path).map_err(|_| "DIRECTORY_UNAVAILABLE")? {
                queue.push(entry.map_err(|_| "DIRECTORY_UNAVAILABLE")?.path());
            }
        } else {
            reject_multiple_links(&path)?;
        }
        paths.push(path);
    }
    restrict_to_current_user(&paths)?;
    if !marker.exists() {
        atomic_json(
            &marker,
            &json!({"schemaVersion":1,"owner":"CodeCraft.Cursor"}),
            false,
        )?;
    }
    Ok(root)
}

// Applying a DACL to an external file through a hard link would modify that
// file's security too. Inventory must reject such files before touching ACLs.
fn reject_multiple_links(path: &Path) -> Result<()> {
    let file = File::open(path).map_err(|_| "FILE_UNREADABLE")?;
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        };
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err("FILE_METADATA_FAILED");
        }
        if info.nNumberOfLinks > 1 {
            return Err("HARD_LINK_REJECTED");
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if file.metadata().map_err(|_| "FILE_METADATA_FAILED")?.nlink() > 1 {
            return Err("HARD_LINK_REJECTED");
        }
    }
    Ok(())
}

#[cfg(windows)]
fn restrict_to_current_user(paths: &[PathBuf]) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
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
    unsafe {
        let mut token = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err("USER_TOKEN_UNAVAILABLE");
        }
        let mut length = 0;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut length);
        let mut data = vec![0usize; (length as usize).div_ceil(std::mem::size_of::<usize>())];
        let ok = GetTokenInformation(
            token,
            TokenUser,
            data.as_mut_ptr().cast(),
            length,
            &mut length,
        );
        CloseHandle(token);
        if ok == 0 {
            return Err("USER_SID_UNAVAILABLE");
        }
        let mut sid = std::ptr::null_mut();
        if ConvertSidToStringSidW((*data.as_ptr().cast::<TOKEN_USER>()).User.Sid, &mut sid) == 0 {
            return Err("USER_SID_UNAVAILABLE");
        }
        let _sid = Local(sid.cast());
        let mut length = 0;
        while *sid.add(length) != 0 {
            length += 1;
        }
        let sid_text = String::from_utf16_lossy(std::slice::from_raw_parts(sid, length));
        let sddl = format!("D:P(A;OICI;FA;;;{sid_text})")
            .encode_utf16()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let mut descriptor = std::ptr::null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            std::ptr::null_mut(),
        ) == 0
        {
            return Err("ACL_CREATE_FAILED");
        }
        let _descriptor = Local(descriptor);
        let mut acl = std::ptr::null_mut();
        let mut present = 0;
        let mut defaulted = 0;
        if GetSecurityDescriptorDacl(descriptor, &mut present, &mut acl, &mut defaulted) == 0
            || present == 0
            || acl.is_null()
        {
            return Err("ACL_CREATE_FAILED");
        }
        for path in paths {
            reject_link(path)?;
            let wide = path
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>();
            if SetNamedSecurityInfoW(
                wide.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                acl,
                std::ptr::null_mut(),
            ) != 0
            {
                return Err("ACL_APPLY_FAILED");
            }
        }
    }
    Ok(())
}
#[cfg(not(windows))]
fn restrict_to_current_user(paths: &[PathBuf]) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    for path in paths {
        fs::set_permissions(
            path,
            fs::Permissions::from_mode(if path.is_dir() { 0o700 } else { 0o600 }),
        )
        .map_err(|_| "ACL_APPLY_FAILED")?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn root() -> PathBuf {
        let parent = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("runs");
        fs::create_dir_all(&parent).unwrap();
        private_directory(&parent.join(format!("files-{}", Uuid::new_v4()))).unwrap()
    }
    #[cfg(windows)]
    pub(crate) fn junction(link: &Path, target: &Path) {
        use std::os::windows::process::CommandExt;
        let link_text = link.to_str().unwrap();
        let target_text = target.to_str().unwrap();
        let output = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", "New-Item -ItemType Junction -Path $env:CODECRAFT_TEST_LINK -Target $env:CODECRAFT_TEST_TARGET -ErrorAction Stop | Out-Null"])
            .env("CODECRAFT_TEST_LINK", link_text.strip_prefix(r"\\?\").unwrap_or(link_text)).env("CODECRAFT_TEST_TARGET", target_text.strip_prefix(r"\\?\").unwrap_or(target_text)).creation_flags(0x08000000).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(link.is_dir());
    }
    #[test]
    fn an_owned_directory_cannot_change_external_file_permissions_through_a_hard_link() {
        let root = root();
        let owned = private_directory(&root.join("owned")).unwrap();
        let external = root.join("external.txt");
        fs::write(&external, "keep").unwrap();
        fs::hard_link(&external, owned.join("linked.txt")).unwrap();
        assert_eq!(private_directory(&owned), Err("HARD_LINK_REJECTED"));
        assert_eq!(fs::read_to_string(external).unwrap(), "keep");
    }
    #[cfg(windows)]
    #[test]
    fn junction_ancestors_cannot_redirect_a_private_directory_or_child() {
        let root = root();
        let actual = private_directory(&root.join("actual")).unwrap();
        let link = root.join("link");
        junction(&link, &actual);
        assert_eq!(private_directory(&link), Err("REPARSE_POINT_REJECTED"));
        assert_eq!(
            private_directory(&link.join("nested")),
            Err("REPARSE_POINT_REJECTED")
        );
        assert_eq!(
            child(&root, "link/message.json"),
            Err("REPARSE_POINT_REJECTED")
        );
    }
    #[test]
    fn unowned_content_and_escaping_names_are_rejected_without_modification() {
        let root = root();
        let unowned = root.join("unowned");
        fs::create_dir(&unowned).unwrap();
        fs::write(unowned.join("user.txt"), "keep").unwrap();
        assert_eq!(private_directory(&unowned), Err("DIRECTORY_NOT_OWNED"));
        assert_eq!(
            fs::read_to_string(unowned.join("user.txt")).unwrap(),
            "keep"
        );
        for name in [
            "../outside",
            "file:stream",
            "name.",
            "C:/outside",
            "",
            "/outside",
        ] {
            assert!(child(&root, name).is_err(), "{name}");
        }
    }
    #[test]
    fn atomic_publication_does_not_overwrite_and_locks_are_exclusive() {
        let root = root();
        let path = child(&root, "message.json").unwrap();
        atomic_json(&path, &json!({"first":true}), false).unwrap();
        assert!(atomic_json(&path, &json!({"second":true}), false).is_err());
        assert_eq!(read_json(&path).unwrap(), json!({"first":true}));
        atomic_json(&path, &json!({"second":true}), true).unwrap();
        assert_eq!(read_json(&path).unwrap(), json!({"second":true}));
        let lock_path = root.join("test.lock");
        let guard = lock(&lock_path).unwrap();
        assert!(lock(&lock_path).is_err());
        drop(guard);
        assert!(lock(&lock_path).is_ok());
    }
    #[cfg(windows)]
    #[test]
    fn transient_windows_file_sharing_does_not_disconnect_the_channel() {
        use std::{os::windows::fs::OpenOptionsExt, thread, time::Duration};
        use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};
        let root = root();
        let path = root.join("heartbeat.json");
        atomic_json(&path, &json!({"sequence":1}), false).unwrap();
        // A scanner can briefly hold the heartbeat without delete sharing.
        let reader = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(&path)
            .unwrap();
        let release = thread::spawn(move || {
            thread::sleep(Duration::from_millis(60));
            drop(reader);
        });
        let replaced = atomic_json(&path, &json!({"sequence":2}), true);
        release.join().unwrap();
        assert_eq!(replaced, Ok(()));
        assert_eq!(read_json(&path).unwrap(), json!({"sequence":2}));
        let exclusive = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        let release = thread::spawn(move || {
            thread::sleep(Duration::from_millis(60));
            drop(exclusive);
        });
        let read = read_json(&path);
        release.join().unwrap();
        assert_eq!(read.unwrap(), json!({"sequence":2}));
    }
    #[cfg(windows)]
    #[test]
    fn persistent_windows_sharing_failure_preserves_the_old_file() {
        use std::{
            os::windows::fs::OpenOptionsExt,
            time::{Duration, Instant},
        };
        let root = root();
        let path = root.join("heartbeat.json");
        atomic_json(&path, &json!({"sequence":1}), false).unwrap();
        let exclusive = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        let started = Instant::now();
        assert_eq!(
            atomic_json(&path, &json!({"sequence":2}), true),
            Err("FILE_REPLACE_FAILED")
        );
        assert_eq!(read_json(&path), Err("FILE_UNREADABLE"));
        assert!(started.elapsed() < Duration::from_secs(3));
        drop(exclusive);
        assert_eq!(read_json(&path).unwrap(), json!({"sequence":1}));
        assert!(!fs::read_dir(&root).unwrap().any(|entry| entry
            .unwrap()
            .path()
            .extension()
            .is_some_and(|ext| ext == "tmp")));
    }
    #[cfg(windows)]
    #[test]
    fn root_and_new_children_have_a_single_current_user_acl() {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::{
            Foundation::LocalFree,
            Security::{
                Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT},
                GetSecurityDescriptorControl, DACL_SECURITY_INFORMATION, SE_DACL_PROTECTED,
            },
        };
        let root = root();
        let file = root.join("private.json");
        atomic_json(&file, &json!({"test":true}), false).unwrap();
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
    }
}
