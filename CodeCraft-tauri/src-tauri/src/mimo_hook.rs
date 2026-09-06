use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::{self, Command, Stdio},
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
use windows::Win32::{
    Foundation::CloseHandle,
    System::Threading::{OpenProcess, SetProcessAffinityMask, PROCESS_SET_INFORMATION},
};

use super::approval_policy::{self, ApprovalMode};

pub(crate) const PROTOCOL_VERSION: &str = "0.4";
pub(crate) const PLUGIN_VERSION: &str = "0.4.9";
const OWNER: &str = "CodeCraft";
const PLUGIN_FILE_NAME: &str = "codecraft-mimo-plugin.js";
const MANIFEST_FILE_NAME: &str = ".codecraft-mimo-plugin.json";
const BUNDLED_PLUGIN: &str = include_str!("../mimo-plugin/codecraft-mimo-plugin.js");
const INSTANCE_STALE_MS: u64 = 20_000;
const RUNNING_VERSION_CACHE_TTL_MS: u64 = 2_000;
static RUNNING_VERSION_CACHE: OnceLock<Mutex<Option<(std::time::Instant, Vec<String>)>>> =
    OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum MimoHookInstallState {
    NotInstalled,
    Installed,
    SyncedRestartRequired,
    Modified,
    Conflict,
    Incompatible,
    Error,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MimoHookStatus {
    pub(crate) state: MimoHookInstallState,
    pub(crate) install_path: String,
    pub(crate) bundled_version: &'static str,
    pub(crate) installed_version: Option<String>,
    pub(crate) running_versions: Vec<String>,
    pub(crate) error: Option<String>,
}

impl MimoHookStatus {
    pub(crate) fn installed(&self) -> bool {
        matches!(
            self.state,
            MimoHookInstallState::Installed | MimoHookInstallState::SyncedRestartRequired
        )
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManagedFile {
    name: String,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct InstallManifest {
    owner: String,
    install_id: String,
    plugin_version: String,
    protocol_version: String,
    files: Vec<ManagedFile>,
    installed_at: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstanceSnapshot {
    protocol_version: String,
    plugin_version: String,
    heartbeat: u64,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn sha256(bytes: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update(bytes);
    format!("{:x}", hash.finalize())
}

fn base_data_dir() -> PathBuf {
    if let Some(local) = env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local).join("CodeCraft");
    }
    if let Some(home) = env::var_os("USERPROFILE").or_else(|| env::var_os("HOME")) {
        return PathBuf::from(home).join(".codecraft");
    }
    PathBuf::from(".")
}

pub(crate) fn hook_dir() -> PathBuf {
    base_data_dir().join("mimo-hook")
}

fn control_path() -> PathBuf {
    hook_dir().join("control").join("enabled.json")
}

pub(crate) fn instances_dir() -> PathBuf {
    hook_dir().join("instances")
}

pub(crate) fn inbox_dir() -> PathBuf {
    hook_dir().join("inbox")
}

pub(crate) fn outbox_dir() -> PathBuf {
    hook_dir().join("outbox")
}

pub(crate) fn write_outbox_decision(value: &serde_json::Value) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    if bytes.len() > 256 * 1024 {
        return Err("Mimo decision envelope exceeded its size limit".to_string());
    }
    let name = format!("{:020}-{}.json", now_ms(), Uuid::new_v4());
    atomic_write(&outbox_dir().join(name), &bytes)
}

fn global_config_dir() -> Result<PathBuf, String> {
    if let Some(override_path) = env::var_os("MIMOCODE_CONFIG_DIR") {
        let path = PathBuf::from(override_path);
        if !path.is_absolute() {
            return Err("MIMOCODE_CONFIG_DIR must be an absolute path".to_string());
        }
        return Ok(path);
    }
    let home = env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
        .ok_or_else(|| "Unable to locate the user profile for Mimo".to_string())?;
    Ok(home.join(".config").join("mimocode"))
}

pub(crate) fn plugin_dir() -> Result<PathBuf, String> {
    Ok(global_config_dir()?.join("plugin"))
}

fn plugin_path(directory: &Path) -> PathBuf {
    directory.join(PLUGIN_FILE_NAME)
}

fn manifest_path(directory: &Path) -> PathBuf {
    directory.join(MANIFEST_FILE_NAME)
}

fn read_manifest(directory: &Path) -> Result<Option<InstallManifest>, String> {
    let path = manifest_path(directory);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(&path).map_err(|error| error.to_string())?;
    let manifest = serde_json::from_slice::<InstallManifest>(&bytes)
        .map_err(|error| format!("Invalid Mimo plugin ownership manifest: {error}"))?;
    Ok(Some(manifest))
}

fn validate_manifest(manifest: &InstallManifest) -> Result<&ManagedFile, String> {
    if manifest.owner != OWNER {
        return Err("The Mimo plugin manifest is not owned by CodeCraft".to_string());
    }
    manifest
        .files
        .iter()
        .find(|file| file.name == PLUGIN_FILE_NAME)
        .ok_or_else(|| {
            "The Mimo plugin manifest does not declare the managed plugin".to_string()
        })
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Mimo plugin target has no parent directory".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = parent.join(format!(
        ".{}.{}-{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("codecraft"),
        process::id(),
        Uuid::new_v4()
    ));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    file.write_all(bytes).map_err(|error| error.to_string())?;
    file.flush().map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    drop(file);

    if let Err(error) = replace_file(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(())
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;

    use windows::{
        core::PCWSTR,
        Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING},
    };

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    unsafe {
        MoveFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(destination.as_ptr()),
            MOVEFILE_REPLACE_EXISTING,
        )
    }
    .map_err(|error| error.to_string())
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> Result<(), String> {
    fs::rename(source, destination).map_err(|error| error.to_string())
}

fn write_control_with_mode(
    enabled: bool,
    generation: &str,
    approval_mode: ApprovalMode,
) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(&json!({
        "protocolVersion": PROTOCOL_VERSION,
        "enabled": enabled,
        "approvalMode": approval_mode,
        "generation": generation,
        "updatedAt": now_ms()
    }))
    .map_err(|error| error.to_string())?;
    atomic_write(&control_path(), &bytes)
}

fn write_control(enabled: bool, generation: &str) -> Result<(), String> {
    write_control_with_mode(enabled, generation, approval_policy::load_settings().mode)
}

pub(crate) fn sync_approval_mode(approval_mode: ApprovalMode) -> Result<(), String> {
    let current = fs::read(control_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
    let enabled = current
        .as_ref()
        .and_then(|value| value.get("enabled"))
        .and_then(Value::as_bool)
        .unwrap_or_else(|| {
            plugin_dir()
                .ok()
                .map(|directory| status_at(&directory).installed())
                .unwrap_or(false)
        });
    let generation = current
        .as_ref()
        .and_then(|value| value.get("generation"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    write_control_with_mode(enabled, &generation, approval_mode)
}

fn current_running_versions() -> Vec<String> {
    let cache = RUNNING_VERSION_CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(cache) = cache.lock() {
        if let Some((created_at, versions)) = cache.as_ref() {
            if created_at.elapsed().as_millis() <= RUNNING_VERSION_CACHE_TTL_MS as u128 {
                return versions.clone();
            }
        }
    }
    let now = now_ms();
    let mut versions = Vec::new();
    let Ok(entries) = fs::read_dir(instances_dir()) else {
        return versions;
    };
    for entry in entries.flatten().take(128) {
        let Ok(bytes) = fs::read(entry.path()) else {
            continue;
        };
        let Ok(snapshot) = serde_json::from_slice::<InstanceSnapshot>(&bytes) else {
            continue;
        };
        if snapshot.protocol_version != PROTOCOL_VERSION
            || now.saturating_sub(snapshot.heartbeat) > INSTANCE_STALE_MS
        {
            continue;
        }
        if !versions.contains(&snapshot.plugin_version) {
            versions.push(snapshot.plugin_version);
        }
    }
    versions.sort();
    if let Ok(mut cache) = cache.lock() {
        *cache = Some((std::time::Instant::now(), versions.clone()));
    }
    versions
}

fn normalize_plugin_path(value: &str) -> String {
    value
        .trim()
        .trim_start_matches("file://")
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

fn plugin_entry_path(value: &Value) -> Option<&str> {
    value
        .as_str()
        .or_else(|| value.get("url").and_then(Value::as_str))
        .or_else(|| value.get("path").and_then(Value::as_str))
}

#[cfg(windows)]
fn limit_debug_config_affinity(process_id: u32) -> Result<(), String> {
    unsafe {
        let handle = OpenProcess(PROCESS_SET_INFORMATION, false, process_id)
            .map_err(|error| error.to_string())?;
        let result = SetProcessAffinityMask(handle, 0b11usize).map_err(|error| error.to_string());
        let _ = CloseHandle(handle);
        result
    }
}

fn debug_config_plugin_loaded(directory: &Path) -> Option<bool> {
    let mut command = Command::new("mimo");
    command
        .args(["debug", "config"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    let child = command.spawn().ok()?;
    #[cfg(windows)]
    let _ = limit_debug_config_affinity(child.id());
    let output = child.wait_with_output().ok()?;
    if !output.status.success() || output.stdout.len() > 1_024 * 1_024 {
        return None;
    }
    let config = serde_json::from_slice::<Value>(&output.stdout).ok()?;
    let plugins = config.get("plugin")?.as_array()?;
    let expected = normalize_plugin_path(&plugin_path(directory).display().to_string());
    Some(plugins.iter().filter_map(plugin_entry_path).any(|entry| {
        let candidate = normalize_plugin_path(entry);
        candidate == expected || candidate.ends_with(&format!("/{PLUGIN_FILE_NAME}")) && candidate.ends_with(&expected)
    }))
}

fn restart_required_for_running_versions(
    installed_version: &str,
    running_versions: &[String],
) -> bool {
    installed_version == PLUGIN_VERSION
        && running_versions
            .iter()
            .any(|version| version != PLUGIN_VERSION)
}

fn status_at(directory: &Path) -> MimoHookStatus {
    let install_path = plugin_path(directory).display().to_string();
    let running_versions = current_running_versions();
    let plugin_exists = plugin_path(directory).exists();
    let manifest = match read_manifest(directory) {
        Ok(manifest) => manifest,
        Err(error) => {
            return MimoHookStatus {
                state: MimoHookInstallState::Error,
                install_path,
                bundled_version: PLUGIN_VERSION,
                installed_version: None,
                running_versions,
                error: Some(error),
            }
        }
    };

    let Some(manifest) = manifest else {
        return MimoHookStatus {
            state: if plugin_exists {
                MimoHookInstallState::Conflict
            } else {
                MimoHookInstallState::NotInstalled
            },
            install_path,
            bundled_version: PLUGIN_VERSION,
            installed_version: None,
            running_versions,
            error: plugin_exists.then(|| {
                "A plugin with the CodeCraft filename exists without a CodeCraft ownership manifest"
                    .to_string()
            }),
        };
    };

    let installed_version = Some(manifest.plugin_version.clone());
    let managed = match validate_manifest(&manifest) {
        Ok(file) => file,
        Err(error) => {
            return MimoHookStatus {
                state: MimoHookInstallState::Conflict,
                install_path,
                bundled_version: PLUGIN_VERSION,
                installed_version,
                running_versions,
                error: Some(error),
            }
        }
    };
    let Ok(bytes) = fs::read(plugin_path(directory)) else {
        return MimoHookStatus {
            state: MimoHookInstallState::Modified,
            install_path,
            bundled_version: PLUGIN_VERSION,
            installed_version,
            running_versions,
            error: Some("The managed Mimo plugin file is missing".to_string()),
        };
    };
    if sha256(&bytes) != managed.sha256 {
        return MimoHookStatus {
            state: MimoHookInstallState::Modified,
            install_path,
            bundled_version: PLUGIN_VERSION,
            installed_version,
            running_versions,
            error: Some("The managed Mimo plugin was modified after installation".to_string()),
        };
    }
    if manifest.protocol_version != PROTOCOL_VERSION {
        return MimoHookStatus {
            state: MimoHookInstallState::Incompatible,
            install_path,
            bundled_version: PLUGIN_VERSION,
            installed_version,
            running_versions,
            error: Some(format!(
                "Installed protocol {} is incompatible with {}",
                manifest.protocol_version, PROTOCOL_VERSION
            )),
        };
    }

    MimoHookStatus {
        state: if restart_required_for_running_versions(&manifest.plugin_version, &running_versions)
        {
            MimoHookInstallState::SyncedRestartRequired
        } else {
            MimoHookInstallState::Installed
        },
        install_path,
        bundled_version: PLUGIN_VERSION,
        installed_version,
        running_versions,
        error: None,
    }
}

fn install_at(directory: &Path, update_control: bool) -> Result<MimoHookStatus, String> {
    let before = status_at(directory);
    if matches!(
        before.state,
        MimoHookInstallState::Modified
            | MimoHookInstallState::Conflict
            | MimoHookInstallState::Error
    ) {
        return Err(before
            .error
            .unwrap_or_else(|| "Mimo plugin ownership validation failed".to_string()));
    }

    let install_id = read_manifest(directory)
        .ok()
        .flatten()
        .map(|manifest| manifest.install_id)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let plugin_bytes = BUNDLED_PLUGIN.as_bytes();
    atomic_write(&plugin_path(directory), plugin_bytes)?;
    let manifest = InstallManifest {
        owner: OWNER.to_string(),
        install_id: install_id.clone(),
        plugin_version: PLUGIN_VERSION.to_string(),
        protocol_version: PROTOCOL_VERSION.to_string(),
        files: vec![ManagedFile {
            name: PLUGIN_FILE_NAME.to_string(),
            sha256: sha256(plugin_bytes),
        }],
        installed_at: now_ms(),
    };
    let bytes = serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?;
    atomic_write(&manifest_path(directory), &bytes)?;
    if update_control {
        write_control(true, &install_id)?;
    }
    Ok(status_at(directory))
}

pub(crate) fn install() -> Result<MimoHookStatus, String> {
    install_at(&plugin_dir()?, true)
}

pub(crate) fn status() -> Result<MimoHookStatus, String> {
    let directory = plugin_dir()?;
    let mut status = status_at(&directory);
    if status.installed() {
        if debug_config_plugin_loaded(&directory) == Some(false) {
            status.state = MimoHookInstallState::SyncedRestartRequired;
            status.error = Some(
                "The Mimo plugin is installed but not loaded; restart Mimo to activate it"
                    .to_string(),
            );
        }
    }
    Ok(status)
}

pub(crate) fn status_and_sync() -> Result<MimoHookStatus, String> {
    status_and_sync_at(&plugin_dir()?, true)
}

fn status_and_sync_at(
    directory: &Path,
    update_control: bool,
) -> Result<MimoHookStatus, String> {
    let status = status_at(&directory);
    let needs_sync = match status.state {
        MimoHookInstallState::Installed => {
            status.installed_version.as_deref() != Some(PLUGIN_VERSION)
        }
        MimoHookInstallState::Incompatible => true,
        _ => false,
    };
    if !needs_sync {
        return Ok(status);
    }
    let mut synced = install_at(directory, update_control)?;
    synced.state = MimoHookInstallState::SyncedRestartRequired;
    Ok(synced)
}

fn uninstall_at(directory: &Path, update_control: bool) -> Result<(), String> {
    let status = status_at(directory);
    if status.state == MimoHookInstallState::NotInstalled {
        return Ok(());
    }
    if !matches!(
        status.state,
        MimoHookInstallState::Installed | MimoHookInstallState::SyncedRestartRequired
    ) {
        return Err(status.error.unwrap_or_else(|| {
            "Refusing to remove an unowned or modified Mimo plugin".to_string()
        }));
    }
    let manifest = read_manifest(directory)?
        .ok_or_else(|| "The CodeCraft Mimo ownership manifest is missing".to_string())?;
    let managed = validate_manifest(&manifest)?;
    let bytes = fs::read(plugin_path(directory)).map_err(|error| error.to_string())?;
    if sha256(&bytes) != managed.sha256 {
        return Err(
            "The Mimo plugin was modified; remove it manually after review".to_string(),
        );
    }

    if update_control {
        write_control(false, &Uuid::new_v4().to_string())?;
    }
    fs::remove_file(plugin_path(directory)).map_err(|error| error.to_string())?;
    fs::remove_file(manifest_path(directory)).map_err(|error| error.to_string())?;
    Ok(())
}

pub(crate) fn uninstall() -> Result<(), String> {
    uninstall_at(&plugin_dir()?, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_plugin_dir(name: &str) -> PathBuf {
        let root = env::temp_dir().join(format!(
            "codecraft-Mimo-hook-{name}-{}-{}",
            process::id(),
            Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn installs_and_detects_managed_plugin() {
        let root = temp_plugin_dir("install");
        let status = install_at(&root, false).unwrap();
        // A machine-wide Mimo process still running an older plugin makes the
        // state SyncedRestartRequired, so only the installed outcome is asserted.
        assert!(status.installed(), "unexpected state {:?}", status.state);
        assert!(plugin_path(&root).exists());
        assert!(manifest_path(&root).exists());
        assert_eq!(
            status_at(&root).installed_version.as_deref(),
            Some(PLUGIN_VERSION)
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn refuses_to_overwrite_unowned_conflict() {
        let root = temp_plugin_dir("conflict");
        fs::write(plugin_path(&root), b"third party").unwrap();
        let status = status_at(&root);
        assert_eq!(status.state, MimoHookInstallState::Conflict);
        assert!(install_at(&root, false).is_err());
        assert_eq!(fs::read(plugin_path(&root)).unwrap(), b"third party");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn detects_modification_and_refuses_uninstall() {
        let root = temp_plugin_dir("modified");
        install_at(&root, false).unwrap();
        fs::write(plugin_path(&root), b"changed by user").unwrap();
        assert_eq!(status_at(&root).state, MimoHookInstallState::Modified);
        assert!(uninstall_at(&root, false).is_err());
        assert!(plugin_path(&root).exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn uninstall_removes_only_managed_files() {
        let root = temp_plugin_dir("uninstall");
        let other = root.join("other-plugin.js");
        fs::write(&other, b"keep").unwrap();
        install_at(&root, false).unwrap();
        uninstall_at(&root, false).unwrap();
        assert!(!plugin_path(&root).exists());
        assert!(!manifest_path(&root).exists());
        assert_eq!(fs::read(other).unwrap(), b"keep");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn atomically_replaces_an_existing_file() {
        let root = temp_plugin_dir("replace");
        let target = root.join("state.json");
        fs::write(&target, b"old").unwrap();
        atomic_write(&target, b"new").unwrap();
        assert_eq!(fs::read(target).unwrap(), b"new");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn syncs_an_owned_unmodified_plugin_from_an_older_protocol() {
        let root = temp_plugin_dir("protocol-sync");
        install_at(&root, false).unwrap();
        let mut manifest = read_manifest(&root).unwrap().unwrap();
        manifest.protocol_version = "0.1".to_string();
        manifest.plugin_version = "0.0.9".to_string();
        fs::write(
            manifest_path(&root),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();

        assert_eq!(
            status_at(&root).state,
            MimoHookInstallState::Incompatible
        );
        let status = status_and_sync_at(&root, false).unwrap();
        assert_eq!(
            status.state,
            MimoHookInstallState::SyncedRestartRequired
        );
        assert_eq!(status.installed_version.as_deref(), Some(PLUGIN_VERSION));
        assert_eq!(
            read_manifest(&root).unwrap().unwrap().protocol_version,
            PROTOCOL_VERSION
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn keeps_restart_required_visible_while_an_older_plugin_is_running() {
        assert!(restart_required_for_running_versions(
            PLUGIN_VERSION,
            &["0.1.1".to_string(), PLUGIN_VERSION.to_string()]
        ));
        assert!(!restart_required_for_running_versions(
            "0.1.1",
            &["0.1.1".to_string()]
        ));
        assert!(!restart_required_for_running_versions(
            PLUGIN_VERSION,
            &[PLUGIN_VERSION.to_string()]
        ));
    }
}
