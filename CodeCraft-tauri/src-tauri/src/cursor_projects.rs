//! Filesystem identity for locally selected projects. Reported paths must come
//! from a verified native event, never from model arguments or the MCP cwd.
use crate::{protocol, schema::Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Identity {
    volume_guid: String,
    volume: u64,
    file_id: [u8; 16],
    created: u64,
}
impl Identity {
    pub fn key(&self) -> String {
        let mut bytes = self.volume.to_be_bytes().to_vec();
        bytes.extend_from_slice(&self.file_id);
        bytes.extend_from_slice(&self.created.to_be_bytes());
        bytes.extend_from_slice(self.volume_guid.as_bytes());
        protocol::sha256(&bytes)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Directory {
    pub final_path: PathBuf,
    pub identity: Identity,
}

fn reported_path(text: &str) -> Result<PathBuf> {
    if text.len() > 32760 || text.contains('\0') {
        return Err("PROJECT_SCOPE_UNVERIFIED");
    }
    // Cursor's native Windows events have been observed using /D:/... . Do
    // not URI-decode arbitrary strings, expand variables, or accept file: URLs.
    #[cfg(windows)]
    let text = if text.as_bytes().first() == Some(&b'/')
        && text.as_bytes().get(1).is_some_and(u8::is_ascii_alphabetic)
        && text.as_bytes().get(2) == Some(&b':')
        && matches!(text.as_bytes().get(3), Some(b'/' | b'\\'))
    {
        &text[1..]
    } else {
        text
    };
    let path = PathBuf::from(text);
    if !path.is_absolute() {
        return Err("PROJECT_SCOPE_UNVERIFIED");
    }
    Ok(path)
}

pub fn resolve_reported(text: &str) -> Result<Directory> {
    resolve(&reported_path(text)?)
}

#[cfg(windows)]
pub fn resolve(path: &Path) -> Result<Directory> {
    use std::{
        fs::File,
        os::windows::{
            ffi::{OsStrExt, OsStringExt},
            io::{AsRawHandle, FromRawHandle},
        },
    };
    use windows_sys::Win32::{
        Foundation::INVALID_HANDLE_VALUE,
        Storage::FileSystem::{
            CreateFileW, FileIdInfo, GetDriveTypeW, GetFileInformationByHandle,
            GetFileInformationByHandleEx, GetFinalPathNameByHandleW, BY_HANDLE_FILE_INFORMATION,
            FILE_ATTRIBUTE_DIRECTORY, FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_INFO,
            FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
            OPEN_EXISTING, VOLUME_NAME_GUID,
        },
    };
    if !path.is_absolute() {
        return Err("PROJECT_SCOPE_UNVERIFIED");
    }
    // Local Windows only. Reject UNC/device namespaces before opening them,
    // and mapped network drives before querying their directory handles.
    let drive = match path.components().next() {
        Some(std::path::Component::Prefix(prefix)) => match prefix.kind() {
            std::path::Prefix::Disk(letter) | std::path::Prefix::VerbatimDisk(letter) => letter,
            _ => return Err("PROJECT_SCOPE_UNVERIFIED"),
        },
        _ => return Err("PROJECT_SCOPE_UNVERIFIED"),
    };
    let drive_root = [drive as u16, b':' as u16, b'\\' as u16, 0];
    if !matches!(unsafe { GetDriveTypeW(drive_root.as_ptr()) }, 2 | 3 | 6) {
        return Err("PROJECT_SCOPE_UNVERIFIED");
    }
    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    if wide[..wide.len() - 1].contains(&0) {
        return Err("PROJECT_SCOPE_UNVERIFIED");
    }
    unsafe {
        let handle = CreateFileW(
            wide.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        );
        if handle == INVALID_HANDLE_VALUE {
            return Err("PROJECT_SCOPE_UNVERIFIED");
        }
        let file = File::from_raw_handle(handle);
        let handle = file.as_raw_handle();
        let mut metadata: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
        let mut identity: FILE_ID_INFO = std::mem::zeroed();
        if GetFileInformationByHandle(handle, &mut metadata) == 0
            || metadata.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0
            || GetFileInformationByHandleEx(
                handle,
                FileIdInfo,
                (&mut identity as *mut FILE_ID_INFO).cast(),
                std::mem::size_of::<FILE_ID_INFO>() as u32,
            ) == 0
        {
            return Err("PROJECT_SCOPE_UNVERIFIED");
        }
        let length = GetFinalPathNameByHandleW(handle, std::ptr::null_mut(), 0, 0);
        if length == 0 || length > 32768 {
            return Err("PROJECT_SCOPE_UNVERIFIED");
        }
        let mut name = vec![0u16; length as usize + 1];
        let count = GetFinalPathNameByHandleW(handle, name.as_mut_ptr(), name.len() as u32, 0);
        if count == 0 || count as usize >= name.len() {
            return Err("PROJECT_SCOPE_UNVERIFIED");
        }
        let guid_length =
            GetFinalPathNameByHandleW(handle, std::ptr::null_mut(), 0, VOLUME_NAME_GUID);
        if guid_length == 0 || guid_length > 32768 {
            return Err("PROJECT_SCOPE_UNVERIFIED");
        }
        let mut guid_name = vec![0u16; guid_length as usize + 1];
        let guid_count = GetFinalPathNameByHandleW(
            handle,
            guid_name.as_mut_ptr(),
            guid_name.len() as u32,
            VOLUME_NAME_GUID,
        );
        if guid_count == 0 || guid_count as usize >= guid_name.len() {
            return Err("PROJECT_SCOPE_UNVERIFIED");
        }
        let guid_text = String::from_utf16(&guid_name[..guid_count as usize])
            .map_err(|_| "PROJECT_SCOPE_UNVERIFIED")?;
        let guid_end = guid_text.find("}\\").ok_or("PROJECT_SCOPE_UNVERIFIED")?;
        if !guid_text.starts_with(r"\\?\Volume{") {
            return Err("PROJECT_SCOPE_UNVERIFIED");
        }
        Ok(Directory {
            final_path: PathBuf::from(std::ffi::OsString::from_wide(&name[..count as usize])),
            identity: Identity {
                volume_guid: guid_text[..guid_end + 1].to_ascii_lowercase(),
                volume: identity.VolumeSerialNumber,
                file_id: identity.FileId.Identifier,
                created: ((metadata.ftCreationTime.dwHighDateTime as u64) << 32)
                    | metadata.ftCreationTime.dwLowDateTime as u64,
            },
        })
    }
}
#[cfg(not(windows))]
pub fn resolve(_path: &Path) -> Result<Directory> {
    Err("WINDOWS_REQUIRED")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Project {
    pub directory: Directory,
    pub revision: u64,
    pub enabled: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Registry(BTreeMap<String, Project>);

/// Persisted snapshot. Deserialization alone is never authorization: capture
/// and revalidate reopen the directories and compare the backend registry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Workspace {
    roots: Vec<Directory>,
    cwd: Directory,
    revisions: BTreeMap<String, u64>,
}
impl Workspace {
    pub fn revisions(&self) -> &BTreeMap<String, u64> {
        &self.revisions
    }
}

impl Registry {
    /// Called only by a successful desktop installation/settings transaction.
    /// Persistence and model project revisions must commit together in broker.
    pub fn set(&mut self, path: &Path, enabled: bool) -> Result<(String, u64)> {
        let directory = resolve(path)?;
        let key = directory.identity.key();
        let revision = match self.0.get(&key) {
            Some(old) if old.enabled == enabled => old.revision,
            Some(old) => old
                .revision
                .checked_add(1)
                .ok_or("PROJECT_REVISION_OVERFLOW")?,
            None => 1,
        };
        self.0.insert(
            key.clone(),
            Project {
                directory,
                revision,
                enabled,
            },
        );
        Ok((key, revision))
    }
    pub fn project(&self, key: &str) -> Option<&Project> {
        self.0.get(key)
    }
    pub fn entries(&self) -> impl Iterator<Item = (&String, &Project)> {
        self.0.iter()
    }
    pub(crate) fn disable(&mut self, key: &str) -> Result<()> {
        let project = self.0.get_mut(key).ok_or("PROJECT_NOT_FOUND")?;
        if project.enabled {
            project.enabled = false;
            project.revision = project
                .revision
                .checked_add(1)
                .ok_or("PROJECT_REVISION_OVERFLOW")?;
        }
        Ok(())
    }

    pub fn capture(&self, roots: &[String], cwd: &str) -> Result<Workspace> {
        if roots.is_empty() || roots.len() > 64 {
            return Err("PROJECT_SCOPE_UNVERIFIED");
        }
        let cwd = resolve_reported(cwd)?;
        let roots = roots
            .iter()
            .map(|root| resolve_reported(root))
            .collect::<Result<Vec<_>>>()?;
        let mut revisions = BTreeMap::new();
        for root in &roots {
            let key = root.identity.key();
            let project = self.0.get(&key).ok_or("PROJECT_NOT_ENABLED")?;
            if !project.enabled || project.directory.identity != root.identity {
                return Err("PROJECT_NOT_ENABLED");
            }
            revisions.insert(key, project.revision);
        }
        // Compare directory identities along the resolved cwd ancestry, never
        // a textual prefix (foo and foobar are separate directories).
        let mut inside = false;
        for ancestor in cwd.final_path.ancestors() {
            let identity = resolve(ancestor)?.identity;
            if roots.iter().any(|root| root.identity == identity) {
                inside = true;
                break;
            }
        }
        if !inside {
            return Err("PROJECT_SCOPE_UNVERIFIED");
        }
        Ok(Workspace {
            roots,
            cwd,
            revisions,
        })
    }
    /// Called at activation, consumption and human submission with fresh
    /// native roots. Also reopens every path, catching replaced directories
    /// and changed junction targets even without a configuration revision.
    pub fn revalidate(&self, workspace: &Workspace, roots: &[String], cwd: &str) -> Result<()> {
        let current = self.capture(roots, cwd)?;
        if current.revisions != workspace.revisions
            || current.cwd.identity != workspace.cwd.identity
        {
            return Err("PROJECT_SCOPE_CHANGED");
        }
        for root in &workspace.roots {
            let key = root.identity.key();
            if !current
                .roots
                .iter()
                .any(|value| value.identity.key() == key)
            {
                return Err("PROJECT_SCOPE_CHANGED");
            }
        }
        Ok(())
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::fs;
    fn root() -> PathBuf {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("runs")
            .join(format!("projects-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        root
    }
    fn text(path: &Path) -> String {
        path.to_str().unwrap().to_string()
    }
    #[test]
    fn changing_a_workspace_junction_target_cannot_reuse_the_previous_project_scope() {
        let root = root();
        let a = root.join("a");
        let b = root.join("b");
        let link = root.join("workspace");
        fs::create_dir(&a).unwrap();
        fs::create_dir(&b).unwrap();
        crate::files::tests::junction(&link, &a);
        let mut registry = Registry::default();
        registry.set(&a, true).unwrap();
        registry.set(&b, true).unwrap();
        let roots = [text(&link)];
        let workspace = registry.capture(&roots, &text(&link)).unwrap();
        fs::remove_dir(&link).unwrap();
        crate::files::tests::junction(&link, &b);
        assert_eq!(
            registry.revalidate(&workspace, &roots, &text(&link)),
            Err("PROJECT_SCOPE_CHANGED")
        );
    }
    #[test]
    fn rename_preserves_identity_but_replacement_at_the_same_path_does_not() {
        let root = root();
        let original = root.join("original");
        fs::create_dir(&original).unwrap();
        let mut registry = Registry::default();
        let (key, revision) = registry.set(&original, true).unwrap();
        let first = registry
            .capture(&[text(&original)], &text(&original))
            .unwrap();
        let renamed = root.join("renamed");
        fs::rename(&original, &renamed).unwrap();
        assert_eq!(registry.set(&renamed, true).unwrap(), (key, revision));
        registry
            .revalidate(&first, &[text(&renamed)], &text(&renamed))
            .unwrap();
        fs::create_dir(&original).unwrap();
        assert_eq!(
            registry.capture(&[text(&original)], &text(&original)),
            Err("PROJECT_NOT_ENABLED")
        );
    }
    #[test]
    fn every_reported_root_and_revision_are_required_and_cwd_uses_identity_ancestry() {
        let root = root();
        let a = root.join("a");
        let b = root.join("ab");
        let sub = a.join("sub");
        fs::create_dir_all(&sub).unwrap();
        fs::create_dir(&b).unwrap();
        let mut registry = Registry::default();
        registry.set(&a, true).unwrap();
        assert!(registry.capture(&[text(&a)], &text(&b)).is_err());
        assert!(registry
            .capture(&[text(&a), text(&b)], &text(&sub))
            .is_err());
        registry.set(&b, true).unwrap();
        let roots = [text(&a), text(&b)];
        let workspace = registry.capture(&roots, &text(&sub)).unwrap();
        registry.set(&b, false).unwrap();
        registry.set(&b, true).unwrap();
        assert_eq!(
            registry.revalidate(&workspace, &roots, &text(&sub)),
            Err("PROJECT_SCOPE_CHANGED")
        );
    }
    #[test]
    fn cursor_drive_prefix_and_case_resolve_but_relative_and_file_paths_fail() {
        let root = root();
        let native = resolve(&root).unwrap();
        assert_eq!(
            resolve_reported(&format!("/{}", text(&root).replace('\\', "/")))
                .unwrap()
                .identity,
            native.identity
        );
        assert_eq!(
            resolve_reported(&text(&root).to_uppercase())
                .unwrap()
                .identity,
            native.identity
        );
        for path in ["relative", "C:relative", "file:///C:/", "C:/bad\0name"] {
            assert!(resolve_reported(path).is_err());
        }
        let file = root.join("file");
        fs::write(&file, "x").unwrap();
        assert!(resolve(&file).is_err());
    }
}
