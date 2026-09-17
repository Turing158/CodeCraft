//! Bounded, recoverable file operations used only by the WorkBuddy integration.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub(crate) const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

pub(crate) fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    reject_links(path)?;
    let mut data = Vec::new();
    File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit + 1)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() as u64 > limit {
        return Err("WorkBuddy 文件超过大小限制".into());
    }
    Ok(data)
}

pub(crate) fn reject_links(path: &Path) -> Result<(), String> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(meta) => {
                #[cfg(windows)]
                let linked = {
                    use std::os::windows::fs::MetadataExt;
                    meta.file_attributes() & 0x400 != 0
                };
                #[cfg(not(windows))]
                let linked = meta.file_type().is_symlink();
                if linked {
                    return Err("WorkBuddy 路径包含符号链接或 junction".into());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}

pub(crate) fn lock(directory: &Path) -> Result<File, String> {
    reject_links(directory)?;
    fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let path = directory.join(".codecraft-workbuddy.lock");
    reject_links(&path)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.try_lock()
        .map_err(|_| "另一个 CodeCraft 实例正在修改 WorkBuddy 配置".to_string())?;
    Ok(file)
}

pub(crate) fn backup(path: &Path, bytes: &[u8]) -> Result<PathBuf, String> {
    let name = path
        .file_name()
        .ok_or("WorkBuddy 备份目标无文件名")?
        .to_string_lossy();
    let digest = format!("{:x}", Sha256::digest(bytes));
    for suffix in [".bak".to_string(), format!(".{}.bak", &digest[..16])] {
        let target = path.with_file_name(format!("{name}{suffix}"));
        reject_links(&target)?;
        match OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&target)
        {
            Ok(mut file) => {
                file.write_all(bytes)
                    .and_then(|_| file.sync_all())
                    .map_err(|e| e.to_string())?;
                return Ok(target);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if read_bounded(&target, MAX_CONFIG_BYTES)? == bytes {
                    return Ok(target);
                }
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    Err("WorkBuddy 备份 hash 冲突；原备份已保留".into())
}

pub(crate) fn atomic_write(path: &Path, bytes: &[u8], keep_backup: bool) -> Result<(), String> {
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err("WorkBuddy 写入超过大小限制".into());
    }
    reject_links(path)?;
    let parent = path.parent().ok_or("WorkBuddy 文件缺少父目录")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    if path.exists() {
        let old = read_bounded(path, MAX_CONFIG_BYTES)?;
        if old == bytes {
            return Ok(());
        }
        if keep_backup {
            backup(path, &old)?;
        }
    }
    let temporary = parent.join(format!(".codecraft-{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        drop(file);
        replace(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(windows)]
fn replace(source: &Path, destination: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        core::PCWSTR,
        Win32::Storage::FileSystem::{
            MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        },
    };
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    unsafe {
        MoveFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(destination.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|e| e.to_string())
}

#[cfg(not(windows))]
fn replace(source: &Path, destination: &Path) -> Result<(), String> {
    fs::rename(source, destination).map_err(|e| e.to_string())
}

pub(crate) struct Change {
    pub path: PathBuf,
    pub before: Option<Vec<u8>>,
    pub after: Option<Vec<u8>>,
}

impl Change {
    pub(crate) fn capture(path: PathBuf, after: Option<Vec<u8>>) -> Result<Self, String> {
        reject_links(&path)?;
        let before = if path.exists() {
            Some(read_bounded(&path, MAX_CONFIG_BYTES)?)
        } else {
            None
        };
        Ok(Self {
            path,
            before,
            after,
        })
    }
    fn current(&self) -> Result<Option<Vec<u8>>, String> {
        if self.path.exists() {
            read_bounded(&self.path, MAX_CONFIG_BYTES).map(Some)
        } else {
            Ok(None)
        }
    }
}

/// Call with the installation lock held. Preflight and back up the entire edit
/// before touching any managed file. Rollback never overwrites a concurrent edit.
pub(crate) fn transaction(changes: Vec<Change>) -> Result<(), String> {
    let changes: Vec<_> = changes
        .into_iter()
        .filter(|c| c.before != c.after)
        .collect();
    for change in &changes {
        if change.current()? != change.before {
            return Err("WorkBuddy 文件已被其他进程修改".into());
        }
        if let Some(before) = &change.before {
            backup(&change.path, before)?;
        }
    }
    for (index, change) in changes.iter().enumerate() {
        let result = (|| {
            if change.current()? != change.before {
                return Err("WorkBuddy 文件 hash 冲突".to_string());
            }
            match &change.after {
                Some(bytes) => atomic_write(&change.path, bytes, false),
                None => fs::remove_file(&change.path).map_err(|e| e.to_string()),
            }
        })();
        if let Err(error) = result {
            let mut rollback_conflict = false;
            for done in changes[..index].iter().rev() {
                if done.current().ok().as_ref() != Some(&done.after) {
                    rollback_conflict = true;
                    continue;
                }
                let result = match &done.before {
                    Some(bytes) => atomic_write(&done.path, bytes, false),
                    None => fs::remove_file(&done.path).map_err(|e| e.to_string()),
                };
                rollback_conflict |= result.is_err();
            }
            return Err(format!(
                "{error}；{}",
                if rollback_conflict {
                    "恢复存在冲突，请使用保留的 .bak 备份"
                } else {
                    "已恢复本次修改，.bak 备份保留"
                }
            ));
        }
    }
    Ok(())
}

/// Parse JSONC without changing string contents (URLs, escaped quotes, etc.).
pub(crate) fn parse_jsonc(bytes: &[u8]) -> Result<Value, String> {
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let mut clean = bytes.to_vec();
    let mut i = 0;
    let mut string = false;
    while i < clean.len() {
        if string {
            if clean[i] == b'\\' {
                i += 2;
                continue;
            }
            if clean[i] == b'"' {
                string = false;
            }
        } else if clean[i] == b'"' {
            string = true;
        } else if clean[i..].starts_with(b"//") {
            while i < clean.len() && clean[i] != b'\n' {
                clean[i] = b' ';
                i += 1;
            }
            continue;
        } else if clean[i..].starts_with(b"/*") {
            clean[i] = b' ';
            clean[i + 1] = b' ';
            i += 2;
            while i + 1 < clean.len() && !clean[i..].starts_with(b"*/") {
                clean[i] = b' ';
                i += 1;
            }
            if i + 1 >= clean.len() {
                return Err("WorkBuddy JSONC 注释未闭合".into());
            }
            clean[i] = b' ';
            clean[i + 1] = b' ';
            i += 2;
            continue;
        }
        i += 1;
    }
    i = 0;
    string = false;
    while i < clean.len() {
        if string {
            if clean[i] == b'\\' {
                i += 2;
                continue;
            }
            if clean[i] == b'"' {
                string = false;
            }
        } else if clean[i] == b'"' {
            string = true;
        } else if clean[i] == b',' {
            let follows_value = clean[..i]
                .iter()
                .rev()
                .find(|b| !b.is_ascii_whitespace())
                .is_some_and(|b| !matches!(b, b'[' | b'{' | b':' | b','));
            if follows_value
                && clean[i + 1..]
                    .iter()
                    .find(|b| !b.is_ascii_whitespace())
                    .is_some_and(|b| matches!(b, b'}' | b']'))
            {
                clean[i] = b' ';
            }
        }
        i += 1;
    }
    serde_json::from_slice(&clean).map_err(|e| format!("WorkBuddy settings JSON/JSONC 无效：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .join(format!("workbuddy-files-{}", Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn jsonc_preserves_strings_and_rejects_unclosed_comments() {
        let value = parse_jsonc(
            br#"{/* note */ "url":"https://x/?a=1", "items":[1,], // note
        "unknown":{"keep":true,},}"#,
        )
        .unwrap();
        assert_eq!(value["url"], "https://x/?a=1");
        assert_eq!(value["unknown"]["keep"], true);
        assert!(parse_jsonc(b"{/* unfinished").is_err());
        assert!(parse_jsonc(b"{,}").is_err());
        assert!(parse_jsonc(b"[,]").is_err());
    }
    #[test]
    fn upgrades_preserve_the_first_backup_and_every_distinct_version() {
        let root = Scratch::new();
        let path = root.0.join("settings.json");
        atomic_write(&path, b"one", true).unwrap();
        atomic_write(&path, b"two", true).unwrap();
        atomic_write(&path, b"three", true).unwrap();
        assert_eq!(fs::read(root.0.join("settings.json.bak")).unwrap(), b"one");
        assert_eq!(fs::read(&path).unwrap(), b"three");
        assert_eq!(fs::read_dir(&root.0).unwrap().count(), 3);
    }
    #[test]
    fn concurrent_edits_are_not_overwritten() {
        let root = Scratch::new();
        let path = root.0.join("settings.json");
        atomic_write(&path, b"one", false).unwrap();
        let change = Change::capture(path.clone(), Some(b"two".to_vec())).unwrap();
        atomic_write(&path, b"user edit", false).unwrap();
        assert!(transaction(vec![change]).is_err());
        assert_eq!(fs::read(path).unwrap(), b"user edit");
    }
    #[test]
    fn configuration_lock_excludes_another_instance() {
        let root = Scratch::new();
        let first = lock(&root.0).unwrap();
        assert!(lock(&root.0).is_err());
        drop(first);
        assert!(lock(&root.0).is_ok());
    }
}
