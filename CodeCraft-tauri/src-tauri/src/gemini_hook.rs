//! Gemini CLI Hook-only integration.
//!
//! The handler is deliberately fire-and-forget.  It never emits a decision,
//! blocks a tool, or waits for CodeCraft.  Its stdout is always exactly one
//! empty JSON object so Gemini's native behavior remains authoritative.

use std::{
    env,
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{json, Value};

use super::gemini;

pub(crate) const HOOK_ARGUMENT: &str = "--codecraft-gemini-hook";
pub(crate) const HOOK_EVENTS: [&str; 8] = [
    "SessionStart",
    "SessionEnd",
    "BeforeAgent",
    "AfterAgent",
    "BeforeTool",
    "AfterTool",
    "Notification",
    "PreCompress",
];
pub(crate) const MAX_INPUT_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_DETAIL_CHARS: usize = 256 * 1024;
pub(crate) const MAX_TEXT_CHARS: usize = 64 * 1024;
pub(crate) const MAX_PLAN_CHARS: usize = 512 * 1024;
const MAX_CAPTURE_BYTES: usize = MAX_INPUT_BYTES;
const HOOK_TIMEOUT_MS: u64 = 750;
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

pub(crate) fn gemini_hook_dir() -> PathBuf {
    base_data_dir().join("gemini-hook")
}

pub(crate) fn gemini_hook_inbox_dir() -> PathBuf {
    gemini_hook_dir().join("inbox")
}

fn settings_path() -> Result<PathBuf, String> {
    let home = env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
        .ok_or_else(|| "无法定位用户目录".to_string())?;
    Ok(home.join(".gemini").join("settings.json"))
}

fn capped(value: &Value, limit: usize) -> Value {
    match value {
        Value::String(text) => {
            let mut chars = text.chars();
            let mut result: String = chars.by_ref().take(limit).collect();
            if chars.next().is_some() {
                result.push('\u{2026}');
            }
            Value::String(result)
        }
        Value::Array(values) => Value::Array(values.iter().take(16).map(|value| capped(value, limit)).collect()),
        Value::Object(values) => Value::Object(values.iter().take(64).map(|(key, value)| (key.clone(), capped(value, limit))).collect()),
        _ => value.clone(),
    }
}

fn sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    ["token", "secret", "password", "authorization", "api_key", "apikey", "cookie", "credential", "env"].iter().any(|needle| key.contains(needle))
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
    let Some(object) = value.as_object() else { return capped(value, MAX_TEXT_CHARS); };
    let mut safe = serde_json::Map::new();
    for (key, value) in object {
        if sensitive_key(key) {
            continue;
        }
        let limit = if key == "plan" {
            MAX_PLAN_CHARS
        } else if ["prompt", "response", "prompt_response", "message", "last_assistant_message"].contains(&key.as_str()) {
            MAX_TEXT_CHARS
        } else {
            MAX_DETAIL_CHARS
        };
        safe.insert(key.clone(), sanitize_value(value, limit));
    }
    Value::Object(safe)
}

fn sanitize_payload(payload: &Value) -> Value {
    let Some(object) = payload.as_object() else { return json!({"hook_event_name":"invalid"}); };
    let mut safe = serde_json::Map::new();
    for key in [
        "hook_event_name", "session_id", "transcript_path", "cwd", "timestamp",
        "tool_name", "tool_call_id", "tool_use_id", "original_request_name", "mcp_context",
        "pid", "parent_pid", "process_created_at", "console_window", "title", "reason",
        "notification_type", "plan_filename", "truncated", "source", "trigger",
    ] {
        if let Some(value) = object.get(key) {
            if !sensitive_key(key) {
                safe.insert(key.to_string(), capped(value, 4_096));
            }
        }
    }
    for key in ["prompt", "response", "prompt_response", "agent_response", "last_assistant_message", "message", "plan"] {
        if let Some(value) = object.get(key) {
            let limit = if key == "plan" { MAX_PLAN_CHARS } else { MAX_TEXT_CHARS };
            safe.insert(key.to_string(), capped(value, limit));
        }
    }
    for key in ["tool_input", "tool_response", "details"] {
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
    object.insert("pid".to_string(), Value::from(process::id()));
    #[cfg(windows)]
    {
        use windows::Win32::System::{
            Console::GetConsoleWindow,
            Threading::{GetCurrentProcess, GetProcessTimes},
        };
        use windows::Win32::Foundation::FILETIME;
        let window = unsafe { GetConsoleWindow() };
        if !window.is_invalid() {
            object.insert("console_window".to_string(), Value::from(window.0 as isize));
        }
        let mut created = FILETIME::default();
        let mut exited = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        if unsafe { GetProcessTimes(GetCurrentProcess(), &mut created, &mut exited, &mut kernel, &mut user) }.is_ok() {
            let ticks = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
            let unix_ms = ticks / 10_000 - 11_644_473_600_000;
            object.insert("process_created_at".to_string(), Value::from(unix_ms));
        }
    }
    enriched
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct HookEnvelope {
    captured_at: u64,
    payload: Value,
}

fn write_capture(payload: &Value) -> Result<(), String> {
    let inbox = gemini_hook_inbox_dir();
    fs::create_dir_all(&inbox).map_err(|error| error.to_string())?;
    let bytes = serde_json::to_vec(&HookEnvelope { captured_at: now_ms(), payload: enrich_process_metadata(payload) })
        .map_err(|error| error.to_string())?;
    if bytes.len() > MAX_CAPTURE_BYTES {
        return Err("Gemini Hook 观察数据超过限制".to_string());
    }
    let stem = format!("{:020}-{}", now_ms(), process::id());
    let temporary = inbox.join(format!("{stem}.tmp"));
    let target = inbox.join(format!("{stem}.json"));
    let mut file = OpenOptions::new().write(true).create_new(true).open(&temporary).map_err(|error| error.to_string())?;
    file.write_all(&bytes).map_err(|error| error.to_string())?;
    file.flush().map_err(|error| error.to_string())?;
    drop(file);
    fs::rename(temporary, target).map_err(|error| error.to_string())
}

fn read_payload() -> Result<Value, String> {
    let mut bytes = Vec::with_capacity(8 * 1024);
    let mut stdin = io::stdin().take((MAX_INPUT_BYTES + 1) as u64);
    stdin.read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    if bytes.len() > MAX_INPUT_BYTES {
        return Err("Gemini Hook 输入超过 1 MiB".to_string());
    }
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

pub fn capture_gemini_hook() -> Result<(), String> {
    let result = read_payload().and_then(|payload| write_capture(&payload));
    if let Err(error) = result {
        eprintln!("CodeCraft Gemini Hook observation skipped: {error}");
    }
    // Gemini expects a JSON object for synchronous command hooks.  Keep this
    // as the only stdout write, including on malformed input.
    println!("{{}}");
    Ok(())
}

pub(crate) fn drain_inbox_events() -> Result<Vec<gemini::GeminiEvent>, String> {
    let inbox = gemini_hook_inbox_dir();
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
                    if let Some(event) = gemini::event_from_payload(envelope.payload, envelope.captured_at) {
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

fn is_codecraft_handler(value: &Value) -> bool {
    value.get("name").and_then(Value::as_str) == Some("codecraft-gemini-observer")
        || ["command", "commandWindows"].iter().any(|key| value.get(*key).and_then(Value::as_str).is_some_and(|command| command.contains(HOOK_ARGUMENT)))
}

fn handler(command: &str) -> Value {
    json!({
        "name": "codecraft-gemini-observer",
        "type": "command",
        "command": command,
        "timeout": HOOK_TIMEOUT_MS,
        "description": "CodeCraft Gemini CLI read-only observer"
    })
}

fn merge_settings(root: &mut Value, command: &str) -> Result<(), String> {
    let object = root.as_object_mut().ok_or_else(|| "Gemini settings.json 必须是 JSON 对象".to_string())?;
    let hooks = object.entry("hooks").or_insert_with(|| json!({}));
    let hooks = hooks.as_object_mut().ok_or_else(|| "Gemini hooks 配置必须是对象".to_string())?;
    for event in HOOK_EVENTS {
        let groups = hooks.entry(event).or_insert_with(|| json!([]));
        let groups = groups.as_array_mut().ok_or_else(|| format!("Gemini hooks.{event} 必须是数组"))?;
        let mut found = false;
        for group in groups.iter_mut() {
            if let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                handlers.retain(|item| !is_codecraft_handler(item));
                handlers.push(handler(command));
                found = true;
                break;
            }
        }
        if !found {
            groups.push(json!({"hooks": [handler(command)]}));
        }
    }
    Ok(())
}

fn remove_settings(root: &mut Value) -> Result<bool, String> {
    let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) else {
        return Ok(false);
    };
    let mut changed = false;
    for event in HOOK_EVENTS {
        if let Some(groups) = hooks.get_mut(event).and_then(Value::as_array_mut) {
            for group in groups.iter_mut() {
                if let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                    let before = handlers.len();
                    handlers.retain(|item| !is_codecraft_handler(item));
                    changed |= handlers.len() != before;
                }
            }
            groups.retain(|group| group.get("hooks").and_then(Value::as_array).is_none_or(|items| !items.is_empty()));
            if groups.is_empty() {
                hooks.remove(event);
            }
        }
    }
    Ok(changed)
}

fn atomic_write_settings(path: &Path, root: &Value) -> Result<(), String> {
    let parent = path.parent().ok_or_else(|| "Gemini settings 路径无效".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let bytes = serde_json::to_vec_pretty(root).map_err(|error| error.to_string())?;
    let temporary = parent.join(format!("settings.json.codecraft-{}.tmp", process::id()));
    fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
    fs::rename(temporary, path).map_err(|error| error.to_string())
}

fn create_backup(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or_else(|| "Gemini settings 路径无效".to_string())?;
    for attempt in 0..10 {
        let suffix = now_ms().saturating_add(attempt);
        let backup = parent.join(format!("settings.json.{suffix}.bak"));
        match OpenOptions::new().write(true).create_new(true).open(&backup) {
            Ok(mut file) => {
                file.write_all(bytes).map_err(|error| error.to_string())?;
                file.flush().map_err(|error| error.to_string())?;
                return Ok(());
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("无法创建 Gemini settings 备份".to_string())
}

fn assert_unchanged(path: &Path, original: &[u8]) -> Result<(), String> {
    let current = fs::read(path).unwrap_or_default();
    if !path.exists() && original == b"{}" {
        return Ok(());
    }
    if current != original {
        return Err("Gemini settings.json 在修改期间发生变化，请重新读取后修复".to_string());
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
        .ok_or_else(|| "Gemini settings 路径无效".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let lock_path = parent.join("settings.json.codecraft.lock");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                "Gemini settings.json 正在被 CodeCraft 另一实例修改，请稍后重试".to_string()
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
    let current = fs::read(&path).unwrap_or_else(|_| b"{}".to_vec());
    let mut root: Value = serde_json::from_slice(&current).map_err(|error| format!("Gemini settings.json 无法解析：{error}"))?;
    let command = hook_command(executable);
    merge_settings(&mut root, &command)?;
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
    let mut root: Value = serde_json::from_slice(&current).map_err(|error| format!("Gemini settings.json 无法解析：{error}"))?;
    if !remove_settings(&mut root)? {
        return Ok(());
    }
    create_backup(&path, &current)?;
    assert_unchanged(&path, &current)?;
    atomic_write_settings(&path, &root)
}

pub(crate) fn installed() -> Result<bool, String> {
    let path = settings_path()?;
    let Ok(bytes) = fs::read(path) else { return Ok(false); };
    let Ok(root) = serde_json::from_slice::<Value>(&bytes) else { return Ok(false); };
    Ok(HOOK_EVENTS.iter().all(|event| {
        root.get("hooks").and_then(Value::as_object).and_then(|hooks| hooks.get(*event)).and_then(Value::as_array).is_some_and(|groups| groups.iter().any(|group| group.get("hooks").and_then(Value::as_array).is_some_and(|handlers| handlers.iter().any(is_codecraft_handler))))
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
        let mut root = json!({"unknown":true,"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"other"}]}]}});
        merge_settings(&mut root, "codecraft --codecraft-gemini-hook").unwrap();
        merge_settings(&mut root, "codecraft --codecraft-gemini-hook").unwrap();
        assert_eq!(root["unknown"], true);
        let handlers = root["hooks"]["SessionStart"][0]["hooks"].as_array().unwrap();
        assert_eq!(handlers.iter().filter(|value| is_codecraft_handler(value)).count(), 1);
        assert_eq!(handlers.len(), 2);
    }
}
