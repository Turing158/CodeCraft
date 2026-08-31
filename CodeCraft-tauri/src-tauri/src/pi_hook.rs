//! PI global-extension ownership and the versioned file-IPC envelope.
//!
//! PI global-extension ownership, lifecycle observation, and approval IPC.

#![allow(dead_code)]

use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub(crate) const SCHEMA_VERSION: &str = "1.0";
pub(crate) const PROTOCOL_VERSION: &str = "1.0";
pub(crate) const EXTENSION_VERSION: &str = "0.2.1";
pub(crate) const MAX_ENVELOPE_BYTES: u64 = 256 * 1024;
pub(crate) const MAX_ID_LENGTH: usize = 128;
pub(crate) const NO_EXPIRY: u64 = 0;

const OWNER: &str = "CodeCraft";
const EXTENSION_FILE_NAME: &str = "index.ts";
const MANIFEST_FILE_NAME: &str = ".codecraft-pi.json";
const BUNDLED_EXTENSION: &str = include_str!("../assets/pi/codecraft/index.ts");

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
    base_data_dir().join("pi-hook")
}

pub(crate) fn instances_dir() -> PathBuf {
    hook_dir().join("instances")
}

pub(crate) fn inbox_dir(extension_instance_id: &str) -> Result<PathBuf, String> {
    Ok(hook_dir().join("inbox").join(validated_component(
        extension_instance_id,
        "extensionInstanceId",
    )?))
}

pub(crate) fn outbox_dir(extension_instance_id: &str) -> Result<PathBuf, String> {
    Ok(hook_dir().join("outbox").join(validated_component(
        extension_instance_id,
        "extensionInstanceId",
    )?))
}

pub(crate) fn processing_dir(extension_instance_id: &str) -> Result<PathBuf, String> {
    Ok(hook_dir().join("processing").join(validated_component(
        extension_instance_id,
        "extensionInstanceId",
    )?))
}

pub(crate) fn receipts_dir(extension_instance_id: &str) -> Result<PathBuf, String> {
    Ok(hook_dir().join("receipts").join(validated_component(
        extension_instance_id,
        "extensionInstanceId",
    )?))
}

fn validated_component<'a>(value: &'a str, label: &str) -> Result<&'a str, String> {
    if value.is_empty() || value.len() > MAX_ID_LENGTH {
        return Err(format!("{label} must be 1-{MAX_ID_LENGTH} bytes"));
    }
    if value == "."
        || value == ".."
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(format!("{label} contains an invalid path component"));
    }
    Ok(value)
}

fn validated_opaque(value: &str, label: &str, max_length: usize) -> Result<(), String> {
    if value.is_empty() || value.len() > max_length || value.contains('\0') {
        return Err(format!(
            "{label} must be non-empty and at most {max_length} bytes"
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PiEnvelope {
    pub(crate) schema_version: String,
    pub(crate) protocol_version: String,
    pub(crate) message_type: String,
    pub(crate) message_id: String,
    pub(crate) install_id: String,
    pub(crate) extension_instance_id: String,
    pub(crate) endpoint_epoch: String,
    pub(crate) stream_id: String,
    pub(crate) stream_epoch: String,
    pub(crate) session_id: Option<String>,
    pub(crate) run_id: Option<String>,
    #[serde(default)]
    pub(crate) cwd: Option<String>,
    #[serde(default)]
    pub(crate) origin: Option<String>,
    #[serde(default)]
    pub(crate) event_type: Option<String>,
    pub(crate) sequence: u64,
    pub(crate) created_at: u64,
    pub(crate) expires_at: u64,
    pub(crate) payload: Value,
}

impl PiEnvelope {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(format!(
                "Unsupported PI schema version {}",
                self.schema_version
            ));
        }
        if self.protocol_version != PROTOCOL_VERSION {
            return Err(format!(
                "Unsupported PI protocol version {}",
                self.protocol_version
            ));
        }
        if !matches!(
            self.message_type.as_str(),
            "event" | "request" | "decision" | "receipt" | "heartbeat" | "checkpoint" | "nack"
        ) {
            return Err(format!("Unsupported PI message type {}", self.message_type));
        }
        validated_component(&self.message_id, "messageId")?;
        Uuid::parse_str(&self.message_id)
            .map_err(|_| "messageId must be an independent UUID".to_string())?;
        validated_component(&self.install_id, "installId")?;
        validated_component(&self.extension_instance_id, "extensionInstanceId")?;
        validated_opaque(&self.endpoint_epoch, "endpointEpoch", 256)?;
        validated_opaque(&self.stream_id, "streamId", 512)?;
        validated_component(&self.stream_epoch, "streamEpoch")?;
        if let Some(session_id) = self.session_id.as_deref() {
            validated_component(session_id, "sessionId")?;
        }
        if let Some(run_id) = self.run_id.as_deref() {
            validated_component(run_id, "runId")?;
        }
        if let Some(cwd) = self.cwd.as_deref() {
            validated_opaque(cwd, "cwd", 4096)?;
        }
        if let Some(origin) = self.origin.as_deref() {
            validated_opaque(origin, "origin", 64)?;
        }
        if let Some(event_type) = self.event_type.as_deref() {
            validated_opaque(event_type, "eventType", 128)?;
        }
        if self.expires_at != NO_EXPIRY && self.expires_at < self.created_at {
            return Err("expiresAt must not be earlier than createdAt".to_string());
        }
        let payload_size = serde_json::to_vec(&self.payload)
            .map_err(|error| format!("Unable to serialize PI payload: {error}"))?
            .len();
        if payload_size > MAX_ENVELOPE_BYTES as usize {
            return Err("PI payload exceeded its size limit".to_string());
        }
        Ok(())
    }

    pub(crate) fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() as u64 > MAX_ENVELOPE_BYTES {
            return Err("PI envelope exceeded its size limit".to_string());
        }
        let envelope = serde_json::from_slice::<Self>(bytes)
            .map_err(|error| format!("Invalid PI envelope: {error}"))?;
        envelope.validate()?;
        Ok(envelope)
    }
}

pub(crate) fn stable_request_id(
    install_id: &str,
    session_id: &str,
    run_id: &str,
    turn_index: u64,
    tool_call_id: &str,
    tool_name: &str,
    canonical_input_digest: &str,
) -> String {
    let material = format!(
        "{PROTOCOL_VERSION}|{install_id}|{session_id}|{run_id}|{turn_index}|{tool_call_id}|{tool_name}|{canonical_input_digest}"
    );
    sha256(material.as_bytes())
}

pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() as u64 > MAX_ENVELOPE_BYTES {
        return Err("PI file payload exceeded its size limit".to_string());
    }
    let parent = path
        .parent()
        .ok_or_else(|| "PI target has no parent directory".to_string())?;
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

pub(crate) fn write_inbox_envelope(envelope: &PiEnvelope) -> Result<PathBuf, String> {
    envelope.validate()?;
    let directory = inbox_dir(&envelope.extension_instance_id)?;
    let path = directory.join(format!("{}.json", envelope.message_id));
    let bytes = serde_json::to_vec(envelope).map_err(|error| error.to_string())?;
    atomic_write(&path, &bytes)?;
    Ok(path)
}

pub(crate) fn write_decision(
    request: &PiEnvelope,
    request_id: &str,
    decision: &str,
    actor: &str,
) -> Result<PathBuf, String> {
    write_review_decision(request, request_id, decision, actor, Value::Null)
}

pub(crate) fn write_review_decision(
    request: &PiEnvelope,
    request_id: &str,
    decision: &str,
    actor: &str,
    details: Value,
) -> Result<PathBuf, String> {
    validated_opaque(request_id, "requestId", MAX_ID_LENGTH)?;
    if !matches!(
        decision,
        "allowOnce" | "allowSession" | "deny" | "answer" | "execute" | "refine"
    ) {
        return Err("Unsupported PI approval decision".to_string());
    }
    if !details.is_null() && !details.is_object() {
        return Err("PI decision details must be an object".to_string());
    }
    let now = now_ms();
    let mut payload = json!({
        "requestId": request_id,
        "decision": decision,
        "actor": actor,
        "decidedAt": now,
    });
    if let (Some(payload), Some(details)) = (payload.as_object_mut(), details.as_object()) {
        payload.extend(details.clone());
    }
    let envelope = PiEnvelope {
        schema_version: SCHEMA_VERSION.to_string(),
        protocol_version: PROTOCOL_VERSION.to_string(),
        message_type: "decision".to_string(),
        message_id: Uuid::new_v4().to_string(),
        install_id: request.install_id.clone(),
        extension_instance_id: request.extension_instance_id.clone(),
        endpoint_epoch: request.endpoint_epoch.clone(),
        stream_id: request.stream_id.clone(),
        stream_epoch: request.stream_epoch.clone(),
        session_id: request.session_id.clone(),
        run_id: request.run_id.clone(),
        cwd: request.cwd.clone(),
        origin: Some("codecraft".to_string()),
        event_type: Some("decision".to_string()),
        sequence: request.sequence,
        created_at: now,
        expires_at: NO_EXPIRY,
        payload,
    };
    envelope.validate()?;
    let directory = outbox_dir(&envelope.extension_instance_id)?;
    let path = directory.join(format!("{}.json", envelope.message_id));
    let bytes = serde_json::to_vec(&envelope).map_err(|error| error.to_string())?;
    atomic_write(&path, &bytes)?;
    Ok(path)
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PiHookInstallState {
    NotInstalled,
    Installed,
    Modified,
    Conflict,
    Incompatible,
    Error,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PiHookStatus {
    pub(crate) state: PiHookInstallState,
    pub(crate) install_path: String,
    pub(crate) bundled_version: &'static str,
    pub(crate) installed_version: Option<String>,
    pub(crate) install_id: Option<String>,
    pub(crate) error: Option<String>,
}

impl PiHookStatus {
    pub(crate) fn installed(&self) -> bool {
        self.state == PiHookInstallState::Installed
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
struct InstallManifest {
    owner: String,
    install_id: String,
    extension_version: String,
    protocol_version: String,
    sha256: String,
    installed_at: u64,
}

fn default_agent_dir() -> Result<PathBuf, String> {
    if let Some(override_path) = env::var_os("PI_CODING_AGENT_DIR") {
        let path = PathBuf::from(override_path);
        if !path.is_absolute() {
            return Err("PI_CODING_AGENT_DIR must be an absolute path".to_string());
        }
        return Ok(path);
    }
    let home = env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
        .ok_or_else(|| "Unable to locate the user profile for PI".to_string())?;
    Ok(home.join(".pi").join("agent"))
}

fn extension_dir(agent_dir: &Path) -> PathBuf {
    agent_dir.join("extensions").join("codecraft")
}

fn extension_path(agent_dir: &Path) -> PathBuf {
    extension_dir(agent_dir).join(EXTENSION_FILE_NAME)
}

fn manifest_path(agent_dir: &Path) -> PathBuf {
    extension_dir(agent_dir).join(MANIFEST_FILE_NAME)
}

fn read_manifest(agent_dir: &Path) -> Result<Option<InstallManifest>, String> {
    let path = manifest_path(agent_dir);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| format!("Invalid CodeCraft PI ownership manifest: {error}"))
}

fn status_at(agent_dir: &Path) -> PiHookStatus {
    let install_path = extension_path(agent_dir).display().to_string();
    let extension_exists = extension_path(agent_dir).exists();
    let manifest = match read_manifest(agent_dir) {
        Ok(manifest) => manifest,
        Err(error) => {
            return PiHookStatus {
                state: PiHookInstallState::Error,
                install_path,
                bundled_version: EXTENSION_VERSION,
                installed_version: None,
                install_id: None,
                error: Some(error),
            }
        }
    };
    let Some(manifest) = manifest else {
        return PiHookStatus {
            state: if extension_exists {
                PiHookInstallState::Conflict
            } else {
                PiHookInstallState::NotInstalled
            },
            install_path,
            bundled_version: EXTENSION_VERSION,
            installed_version: None,
            install_id: None,
            error: extension_exists.then(|| {
                "A PI extension with the CodeCraft path exists without an ownership manifest"
                    .to_string()
            }),
        };
    };
    let installed_version = Some(manifest.extension_version.clone());
    let install_id = Some(manifest.install_id.clone());
    if manifest.owner != OWNER {
        return PiHookStatus {
            state: PiHookInstallState::Conflict,
            install_path,
            bundled_version: EXTENSION_VERSION,
            installed_version,
            install_id,
            error: Some("The PI extension manifest is not owned by CodeCraft".to_string()),
        };
    }
    if manifest.protocol_version != PROTOCOL_VERSION {
        return PiHookStatus {
            state: PiHookInstallState::Incompatible,
            install_path,
            bundled_version: EXTENSION_VERSION,
            installed_version,
            install_id,
            error: Some(format!(
                "Installed PI protocol {} is incompatible with {}",
                manifest.protocol_version, PROTOCOL_VERSION
            )),
        };
    }
    let Ok(bytes) = fs::read(extension_path(agent_dir)) else {
        return PiHookStatus {
            state: PiHookInstallState::Modified,
            install_path,
            bundled_version: EXTENSION_VERSION,
            installed_version,
            install_id,
            error: Some("The managed PI extension file is missing".to_string()),
        };
    };
    if sha256(&bytes) != manifest.sha256 {
        return PiHookStatus {
            state: PiHookInstallState::Modified,
            install_path,
            bundled_version: EXTENSION_VERSION,
            installed_version,
            install_id,
            error: Some("The managed PI extension was modified after installation".to_string()),
        };
    }
    PiHookStatus {
        state: if manifest.extension_version == EXTENSION_VERSION {
            PiHookInstallState::Installed
        } else {
            PiHookInstallState::Incompatible
        },
        install_path,
        bundled_version: EXTENSION_VERSION,
        installed_version,
        install_id,
        error: None,
    }
}

fn install_at(agent_dir: &Path) -> Result<PiHookStatus, String> {
    let before = status_at(agent_dir);
    if matches!(
        before.state,
        PiHookInstallState::Modified | PiHookInstallState::Conflict | PiHookInstallState::Error
    ) {
        return Err(before
            .error
            .unwrap_or_else(|| "PI extension ownership validation failed".to_string()));
    }
    let install_id = before
        .install_id
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let extension_path = extension_path(agent_dir);
    if extension_path.exists() {
        fs::copy(&extension_path, extension_path.with_extension("ts.bak"))
            .map_err(|error| format!("Unable to back up the existing PI extension: {error}"))?;
    }
    atomic_write(&extension_path, BUNDLED_EXTENSION.as_bytes())?;
    let manifest = InstallManifest {
        owner: OWNER.to_string(),
        install_id,
        extension_version: EXTENSION_VERSION.to_string(),
        protocol_version: PROTOCOL_VERSION.to_string(),
        sha256: sha256(BUNDLED_EXTENSION.as_bytes()),
        installed_at: now_ms(),
    };
    let bytes = serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?;
    atomic_write(&manifest_path(agent_dir), &bytes)?;
    Ok(status_at(agent_dir))
}

pub(crate) fn status() -> Result<PiHookStatus, String> {
    Ok(status_at(&default_agent_dir()?))
}

pub(crate) fn install() -> Result<PiHookStatus, String> {
    install_at(&default_agent_dir()?)
}

pub(crate) fn uninstall() -> Result<(), String> {
    let agent_dir = default_agent_dir()?;
    let status = status_at(&agent_dir);
    if status.state == PiHookInstallState::NotInstalled {
        return Ok(());
    }
    if status.state != PiHookInstallState::Installed {
        return Err(status.error.unwrap_or_else(|| {
            "Refusing to remove an unowned or modified PI extension".to_string()
        }));
    }
    fs::remove_file(extension_path(&agent_dir)).map_err(|error| error.to_string())?;
    fs::remove_file(manifest_path(&agent_dir)).map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_dir(name: &str) -> PathBuf {
        let root = env::temp_dir().join(format!("codecraft-pi-hook-{name}-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn envelope(message_type: &str) -> PiEnvelope {
        PiEnvelope {
            schema_version: SCHEMA_VERSION.to_string(),
            protocol_version: PROTOCOL_VERSION.to_string(),
            message_type: message_type.to_string(),
            message_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            install_id: "install-1".to_string(),
            extension_instance_id: "instance-1".to_string(),
            endpoint_epoch: "instance-1:generation:1".to_string(),
            stream_id: "control".to_string(),
            stream_epoch: "epoch-1".to_string(),
            session_id: Some("session-1".to_string()),
            run_id: Some("run-1".to_string()),
            cwd: Some("C:/project".to_string()),
            origin: Some("pi".to_string()),
            event_type: Some("event".to_string()),
            sequence: 1,
            created_at: 10,
            expires_at: 20,
            payload: json!({"ok": true}),
        }
    }

    #[test]
    fn validates_and_round_trips_envelope() {
        let value = envelope("event");
        let bytes = serde_json::to_vec(&value).unwrap();
        assert_eq!(PiEnvelope::from_bytes(&bytes).unwrap(), value);
    }

    #[test]
    fn rejects_path_escape_and_unknown_message_type() {
        let mut value = envelope("event");
        value.extension_instance_id = "../escape".to_string();
        assert!(value.validate().is_err());
        let mut value = envelope("event");
        value.message_id = "message-1".to_string();
        assert!(value.validate().is_err());
        let value = envelope("unknown");
        assert!(value.validate().is_err());

        let mut wire = serde_json::to_value(envelope("event")).unwrap();
        wire["unexpected"] = json!(true);
        assert!(PiEnvelope::from_bytes(&serde_json::to_vec(&wire).unwrap()).is_err());
    }

    #[test]
    fn rejects_expired_order_and_incompatible_versions() {
        let mut value = envelope("request");
        value.expires_at = 9;
        assert!(value.validate().is_err());
        value.expires_at = 20;
        assert!(value.validate().is_ok());
        value.expires_at = NO_EXPIRY;
        assert!(value.validate().is_ok());
        value.protocol_version = "2.0".to_string();
        assert!(value.validate().is_err());
    }

    #[test]
    fn stable_request_id_is_repeatable_and_input_bound() {
        let first = stable_request_id("install", "session", "run", 0, "call", "read", "digest");
        let second = stable_request_id("install", "session", "run", 0, "call", "read", "digest");
        let changed = stable_request_id("install", "session", "run", 0, "call", "read", "other");
        assert_eq!(first, second);
        assert_ne!(first, changed);
        assert_eq!(first.len(), 64);
    }

    #[test]
    fn atomic_write_replaces_existing_file_and_writes_inbox() {
        let root = temp_dir("atomic");
        let target = root.join("state.json");
        atomic_write(&target, br#"{"state":"new"}"#).unwrap();
        atomic_write(&target, br#"{"state":"replaced"}"#).unwrap();
        assert_eq!(
            fs::read_to_string(&target).unwrap(),
            r#"{"state":"replaced"}"#
        );

        let inbox = root.join("inbox");
        fs::create_dir_all(&inbox).unwrap();
        let path = inbox.join("550e8400-e29b-41d4-a716-446655440000.json");
        atomic_write(&path, &serde_json::to_vec(&envelope("event")).unwrap()).unwrap();
        assert!(path.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn install_status_and_uninstall_protect_modified_files() {
        let root = temp_dir("install");
        let installed = install_at(&root).unwrap();
        assert!(installed.installed());
        assert!(extension_path(&root).exists());
        assert!(manifest_path(&root).exists());
        assert!(extension_path(&root).with_extension("ts.bak").exists() == false);

        fs::write(extension_path(&root), b"modified").unwrap();
        assert_eq!(status_at(&root).state, PiHookInstallState::Modified);
        assert!(install_at(&root).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn installation_creates_backup_when_upgrading_managed_file() {
        let root = temp_dir("backup");
        install_at(&root).unwrap();
        let mut manifest: InstallManifest =
            serde_json::from_slice(&fs::read(manifest_path(&root)).unwrap()).unwrap();
        manifest.extension_version = "0.0.1".to_string();
        fs::write(manifest_path(&root), serde_json::to_vec(&manifest).unwrap()).unwrap();
        // The stale manifest is incompatible, so installation is allowed and backs up the file.
        install_at(&root).unwrap();
        assert!(extension_path(&root).with_extension("ts.bak").exists());
        let _ = fs::remove_dir_all(root);
    }
}
