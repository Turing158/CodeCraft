//! Kimi CLI Hook-only integration.
//!
//! The handler is deliberately fire-and-forget. It never emits a decision,
//! blocks a tool, or waits for CodeCraft. Kimi's documented fail-open behavior
//! remains authoritative when CodeCraft is unavailable.

use std::{
    env,
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{json, Value};
use toml::value::Table;

use super::kimi;

pub(crate) const HOOK_ARGUMENT: &str = "--codecraft-kimi-hook";
pub(crate) const HOOK_EVENTS: [&str; 20] = [
    "UserPromptSubmit",
    "UserPromptQueued",
    "PreToolUse",
    "Stop",
    "TurnStarted",
    "PostToolUse",
    "PostToolUseFailure",
    "PermissionRequest",
    "PermissionResult",
    "SessionStart",
    "SessionEnd",
    "SessionHeartbeat",
    "SubagentStart",
    "SubagentStop",
    "TaskStarted",
    "StopFailure",
    "Interrupt",
    "PreCompact",
    "PostCompact",
    "Notification",
];
pub(crate) const MAX_INPUT_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_DETAIL_CHARS: usize = 256 * 1024;
pub(crate) const MAX_TEXT_CHARS: usize = 64 * 1024;
pub(crate) const MAX_PLAN_CHARS: usize = 512 * 1024;
const MAX_CAPTURE_BYTES: usize = MAX_INPUT_BYTES;
const INBOX_FILE_TTL_MS: u128 = 24 * 60 * 60 * 1_000;
const MAX_INBOX_FILES: usize = 10_000;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

fn base_data_dir() -> PathBuf {
    if let Some(local) = env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local).join("CodeCraft");
    }
    env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
        .map(|path| path.join(".codecraft"))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub(crate) fn kimi_hook_dir() -> PathBuf {
    base_data_dir().join("kimi-hook")
}

pub(crate) fn kimi_hook_inbox_dir() -> PathBuf {
    kimi_hook_dir().join("inbox")
}

fn settings_path() -> Result<PathBuf, String> {
    let home = env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
        .ok_or_else(|| "无法定位用户目录".to_string())?;
    Ok(home.join(".kimi-code").join("config.toml"))
}

fn capped(value: &Value, limit: usize) -> Value {
    match value {
        Value::String(text) => {
            let mut chars = text.chars();
            let mut result: String = chars.by_ref().take(limit).collect();
            if chars.next().is_some() {
                result.push('\u{2026}');
            }
            Value::String(redact_text(&result))
        }
        Value::Array(values) => Value::Array(
            values
                .iter()
                .take(16)
                .map(|value| capped(value, limit))
                .collect(),
        ),
        Value::Object(values) => Value::Object(
            values
                .iter()
                .take(64)
                .map(|(key, value)| (key.clone(), capped(value, limit)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

pub(crate) fn redact_text(text: &str) -> String {
    static RULES: std::sync::OnceLock<Vec<regex::Regex>> = std::sync::OnceLock::new();
    let rules = RULES.get_or_init(|| [
        r#"(?i)\b(?:bearer|basic)\s+[^\s\"'<>]+"#,
        r#"(?i)(?:[\w-]*(?:token|secret|password|passwd|api[_-]?key|authorization|cookie|credential)[\w-]*)[\"']?\s*[:=]\s*(?:\"[^\"]*\"|'[^']*'|[^\s&;,]+)"#,
        r#"(?i)--(?:token|secret|password|api[_-]?key|credential|identity-file)\s+(?:\"[^\"]*\"|'[^']*'|[^\s]+)"#,
        r#"(?i)https?://[^\s/@]+:[^\s/@]+@"#,
        r#"(?s)-----BEGIN [^-]*PRIVATE KEY-----.*?-----END [^-]*PRIVATE KEY-----"#,
    ].iter().map(|pattern| regex::Regex::new(pattern).expect("valid redaction pattern")).collect());
    rules.iter().fold(text.to_string(), |result, rule| {
        rule.replace_all(&result, "[REDACTED]").into_owned()
    })
}

fn sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "token",
        "secret",
        "password",
        "authorization",
        "api_key",
        "apikey",
        "cookie",
        "credential",
        "env",
    ]
    .iter()
    .any(|needle| key.contains(needle))
}

fn sanitize_value(value: &Value, limit: usize) -> Value {
    match value {
        Value::Array(values) => Value::Array(
            values
                .iter()
                .take(16)
                .map(|value| sanitize_value(value, limit))
                .collect(),
        ),
        Value::Object(values) => {
            let mut safe = serde_json::Map::new();
            for (key, value) in values {
                if sensitive_key(key) {
                    continue;
                }
                safe.insert(key.clone(), sanitize_value(value, limit));
            }
            Value::Object(safe)
        }
        _ => capped(value, limit),
    }
}

fn sanitize_object(value: &Value) -> Value {
    let Some(object) = value.as_object() else {
        return capped(value, MAX_TEXT_CHARS);
    };
    let mut safe = serde_json::Map::new();
    for (key, value) in object {
        if sensitive_key(key) {
            continue;
        }
        let limit = if key == "plan" {
            MAX_PLAN_CHARS
        } else if [
            "prompt",
            "response",
            "prompt_response",
            "message",
            "last_assistant_message",
        ]
        .contains(&key.as_str())
        {
            MAX_TEXT_CHARS
        } else {
            MAX_DETAIL_CHARS
        };
        safe.insert(key.clone(), sanitize_value(value, limit));
    }
    Value::Object(safe)
}

fn sanitize_payload(payload: &Value) -> Value {
    let Some(object) = payload.as_object() else {
        return json!({"hook_event_name":"invalid"});
    };
    let mut safe = serde_json::Map::new();
    for key in [
        "hook_event_name",
        "session_id",
        "session_title",
        "client_type",
        "cwd",
        "timestamp",
        "tool_name",
        "tool_call_id",
        "tool_use_id",
        "prompt_id",
        "turn_id",
        "task_id",
        "origin_kind",
        "origin_name",
        "queue_length",
        "model",
        "profile",
        "source",
        "matcher",
        "mcp_context",
        "pid",
        "parent_pid",
        "process_created_at",
        "console_window",
        "title",
        "reason",
        "notification_type",
        "plan_filename",
        "truncated",
        "source",
        "trigger",
    ] {
        if let Some(value) = object.get(key) {
            if !sensitive_key(key) {
                safe.insert(key.to_string(), sanitize_value(value, 4_096));
            }
        }
    }
    for key in [
        "prompt",
        "response",
        "prompt_response",
        "agent_response",
        "last_assistant_message",
        "message",
        "description",
        "reason",
        "plan",
    ] {
        if let Some(value) = object.get(key) {
            let limit = if key == "plan" {
                MAX_PLAN_CHARS
            } else {
                MAX_TEXT_CHARS
            };
            safe.insert(key.to_string(), sanitize_value(value, limit));
        }
    }
    for key in [
        "tool_input",
        "tool_output",
        "tool_response",
        "details",
        "display",
    ] {
        if let Some(value) = object.get(key) {
            safe.insert(key.to_string(), sanitize_object(value));
        }
    }
    Value::Object(safe)
}

fn enrich_process_metadata(payload: &Value) -> Value {
    let mut enriched = sanitize_payload(payload);
    let Some(object) = enriched.as_object_mut() else {
        return enriched;
    };
    // Never trust process/window metadata supplied on stdin. The hook runner
    // is the only authority for the short-lived local navigation binding.
    object.remove("parent_pid");
    object.remove("process_created_at");
    object.remove("console_window");
    object.remove("pid");
    #[cfg(windows)]
    {
        if let Some((pid, created)) = kimi_ancestor() {
            object.insert("pid".to_string(), Value::from(pid));
            object.insert("process_created_at".to_string(), Value::from(created));
            if let Some(window) = crate::kimi_focus::capture_terminal_window(pid) {
                object.insert("console_window".to_string(), Value::from(window.handle));
                object.insert("window_process_id".to_string(), Value::from(window.pid));
                object.insert(
                    "window_process_created_at".to_string(),
                    Value::from(window.created_at),
                );
                object.insert("shared_terminal".to_string(), Value::from(window.shared));
            }
        }
    }
    enriched
}

#[cfg(windows)]
fn kimi_ancestor() -> Option<(u32, u64)> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, FILETIME, INVALID_HANDLE_VALUE},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
                TH32CS_SNAPPROCESS,
            },
            Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
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
        let mut valid = Process32FirstW(snapshot, &mut entry);
        while valid != 0 {
            let len = entry
                .szExeFile
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(entry.szExeFile.len());
            processes.insert(
                entry.th32ProcessID,
                (
                    entry.th32ParentProcessID,
                    String::from_utf16_lossy(&entry.szExeFile[..len]),
                ),
            );
            valid = Process32NextW(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
        let mut pid = process::id();
        for _ in 0..16 {
            pid = processes.get(&pid)?.0;
            let name = &processes.get(&pid)?.1;
            if !name.eq_ignore_ascii_case("kimi.exe") && !name.eq_ignore_ascii_case("kimi-cli.exe")
            {
                continue;
            }
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return None;
            }
            let mut created: FILETIME = std::mem::zeroed();
            let mut exited = created;
            let mut kernel = created;
            let mut user = created;
            let ok = GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user);
            CloseHandle(handle);
            if ok == 0 {
                return None;
            }
            let ticks =
                (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
            return Some((pid, (ticks / 10_000).saturating_sub(11_644_473_600_000)));
        }
        None
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct HookEnvelope {
    captured_at: u64,
    payload: Value,
}

fn write_capture(payload: &Value) -> Result<(), String> {
    let inbox = kimi_hook_inbox_dir();
    fs::create_dir_all(&inbox).map_err(|error| error.to_string())?;
    let bytes = serde_json::to_vec(&HookEnvelope {
        captured_at: now_ms(),
        payload: enrich_process_metadata(payload),
    })
    .map_err(|error| error.to_string())?;
    if bytes.len() > MAX_CAPTURE_BYTES {
        return Err("Kimi Hook 观察数据超过限制".to_string());
    }
    let stem = format!("{:020}-{}", now_ms(), process::id());
    let temporary = inbox.join(format!("{stem}.tmp"));
    let target = inbox.join(format!("{stem}.json"));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    file.write_all(&bytes).map_err(|error| error.to_string())?;
    file.flush().map_err(|error| error.to_string())?;
    drop(file);
    fs::rename(temporary, target).map_err(|error| error.to_string())
}

fn read_payload() -> Result<Value, String> {
    let mut bytes = Vec::with_capacity(8 * 1024);
    let mut stdin = io::stdin().take((MAX_INPUT_BYTES + 1) as u64);
    stdin
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > MAX_INPUT_BYTES {
        return Err("Kimi Hook 输入超过 1 MiB".to_string());
    }
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

pub fn capture_kimi_hook() -> Result<(), String> {
    let result = read_payload().and_then(|payload| write_capture(&payload));
    if let Err(error) = result {
        eprintln!("CodeCraft Kimi Hook observation skipped: {error}");
    }
    // Kimi hooks use exit status and optional text. Empty stdout preserves the
    // native flow for both blockable and observation-only events.
    Ok(())
}

pub(crate) fn drain_inbox_events() -> Result<Vec<kimi::KimiEvent>, String> {
    let inbox = kimi_hook_inbox_dir();
    if !inbox.exists() {
        return Ok(Vec::new());
    }
    let now = SystemTime::now();
    let mut paths: Vec<_> = fs::read_dir(&inbox)
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .collect();
    paths.sort();
    let mut events = Vec::new();
    for path in paths.into_iter().take(MAX_INBOX_FILES) {
        let stale = fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age.as_millis() > INBOX_FILE_TTL_MS);
        if !stale {
            if let Ok(text) = fs::read_to_string(&path) {
                if let Ok(envelope) = serde_json::from_str::<HookEnvelope>(&text) {
                    if let Some(event) =
                        kimi::event_from_payload(envelope.payload, envelope.captured_at)
                    {
                        events.push(event);
                    }
                }
            }
        }
        let _ = fs::remove_file(path);
    }
    Ok(events)
}

fn hook_command(executable: &Path) -> String {
    #[cfg(windows)]
    {
        let escaped = executable.display().to_string().replace('"', "\"\"");
        return format!("cmd.exe /d /s /c \"\"{escaped}\" {HOOK_ARGUMENT}\"");
    }
    #[cfg(not(windows))]
    {
        let escaped = executable.display().to_string().replace('\'', "'\\''");
        format!("'{escaped}' {HOOK_ARGUMENT}")
    }
}

fn is_codecraft_handler(value: &toml::Value) -> bool {
    value
        .get("command")
        .and_then(toml::Value::as_str)
        .is_some_and(|command| command.contains(HOOK_ARGUMENT))
}

fn handler(command: &str, event: &str) -> toml::Value {
    let mut table = Table::new();
    table.insert("event".to_string(), toml::Value::String(event.to_string()));
    table.insert(
        "command".to_string(),
        toml::Value::String(command.to_string()),
    );
    table.insert("timeout".to_string(), toml::Value::Integer(2));
    toml::Value::Table(table)
}

fn merge_settings(root: &mut toml::Value, command: &str) -> Result<(), String> {
    let object = root
        .as_table_mut()
        .ok_or_else(|| "Kimi config.toml must be a table".to_string())?;
    let hooks = object
        .entry("hooks".to_string())
        .or_insert_with(|| toml::Value::Array(Vec::new()));
    let hooks = hooks
        .as_array_mut()
        .ok_or_else(|| "Kimi hooks must be an array".to_string())?;
    hooks.retain(|item| !is_codecraft_handler(item));
    for event in HOOK_EVENTS {
        hooks.push(handler(command, event));
    }
    Ok(())
}

fn remove_settings(root: &mut toml::Value) -> Result<bool, String> {
    let Some(hooks) = root.get_mut("hooks").and_then(toml::Value::as_array_mut) else {
        return Ok(false);
    };
    let before = hooks.len();
    hooks.retain(|item| !is_codecraft_handler(item));
    Ok(hooks.len() != before)
}

fn atomic_write_settings(path: &Path, root: &toml::Value) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Kimi settings 路径无效".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let bytes = toml::to_string_pretty(root)
        .map_err(|error| error.to_string())?
        .into_bytes();
    let temporary = parent.join(format!("config.toml.codecraft-{}.tmp", process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    file.write_all(&bytes).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    drop(file);
    fs::rename(temporary, path).map_err(|error| error.to_string())
}

fn create_backup(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Kimi settings 路径无效".to_string())?;
    for attempt in 0..10 {
        let suffix = now_ms().saturating_add(attempt);
        let backup = parent.join(format!("config.toml.{suffix}.bak"));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&backup)
        {
            Ok(mut file) => {
                file.write_all(bytes).map_err(|error| error.to_string())?;
                file.flush().map_err(|error| error.to_string())?;
                return Ok(());
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("无法创建 Kimi settings 备份".to_string())
}

fn assert_unchanged(path: &Path, original: &[u8]) -> Result<(), String> {
    let current = fs::read(path).unwrap_or_default();
    if !path.exists() && original == b"{}" {
        return Ok(());
    }
    if current != original {
        return Err("Kimi config.toml 在修改期间发生变化，请重新读取后修复".to_string());
    }
    Ok(())
}

struct SettingsLock {
    path: PathBuf,
}

impl Drop for SettingsLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn acquire_settings_lock(path: &Path) -> Result<SettingsLock, String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Kimi settings 路径无效".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let lock_path = parent.join("config.toml.codecraft.lock");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                "Kimi config.toml 正在被 CodeCraft 另一实例修改，请稍后重试".to_string()
            } else {
                error.to_string()
            }
        })?;
    writeln!(file, "pid={}", process::id()).map_err(|error| error.to_string())?;
    file.flush().map_err(|error| error.to_string())?;
    Ok(SettingsLock { path: lock_path })
}

pub(crate) fn install(executable: &Path) -> Result<(), String> {
    let path = settings_path()?;
    let _lock = acquire_settings_lock(&path)?;
    let current = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error.to_string()),
    };
    let text = std::str::from_utf8(&current).map_err(|error| error.to_string())?;
    let mut root: toml::Value = if text.trim().is_empty() {
        toml::Value::Table(Table::new())
    } else {
        text.parse()
            .map_err(|error| format!("Invalid Kimi config.toml: {error}"))?
    };
    let command = hook_command(executable);
    let original = root.clone();
    merge_settings(&mut root, &command)?;
    if root == original {
        return Ok(());
    }
    if path.exists() {
        create_backup(&path, &current)?;
    }
    assert_unchanged(&path, &current)?;
    atomic_write_settings(&path, &root)
}

pub(crate) fn uninstall() -> Result<(), String> {
    let path = settings_path()?;
    if !path.exists() {
        return Ok(());
    }
    let _lock = acquire_settings_lock(&path)?;
    let current = fs::read(&path).map_err(|error| error.to_string())?;
    let text = std::str::from_utf8(&current).map_err(|error| error.to_string())?;
    let mut root: toml::Value = if text.trim().is_empty() {
        toml::Value::Table(Table::new())
    } else {
        text.parse()
            .map_err(|error| format!("Invalid Kimi config.toml: {error}"))?
    };
    if !remove_settings(&mut root)? {
        return Ok(());
    }
    create_backup(&path, &current)?;
    assert_unchanged(&path, &current)?;
    atomic_write_settings(&path, &root)
}

pub(crate) fn installed() -> Result<bool, String> {
    let path = settings_path()?;
    let Ok(bytes) = fs::read(path) else {
        return Ok(false);
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return Ok(false);
    };
    let Ok(root) = text.parse::<toml::Value>() else {
        return Ok(false);
    };
    Ok(root
        .get("hooks")
        .and_then(toml::Value::as_array)
        .is_some_and(|hooks| {
            HOOK_EVENTS.iter().all(|event| {
                hooks.iter().any(|item| {
                    item.get("event").and_then(toml::Value::as_str) == Some(*event)
                        && is_codecraft_handler(item)
                })
            })
        }))
}

pub(crate) fn handler_command_for_test(executable: &Path) -> String {
    hook_command(executable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sanitizer_drops_credentials_and_keeps_commands() {
        let payload = sanitize_payload(&json!({
            "hook_event_name":"BeforeTool",
            "session_id":"s",
            "tool_input":{"command":"echo hi", "apiKey":"secret"},
            "env":{"TOKEN":"drop"}
        }));
        assert_eq!(payload["tool_input"]["command"], "echo hi");
        assert!(payload["tool_input"]["apiKey"].is_null());
        assert!(payload["env"].is_null());
    }

    #[test]
    fn merge_preserves_other_hooks_and_is_idempotent() {
        let mut root: toml::Value =
            "unknown = true\n[[hooks]]\nevent = \"Notification\"\ncommand = \"other\"\n"
                .parse()
                .unwrap();
        merge_settings(&mut root, "codecraft --codecraft-kimi-hook").unwrap();
        merge_settings(&mut root, "codecraft --codecraft-kimi-hook").unwrap();
        assert_eq!(root["unknown"].as_bool(), Some(true));
        let handlers = root["hooks"].as_array().unwrap();
        assert_eq!(
            handlers
                .iter()
                .filter(|value| is_codecraft_handler(value))
                .count(),
            HOOK_EVENTS.len()
        );
        assert_eq!(handlers.len(), HOOK_EVENTS.len() + 1);
    }

    #[test]
    fn capture_keeps_tool_output_and_redacts_nested_and_inline_secrets() {
        let safe = sanitize_payload(&json!({
            "hook_event_name":"PostToolUse", "session_id":"s",
            "tool_output":"contents", "tool_input":{"command":"curl -H 'Authorization: Bearer abc123' https://host/?token=hidden", "password":"secret"},
            "prompt":[{"type":"text", "text":"api_key=private-value", "env":{"KEY":"hidden"}}]
        }));
        assert_eq!(safe["tool_output"], "contents");
        let serialized = safe.to_string();
        for secret in ["abc123", "hidden", "private-value", "secret"] {
            assert!(!serialized.contains(secret), "{serialized}");
        }
    }

    #[test]
    fn capture_keeps_plan_review_display_and_redacts_its_content() {
        let safe = sanitize_payload(&json!({
            "hook_event_name":"PermissionRequest", "session_id":"s",
            "tool_name":"ExitPlanMode", "tool_input":{},
            "display":{
                "kind":"plan_review", "path":"plans/review.md",
                "plan":"# Proposed plan\napi_key=private-value",
                "options":[{"label":"Small change", "description":"Keep the API"}],
                "token":"hidden"
            }
        }));
        assert_eq!(safe["display"]["kind"], "plan_review");
        assert_eq!(safe["display"]["path"], "plans/review.md");
        assert_eq!(safe["display"]["plan"], "# Proposed plan\n[REDACTED]");
        assert_eq!(safe["display"]["options"][0]["label"], "Small change");
        assert!(safe["display"].get("token").is_none());
    }

    #[test]
    fn uninstall_retains_unowned_configuration() {
        let mut root: toml::Value =
            "model = 'custom'\n[[hooks]]\nevent = 'Stop'\ncommand = 'other'\n"
                .parse()
                .unwrap();
        let original = root.clone();
        merge_settings(&mut root, "codecraft --codecraft-kimi-hook").unwrap();
        assert!(remove_settings(&mut root).unwrap());
        assert_eq!(root, original);
        assert!(!remove_settings(&mut root).unwrap());
    }
}
