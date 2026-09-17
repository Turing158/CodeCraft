//! WorkBuddy/CodeBuddy plugin, protocol detection and loopback HTTP bridge.
//!
//! Every interaction request is captured for read-only inspection and receives
//! an empty native-safe response. Decisions remain in WorkBuddy.

use std::{
    collections::HashMap,
    env, fs,
    fs::OpenOptions,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    body::Bytes,
    extract::DefaultBodyLimit,
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::Manager;
use tokio::net::TcpListener;
use uuid::Uuid;

use crate::{workbuddy, workbuddy_files, WorkBuddyIntegrationState};

pub(crate) const HOOK_ARGUMENT: &str = "--codecraft-workbuddy-hook";
pub(crate) const PLUGIN_VERSION: &str = "0.1.0";
pub(crate) const INSTALL_ID: &str = "codecraft-workbuddy";
pub(crate) const OWNER: &str = "CodeCraft";
pub(crate) const SUPPORTED_EVENTS: [&str; 16] = [
    "SessionStart",
    "SessionEnd",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PostToolUseFailure",
    "Stop",
    "SubagentStart",
    "SubagentStop",
    "Notification",
    "PermissionRequest",
    "PermissionDenied",
    "Elicitation",
    "ElicitationResult",
    "PreCompact",
    "PostCompact",
];

const MAX_BODY_BYTES: usize = 256 * 1024;
const MAX_INBOX_FILES: usize = 512;
const MAX_INBOX_BYTES: u64 = 32 * 1024 * 1024;
const INBOX_RETENTION_MS: u64 = 24 * 60 * 60 * 1000;
const TOKEN_TTL_MS: u64 = 10 * 60 * 1000;
const MAX_NONCES: usize = 2_048;
const NONCE_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_CLOCK_SKEW_MS: u64 = 2 * 60 * 1_000;
const BRIDGE_CONFIG_FILE: &str = "workbuddy-bridge.json";
const SETTINGS_PLUGIN_KEY: &str = "codecraft-workbuddy@codecraft-workbuddy";
const LEGACY_SETTINGS_PLUGIN_KEY: &str = "codecraft-workbuddy@local";

const PLUGIN_MANIFEST: &str =
    include_str!("../assets/workbuddy/codecraft/.codebuddy-plugin/plugin.json");
const PLUGIN_HOOKS: &str = include_str!("../assets/workbuddy/codecraft/hooks/hooks.json");
const PLUGIN_SCRIPT: &str =
    include_str!("../assets/workbuddy/codecraft/scripts/workbuddy-bridge.mjs");

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum WorkBuddyHookInstallState {
    NotInstalled,
    Installed,
    SyncedRestartRequired,
    Disabled,
    Modified,
    Conflict,
    Incompatible,
    Error,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkBuddyHookStatus {
    pub(crate) state: WorkBuddyHookInstallState,
    pub(crate) files_installed: bool,
    pub(crate) enabled: bool,
    pub(crate) registered: bool,
    pub(crate) loaded: bool,
    pub(crate) connected: bool,
    pub(crate) bridge_ready: bool,
    pub(crate) install_path: String,
    pub(crate) bundled_version: &'static str,
    pub(crate) installed_version: Option<String>,
    pub(crate) workbuddy_version: Option<String>,
    pub(crate) cli_version: Option<String>,
    pub(crate) error: Option<String>,
}

impl WorkBuddyHookStatus {
    pub(crate) fn installed(&self) -> bool {
        self.files_installed
    }

    fn problem(mut self, state: WorkBuddyHookInstallState, message: &str) -> Self {
        self.state = state;
        self.error = Some(message.to_string());
        self
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkBuddyBridgeInfo {
    pub(crate) bridge_instance_id: String,
    pub(crate) plugin_instance_id: String,
    pub(crate) endpoint: String,
    pub(crate) started_at: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkBuddyEnvironmentReport {
    pub(crate) detected: bool,
    pub(crate) desktop_version: Option<String>,
    pub(crate) cli_version: Option<String>,
    pub(crate) plugin_runtime_version: Option<String>,
    pub(crate) executable_path: Option<String>,
    pub(crate) settings_path: String,
    pub(crate) docs_available: bool,
    pub(crate) protocol: &'static str,
    pub(crate) protocol_version: u32,
    pub(crate) protocol_frozen: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BridgeConfig<'a> {
    protocol: &'static str,
    protocol_version: u32,
    bridge_instance_id: &'a str,
    plugin_instance_id: &'a str,
    endpoint: &'a str,
    token: &'a str,
    started_at: u64,
    expires_at: u64,
}

struct BridgeCredential {
    token: String,
    expires_at: u64,
}

#[derive(Clone)]
struct BridgeState {
    app: tauri::AppHandle,
    bridge_instance_id: String,
    plugin_instance_id: String,
    credentials: Arc<Mutex<BridgeCredential>>,
    endpoint: String,
    _instance_lock: Arc<fs::File>,
    started_at: u64,
    nonces: Arc<Mutex<HashMap<String, Instant>>>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or_default()
}

fn base_data_dir() -> PathBuf {
    crate::approval_policy::base_data_dir()
}

fn bridge_config_path() -> PathBuf {
    base_data_dir().join(BRIDGE_CONFIG_FILE)
}

fn inbox_dir() -> PathBuf {
    base_data_dir().join("workbuddy-hook").join("inbox")
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn atomic_write(path: &Path, bytes: &[u8], backup_existing: bool) -> Result<(), String> {
    workbuddy_files::atomic_write(path, bytes, backup_existing)
}

fn workbuddy_home() -> PathBuf {
    #[cfg(test)]
    if let Some(path) = TEST_HOME.with(|path| path.borrow().clone()) {
        return path;
    }
    if let Some(home) = env::var_os("WORKBUDDY_HOME") {
        return PathBuf::from(home);
    }
    env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
        .map(|home| home.join(".workbuddy"))
        .unwrap_or_else(|| PathBuf::from(".workbuddy"))
}

#[cfg(test)]
thread_local! { static TEST_HOME: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) }; }

fn plugin_root() -> PathBuf {
    marketplace_root().join(INSTALL_ID)
}

fn marketplace_root() -> PathBuf {
    workbuddy_home()
        .join("plugins")
        .join("marketplaces")
        .join(INSTALL_ID)
}

fn marketplace_manifest_path() -> PathBuf {
    marketplace_root()
        .join(".codebuddy-plugin")
        .join("marketplace.json")
}

fn settings_path() -> PathBuf {
    workbuddy_home().join("settings.json")
}

fn plugin_file_specs() -> [(&'static str, &'static str); 3] {
    [
        (".codebuddy-plugin/plugin.json", PLUGIN_MANIFEST),
        ("hooks/hooks.json", PLUGIN_HOOKS),
        ("scripts/workbuddy-bridge.mjs", PLUGIN_SCRIPT),
    ]
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstallManifest {
    owner: String,
    install_id: String,
    plugin_version: String,
    protocol: String,
    protocol_version: u32,
    file_hashes: HashMap<String, String>,
}

fn owner_manifest_path() -> PathBuf {
    plugin_root().join(".codecraft-workbuddy-plugin.json")
}

fn settings_with_plugin(enabled: bool) -> Result<Value, String> {
    let path = settings_path();
    let mut root = if path.exists() {
        workbuddy_files::parse_jsonc(&workbuddy_files::read_bounded(
            &path,
            workbuddy_files::MAX_CONFIG_BYTES,
        )?)?
    } else {
        json!({})
    };
    let object = root
        .as_object_mut()
        .ok_or_else(|| "WorkBuddy settings.json 根节点必须是对象".to_string())?;
    if let Some(existing) = object
        .get("extraKnownMarketplaces")
        .and_then(|value| value.get(INSTALL_ID))
    {
        let expected = json!({"source":{"source":"directory","path":marketplace_root().display().to_string()}});
        if existing != &expected {
            return Err("WorkBuddy marketplace 配置冲突；用户条目已保留".into());
        }
    }
    if read_manifest()?.is_none()
        && object.get("enabledPlugins").is_some_and(|value| {
            value.get(SETTINGS_PLUGIN_KEY).is_some()
                || value.get(LEGACY_SETTINGS_PLUGIN_KEY).is_some()
        })
    {
        return Err("WorkBuddy 插件配置缺少 CodeCraft 归属证明".into());
    }
    let plugins = object
        .entry("enabledPlugins")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| "WorkBuddy enabledPlugins 必须是对象".to_string())?;
    if enabled {
        plugins.remove(LEGACY_SETTINGS_PLUGIN_KEY);
        plugins.insert(SETTINGS_PLUGIN_KEY.to_string(), Value::Bool(true));
    } else {
        plugins.remove(SETTINGS_PLUGIN_KEY);
        plugins.remove(LEGACY_SETTINGS_PLUGIN_KEY);
    }
    let marketplaces = object
        .entry("extraKnownMarketplaces")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| "WorkBuddy extraKnownMarketplaces 必须是对象".to_string())?;
    if enabled {
        marketplaces.insert(
            INSTALL_ID.to_string(),
            json!({
                "source": {
                    "source": "directory",
                    "path": marketplace_root().display().to_string()
                }
            }),
        );
    } else {
        marketplaces.remove(INSTALL_ID);
    }
    Ok(root)
}

fn read_manifest() -> Result<Option<InstallManifest>, String> {
    if !owner_manifest_path().exists() {
        return Ok(None);
    }
    serde_json::from_slice(&workbuddy_files::read_bounded(
        &owner_manifest_path(),
        workbuddy_files::MAX_CONFIG_BYTES,
    )?)
    .map(Some)
    .map_err(|error| format!("CodeCraft WorkBuddy ownership manifest 无效：{error}"))
}

fn installed_enabled() -> bool {
    let Ok(bytes) =
        workbuddy_files::read_bounded(&settings_path(), workbuddy_files::MAX_CONFIG_BYTES)
    else {
        return false;
    };
    workbuddy_files::parse_jsonc(&bytes)
        .ok()
        .and_then(|value| {
            value
                .get("enabledPlugins")?
                .get(SETTINGS_PLUGIN_KEY)
                .or_else(|| value.get("enabledPlugins")?.get(LEGACY_SETTINGS_PLUGIN_KEY))
                .cloned()
        })
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn installed_plugin_registered() -> bool {
    let Ok(bytes) = workbuddy_files::read_bounded(
        &workbuddy_home().join("plugins/installed_plugins.json"),
        workbuddy_files::MAX_CONFIG_BYTES,
    ) else {
        return false;
    };
    serde_json::from_slice::<Value>(&bytes)
        .ok()
        .and_then(|value| {
            value
                .get("plugins")?
                .get(SETTINGS_PLUGIN_KEY)
                .or_else(|| value.get("plugins")?.get(LEGACY_SETTINGS_PLUGIN_KEY))
                .cloned()
        })
        .and_then(|value| value.as_array().map(|items| !items.is_empty()))
        .unwrap_or(false)
}

pub(crate) fn status() -> Result<WorkBuddyHookStatus, String> {
    workbuddy_files::reject_links(&plugin_root())?;
    let report = detect_environment();
    let root = plugin_root();
    let mut status = WorkBuddyHookStatus {
        state: WorkBuddyHookInstallState::NotInstalled,
        files_installed: false,
        enabled: installed_enabled(),
        registered: installed_plugin_registered(),
        loaded: false,
        connected: false,
        bridge_ready: false,
        install_path: root.display().to_string(),
        bundled_version: PLUGIN_VERSION,
        installed_version: None,
        workbuddy_version: report.desktop_version,
        cli_version: report.cli_version,
        error: None,
    };
    if !owner_manifest_path().exists()
        && !marketplace_manifest_path().exists()
        && plugin_file_specs()
            .iter()
            .all(|(relative, _)| !root.join(relative).exists())
    {
        return Ok(if status.enabled {
            status.problem(
                WorkBuddyHookInstallState::Conflict,
                "WorkBuddy settings 启用了 CodeCraft 插件，但本地 marketplace 缺失",
            )
        } else {
            status
        });
    }
    let Some(manifest) = read_manifest()? else {
        return Ok(status.problem(
            WorkBuddyHookInstallState::Conflict,
            "CodeCraft WorkBuddy ownership manifest 缺失",
        ));
    };
    status.installed_version = Some(manifest.plugin_version.clone());
    if manifest.owner != OWNER
        || manifest.install_id != INSTALL_ID
        || manifest.protocol != workbuddy::PROTOCOL
        || manifest.protocol_version != workbuddy::PROTOCOL_VERSION
    {
        return Ok(status.problem(
            WorkBuddyHookInstallState::Conflict,
            "WorkBuddy 插件归属或协议版本冲突",
        ));
    }
    // Physical installation is independent of enablement, registry entries,
    // and runtime activity. Integrity failures remain visible as installed
    // files needing attention; install/uninstall still enforce ownership/hash.
    status.files_installed = marketplace_manifest_path().is_file()
        && plugin_file_specs()
            .iter()
            .all(|(relative, _)| root.join(relative).is_file());
    let marketplace_valid = workbuddy_files::read_bounded(
        &marketplace_manifest_path(),
        workbuddy_files::MAX_CONFIG_BYTES,
    )
    .ok()
    .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
    .is_some_and(|value| {
        value.get("name").and_then(Value::as_str) == Some(INSTALL_ID)
            && value.pointer("/owner/name").and_then(Value::as_str) == Some(OWNER)
            && value
                .get("plugins")
                .and_then(Value::as_array)
                .is_some_and(|plugins| {
                    plugins.len() == 1
                        && plugins.iter().any(|plugin| {
                            plugin.get("name").and_then(Value::as_str) == Some(INSTALL_ID)
                                && plugin.get("source").and_then(Value::as_str)
                                    == Some("./codecraft-workbuddy")
                        })
                })
    });
    if !marketplace_valid {
        return Ok(status.problem(
            WorkBuddyHookInstallState::Conflict,
            "CodeCraft WorkBuddy marketplace manifest 缺失或冲突",
        ));
    }
    if let Some(expected) = manifest
        .file_hashes
        .get("../.codebuddy-plugin/marketplace.json")
    {
        if sha256(&workbuddy_files::read_bounded(
            &marketplace_manifest_path(),
            workbuddy_files::MAX_CONFIG_BYTES,
        )?) != *expected
        {
            return Ok(status.problem(
                WorkBuddyHookInstallState::Modified,
                "marketplace 文件已被修改",
            ));
        }
    }
    for (relative, _) in plugin_file_specs() {
        let Some(expected) = manifest.file_hashes.get(relative) else {
            return Ok(status.problem(
                WorkBuddyHookInstallState::Modified,
                "插件 hash 清单缺少文件",
            ));
        };
        let Ok(bytes) =
            workbuddy_files::read_bounded(&root.join(relative), workbuddy_files::MAX_CONFIG_BYTES)
        else {
            return Ok(status.problem(WorkBuddyHookInstallState::Modified, "插件文件缺失或不可读"));
        };
        if sha256(&bytes) != *expected {
            return Ok(status.problem(WorkBuddyHookInstallState::Modified, "插件文件已被修改"));
        }
    }
    status.state = if !status.enabled {
        WorkBuddyHookInstallState::Disabled
    } else if !status.registered {
        WorkBuddyHookInstallState::SyncedRestartRequired
    } else {
        WorkBuddyHookInstallState::Installed
    };
    // Waiting for WorkBuddy is a normal lifecycle state, not a sticky error.
    Ok(status)
}

pub(crate) fn runtime_status(
    state: &WorkBuddyIntegrationState,
    snapshot: &workbuddy::WorkBuddySnapshot,
) -> Result<WorkBuddyHookStatus, String> {
    let bridge = state.bridge.lock().map_err(|e| e.to_string())?.clone();
    let bridge_error = state.hook_error.lock().map_err(|e| e.to_string())?.clone();
    Ok(with_runtime(
        status()?,
        bridge.as_ref(),
        bridge_error,
        snapshot,
    ))
}

fn with_runtime(
    mut status: WorkBuddyHookStatus,
    bridge: Option<&WorkBuddyBridgeInfo>,
    bridge_error: Option<String>,
    snapshot: &workbuddy::WorkBuddySnapshot,
) -> WorkBuddyHookStatus {
    status.bridge_ready = bridge.is_some() && bridge_error.is_none();
    status.error = status
        .error
        .or(bridge_error)
        .or_else(|| snapshot.integration_error.clone());
    // A registry entry cannot prove loading. Only accepted events from the
    // current bridge do; old inbox records cannot establish a new connection.
    let current_sessions = snapshot
        .sessions
        .iter()
        .filter(|session| {
            bridge.is_some_and(|bridge| {
                session.plugin_instance_id.as_deref() == Some(bridge.plugin_instance_id.as_str())
            }) && session.event_count > 0
        })
        .collect::<Vec<_>>();
    status.loaded = status.files_installed && status.enabled && !current_sessions.is_empty();
    status.connected = status.loaded
        && status.bridge_ready
        && status.error.is_none()
        && current_sessions
            .iter()
            .any(|session| session.ended_at.is_none());
    status
}

pub(crate) fn install() -> Result<WorkBuddyHookStatus, String> {
    let _lock = workbuddy_files::lock(&workbuddy_home())?;
    let mut settings_change = workbuddy_files::Change::capture(settings_path(), None)?;
    let before = status()?;
    if matches!(
        before.state,
        WorkBuddyHookInstallState::Modified | WorkBuddyHookInstallState::Conflict
    ) {
        return Err(before
            .error
            .unwrap_or_else(|| "拒绝覆盖已修改的 WorkBuddy 插件".to_string()));
    }
    // Parse and validate the user's configuration before writing any plugin file.
    let settings = settings_with_plugin(true)?;
    let marketplace_manifest = json!({
        "name": INSTALL_ID,
        "description": "CodeCraft WorkBuddy local marketplace",
        "owner": {"name": OWNER},
        "plugins": [{
            "name": INSTALL_ID,
            "source": format!("./{INSTALL_ID}"),
            "description": "CodeCraft read-only WorkBuddy session observer",
            "version": PLUGIN_VERSION
        }]
    });
    let marketplace_bytes =
        serde_json::to_vec_pretty(&marketplace_manifest).map_err(|e| e.to_string())?;
    let mut changes = vec![workbuddy_files::Change::capture(
        marketplace_manifest_path(),
        Some(marketplace_bytes.clone()),
    )?];
    for (relative, content) in plugin_file_specs() {
        changes.push(workbuddy_files::Change::capture(
            plugin_root().join(relative),
            Some(content.as_bytes().to_vec()),
        )?);
    }
    let mut file_hashes = plugin_file_specs()
        .into_iter()
        .map(|(relative, content)| (relative.to_string(), sha256(content.as_bytes())))
        .collect::<HashMap<_, _>>();
    file_hashes.insert(
        "../.codebuddy-plugin/marketplace.json".into(),
        sha256(&marketplace_bytes),
    );
    let manifest = json!({
        "owner": OWNER,
        "installId": INSTALL_ID,
        "pluginVersion": PLUGIN_VERSION,
        "protocol": workbuddy::PROTOCOL,
        "protocolVersion": workbuddy::PROTOCOL_VERSION,
        "fileHashes": file_hashes,
        "installedAt": now_ms(),
    });
    changes.push(workbuddy_files::Change::capture(
        owner_manifest_path(),
        Some(serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?),
    )?);
    settings_change.after = Some(serde_json::to_vec_pretty(&settings).map_err(|e| e.to_string())?);
    changes.push(settings_change);
    workbuddy_files::transaction(changes)?;
    status()
}

pub(crate) fn uninstall() -> Result<(), String> {
    let _lock = workbuddy_files::lock(&workbuddy_home())?;
    let mut settings_change = workbuddy_files::Change::capture(settings_path(), None)?;
    let current = status()?;
    if current.state == WorkBuddyHookInstallState::NotInstalled {
        return Ok(());
    }
    if !matches!(
        current.state,
        WorkBuddyHookInstallState::Installed
            | WorkBuddyHookInstallState::SyncedRestartRequired
            | WorkBuddyHookInstallState::Disabled
            | WorkBuddyHookInstallState::Incompatible
    ) {
        return Err(current
            .error
            .unwrap_or_else(|| "拒绝移除非 CodeCraft 所有的 WorkBuddy 插件".to_string()));
    }
    let settings = settings_with_plugin(false)?;
    settings_change.after = Some(serde_json::to_vec_pretty(&settings).map_err(|e| e.to_string())?);
    let mut changes = vec![settings_change];
    for (relative, _) in plugin_file_specs() {
        let path = plugin_root().join(relative);
        if path.exists() {
            changes.push(workbuddy_files::Change::capture(path, None)?);
        }
    }
    for path in [owner_manifest_path(), marketplace_manifest_path()] {
        if path.exists() {
            changes.push(workbuddy_files::Change::capture(path, None)?);
        }
    }
    workbuddy_files::transaction(changes)?;
    Ok(())
}

pub(crate) fn detect_environment() -> WorkBuddyEnvironmentReport {
    let install_dir = env::var_os("WORKBUDDY_INSTALL_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            let candidate = PathBuf::from(r"D:\software\WorkBuddy");
            candidate.exists().then_some(candidate)
        });
    let desktop_version = install_dir
        .as_ref()
        .and_then(|dir| fs::read_to_string(dir.join("version")).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let cli_package = install_dir.as_ref().map(|dir| {
        dir.join("resources")
            .join("app.asar.unpacked")
            .join("cli")
            .join("package.json")
    });
    let cli_version = cli_package
        .as_ref()
        .and_then(|path| fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| {
            value
                .get("publishConfig")?
                .get("customPackage")?
                .get("version")?
                .as_str()
                .map(str::to_string)
        });
    let docs_available = cli_package
        .as_ref()
        .and_then(|path| path.parent())
        .map(|path| path.join("dist/web-ui/docs/en/cli/hooks.md").is_file())
        .unwrap_or(false);
    let plugin_runtime_version =
        fs::read_to_string(workbuddy_home().join("plugins/installed_plugins.json"))
            .ok()
            .and_then(|text| {
                text.find("5.5.3-wb")
                    .map(|start| text[start..].split('"').next().unwrap_or("").to_string())
            });
    WorkBuddyEnvironmentReport {
        detected: desktop_version.is_some() || cli_version.is_some() || settings_path().exists(),
        desktop_version,
        cli_version,
        plugin_runtime_version,
        executable_path: install_dir.map(|dir| dir.join("WorkBuddy.exe").display().to_string()),
        settings_path: settings_path().display().to_string(),
        docs_available,
        protocol: workbuddy::PROTOCOL,
        protocol_version: workbuddy::PROTOCOL_VERSION,
        protocol_frozen: false,
    }
}

fn validate_headers(
    headers: &HeaderMap,
    plugin_instance_id: &str,
    credentials: &Mutex<BridgeCredential>,
    nonce_store: &Mutex<HashMap<String, Instant>>,
) -> Result<(), Response> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if !content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .eq_ignore_ascii_case("application/json")
    {
        return Err((
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Json(json!({"error":"content_type_required"})),
        )
            .into_response());
    }
    let token = headers
        .get("x-codecraft-workbuddy-token")
        .and_then(|value| value.to_str().ok());
    let plugin_id = headers
        .get("x-codecraft-workbuddy-plugin-id")
        .and_then(|value| value.to_str().ok());
    let protocol = headers
        .get("x-codecraft-workbuddy-protocol")
        .and_then(|value| value.to_str().ok());
    let protocol_version = headers
        .get("x-codecraft-workbuddy-protocol-version")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u32>().ok());
    let credential_valid = credentials.lock().ok().is_some_and(|credential| {
        credential.expires_at > now_ms()
            && token.is_some_and(|token| crate::lan_config::tokens_match(&credential.token, token))
    });
    if !credential_valid
        || plugin_id != Some(plugin_instance_id)
        || protocol != Some(workbuddy::PROTOCOL)
        || protocol_version != Some(workbuddy::PROTOCOL_VERSION)
    {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"unauthorized"})),
        )
            .into_response());
    }
    let Some(nonce) = headers
        .get("x-codecraft-workbuddy-nonce")
        .and_then(|value| value.to_str().ok())
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 128
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        })
    else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"nonce_required"})),
        )
            .into_response());
    };
    let Some(sent_at) = headers
        .get("x-codecraft-workbuddy-sent-at")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
    else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"timestamp_required"})),
        )
            .into_response());
    };
    if now_ms().abs_diff(sent_at) > MAX_CLOCK_SKEW_MS {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"stale_request"})),
        )
            .into_response());
    }
    let mut nonces = nonce_store.lock().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error":"nonce_store_unavailable"})),
        )
            .into_response()
    })?;
    let now = Instant::now();
    nonces.retain(|_, seen| now.duration_since(*seen) <= NONCE_TTL);
    if nonces.contains_key(nonce) {
        return Err((
            StatusCode::CONFLICT,
            Json(json!({"error":"replayed_nonce"})),
        )
            .into_response());
    }
    if nonces.len() >= MAX_NONCES {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error":"nonce_capacity"})),
        )
            .into_response());
    }
    nonces.insert(nonce.to_string(), now);
    Ok(())
}

fn cap_string(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

pub(crate) fn sanitize_payload(input: &Value) -> Value {
    crate::workbuddy_redaction::payload(input)
}

fn normalise_payload(payload: Value, state: &BridgeState, headers: &HeaderMap) -> Value {
    let mut payload = sanitize_payload(&payload);
    let Some(object) = payload.as_object_mut() else {
        return Value::Null;
    };
    // Transport-controlled identity always replaces payload claims.
    object.remove("received_at");
    object.remove("process_id");
    object.remove("process_instance_id");
    object.remove("cwd_hash");
    object.remove("transcript_hash");
    object.insert(
        "event_id".into(),
        json!(headers
            .get("x-codecraft-workbuddy-nonce")
            .and_then(|v| v.to_str().ok())),
    );
    let source_process_id = headers
        .get("x-codecraft-workbuddy-hook-pid")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u32>().ok())
        .and_then(crate::workbuddy_process::host_instance);
    object.insert(
        "plugin_instance_id".to_string(),
        json!(state.plugin_instance_id),
    );
    object.insert("process_instance_id".to_string(), json!(source_process_id));
    if let Some(cwd) = object.get("cwd").and_then(Value::as_str) {
        object.insert("cwd_hash".to_string(), json!(workbuddy::digest_text(cwd)));
    }
    if let Some(transcript) = object.get("transcript_path").and_then(Value::as_str) {
        object.insert(
            "transcript_hash".to_string(),
            json!(workbuddy::digest_text(transcript)),
        );
    }
    object.remove("cwd");
    object.remove("transcript_path");
    object.remove("workbuddy_version");
    if let Some(version) = detect_environment().desktop_version {
        object.insert("workbuddy_version".to_string(), json!(version));
    }
    // The installed package baseline must not replace the version that emitted
    // the event; that would hide upgrades and incompatible CLI instances.
    let reported_version = object
        .get("version")
        .and_then(Value::as_str)
        .map(|s| cap_string(s, 128));
    object.insert("cli_version".into(), json!(reported_version));
    payload
}

fn persist_payload(payload: &Value, received_at: u64) -> Result<String, String> {
    let payload_bytes = serde_json::to_vec(payload).map_err(|error| error.to_string())?;
    let payload_hash = sha256(&payload_bytes);
    let envelope = json!({
        "receivedAt": received_at,
        "payloadHash": payload_hash,
        "payload": payload,
    });
    let bytes = serde_json::to_vec(&envelope).map_err(|error| error.to_string())?;
    if bytes.len() > MAX_BODY_BYTES {
        return Err("脱敏事件超过 inbox 大小限制".into());
    }
    let directory = inbox_dir();
    workbuddy_files::reject_links(&directory)?;
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let path = directory.join(format!("{received_at}-{payload_hash}.json"));
    if !path.exists() {
        let temporary = directory.join(format!(".{payload_hash}.{}.tmp", Uuid::new_v4()));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        file.write_all(&bytes).map_err(|error| error.to_string())?;
        file.flush().map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        fs::rename(temporary, &path).map_err(|error| error.to_string())?;
    }
    prune_inbox(&directory, received_at)?;
    Ok(payload_hash)
}

// Only CodeCraft's generated observation envelopes are retained/removed.
// No decision outbox exists in this read-only release.
fn prune_inbox(directory: &Path, now: u64) -> Result<(), String> {
    workbuddy_files::reject_links(directory)?;
    if !directory.exists() {
        return Ok(());
    }
    for extension in ["json", "consumed", "invalid", "tmp"] {
        let mut entries = fs::read_dir(directory)
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) != Some(extension) {
                    return None;
                }
                let metadata = entry.metadata().ok()?;
                if !metadata.is_file() {
                    return None;
                }
                let at = metadata
                    .modified()
                    .ok()?
                    .duration_since(UNIX_EPOCH)
                    .ok()?
                    .as_millis() as u64;
                Some((path, metadata.len(), at))
            })
            .collect::<Vec<_>>();
        entries.sort_by_key(|(_, _, at)| *at);
        let mut total: u64 = entries.iter().map(|(_, size, _)| size).sum();
        let mut count = entries.len();
        for (path, size, at) in entries {
            if now.saturating_sub(at) > INBOX_RETENTION_MS
                || count > MAX_INBOX_FILES
                || total > MAX_INBOX_BYTES
            {
                workbuddy_files::reject_links(&path)?;
                fs::remove_file(path).map_err(|e| e.to_string())?;
                total = total.saturating_sub(size);
                count -= 1;
            }
        }
    }
    Ok(())
}

async fn post_event(State(state): State<BridgeState>, headers: HeaderMap, body: Bytes) -> Response {
    handle_event(state, headers, body, false).await
}

async fn post_interaction(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    handle_event(state, headers, body, true).await
}

async fn handle_event(
    state: BridgeState,
    headers: HeaderMap,
    body: Bytes,
    interaction: bool,
) -> Response {
    if let Err(response) = validate_headers(
        &headers,
        &state.plugin_instance_id,
        &state.credentials,
        &state.nonces,
    ) {
        return response;
    }
    if body.len() > MAX_BODY_BYTES {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(json!({"error":"body_too_large"})),
        )
            .into_response();
    }
    let Ok(payload) = serde_json::from_slice::<Value>(&body) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"invalid_json"})),
        )
            .into_response();
    };
    if !payload.is_object() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"object_required"})),
        )
            .into_response();
    }
    let event_name = payload
        .get("hook_event_name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !SUPPORTED_EVENTS.contains(&event_name) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"unknown_event"})),
        )
            .into_response();
    }
    let interaction_event = matches!(
        event_name,
        "PreToolUse" | "PermissionRequest" | "Elicitation"
    );
    if interaction != interaction_event {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"route_mismatch"})),
        )
            .into_response();
    }
    let received_at = now_ms();
    let payload = normalise_payload(payload, &state, &headers);
    let Ok(payload_hash) = persist_payload(&payload, received_at) else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error":"inbox_unavailable"})),
        )
            .into_response();
    };
    let app_state = state.app.state::<WorkBuddyIntegrationState>();
    if let Ok(mut store) = app_state.store.lock() {
        store.ingest(&payload, &payload_hash, received_at);
        let path = inbox_dir().join(format!("{received_at}-{payload_hash}.json"));
        let _ = fs::rename(&path, path.with_extension("consumed"));
    }
    Json(json!({"ok":true,"mode":"observe","protocolFrozen":false})).into_response()
}

async fn get_capabilities() -> impl IntoResponse {
    let report = detect_environment();
    Json(json!({
        "protocol": workbuddy::PROTOCOL,
        "protocolVersion": workbuddy::PROTOCOL_VERSION,
        "desktopVersion": report.desktop_version,
        "cliVersion": report.cli_version,
        "canObserve": true,
        "canApproveTools": false,
        "canAnswerQuestions": false,
        "canApprovePlans": false,
        "canStreamOutput": false,
        "protocolFrozen": false,
        "reason": "WorkBuddy 仅支持只读观察，请前往 WorkBuddy 中处理",
    }))
}

async fn get_health(State(state): State<BridgeState>) -> impl IntoResponse {
    Json(json!({
        "ok":true,
        "bridgeInstanceId":state.bridge_instance_id,
        "pluginInstanceId":state.plugin_instance_id,
        "startedAt":state.started_at,
        "protocol":workbuddy::PROTOCOL,
        "protocolVersion":workbuddy::PROTOCOL_VERSION,
    }))
}

async fn post_health(State(state): State<BridgeState>, headers: HeaderMap) -> Response {
    if let Err(response) = validate_headers(
        &headers,
        &state.plugin_instance_id,
        &state.credentials,
        &state.nonces,
    ) {
        return response;
    }
    Json(json!({"ok":true,"mode":"observe","protocolFrozen":false})).into_response()
}

fn rotate_credential(state: &BridgeState) -> Result<(), String> {
    let mut credential = state.credentials.lock().map_err(|e| e.to_string())?;
    if credential.expires_at.saturating_sub(now_ms()) > TOKEN_TTL_MS / 2 {
        return Ok(());
    }
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let expires_at = now_ms().saturating_add(TOKEN_TTL_MS);
    let config = BridgeConfig {
        protocol: workbuddy::PROTOCOL,
        protocol_version: workbuddy::PROTOCOL_VERSION,
        bridge_instance_id: &state.bridge_instance_id,
        plugin_instance_id: &state.plugin_instance_id,
        endpoint: &state.endpoint,
        token: &token,
        started_at: state.started_at,
        expires_at,
    };
    // Runtime tokens are disposable; the startup before-image is backed up.
    atomic_write(
        &bridge_config_path(),
        &serde_json::to_vec(&config).map_err(|e| e.to_string())?,
        false,
    )?;
    *credential = BridgeCredential { token, expires_at };
    Ok(())
}

pub(crate) fn start_bridge(app: tauri::AppHandle) -> Result<WorkBuddyBridgeInfo, String> {
    let instance_lock = Arc::new(workbuddy_files::lock(
        &base_data_dir().join("workbuddy-hook"),
    )?);
    let bridge_instance_id = Uuid::new_v4().to_string();
    let plugin_instance_id = Uuid::new_v4().to_string();
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let started_at = now_ms();
    let expires_at = started_at.saturating_add(TOKEN_TTL_MS);
    let listener = tauri::async_runtime::block_on(async {
        TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await
    })
    .map_err(|error| format!("无法监听 WorkBuddy 本地桥：{error}"))?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    let endpoint = format!("http://127.0.0.1:{}", address.port());
    let config = BridgeConfig {
        protocol: workbuddy::PROTOCOL,
        protocol_version: workbuddy::PROTOCOL_VERSION,
        bridge_instance_id: &bridge_instance_id,
        plugin_instance_id: &plugin_instance_id,
        endpoint: &endpoint,
        token: &token,
        started_at,
        expires_at,
    };
    atomic_write(
        &bridge_config_path(),
        serde_json::to_vec_pretty(&config)
            .map_err(|error| error.to_string())?
            .as_slice(),
        true,
    )?;
    let state = BridgeState {
        app: app.clone(),
        bridge_instance_id: bridge_instance_id.clone(),
        plugin_instance_id: plugin_instance_id.clone(),
        credentials: Arc::new(Mutex::new(BridgeCredential { token, expires_at })),
        endpoint: endpoint.clone(),
        _instance_lock: instance_lock,
        started_at,
        nonces: Arc::new(Mutex::new(HashMap::new())),
    };
    let router = Router::new()
        .route("/api/workbuddy/events", post(post_event))
        .route("/api/workbuddy/interactions", post(post_interaction))
        .route("/api/workbuddy/capabilities", get(get_capabilities))
        .route("/api/workbuddy/health", get(get_health).post(post_health))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state.clone());
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        loop {
            tick.tick().await;
            let result =
                rotate_credential(&state).and_then(|_| prune_inbox(&inbox_dir(), now_ms()));
            let integration = state.app.state::<WorkBuddyIntegrationState>();
            if let Ok(mut error) = integration.hook_error.lock() {
                *error = result.err();
            }
            if let Ok(mut store) = integration.store.lock() {
                store.maintain(now_ms());
            };
        }
    });
    tauri::async_runtime::spawn(async move {
        let result = axum::serve(listener, router).await;
        let integration = app.state::<WorkBuddyIntegrationState>();
        if let Ok(mut bridge) = integration.bridge.lock() {
            *bridge = None;
        }
        if let Ok(mut error) = integration.hook_error.lock() {
            *error = Some(format!("WorkBuddy 本地桥已停止：{result:?}"));
        };
    });
    Ok(WorkBuddyBridgeInfo {
        bridge_instance_id,
        plugin_instance_id,
        endpoint,
        started_at,
    })
}

pub(crate) fn drain_inbox_events(app: &tauri::AppHandle) -> Result<usize, String> {
    let directory = inbox_dir();
    workbuddy_files::reject_links(&directory)?;
    if !directory.exists() {
        return Ok(0);
    }
    let mut paths = fs::read_dir(&directory)
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    paths.sort();
    let mut count = 0;
    let state = app.state::<WorkBuddyIntegrationState>();
    for path in paths.into_iter().take(100) {
        if workbuddy_files::reject_links(&path).is_err() {
            continue;
        }
        let Ok(bytes) = workbuddy_files::read_bounded(&path, MAX_BODY_BYTES as u64) else {
            continue;
        };
        let Ok(envelope) = serde_json::from_slice::<Value>(&bytes) else {
            let _ = fs::rename(&path, path.with_extension("invalid"));
            continue;
        };
        let Some(payload) = envelope.get("payload") else {
            let _ = fs::rename(&path, path.with_extension("invalid"));
            continue;
        };
        let hash = envelope
            .get("payloadHash")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let received_at = envelope
            .get("receivedAt")
            .and_then(Value::as_u64)
            .unwrap_or_else(now_ms);
        if !payload.is_object()
            || sha256(&serde_json::to_vec(payload).unwrap_or_default()) != hash
            || received_at > now_ms().saturating_add(MAX_CLOCK_SKEW_MS)
        {
            let _ = fs::rename(&path, path.with_extension("invalid"));
            continue;
        }
        if let Ok(mut store) = state.store.lock() {
            store.ingest(&sanitize_payload(payload), hash, received_at);
        } else {
            continue;
        }
        let marker = path.with_extension("consumed");
        let _ = fs::rename(path, marker);
        count += 1;
    }
    Ok(count)
}

pub fn capture_workbuddy_hook() -> Result<(), String> {
    let mut input = Vec::new();
    io::stdin()
        .take((MAX_BODY_BYTES + 1) as u64)
        .read_to_end(&mut input)
        .map_err(|error| error.to_string())?;
    if input.len() > MAX_BODY_BYTES {
        return Err("WorkBuddy Hook 输入超过大小限制".to_string());
    }
    let trimmed = std::str::from_utf8(&input)
        .map_err(|error| error.to_string())?
        .trim();
    if trimmed.is_empty() {
        println!("{{}}");
        return Ok(());
    }
    serde_json::from_str::<Value>(trimmed)
        .map_err(|error| format!("WorkBuddy Hook 输入必须是单个 JSON 文档：{error}"))?;
    println!("{{}}");
    io::stdout().flush().map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (key, value) in [
            ("content-type", "application/json".to_string()),
            ("x-codecraft-workbuddy-token", "a".repeat(64)),
            ("x-codecraft-workbuddy-plugin-id", "plugin".to_string()),
            (
                "x-codecraft-workbuddy-protocol",
                workbuddy::PROTOCOL.to_string(),
            ),
            ("x-codecraft-workbuddy-protocol-version", "1".to_string()),
            ("x-codecraft-workbuddy-nonce", "nonce-1".to_string()),
            ("x-codecraft-workbuddy-sent-at", now_ms().to_string()),
        ] {
            headers.insert(
                axum::http::HeaderName::from_bytes(key.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
        }
        headers
    }

    #[test]
    fn bridge_auth_rejects_expired_tokens_invalid_mime_and_replays() {
        let credentials = Mutex::new(BridgeCredential {
            token: "a".repeat(64),
            expires_at: now_ms() + TOKEN_TTL_MS,
        });
        let nonces = Mutex::new(HashMap::new());
        let headers = valid_headers();
        assert!(validate_headers(&headers, "plugin", &credentials, &nonces).is_ok());
        assert_eq!(
            validate_headers(&headers, "plugin", &credentials, &nonces)
                .unwrap_err()
                .status(),
            StatusCode::CONFLICT
        );
        let mut invalid = valid_headers();
        invalid.insert(
            header::CONTENT_TYPE,
            "application/json-malicious".parse().unwrap(),
        );
        assert_eq!(
            validate_headers(&invalid, "plugin", &credentials, &nonces)
                .unwrap_err()
                .status(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE
        );
        credentials.lock().unwrap().expires_at = now_ms() - 1;
        assert_eq!(
            validate_headers(&headers, "plugin", &credentials, &nonces)
                .unwrap_err()
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }

    #[test]
    fn nonce_flood_does_not_evict_live_replay_protection() {
        let credentials = Mutex::new(BridgeCredential {
            token: "a".repeat(64),
            expires_at: now_ms() + TOKEN_TTL_MS,
        });
        let nonces = Mutex::new(
            (0..MAX_NONCES)
                .map(|i| (format!("used-{i}"), Instant::now()))
                .collect(),
        );
        let headers = valid_headers();
        assert_eq!(
            validate_headers(&headers, "plugin", &credentials, &nonces)
                .unwrap_err()
                .status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        assert_eq!(nonces.lock().unwrap().len(), MAX_NONCES);
        assert!(nonces.lock().unwrap().contains_key("used-0"));
    }

    #[test]
    fn inbox_cleanup_caps_consumed_records_and_preserves_unrelated_files() {
        let root = Scratch::new();
        for i in 0..MAX_INBOX_FILES + 3 {
            fs::write(root.0.join(format!("{i}.consumed")), b"{}").unwrap();
        }
        fs::write(root.0.join("user-note.txt"), b"keep").unwrap();
        prune_inbox(&root.0, now_ms()).unwrap();
        let count = fs::read_dir(&root.0)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().and_then(|s| s.to_str()) == Some("consumed"))
            .count();
        assert_eq!(count, MAX_INBOX_FILES);
        assert_eq!(fs::read(root.0.join("user-note.txt")).unwrap(), b"keep");
    }

    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .join(format!("workbuddy-install-{}", Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            TEST_HOME.with(|value| *value.borrow_mut() = Some(path.clone()));
            Self(path)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            TEST_HOME.with(|value| *value.borrow_mut() = None);
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn installation_enablement_registration_and_runtime_are_independent() {
        let root = Scratch::new();
        let installed = install().unwrap();
        assert!(installed.installed());
        assert!(installed.enabled);
        assert!(!installed.registered);
        assert!(!installed.loaded && !installed.connected);
        assert_eq!(
            installed.state,
            WorkBuddyHookInstallState::SyncedRestartRequired
        );
        assert!(installed.error.is_none());

        fs::write(
            root.0.join("plugins/installed_plugins.json"),
            serde_json::to_vec(
                &json!({"plugins":{SETTINGS_PLUGIN_KEY:[{"version":PLUGIN_VERSION}]}}),
            )
            .unwrap(),
        )
        .unwrap();
        let registered = status().unwrap();
        assert!(registered.installed() && registered.registered);
        assert!(!registered.loaded && !registered.connected);

        let mut settings = settings_with_plugin(true).unwrap();
        settings["enabledPlugins"][SETTINGS_PLUGIN_KEY] = json!(false);
        fs::write(settings_path(), serde_json::to_vec(&settings).unwrap()).unwrap();
        let disabled = status().unwrap();
        assert!(disabled.installed());
        assert!(!disabled.enabled);
        assert_eq!(disabled.state, WorkBuddyHookInstallState::Disabled);
        assert!(disabled.error.is_none());
        // Disabled and pending installations must still offer a working uninstall.
        uninstall().unwrap();
        assert!(!status().unwrap().installed());
        install().unwrap();
        assert!(status().unwrap().enabled);
    }

    #[test]
    fn connection_requires_current_bridge_events_and_recovers_without_reinstall() {
        let _root = Scratch::new();
        install().unwrap();
        let bridge = WorkBuddyBridgeInfo {
            bridge_instance_id: "bridge-new".into(),
            plugin_instance_id: "plugin-new".into(),
            endpoint: "http://127.0.0.1:9999".into(),
            started_at: 1,
        };
        let mut store = workbuddy::WorkBuddyStore::default();
        assert!(!store.snapshot().connected);
        let waiting = with_runtime(status().unwrap(), Some(&bridge), None, &store.snapshot());
        assert!(waiting.installed() && waiting.bridge_ready);
        assert!(!waiting.loaded && !waiting.connected);

        let old = json!({"hook_event_name":"SessionStart","session_id":"old","plugin_instance_id":"plugin-old"});
        store.ingest(&old, "old-hash", 1);
        let stale = with_runtime(status().unwrap(), Some(&bridge), None, &store.snapshot());
        assert!(!stale.loaded && !stale.connected);
        let current = json!({"hook_event_name":"SessionStart","session_id":"current","plugin_instance_id":"plugin-new"});
        store.ingest(&current, "new-hash", 2);
        let connected = with_runtime(status().unwrap(), Some(&bridge), None, &store.snapshot());
        assert!(connected.loaded && connected.connected);
        // Runtime evidence works even if WorkBuddy has not updated its registry.
        assert!(!connected.registered);
        let failed = with_runtime(
            status().unwrap(),
            Some(&bridge),
            Some("bridge failed".into()),
            &store.snapshot(),
        );
        assert!(failed.installed() && failed.loaded);
        assert!(!failed.connected && !failed.bridge_ready);
        assert!(with_runtime(status().unwrap(), Some(&bridge), None, &store.snapshot()).connected);

        let ended = json!({"hook_event_name":"SessionEnd","session_id":"current","plugin_instance_id":"plugin-new"});
        store.ingest(&ended, "ended-hash", 3);
        let idle = with_runtime(status().unwrap(), Some(&bridge), None, &store.snapshot());
        assert!(idle.loaded);
        assert!(!idle.connected);
        uninstall().unwrap();
        let removed = with_runtime(status().unwrap(), Some(&bridge), None, &store.snapshot());
        assert!(!removed.installed() && !removed.loaded && !removed.connected);
    }

    #[test]
    fn install_uninstall_preserves_other_settings_and_first_backup() {
        let _root = Scratch::new();
        let original = br#"{/* user comments */"hooks":{"Stop":[]},"enabledPlugins":{"other@market":true},"unknown":{"keep":7},}"#;
        fs::write(settings_path(), original).unwrap();
        install().unwrap();
        let settings = workbuddy_files::parse_jsonc(&fs::read(settings_path()).unwrap()).unwrap();
        assert_eq!(settings["unknown"]["keep"], 7);
        assert_eq!(settings["enabledPlugins"]["other@market"], true);
        let unrelated = plugin_root().join("user-notes.txt");
        fs::write(&unrelated, b"keep").unwrap();
        uninstall().unwrap();
        assert_eq!(
            status().unwrap().state,
            WorkBuddyHookInstallState::NotInstalled
        );
        assert_eq!(
            fs::read(settings_path().with_file_name("settings.json.bak")).unwrap(),
            original
        );
        assert_eq!(fs::read(&unrelated).unwrap(), b"keep");
        assert!(!owner_manifest_path().exists());
        assert!(plugin_file_specs()
            .iter()
            .all(|(path, _)| !plugin_root().join(path).exists()));
        install().unwrap();
        assert_eq!(
            fs::read(settings_path().with_file_name("settings.json.bak")).unwrap(),
            original
        );
    }

    #[test]
    fn malformed_settings_do_not_leave_a_partial_plugin_installation() {
        let _root = Scratch::new();
        fs::write(settings_path(), b"{invalid").unwrap();
        assert!(install().is_err());
        assert!(!owner_manifest_path().exists());
        assert!(!marketplace_manifest_path().exists());
        assert_eq!(fs::read(settings_path()).unwrap(), b"{invalid");
    }

    #[test]
    fn modified_marketplace_or_plugin_is_not_replaced_or_uninstalled() {
        let _root = Scratch::new();
        install().unwrap();
        fs::write(plugin_root().join("hooks/hooks.json"), b"user edit").unwrap();
        assert!(install().is_err());
        assert!(uninstall().is_err());
        assert_eq!(
            fs::read(plugin_root().join("hooks/hooks.json")).unwrap(),
            b"user edit"
        );
    }

    #[test]
    fn conflicting_marketplace_settings_are_preserved() {
        let _root = Scratch::new();
        let user = json!({"extraKnownMarketplaces":{INSTALL_ID:{"source":{"source":"directory","path":"D:/other-plugin"}}}});
        fs::write(settings_path(), serde_json::to_vec(&user).unwrap()).unwrap();
        assert!(install().is_err());
        assert!(!owner_manifest_path().exists());
        assert_eq!(
            workbuddy_files::parse_jsonc(&fs::read(settings_path()).unwrap()).unwrap(),
            user
        );
    }

    #[test]
    fn sanitizer_removes_secrets_and_caps_urls() {
        let value = json!({
            "token":"secret",
            "url":"https://example.test/path?access_token=secret",
            "tool_input":{"command":"x".repeat(5000)}
        });
        let safe = sanitize_payload(&value);
        assert!(safe["token"].is_null());
        assert_eq!(safe["url"], "https://example.test/path");
        assert!(
            safe["tool_input"]["command"]
                .as_str()
                .unwrap()
                .chars()
                .count()
                <= 4096
        );
        assert!(safe["tool_input"]["command"]
            .as_str()
            .unwrap()
            .contains("sha256="));
    }

    #[test]
    fn supported_events_include_interaction_and_lifecycle_events() {
        assert!(SUPPORTED_EVENTS.contains(&"PreToolUse"));
        assert!(SUPPORTED_EVENTS.contains(&"AskUserQuestion") == false);
        assert!(SUPPORTED_EVENTS.contains(&"SessionEnd"));
    }

    #[test]
    fn environment_report_is_explicitly_read_only_until_fixtures_are_frozen() {
        let report = detect_environment();
        assert!(!report.protocol_frozen);
        assert_eq!(report.protocol_version, workbuddy::PROTOCOL_VERSION);
    }
}
