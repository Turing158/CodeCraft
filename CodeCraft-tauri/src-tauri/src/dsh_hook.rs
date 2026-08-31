//! DeepSeek Harness user overlay installation and authenticated local bridge.

use std::{
    env, fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::Manager;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::oneshot;
use url::Url;
use uuid::Uuid;

use crate::{dsh, DshIntegrationState};

pub(crate) const PLUGIN_VERSION: &str = "0.1.2";
pub(crate) const TARGET_DSH_VERSION: &str = "0.1.1-rc.2";
pub(crate) const SUPPORTED_DSH_VERSIONS: &[&str] = &["0.1.1-rc.2", "0.1.2-alpha.2"];
// The previous CodeCraft bundle is safe to replace; any other hash remains a user modification.
const LEGACY_BUNDLED_PLUGIN_SHA256: &str =
    "91f96091cb4c344d51bddca7ed47f6267408038079caaa31ef2076279427a936";
const PREVIOUS_BUNDLED_PLUGIN_SHA256: &str =
    "e25c0f3ccd7bdb32a17b82513cfa375f3e28c74d2e6d283dc81f74acc1a41d5c";
const OWNER: &str = "CodeCraft";
const PATCH_START: &str = "# codecraft-dsh-plugin:start";
const PATCH_END: &str = "# codecraft-dsh-plugin:end";
const MAX_MESSAGE_BYTES: usize = 256 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const BUNDLED_PLUGIN: &str = include_str!("../assets/dsh/codecraft/index.mjs");
const BUNDLED_MANIFEST: &str = include_str!("../assets/dsh/codecraft/manifest.json");

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

pub(crate) fn hook_dir() -> PathBuf {
    crate::approval_policy::base_data_dir().join("dsh-hook")
}

fn bridge_config_path() -> PathBuf {
    hook_dir().join("bridge.json")
}

fn default_dsh_home() -> Result<PathBuf, String> {
    if let Some(home) = env::var_os("DSH_HOME").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(home));
    }
    env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
        .map(|home| home.join(".dsh"))
        .ok_or_else(|| "Unable to locate the DeepSeek Harness home directory".to_string())
}

fn plugin_dir(home: &Path) -> PathBuf {
    home.join("codecraft")
}

fn plugin_path(home: &Path) -> PathBuf {
    plugin_dir(home).join("index.mjs")
}

fn manifest_path(home: &Path) -> PathBuf {
    plugin_dir(home).join("manifest.json")
}

fn plugin_url(home: &Path) -> Result<String, String> {
    let path = plugin_path(home);
    let absolute = if path.is_absolute() {
        path
    } else {
        env::current_dir()
            .map_err(|error| format!("Unable to resolve DSH plugin path: {error}"))?
            .join(path)
    };
    Url::from_file_path(&absolute)
        .map(|url| url.to_string())
        .map_err(|_| format!("Unable to construct a file URL for {}", absolute.display()))
}

fn patch_path(home: &Path) -> PathBuf {
    home.join("cordis.patch.yml")
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn backup(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "Invalid DSH configuration path".to_string())?;
    fs::copy(path, path.with_file_name(format!("{file_name}.bak")))
        .map_err(|error| format!("Unable to back up {}: {error}", path.display()))?;
    Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8], backup_existing: bool) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    if backup_existing {
        backup(path)?;
    }
    let temporary = path.with_extension(format!(
        "{}.{}.tmp",
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or("file"),
        Uuid::new_v4()
    ));
    fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
    if path.exists() {
        fs::remove_file(path).map_err(|error| error.to_string())?;
    }
    fs::rename(&temporary, path).map_err(|error| error.to_string())
}

fn marker_block(plugin_name: &str) -> String {
    format!(
        "{PATCH_START} owner={OWNER} version={PLUGIN_VERSION}\n- insert:\n    - id: codecraft-dsh-plugin\n      name: \"{plugin_name}\"\n{PATCH_END}\n"
    )
}

fn marker_range(contents: &str) -> Result<Option<(usize, usize)>, String> {
    let Some(start) = contents.find(PATCH_START) else {
        return if contents.contains(PATCH_END) {
            Err("The DSH CodeCraft overlay has an unmatched end marker".to_string())
        } else {
            Ok(None)
        };
    };
    let Some(relative_end) = contents[start..].find(PATCH_END) else {
        return Err("The DSH CodeCraft overlay has an unmatched start marker".to_string());
    };
    let end_marker = start + relative_end + PATCH_END.len();
    let end = contents[end_marker..]
        .find('\n')
        .map(|offset| end_marker + offset + 1)
        .unwrap_or(end_marker);
    if contents[end..].contains(PATCH_START) {
        return Err("Multiple CodeCraft DSH overlay markers were found".to_string());
    }
    Ok(Some((start, end)))
}

fn merge_patch(contents: &str, plugin_name: &str) -> Result<String, String> {
    let block = marker_block(plugin_name);
    if let Some((start, end)) = marker_range(contents)? {
        return Ok(format!(
            "{}{}{}",
            &contents[..start],
            block,
            &contents[end..]
        ));
    }
    let mut merged = contents.to_string();
    if !merged.is_empty() && !merged.ends_with('\n') {
        merged.push('\n');
    }
    if !merged.is_empty() && !merged.ends_with("\n\n") {
        merged.push('\n');
    }
    merged.push_str(&block);
    Ok(merged)
}

fn marker_has_plugin_url(contents: &str, plugin_name: &str) -> bool {
    let single_quoted = format!("name: '{plugin_name}'");
    let double_quoted = format!("name: \"{plugin_name}\"");
    contents.contains(&single_quoted) || contents.contains(&double_quoted)
}

fn remove_patch(contents: &str) -> Result<String, String> {
    let Some((start, end)) = marker_range(contents)? else {
        return Ok(contents.to_string());
    };
    let mut result = format!("{}{}", &contents[..start], &contents[end..]);
    while result.ends_with("\n\n\n") {
        result.pop();
    }
    Ok(result)
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DshHookInstallState {
    NotInstalled,
    Installed,
    Modified,
    Conflict,
    Incompatible,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DshHookStatus {
    pub(crate) state: DshHookInstallState,
    pub(crate) install_path: String,
    pub(crate) bundled_version: &'static str,
    pub(crate) installed_version: Option<String>,
    pub(crate) target_dsh_version: &'static str,
    pub(crate) error: Option<String>,
}

impl DshHookStatus {
    pub(crate) fn installed(&self) -> bool {
        self.state == DshHookInstallState::Installed
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstallManifest {
    owner: String,
    plugin_version: String,
    protocol: String,
    protocol_version: u32,
    target_dsh_version: String,
}

fn supports_dsh_version(version: &str) -> bool {
    SUPPORTED_DSH_VERSIONS.contains(&version)
}

fn supported_dsh_versions_label() -> String {
    SUPPORTED_DSH_VERSIONS.join("、")
}

fn status_at(home: &Path) -> DshHookStatus {
    let install_path = plugin_path(home).display().to_string();
    let plugin_exists = plugin_path(home).exists();
    let patch_contents = fs::read_to_string(patch_path(home)).unwrap_or_default();
    let marker = marker_range(&patch_contents);
    if !plugin_exists && matches!(marker, Ok(None)) {
        return DshHookStatus {
            state: DshHookInstallState::NotInstalled,
            install_path,
            bundled_version: PLUGIN_VERSION,
            installed_version: None,
            target_dsh_version: TARGET_DSH_VERSION,
            error: None,
        };
    }
    let marker = match marker {
        Ok(Some(range)) => range,
        Ok(None) => {
            return DshHookStatus {
                state: DshHookInstallState::Conflict,
                install_path,
                bundled_version: PLUGIN_VERSION,
                installed_version: None,
                target_dsh_version: TARGET_DSH_VERSION,
                error: Some(
                    "The CodeCraft DSH plugin exists without its overlay marker".to_string(),
                ),
            }
        }
        Err(error) => {
            return DshHookStatus {
                state: DshHookInstallState::Conflict,
                install_path,
                bundled_version: PLUGIN_VERSION,
                installed_version: None,
                target_dsh_version: TARGET_DSH_VERSION,
                error: Some(error),
            }
        }
    };
    let _ = marker;
    let manifest = fs::read(manifest_path(home))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<InstallManifest>(&bytes).ok());
    let Some(manifest) = manifest else {
        return DshHookStatus {
            state: DshHookInstallState::Conflict,
            install_path,
            bundled_version: PLUGIN_VERSION,
            installed_version: None,
            target_dsh_version: TARGET_DSH_VERSION,
            error: Some("The CodeCraft DSH ownership manifest is missing or invalid".to_string()),
        };
    };
    if manifest.owner != OWNER
        || manifest.protocol != dsh::PROTOCOL
        || manifest.protocol_version != dsh::PROTOCOL_VERSION
    {
        return DshHookStatus {
            state: DshHookInstallState::Conflict,
            install_path,
            bundled_version: PLUGIN_VERSION,
            installed_version: Some(manifest.plugin_version),
            target_dsh_version: TARGET_DSH_VERSION,
            error: Some(
                "The DSH plugin at the CodeCraft path is not owned by CodeCraft".to_string(),
            ),
        };
    }
    if !supports_dsh_version(&manifest.target_dsh_version) {
        return DshHookStatus {
            state: DshHookInstallState::Incompatible,
            install_path,
            bundled_version: PLUGIN_VERSION,
            installed_version: Some(manifest.plugin_version),
            target_dsh_version: TARGET_DSH_VERSION,
            error: Some(format!(
                "The installed DSH plugin targets {}, but CodeCraft supports {}",
                manifest.target_dsh_version,
                supported_dsh_versions_label()
            )),
        };
    }
    let bytes = fs::read(plugin_path(home)).unwrap_or_default();
    let plugin_hash = sha256(&bytes);
    let bundled_hash = sha256(BUNDLED_PLUGIN.as_bytes());
    let is_known_legacy_bundle = manifest.plugin_version != PLUGIN_VERSION
        && [LEGACY_BUNDLED_PLUGIN_SHA256, PREVIOUS_BUNDLED_PLUGIN_SHA256]
            .contains(&plugin_hash.as_str());
    if plugin_hash != bundled_hash && !is_known_legacy_bundle {
        return DshHookStatus {
            state: DshHookInstallState::Modified,
            install_path,
            bundled_version: PLUGIN_VERSION,
            installed_version: Some(manifest.plugin_version),
            target_dsh_version: TARGET_DSH_VERSION,
            error: Some("The managed DSH plugin was modified after installation".to_string()),
        };
    }
    let expected_plugin_name = match plugin_url(home) {
        Ok(name) => name,
        Err(error) => {
            return DshHookStatus {
                state: DshHookInstallState::Conflict,
                install_path,
                bundled_version: PLUGIN_VERSION,
                installed_version: Some(manifest.plugin_version),
                target_dsh_version: TARGET_DSH_VERSION,
                error: Some(error),
            }
        }
    };
    if !marker_has_plugin_url(&patch_contents[marker.0..marker.1], &expected_plugin_name) {
        return DshHookStatus {
            state: DshHookInstallState::Incompatible,
            install_path,
            bundled_version: PLUGIN_VERSION,
            installed_version: Some(manifest.plugin_version),
            target_dsh_version: TARGET_DSH_VERSION,
            error: Some(
                "The CodeCraft DSH overlay uses a profile-relative plugin path; reinstalling it will migrate the overlay to an absolute file URL".to_string(),
            ),
        };
    }
    let state = if manifest.plugin_version == PLUGIN_VERSION && plugin_hash == bundled_hash {
        DshHookInstallState::Installed
    } else {
        DshHookInstallState::Incompatible
    };
    DshHookStatus {
        state,
        install_path,
        bundled_version: PLUGIN_VERSION,
        installed_version: Some(manifest.plugin_version),
        target_dsh_version: TARGET_DSH_VERSION,
        error: (state != DshHookInstallState::Installed)
            .then(|| "The installed DSH plugin version does not match CodeCraft".to_string()),
    }
}

pub(crate) fn status() -> Result<DshHookStatus, String> {
    Ok(status_at(&default_dsh_home()?))
}

fn install_at(home: &Path) -> Result<DshHookStatus, String> {
    let before = status_at(home);
    if matches!(
        before.state,
        DshHookInstallState::Modified | DshHookInstallState::Conflict
    ) {
        return Err(before
            .error
            .unwrap_or_else(|| "Refusing to replace a conflicting DSH plugin".to_string()));
    }
    let plugin_name = plugin_url(home)?;
    fs::create_dir_all(plugin_dir(home)).map_err(|error| error.to_string())?;
    atomic_write(&plugin_path(home), BUNDLED_PLUGIN.as_bytes(), true)?;
    atomic_write(&manifest_path(home), BUNDLED_MANIFEST.as_bytes(), true)?;
    let patch = fs::read_to_string(patch_path(home)).unwrap_or_default();
    atomic_write(
        &patch_path(home),
        merge_patch(&patch, &plugin_name)?.as_bytes(),
        true,
    )?;
    Ok(status_at(home))
}

pub(crate) fn install() -> Result<DshHookStatus, String> {
    install_at(&default_dsh_home()?)
}

fn uninstall_at(home: &Path) -> Result<(), String> {
    let current = status_at(home);
    if current.state == DshHookInstallState::NotInstalled {
        return Ok(());
    }
    if current.state != DshHookInstallState::Installed {
        return Err(current.error.unwrap_or_else(|| {
            "Refusing to remove an unowned or modified DSH plugin".to_string()
        }));
    }
    let patch = fs::read_to_string(patch_path(home)).unwrap_or_default();
    atomic_write(&patch_path(home), remove_patch(&patch)?.as_bytes(), true)?;
    for path in [plugin_path(home), manifest_path(home)] {
        backup(&path)?;
        fs::remove_file(&path).map_err(|error| error.to_string())?;
    }
    let _ = fs::remove_dir(plugin_dir(home));
    Ok(())
}

pub(crate) fn uninstall() -> Result<(), String> {
    uninstall_at(&default_dsh_home()?)
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DshBridgeInfo {
    pub(crate) bridge_instance_id: String,
    pub(crate) endpoint: String,
    pub(crate) started_at: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BridgeConfig<'a> {
    protocol: &'static str,
    protocol_version: u32,
    bridge_instance_id: &'a str,
    endpoint: &'a str,
    token: &'a str,
    codecraft_process_id: u32,
    started_at: u64,
}

fn validate_bridge_envelope(
    envelope: &dsh::DshEnvelope,
    bridge_id: &str,
    token: &str,
) -> Result<(), String> {
    if envelope.token != token || envelope.bridge_instance_id != bridge_id {
        return Err("unauthorized".to_string());
    }
    if supports_dsh_version(&envelope.dsh_version) {
        return Ok(());
    }
    if envelope.dsh_version == "unknown" {
        return Err(format!(
            "无法确定正在运行的 DeepSeek Harness 版本；CodeCraft 当前兼容 {}",
            supported_dsh_versions_label()
        ));
    }
    Err(format!(
        "正在运行的 DeepSeek Harness {} 与 CodeCraft 兼容版本 {} 不一致",
        envelope.dsh_version,
        supported_dsh_versions_label()
    ))
}

async fn handle_connection<S>(stream: S, app: tauri::AppHandle, bridge_id: String, token: String)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let read = tokio::time::timeout(Duration::from_secs(5), reader.read_line(&mut line)).await;
    let Ok(Ok(bytes)) = read else {
        return;
    };
    if bytes == 0 || bytes > MAX_MESSAGE_BYTES {
        return;
    }
    let mut stream = reader.into_inner();
    let parsed = serde_json::from_str::<dsh::DshEnvelope>(line.trim_end());
    let envelope = match parsed {
        Ok(envelope) => envelope,
        Err(error) => {
            let _ = stream
                .write_all(
                    format!("{}\n", json!({"ok":false,"error":error.to_string()})).as_bytes(),
                )
                .await;
            return;
        }
    };
    let state = app.state::<DshIntegrationState>();
    if let Err(error) = validate_bridge_envelope(&envelope, &bridge_id, &token) {
        if error != "unauthorized" {
            if let Ok(mut hook_error) = state.hook_error.lock() {
                *hook_error = Some(error.clone());
            }
        }
        let _ = stream
            .write_all(format!("{}\n", json!({"ok":false,"error":error})).as_bytes())
            .await;
        return;
    }
    if let Ok(mut hook_error) = state.hook_error.lock() {
        if hook_error.as_deref().is_some_and(|error| {
            error.starts_with("无法确定正在运行的 DeepSeek Harness 版本")
                || error.starts_with("正在运行的 DeepSeek Harness ")
        }) {
            *hook_error = None;
        }
    }
    let request_id = envelope.request_id.clone();
    let ingest = state
        .store
        .lock()
        .map_err(|error| error.to_string())
        .and_then(|mut store| store.ingest(&envelope));
    if let Err(error) = ingest {
        let _ = stream
            .write_all(format!("{}\n", json!({"ok":false,"error":error})).as_bytes())
            .await;
        return;
    }
    let Some(request_id) = request_id else {
        let _ = stream.write_all(b"{\"ok\":true}\n").await;
        return;
    };
    let (sender, receiver) = oneshot::channel::<Value>();
    {
        let Ok(mut pending) = state.pending.lock() else {
            return;
        };
        pending.insert(request_id.clone(), sender);
    }
    let mut disconnect_probe = [0_u8; 1];
    let response = tokio::select! {
        result = receiver => match result {
            Ok(value) => value,
            Err(_) => json!({"decision":"deny","reason":"CodeCraft DSH bridge cancelled the request"}),
        },
        _ = tokio::time::sleep(REQUEST_TIMEOUT) => {
            if let Ok(mut pending) = state.pending.lock() {
                pending.remove(&request_id);
            }
            if let Ok(mut store) = state.store.lock() {
                store.resolve(&request_id);
            }
            json!({"decision":"deny","reason":"CodeCraft DSH bridge timed out or disconnected"})
        },
        _ = stream.read(&mut disconnect_probe) => {
            if let Ok(mut pending) = state.pending.lock() {
                pending.remove(&request_id);
            }
            if let Ok(mut store) = state.store.lock() {
                store.resolve(&request_id);
            }
            return;
        },
    };
    let _ = stream.write_all(format!("{response}\n").as_bytes()).await;
}

#[cfg(windows)]
async fn serve(endpoint: String, app: tauri::AppHandle, bridge_id: String, token: String) {
    use tokio::net::windows::named_pipe::ServerOptions;
    let mut first = true;
    loop {
        let server = ServerOptions::new()
            .first_pipe_instance(first)
            .create(&endpoint);
        let Ok(server) = server else {
            eprintln!("Unable to create the DSH named pipe");
            return;
        };
        first = false;
        if server.connect().await.is_err() {
            continue;
        }
        tauri::async_runtime::spawn(handle_connection(
            server,
            app.clone(),
            bridge_id.clone(),
            token.clone(),
        ));
    }
}

#[cfg(not(windows))]
async fn serve(endpoint: String, app: tauri::AppHandle, bridge_id: String, token: String) {
    use tokio::net::UnixListener;
    let path = PathBuf::from(&endpoint);
    let _ = fs::remove_file(&path);
    let Ok(listener) = UnixListener::bind(&path) else {
        eprintln!("Unable to create the DSH Unix socket");
        return;
    };
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        tauri::async_runtime::spawn(handle_connection(
            stream,
            app.clone(),
            bridge_id.clone(),
            token.clone(),
        ));
    }
}

pub(crate) fn start_bridge(app: tauri::AppHandle) -> Result<DshBridgeInfo, String> {
    let bridge_instance_id = Uuid::new_v4().to_string();
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    #[cfg(windows)]
    let endpoint = format!(
        r"\\.\pipe\codecraft-dsh-{}-{}",
        std::process::id(),
        bridge_instance_id
    );
    #[cfg(not(windows))]
    let endpoint = hook_dir()
        .join(format!("bridge-{bridge_instance_id}.sock"))
        .display()
        .to_string();
    let started_at = now_ms();
    let config = BridgeConfig {
        protocol: dsh::PROTOCOL,
        protocol_version: dsh::PROTOCOL_VERSION,
        bridge_instance_id: &bridge_instance_id,
        endpoint: &endpoint,
        token: &token,
        codecraft_process_id: std::process::id(),
        started_at,
    };
    let bytes = serde_json::to_vec_pretty(&config).map_err(|error| error.to_string())?;
    atomic_write(&bridge_config_path(), &bytes, true)?;
    {
        let state = app.state::<DshIntegrationState>();
        let mut store = state.store.lock().map_err(|error| error.to_string())?;
        store.set_bridge_instance_id(Some(bridge_instance_id.clone()));
    }
    tauri::async_runtime::spawn(serve(
        endpoint.clone(),
        app,
        bridge_instance_id.clone(),
        token,
    ));
    Ok(DshBridgeInfo {
        bridge_instance_id,
        endpoint,
        started_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn temp_home() -> PathBuf {
        let path = env::temp_dir().join(format!("codecraft-dsh-hook-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn installation_preserves_existing_overlay_and_creates_backup() {
        let home = temp_home();
        fs::write(
            patch_path(&home),
            "- insert:\n    - id: existing\n      name: 'existing'\n",
        )
        .unwrap();
        let status = install_at(&home).unwrap();
        assert!(status.installed());
        let patch = fs::read_to_string(patch_path(&home)).unwrap();
        assert!(patch.contains("id: existing"));
        assert!(patch.contains(PATCH_START));
        let plugin_name = plugin_url(&home).unwrap();
        assert!(patch.contains(&format!("name: \"{plugin_name}\"")));
        assert!(patch_path(&home)
            .with_file_name("cordis.patch.yml.bak")
            .exists());
    }

    #[test]
    fn reinstall_migrates_legacy_profile_relative_overlay() {
        let home = temp_home();
        fs::create_dir_all(plugin_dir(&home)).unwrap();
        fs::write(plugin_path(&home), BUNDLED_PLUGIN).unwrap();
        fs::write(manifest_path(&home), BUNDLED_MANIFEST).unwrap();
        fs::write(patch_path(&home), marker_block("./codecraft/index.mjs")).unwrap();

        let before = status_at(&home);
        assert_eq!(before.state, DshHookInstallState::Incompatible);

        let after = install_at(&home).unwrap();
        assert_eq!(after.state, DshHookInstallState::Installed);
        let patch = fs::read_to_string(patch_path(&home)).unwrap();
        let plugin_name = plugin_url(&home).unwrap();
        assert!(patch.contains(&format!("name: \"{plugin_name}\"")));
        assert!(!patch.contains("name: './codecraft/index.mjs'"));
    }

    #[test]
    fn uninstall_removes_only_the_codecraft_marker() {
        let home = temp_home();
        fs::write(
            patch_path(&home),
            "- insert:\n    - id: existing\n      name: 'existing'\n",
        )
        .unwrap();
        install_at(&home).unwrap();
        let patch = fs::read_to_string(patch_path(&home)).unwrap();
        let removed = remove_patch(&patch).unwrap();
        assert!(removed.contains("id: existing"));
        assert!(!removed.contains(PATCH_START));
    }

    #[test]
    fn uninstall_preserves_other_overlay_entries_and_backs_up_managed_files() {
        let home = temp_home();
        fs::write(
            patch_path(&home),
            "- insert:\n    - id: existing\n      name: 'existing'\n",
        )
        .unwrap();
        install_at(&home).unwrap();
        uninstall_at(&home).unwrap();

        let patch = fs::read_to_string(patch_path(&home)).unwrap();
        assert!(patch.contains("id: existing"));
        assert!(!patch.contains(PATCH_START));
        assert!(!plugin_path(&home).exists());
        assert!(!manifest_path(&home).exists());
        assert!(plugin_path(&home).with_extension("mjs.bak").exists());
        assert!(manifest_path(&home).with_extension("json.bak").exists());
    }

    #[test]
    fn status_accepts_a_plugin_manifest_targeting_the_newer_compatible_runtime() {
        let home = temp_home();
        fs::create_dir_all(plugin_dir(&home)).unwrap();
        fs::write(plugin_path(&home), BUNDLED_PLUGIN).unwrap();
        let mut manifest: Value = serde_json::from_str(BUNDLED_MANIFEST).unwrap();
        manifest["targetDshVersion"] = Value::String("0.1.2-alpha.2".to_string());
        fs::write(
            manifest_path(&home),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        fs::write(patch_path(&home), marker_block(&plugin_url(&home).unwrap())).unwrap();

        assert_eq!(status_at(&home).state, DshHookInstallState::Installed);
    }

    fn bridge_envelope() -> dsh::DshEnvelope {
        serde_json::from_value::<dsh::DshEnvelope>(serde_json::json!({
            "protocol": dsh::PROTOCOL,
            "protocolVersion": dsh::PROTOCOL_VERSION,
            "messageType": "heartbeat",
            "messageId": "message-1",
            "bridgeInstanceId": "bridge-1",
            "pluginInstanceId": "plugin-1",
            "pluginVersion": PLUGIN_VERSION,
            "dshVersion": TARGET_DSH_VERSION,
            "dshProcessId": 42,
            "sessionId": Value::Null,
            "turnId": Value::Null,
            "stepId": Value::Null,
            "requestId": Value::Null,
            "capturedAt": 1,
            "workspace": Value::Null,
            "capabilities": {},
            "token": "secret",
            "payload": {}
        }))
        .unwrap()
    }

    #[test]
    fn bridge_identity_rejects_bad_token_and_old_generation() {
        let envelope = bridge_envelope();
        assert_eq!(
            validate_bridge_envelope(&envelope, "bridge-1", "wrong").unwrap_err(),
            "unauthorized"
        );
        assert_eq!(
            validate_bridge_envelope(&envelope, "bridge-2", "secret").unwrap_err(),
            "unauthorized"
        );
    }

    #[test]
    fn bridge_identity_reports_unknown_and_incompatible_runtime_versions() {
        let mut envelope = bridge_envelope();
        envelope.dsh_version = "unknown".to_string();
        assert!(validate_bridge_envelope(&envelope, "bridge-1", "secret")
            .unwrap_err()
            .contains("无法确定"));

        envelope.dsh_version = "0.1.2-alpha.2".to_string();
        assert!(validate_bridge_envelope(&envelope, "bridge-1", "secret").is_ok());

        envelope.dsh_version = "0.1.3-alpha.1".to_string();
        let error = validate_bridge_envelope(&envelope, "bridge-1", "secret").unwrap_err();
        assert!(error.contains("0.1.3-alpha.1"));
        for version in SUPPORTED_DSH_VERSIONS {
            assert!(error.contains(version));
        }
    }
}
