//! Codex hooks: the lightweight event and policy layer for external sessions.
//!
//! Lifecycle events are fire-and-forget, while PermissionRequest waits on a
//! small file-backed broker so CodeCraft can show the request and return the
//! user's decision to the originating process.
//!
//! The hook is invoked by Codex as a subprocess (see `.codex/hooks.json`),
//! receives one JSON object on stdin, and may write a JSON decision on stdout.
//! This module keeps a capture inbox that the app drains into `CodexStore`.
//!
//! The PermissionRequest reply uses Codex's `hookSpecificOutput` decision
//! shape. Manual mode waits for CodeCraft when the app heartbeat is active.

use std::{
    env, fs,
    io::{self, BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process, thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{
    approval_policy,
    codex::{parse_user_input_questions, CodexApprovalDecision, CodexEvent},
};

pub(crate) const HOOK_ARGUMENT: &str = "--codecraft-codex-hook";
pub(crate) const HOOK_EVENTS: [&str; 11] = [
    "SessionStart",
    "SessionEnd",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "Stop",
    "PermissionRequest",
    "PreCompact",
    "PostCompact",
    "SubagentStart",
    "SubagentStop",
];
const APPROVAL_WAIT_TIMEOUT: Duration = Duration::from_secs(300);
const APPROVAL_POLL_INTERVAL: Duration = Duration::from_millis(200);
const APP_HEARTBEAT_STALE: Duration = Duration::from_secs(20);
const APPROVAL_HOOK_TIMEOUT_SECONDS: u64 = 330;
const CODECRAFT_REQUEST_ID_FIELD: &str = "codecraft_request_id";
const CODECRAFT_PLAN_FIELD: &str = "codecraft_plan";
const MAX_CAPTURE_PLAN_CHARS: usize = 12_000;

/// Legacy Hook-local policy retained for settings-file compatibility. The
/// active approval policy is now shared by Claude Code and Codex.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CodexAutoApprovalMode {
    /// Never auto-approve; leave decisions to Codex / the app.
    Manual,
    /// Auto-approve only a narrow allowlist of safe, read-only tools.
    Permission,
    /// Auto-approve everything reaching the hook (audited).
    Auto,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct CodexHookConfig {
    pub enabled: bool,
    pub mode: CodexAutoApprovalMode,
    pub project_dir: Option<String>,
    pub audit_log: bool,
}

impl Default for CodexHookConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: CodexAutoApprovalMode::Manual,
            project_dir: None,
            audit_log: true,
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
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

pub(crate) fn codex_hook_dir() -> PathBuf {
    base_data_dir().join("codex-hook")
}

pub(crate) fn codex_hook_inbox_dir() -> PathBuf {
    codex_hook_dir().join("inbox")
}

fn audit_path() -> PathBuf {
    codex_hook_dir().join("audit.log")
}

fn approval_dir() -> PathBuf {
    codex_hook_dir().join("approvals")
}

fn approval_path(request_id: &str) -> PathBuf {
    approval_dir().join(format!("{}.json", safe_request_stem(request_id)))
}

fn safe_request_stem(request_id: &str) -> String {
    request_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .take(160)
        .collect()
}

fn app_is_running() -> bool {
    let Ok(metadata) = fs::metadata(base_data_dir().join("app-running")) else {
        return false;
    };
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .map(|elapsed| elapsed <= APP_HEARTBEAT_STALE)
        .unwrap_or(false)
}
fn append_audit(mode: &str, event_name: &str, payload: &Value) -> Result<(), String> {
    let dir = codex_hook_dir();
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let session_id = string_field(payload, "session_id").unwrap_or_else(|| "?".to_string());
    let tool = string_field(payload, "tool_name").unwrap_or_else(|| "-".to_string());
    let summary = summary_of(payload);
    let line = format!(
        "{} [{}] event={} session={} tool={} {}\n",
        now_ms(),
        mode,
        event_name,
        session_id,
        tool,
        summary
    );
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(audit_path())
        .map_err(|error| error.to_string())?;
    file.write_all(line.as_bytes())
        .map_err(|error| error.to_string())
}

fn summary_of(payload: &Value) -> String {
    let command = string_field(payload, "command").or_else(|| {
        payload
            .get("tool_input")
            .and_then(|value| string_field(value, "command"))
    });
    if let Some(command) = command {
        let trimmed = command.split_whitespace().collect::<Vec<_>>();
        if !trimmed.is_empty() {
            return format!("cmd={}", trimmed[0]);
        }
    }
    let description = string_field(payload, "description");
    description
        .map(|text| {
            let capped: String = text.chars().take(60).collect();
            format!("desc={capped}")
        })
        .unwrap_or_default()
}

fn activity_summary(payload: &Value) -> String {
    let command = string_field(payload, "command").or_else(|| {
        payload
            .get("tool_input")
            .and_then(|value| string_field(value, "command"))
    });
    let text = command
        .or_else(|| string_field(payload, "description"))
        .or_else(|| {
            payload
                .get("tool_input")
                .and_then(|value| string_field(value, "description"))
        })
        .or_else(|| string_field(payload, "reason"))
        .unwrap_or_else(|| "工具调用".to_string());
    text.chars().take(160).collect()
}

fn tool_name(payload: &Value) -> String {
    string_field(payload, "tool_name").unwrap_or_else(|| "unknown".to_string())
}

fn session_id(payload: &Value) -> String {
    string_field(payload, "session_id").unwrap_or_else(|| "unknown-session".to_string())
}

fn tool_activity_id(payload: &Value) -> String {
    string_field(payload, "tool_use_id")
        .or_else(|| string_field(payload, "tool_call_id"))
        .unwrap_or_else(|| format!("hook-{}", now_ms()))
}

#[derive(Debug, Serialize, Deserialize)]
struct HookEnvelope {
    captured_at: u64,
    payload: Value,
}

fn write_capture(payload: &Value) -> Result<(), String> {
    let inbox = codex_hook_inbox_dir();
    fs::create_dir_all(&inbox).map_err(|error| error.to_string())?;
    let payload = capture_payload(payload);
    let serialized = serde_json::to_vec(&HookEnvelope {
        captured_at: now_ms(),
        payload,
    })
    .map_err(|error| error.to_string())?;
    for attempt in 0..10 {
        let stem = format!("{:020}-{}-{attempt}", now_ms(), process::id());
        let temporary = inbox.join(format!("{stem}.tmp"));
        let target = inbox.join(format!("{stem}.json"));
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary);
        let Ok(mut file) = file else {
            continue;
        };
        file.write_all(&serialized)
            .and_then(|_| file.flush())
            .map_err(|error| error.to_string())?;
        drop(file);
        fs::rename(&temporary, &target).map_err(|error| error.to_string())?;
        return Ok(());
    }
    Err("Unable to allocate a unique Codex hook event file".to_string())
}

fn capture_payload(payload: &Value) -> Value {
    let mut safe = sanitized_capture(payload);
    if string_field(payload, "hook_event_name").as_deref() == Some("Stop") {
        if let Some(plan) = plan_text_from_codex_rollout(payload) {
            if let Some(object) = safe.as_object_mut() {
                object.insert(
                    CODECRAFT_PLAN_FIELD.to_string(),
                    Value::String(plan.chars().take(MAX_CAPTURE_PLAN_CHARS).collect()),
                );
            }
        }
    }
    safe
}

fn capped_string(value: Option<&Value>, max_chars: usize) -> Option<Value> {
    value
        .and_then(Value::as_str)
        .map(|text| Value::String(text.chars().take(max_chars).collect()))
}

fn sanitized_questions(value: Option<&Value>) -> Option<Value> {
    let questions = value?.as_array()?;
    let mut safe_questions = Vec::new();
    for question in questions.iter().take(3) {
        let Some(question) = question.as_object() else {
            continue;
        };
        let mut safe_question = serde_json::Map::new();
        for (key, limit) in [("id", 100), ("header", 100), ("question", 2_000)] {
            if let Some(value) = capped_string(question.get(key), limit) {
                safe_question.insert(key.to_string(), value);
            }
        }
        for key in ["isOther", "isSecret"] {
            if let Some(value) = question.get(key).and_then(Value::as_bool) {
                safe_question.insert(key.to_string(), Value::Bool(value));
            }
        }
        if let Some(options) = question.get("options").and_then(Value::as_array) {
            let safe_options: Vec<Value> = options
                .iter()
                .take(8)
                .filter_map(|option| {
                    let option = option.as_object()?;
                    let label = capped_string(option.get("label"), 200)?;
                    let mut safe_option = serde_json::Map::new();
                    safe_option.insert("label".to_string(), label);
                    if let Some(description) = capped_string(option.get("description"), 500) {
                        safe_option.insert("description".to_string(), description);
                    }
                    Some(Value::Object(safe_option))
                })
                .collect();
            safe_question.insert("options".to_string(), Value::Array(safe_options));
        }
        safe_questions.push(Value::Object(safe_question));
    }
    (!safe_questions.is_empty()).then_some(Value::Array(safe_questions))
}

/// Keep the inbox useful for the UI without persisting arbitrary tool results,
/// environment values, or oversized command arguments.
fn sanitized_capture(payload: &Value) -> Value {
    let mut safe = serde_json::Map::new();
    for key in [
        "hook_event_name",
        "session_id",
        "turn_id",
        "cwd",
        "model",
        "permission_mode",
        "mode",
        "plan_mode",
        "tool_name",
        "tool_use_id",
        "tool_call_id",
        "source",
        "reason",
        CODECRAFT_REQUEST_ID_FIELD,
    ] {
        if let Some(value) = payload.get(key) {
            safe.insert(key.to_string(), value.clone());
        }
    }
    if let Some(mode) = payload.get("collaboration_mode").and_then(|value| {
        value
            .as_str()
            .or_else(|| value.get("mode").and_then(Value::as_str))
    }) {
        safe.insert(
            "collaboration_mode".to_string(),
            Value::String(mode.chars().take(100).collect()),
        );
    }
    for (key, limit) in [("prompt", 4_000), ("last_assistant_message", 8_000)] {
        if let Some(value) = capped_string(payload.get(key), limit) {
            safe.insert(key.to_string(), value);
        }
    }
    if let Some(input) = payload.get("tool_input").and_then(Value::as_object) {
        let mut safe_input = serde_json::Map::new();
        for (key, limit) in [("command", 1_000), ("description", 500)] {
            if let Some(value) = capped_string(input.get(key), limit) {
                safe_input.insert(key.to_string(), value);
            }
        }
        if let Some(questions) = sanitized_questions(input.get("questions")) {
            safe_input.insert("questions".to_string(), questions);
        }
        if !safe_input.is_empty() {
            safe.insert("tool_input".to_string(), Value::Object(safe_input));
        }
    }
    if payload
        .get("permission_suggestions")
        .and_then(Value::as_array)
        .is_some_and(|suggestions| !suggestions.is_empty())
    {
        safe.insert("can_always_allow".to_string(), Value::Bool(true));
    }
    if let Some(response) = payload.get("tool_response").and_then(Value::as_object) {
        let mut safe_response = serde_json::Map::new();
        for key in ["isError", "is_error", "success", "exitCode", "exit_code"] {
            if let Some(value) = response.get(key) {
                safe_response.insert(key.to_string(), value.clone());
            }
        }
        if !safe_response.is_empty() {
            safe.insert("tool_response".to_string(), Value::Object(safe_response));
        }
    }
    Value::Object(safe)
}

fn string_field(payload: &Value, key: &str) -> Option<String> {
    payload.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn current_mode_config() -> CodexHookConfig {
    fs::read(settings_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn settings_path() -> PathBuf {
    codex_hook_dir().join("settings.json")
}

pub(crate) fn save_config(config: &CodexHookConfig) -> Result<(), String> {
    let dir = codex_hook_dir();
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let bytes = serde_json::to_vec_pretty(config).map_err(|error| error.to_string())?;
    fs::write(settings_path(), bytes).map_err(|error| error.to_string())
}

pub(crate) fn load_config() -> CodexHookConfig {
    current_mode_config()
}

fn permission_decision_output(decision: &str) -> Option<Value> {
    let decision = match decision {
        "allow" => json!({ "behavior": "allow" }),
        "deny" => json!({ "behavior": "deny", "message": "CodeCraft hook policy" }),
        _ => return None,
    };
    Some(json!({
        "hookSpecificOutput": {
            "hookEventName": "PermissionRequest",
            "decision": decision
        }
    }))
}

fn permission_session_decision_output(payload: &Value) -> Value {
    let updated_permissions = payload
        .get("permission_suggestions")
        .and_then(Value::as_array)
        .filter(|suggestions| !suggestions.is_empty())
        .cloned()
        .unwrap_or_default();
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PermissionRequest",
            "decision": { "behavior": "allow" },
            "updatedPermissions": updated_permissions
        }
    })
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodexHookApprovalEnvelope {
    request_id: String,
    decision: CodexApprovalDecision,
}

fn permission_request_id(payload: &Value) -> String {
    if let Some(request_id) = string_field(payload, CODECRAFT_REQUEST_ID_FIELD)
        .filter(|request_id| is_hook_approval_request(request_id))
    {
        return request_id;
    }
    let session = session_id(payload);
    let activity = tool_activity_id(payload);
    format!("hook-permission-{session}-{activity}")
}

fn codex_hook_approval_decision(payload: &Value) -> Option<CodexApprovalDecision> {
    let path = approval_path(&permission_request_id(payload));
    let contents = fs::read_to_string(&path).ok()?;
    let _ = fs::remove_file(path);
    serde_json::from_str::<CodexHookApprovalEnvelope>(&contents)
        .ok()
        .map(|envelope| envelope.decision)
}

fn remove_stale_approval(payload: &Value) -> Result<(), String> {
    let path = approval_path(&permission_request_id(payload));
    if path.exists() {
        fs::remove_file(path).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn wait_for_codex_hook_approval(payload: &Value) -> Result<Option<CodexApprovalDecision>, String> {
    let started_at = Instant::now();
    loop {
        if let Some(decision) = codex_hook_approval_decision(payload) {
            return Ok(Some(decision));
        }
        if !app_is_running() || started_at.elapsed() >= APPROVAL_WAIT_TIMEOUT {
            return Ok(None);
        }
        thread::sleep(APPROVAL_POLL_INTERVAL);
    }
}

fn print_codex_hook_decision(payload: &Value, decision: CodexApprovalDecision) {
    let response = match decision {
        CodexApprovalDecision::Accept => permission_decision_output("allow"),
        CodexApprovalDecision::AcceptForSession => {
            Some(permission_session_decision_output(payload))
        }
        CodexApprovalDecision::Decline | CodexApprovalDecision::Cancel => {
            permission_decision_output("deny")
        }
    };
    if let Some(response) = response {
        println!("{response}");
    }
}

fn print_decision(decision: Option<&str>) {
    if let Some(output) = decision.and_then(permission_decision_output) {
        println!("{output}");
    }
}

/// Entry point for `--codecraft-codex-hook`. Reads one JSON object from stdin
/// and either records the event or (for PermissionRequest) applies the
/// configured auto-approval policy.
pub fn capture_codex_hook() -> Result<(), String> {
    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .map_err(|error| error.to_string())?;
    let payload: Value = serde_json::from_str(&input).map_err(|error| error.to_string())?;
    handle_codex_hook(&payload)
}

fn handle_codex_hook(payload: &Value) -> Result<(), String> {
    let event_name = string_field(payload, "hook_event_name")
        .ok_or_else(|| "missing hook_event_name".to_string())?;

    let config = current_mode_config();
    if event_name != "PermissionRequest" {
        write_capture(payload)?;
        if config.audit_log {
            let _ = append_audit("capture", &event_name, payload);
        }
        // Stop and SubagentStop require JSON when a synchronous handler exits
        // successfully. This remains valid when the configured handler is async.
        if event_name == "Stop" || event_name == "SubagentStop" {
            println!("{{}}");
        }
        return Ok(());
    }

    let approval_settings = approval_policy::load_settings();
    let risk = approval_policy::risk_for_codex_hook(payload);
    let decision =
        approval_policy::should_auto_approve(approval_settings.mode, risk).then_some("allow");
    if decision.is_some() {
        print_decision(decision);
        if config.audit_log {
            let label = match approval_settings.mode {
                approval_policy::ApprovalMode::Automatic => "automatic-allow",
                approval_policy::ApprovalMode::Risk => "risk-allow",
                approval_policy::ApprovalMode::Manual => "manual",
            };
            let _ = append_audit(label, &event_name, payload);
        }
        return Ok(());
    }

    if !app_is_running() {
        if config.audit_log {
            let _ = append_audit("native-fallback", &event_name, payload);
        }
        return Ok(());
    }

    let request_id = permission_request_id(payload);
    let mut captured_payload = payload.clone();
    if let Some(object) = captured_payload.as_object_mut() {
        object.insert(
            CODECRAFT_REQUEST_ID_FIELD.to_string(),
            Value::String(request_id),
        );
    }
    remove_stale_approval(&captured_payload)?;
    write_capture(&captured_payload)?;
    if config.audit_log {
        let label = match approval_settings.mode {
            approval_policy::ApprovalMode::Risk => "risk-wait",
            _ => "manual-wait",
        };
        let _ = append_audit(label, &event_name, payload);
    }

    match wait_for_codex_hook_approval(&captured_payload) {
        Ok(Some(decision)) => print_codex_hook_decision(payload, decision),
        Ok(None) => {
            if config.audit_log {
                let _ = append_audit("wait-timeout", &event_name, payload);
            }
        }
        Err(error) => eprintln!("CodeCraft could not read the Codex approval: {error}"),
    }
    Ok(())
}

/// Read and remove captured hook events in timestamp order. Older raw payload
/// files are accepted for forward compatibility with the first implementation.
pub(crate) fn drain_inbox_events() -> Result<Vec<CodexEvent>, String> {
    let inbox = codex_hook_inbox_dir();
    if !inbox.exists() {
        return Ok(Vec::new());
    }
    let mut paths = fs::read_dir(&inbox)
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    paths.sort();

    let mut events = Vec::new();
    for path in paths {
        let parsed = fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<HookEnvelope>(&text).ok())
            .map(|envelope| envelope.payload)
            .or_else(|| {
                fs::read_to_string(&path)
                    .ok()
                    .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            });
        if let Some(payload) = parsed {
            if let Some(event) = event_to_codex_event(&payload) {
                events.push(event);
            }
        }
        let _ = fs::remove_file(path);
    }
    Ok(events)
}

pub(crate) fn is_hook_approval_request(request_id: &str) -> bool {
    request_id.starts_with("hook-permission-")
}

pub(crate) fn submit_approval_decision(
    request_id: &str,
    decision: CodexApprovalDecision,
) -> Result<(), String> {
    if !is_hook_approval_request(request_id) {
        return Err("not a Codex Hook approval request".to_string());
    }
    let directory = approval_dir();
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let target = approval_path(request_id);
    let temporary = directory.join(format!(
        "{}.{}-{}.tmp",
        safe_request_stem(request_id),
        process::id(),
        now_ms()
    ));
    let bytes = serde_json::to_vec(&CodexHookApprovalEnvelope {
        request_id: request_id.to_string(),
        decision,
    })
    .map_err(|error| error.to_string())?;
    fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
    fs::rename(&temporary, target).map_err(|error| error.to_string())
}

fn event_to_codex_event(payload: &Value) -> Option<CodexEvent> {
    let event = string_field(payload, "hook_event_name")?;
    let thread_id = session_id(payload);
    match event.as_str() {
        "SessionStart" => Some(CodexEvent::HookSessionStarted {
            id: thread_id,
            cwd: string_field(payload, "cwd"),
            title: None,
        }),
        "SessionEnd" => Some(CodexEvent::HookSessionEnded { id: thread_id }),
        "UserPromptSubmit" => {
            string_field(payload, "prompt").map(|text| CodexEvent::HookPrompt { thread_id, text })
        }
        "PermissionRequest" => Some(CodexEvent::HookApproval {
            request_id: string_field(payload, CODECRAFT_REQUEST_ID_FIELD)?,
            thread_id,
            tool: tool_name(payload),
            summary: activity_summary(payload),
            cwd: string_field(payload, "cwd"),
            allow_session: payload
                .get("can_always_allow")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }),
        "PreToolUse" => {
            let tool = tool_name(payload);
            let activity_id = tool_activity_id(payload);
            if tool.eq_ignore_ascii_case("request_user_input") {
                let raw_questions = payload
                    .get("tool_input")
                    .and_then(|input| input.get("questions"))
                    .and_then(Value::as_array);
                if raw_questions.is_some_and(|questions| {
                    questions.iter().any(|question| {
                        question
                            .get("question")
                            .and_then(Value::as_str)
                            .is_some_and(|text| !text.trim().is_empty())
                    })
                }) {
                    Some(CodexEvent::HookUserInput {
                        request_id: format!("hook-input-{activity_id}"),
                        thread_id,
                        questions: parse_user_input_questions(raw_questions),
                        cwd: string_field(payload, "cwd"),
                    })
                } else {
                    Some(CodexEvent::HookToolStarted {
                        thread_id,
                        activity_id,
                        tool,
                        summary: activity_summary(payload),
                    })
                }
            } else {
                Some(CodexEvent::HookToolStarted {
                    thread_id,
                    activity_id,
                    tool,
                    summary: activity_summary(payload),
                })
            }
        }
        "PostToolUse" => Some(CodexEvent::HookToolFinished {
            thread_id,
            activity_id: tool_activity_id(payload),
            tool: tool_name(payload),
            summary: activity_summary(payload),
            failed: tool_response_failed(payload.get("tool_response")),
        }),
        "Stop" => {
            if let Some(plan) = plan_text_from_payload(payload) {
                Some(CodexEvent::HookPlan {
                    request_id: plan_request_id(payload, &plan),
                    thread_id,
                    plan,
                    cwd: string_field(payload, "cwd"),
                })
            } else {
                Some(CodexEvent::HookStopped {
                    thread_id,
                    reason: string_field(payload, "last_assistant_message")
                        .or_else(|| string_field(payload, "reason")),
                })
            }
        }
        "SubagentStop" => Some(CodexEvent::HookSubagentStopped { thread_id }),
        _ => None,
    }
}

fn plan_text_from_payload(payload: &Value) -> Option<String> {
    if let Some(plan) = string_field(payload, CODECRAFT_PLAN_FIELD) {
        if !plan.trim().is_empty() {
            return Some(plan);
        }
    }
    let text = string_field(payload, "last_assistant_message")?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    let normalized = trimmed.to_ascii_lowercase();
    let explicit_prompt = normalized.contains("implement this plan?")
        || normalized.contains("clear context and implement")
        || normalized.contains("stay in plan mode")
        || normalized.contains("<proposed_plan>");
    let action_lines = trimmed
        .lines()
        .filter(|line| is_plan_action_line(line))
        .count();
    let structured_plan = trimmed.lines().any(is_plan_heading) && action_lines >= 2;
    let plan_mode = ["permission_mode", "mode", "plan_mode", "collaboration_mode"]
        .iter()
        .filter_map(|key| payload.get(*key))
        .any(mode_value_is_plan);

    (explicit_prompt || structured_plan || plan_mode && action_lines >= 2).then_some(text)
}

fn codex_home_dir() -> PathBuf {
    env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("USERPROFILE")
                .or_else(|| env::var_os("HOME"))
                .map(|home| PathBuf::from(home).join(".codex"))
        })
        .unwrap_or_else(|| PathBuf::from(".codex"))
}

fn find_codex_rollout_in(directory: &Path, suffix: &str) -> Option<PathBuf> {
    let entries = fs::read_dir(directory).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_codex_rollout_in(&path, suffix) {
                return Some(found);
            }
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(suffix))
        {
            return Some(path);
        }
    }
    None
}

fn find_codex_rollout(session_id: &str) -> Option<PathBuf> {
    if session_id.is_empty() {
        return None;
    }
    let suffix = format!("-{session_id}.jsonl");
    find_codex_rollout_in(&codex_home_dir().join("sessions"), &suffix)
}

fn plan_text_from_rollout_file(
    path: &Path,
    session_id: &str,
    turn_id: Option<&str>,
) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let reader = BufReader::new(file);
    let mut latest_plan = None;

    for line in reader.lines().map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(payload) = value.get("payload") else {
            continue;
        };
        if string_field(payload, "type").as_deref() != Some("item_completed") {
            continue;
        }
        if string_field(payload, "thread_id").is_some_and(|thread| thread != session_id) {
            continue;
        }
        if turn_id.is_some() && string_field(payload, "turn_id").as_deref() != turn_id {
            continue;
        }
        let Some(item) = payload.get("item") else {
            continue;
        };
        if string_field(item, "type").as_deref() != Some("Plan") {
            continue;
        }
        let Some(text) = string_field(item, "text") else {
            continue;
        };
        if !text.trim().is_empty() {
            latest_plan = Some(text);
        }
    }

    latest_plan
}

fn plan_text_from_codex_rollout(payload: &Value) -> Option<String> {
    let session_id = string_field(payload, "session_id")?;
    let turn_id = string_field(payload, "turn_id")?;
    let path = find_codex_rollout(&session_id)?;
    plan_text_from_rollout_file(&path, &session_id, Some(&turn_id))
}

fn mode_value_is_plan(value: &Value) -> bool {
    value.as_str().is_some_and(|mode| {
        mode.eq_ignore_ascii_case("plan") || mode.eq_ignore_ascii_case("plan_mode")
    }) || value
        .get("mode")
        .and_then(Value::as_str)
        .is_some_and(|mode| {
            mode.eq_ignore_ascii_case("plan") || mode.eq_ignore_ascii_case("plan_mode")
        })
}

fn is_plan_heading(line: &str) -> bool {
    let normalized = line
        .trim()
        .trim_matches(|character: char| {
            matches!(character, '#' | '*' | '_' | '`' | ':' | '：' | ' ')
        })
        .to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "plan" | "implementation plan" | "proposed plan" | "计划" | "实施计划" | "实行计划"
    )
}

fn is_plan_action_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("- [") {
        return true;
    }
    let digit_count = trimmed
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .count();
    digit_count > 0
        && trimmed
            .chars()
            .nth(digit_count)
            .is_some_and(|character| matches!(character, '.' | ')' | '、'))
}

fn plan_request_id(payload: &Value, plan: &str) -> String {
    let suffix = string_field(payload, "turn_id").unwrap_or_else(|| {
        let hash = plan.bytes().fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
        });
        format!("{hash:016x}")
    });
    format!("hook-plan-{}-{suffix}", session_id(payload))
}

fn tool_response_failed(response: Option<&Value>) -> bool {
    let Some(response) = response else {
        return false;
    };
    response
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || response
            .get("is_error")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        || response.get("success").and_then(Value::as_bool) == Some(false)
        || response
            .get("exitCode")
            .and_then(Value::as_i64)
            .is_some_and(|code| code != 0)
        || response
            .get("exit_code")
            .and_then(Value::as_i64)
            .is_some_and(|code| code != 0)
}

pub(crate) fn read_audit_tail(max_lines: usize) -> Result<String, String> {
    let contents = fs::read_to_string(audit_path()).unwrap_or_default();
    let mut lines = contents
        .lines()
        .rev()
        .take(max_lines.max(1))
        .collect::<Vec<_>>();
    lines.reverse();
    let mut result = lines.join("\n");
    if result.len() > 24_000 {
        result = result
            .chars()
            .rev()
            .take(24_000)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
    }
    Ok(result)
}

#[cfg(windows)]
fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        encoded.push(ALPHABET[(first >> 2) as usize] as char);
        encoded.push(ALPHABET[(((first & 0x03) << 4) | (second >> 4)) as usize] as char);
        encoded.push(if chunk.len() > 1 {
            ALPHABET[(((second & 0x0f) << 2) | (third >> 6)) as usize] as char
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            ALPHABET[(third & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    encoded
}

#[cfg(windows)]
fn hook_command(executable: &Path) -> String {
    let executable = executable.display().to_string();
    let shell_safe = executable
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "\\/:._-".contains(character));
    if shell_safe {
        return format!("{executable} {HOOK_ARGUMENT}");
    }

    let escaped = executable.replace('\'', "''");
    let script = format!("& '{escaped}' {HOOK_ARGUMENT}");
    let utf16 = script
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    format!(
        "powershell.exe -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand {}",
        base64_encode(&utf16)
    )
}

#[cfg(not(windows))]
fn hook_command(executable: &Path) -> String {
    let escaped = executable.display().to_string().replace('\'', "'\\''");
    format!("'{escaped}' {HOOK_ARGUMENT}")
}

fn hook_file_for(path: &Path) -> PathBuf {
    path.join(".codex").join("hooks.json")
}

fn hook_target(project_dir: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(project) = project_dir {
        return Ok(hook_file_for(project));
    }
    let home = env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
        .ok_or_else(|| "Unable to locate the user profile".to_string())?;
    Ok(hook_file_for(&home))
}

fn handler_is_codecraft(handler: &Value) -> bool {
    ["command", "commandWindows"].iter().any(|key| {
        handler
            .get(*key)
            .and_then(Value::as_str)
            .is_some_and(|command| command.contains(HOOK_ARGUMENT))
    })
}

fn hook_file_installed(root: &Value) -> bool {
    let Some(hooks) = root.get("hooks").and_then(Value::as_object) else {
        return false;
    };
    HOOK_EVENTS.iter().all(|event| {
        hooks
            .get(*event)
            .and_then(Value::as_array)
            .is_some_and(|groups| {
                groups.iter().any(|group| {
                    group
                        .get("hooks")
                        .and_then(Value::as_array)
                        .is_some_and(|handlers| handlers.iter().any(handler_is_codecraft))
                })
            })
    })
}

pub(crate) fn codex_hooks_installed(project_dir: Option<&Path>) -> Result<bool, String> {
    let path = hook_target(project_dir)?;
    if !path.exists() {
        return Ok(false);
    }
    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let root = serde_json::from_str::<Value>(&text).map_err(|error| error.to_string())?;
    Ok(hook_file_installed(&root))
}

fn remove_codecraft_hooks(root: &mut Value) -> Result<bool, String> {
    let before = root.clone();
    let object = root
        .as_object_mut()
        .ok_or_else(|| "Codex hooks.json must contain a JSON object".to_string())?;
    let Some(hooks) = object.get_mut("hooks").and_then(Value::as_object_mut) else {
        return Ok(false);
    };
    for event in HOOK_EVENTS {
        let remove_event = if let Some(groups) = hooks.get_mut(event).and_then(Value::as_array_mut)
        {
            for group in groups.iter_mut() {
                if let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                    handlers.retain(|handler| !handler_is_codecraft(handler));
                }
            }
            groups.retain(|group| {
                group
                    .get("hooks")
                    .and_then(Value::as_array)
                    .is_none_or(|handlers| !handlers.is_empty())
            });
            groups.is_empty()
        } else {
            false
        };
        if remove_event {
            hooks.remove(event);
        }
    }
    Ok(*root != before)
}

pub(crate) fn uninstall_codex_hooks(project_dir: Option<&Path>) -> Result<(), String> {
    let path = hook_target(project_dir)?;
    if !path.exists() {
        return Ok(());
    }
    let text = fs::read_to_string(&path).map_err(|error| error.to_string())?;
    let mut root = serde_json::from_str::<Value>(&text).map_err(|error| error.to_string())?;
    if !remove_codecraft_hooks(&mut root)? {
        return Ok(());
    }
    let bytes = serde_json::to_vec_pretty(&root).map_err(|error| error.to_string())?;
    fs::write(path, bytes).map_err(|error| error.to_string())
}

fn handler_matches_command(handler: &Value, command: &str) -> bool {
    handler_is_codecraft(handler)
        || ["command", "commandWindows"]
            .iter()
            .any(|key| handler.get(*key).and_then(Value::as_str) == Some(command))
}

fn update_handler(handler: &mut Value, command: &str, event: &str) {
    if !handler.is_object() {
        *handler = json!({});
    }
    let object = handler.as_object_mut().expect("handler was normalized");
    object.insert("type".to_string(), json!("command"));
    object.insert("command".to_string(), json!(command));
    object.insert("commandWindows".to_string(), json!(command));
    object.insert("async".to_string(), json!(false));
    object.insert(
        "timeout".to_string(),
        json!(if event == "PermissionRequest" {
            APPROVAL_HOOK_TIMEOUT_SECONDS
        } else if event == "SessionEnd" {
            3
        } else {
            10
        }),
    );
    object.insert(
        "statusMessage".to_string(),
        json!(if event == "PermissionRequest" {
            "CodeCraft 正在等待审批"
        } else {
            "CodeCraft 正在采集 Codex Hook 事件"
        }),
    );
}

fn merge_hook_file(path: &Path, command: &str) -> Result<(), String> {
    let mut root = if path.exists() {
        let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
        serde_json::from_str::<Value>(&text).map_err(|error| error.to_string())?
    } else {
        json!({})
    };
    let hooks = root
        .as_object_mut()
        .ok_or_else(|| "Codex hooks.json must contain a JSON object".to_string())?
        .entry("hooks")
        .or_insert_with(|| json!({}));
    let hooks = hooks
        .as_object_mut()
        .ok_or_else(|| "Codex hooks.json hooks must be an object".to_string())?;
    for event in HOOK_EVENTS {
        let groups = hooks.entry(event).or_insert_with(|| json!([]));
        let groups = groups
            .as_array_mut()
            .ok_or_else(|| format!("Codex hooks.{event} must be an array"))?;
        let mut found = false;
        for group in groups.iter_mut() {
            let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
                continue;
            };
            handlers.retain_mut(|handler| {
                if !handler_matches_command(handler, command) {
                    return true;
                }
                if found {
                    return false;
                }
                update_handler(handler, command, event);
                found = true;
                true
            });
        }
        groups.retain(|group| {
            group
                .get("hooks")
                .and_then(Value::as_array)
                .is_none_or(|handlers| !handlers.is_empty())
        });
        if !found {
            let mut handler = json!({});
            update_handler(&mut handler, command, event);
            groups.push(json!({
                "matcher": "*",
                "hooks": [handler]
            }));
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let bytes = serde_json::to_vec_pretty(&root).map_err(|error| error.to_string())?;
    fs::write(path, bytes).map_err(|error| error.to_string())
}

/// Install user-level hooks, or project hooks when a project directory is supplied.
/// Existing hook definitions are preserved and the CodeCraft command is de-duplicated.
pub(crate) fn install_codex_hooks(
    executable: &Path,
    project_dir: Option<&Path>,
) -> Result<String, String> {
    let target = hook_target(project_dir)?;
    merge_hook_file(&target, &hook_command(executable))?;
    Ok(target.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn windows_hook_command_does_not_quote_shell_safe_paths() {
        let command = hook_command(Path::new(
            r"E:\project\CodeCraft\target\debug\codecraft-tauri.exe",
        ));
        assert_eq!(
            command,
            r"E:\project\CodeCraft\target\debug\codecraft-tauri.exe --codecraft-codex-hook"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_hook_command_runs_paths_with_spaces() {
        use std::process::{Command, Stdio};

        let root = env::temp_dir().join(format!(
            "codecraft codex hook command test {}",
            process::id()
        ));
        let script = root.join("hook probe.cmd");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(&script, b"@echo off\r\nexit /b 0\r\n").unwrap();

        let command = hook_command(&script);
        assert!(command.starts_with("powershell.exe "));
        let status = Command::new("cmd.exe")
            .args(["/d", "/s", "/c", &command])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn permission_reply_uses_codex_hook_specific_shape() {
        let reply = permission_decision_output("allow").unwrap();
        assert_eq!(reply["hookSpecificOutput"]["decision"]["behavior"], "allow");
    }

    #[test]
    fn permission_payload_maps_to_a_hook_approval() {
        let payload = json!({
            "hook_event_name":"PermissionRequest",
            "session_id":"s1",
            "tool_name":"apply_patch",
            "tool_input":{"command":"***"},
            "cwd":"C:/repo",
            "codecraft_request_id":"hook-permission-u1",
            "can_always_allow":true
        });
        assert!(matches!(
            event_to_codex_event(&payload),
            Some(CodexEvent::HookApproval {
                request_id,
                thread_id,
                tool,
                allow_session: true,
                ..
            }) if request_id == "hook-permission-u1"
                && thread_id == "s1"
                && tool == "apply_patch"
        ));
    }

    #[test]
    fn session_approval_preserves_codex_permission_suggestions() {
        let payload = json!({
            "permission_suggestions": [{"type":"addRules","rules":["apply_patch"]}]
        });
        let reply = permission_session_decision_output(&payload);
        assert_eq!(reply["hookSpecificOutput"]["decision"]["behavior"], "allow");
        assert_eq!(
            reply["hookSpecificOutput"]["updatedPermissions"][0]["type"],
            "addRules"
        );
    }

    #[test]
    fn hook_payload_maps_to_tool_events() {
        let started = event_to_codex_event(&json!({
            "hook_event_name":"PreToolUse","session_id":"s1","tool_name":"Bash","tool_use_id":"u1","tool_input":{"command":"cargo test"}
        }));
        assert!(matches!(started, Some(CodexEvent::HookToolStarted { .. })));
        let finished = event_to_codex_event(&json!({
            "hook_event_name":"PostToolUse","session_id":"s1","tool_name":"Bash","tool_use_id":"u1","tool_response":{"exit_code":1}
        }));
        assert!(matches!(
            finished,
            Some(CodexEvent::HookToolFinished { failed: true, .. })
        ));
    }

    #[test]
    fn capture_payload_drops_tool_results_and_caps_arguments() {
        let safe = sanitized_capture(&json!({
            "hook_event_name":"PostToolUse",
            "session_id":"s1",
            "tool_name":"Bash",
            "tool_input":{"command":"x".repeat(2000),"secret":"token"},
            "tool_response":{"output":"sensitive","exit_code":0}
        }));
        assert!(safe["tool_input"]["secret"].is_null());
        assert!(safe["tool_response"]["output"].is_null());
        assert_eq!(safe["tool_input"]["command"].as_str().unwrap().len(), 1000);
    }

    #[test]
    fn capture_payload_keeps_sanitized_user_input_questions() {
        let safe = sanitized_capture(&json!({
            "hook_event_name":"PreToolUse",
            "session_id":"s1",
            "tool_name":"request_user_input",
            "tool_input":{
                "questions":[{
                    "id":"q1",
                    "header":"Choose",
                    "question":"How should this continue?",
                    "isOther":true,
                    "options":[{"label":"A","description":"Option A","secret":"drop"}]
                }],
                "token":"drop"
            }
        }));

        assert_eq!(safe["tool_input"]["questions"][0]["id"], "q1");
        assert_eq!(
            safe["tool_input"]["questions"][0]["options"][0]["label"],
            "A"
        );
        assert!(safe["tool_input"]["questions"][0]["options"][0]["secret"].is_null());
        assert!(safe["tool_input"]["token"].is_null());
    }

    #[test]
    fn request_user_input_hook_maps_to_a_read_only_store_event() {
        let event = event_to_codex_event(&json!({
            "hook_event_name":"PreToolUse",
            "session_id":"s1",
            "tool_name":"request_user_input",
            "tool_use_id":"u1",
            "cwd":"C:/repo",
            "tool_input":{
                "questions":[{
                    "id":"q1",
                    "header":"Choose",
                    "question":"Continue?",
                    "options":[{"label":"Yes","description":"continue"}]
                }]
            }
        }));

        assert!(matches!(
            event,
            Some(CodexEvent::HookUserInput {
                request_id,
                thread_id,
                questions,
                ..
            }) if request_id == "hook-input-u1"
                && thread_id == "s1"
                && questions.len() == 1
                && questions[0].id == "q1"
        ));
    }

    #[test]
    fn request_user_input_without_real_questions_stays_a_tool_event() {
        for payload in [
            json!({
                "hook_event_name":"PreToolUse",
                "session_id":"s1",
                "tool_name":"request_user_input",
                "tool_use_id":"missing"
            }),
            json!({
                "hook_event_name":"PreToolUse",
                "session_id":"s1",
                "tool_name":"request_user_input",
                "tool_use_id":"empty",
                "tool_input":{"questions":[]}
            }),
            json!({
                "hook_event_name":"PreToolUse",
                "session_id":"s1",
                "tool_name":"request_user_input",
                "tool_use_id":"blank",
                "tool_input":{"questions":[{"question":"   "}]}
            }),
        ] {
            assert!(matches!(
                event_to_codex_event(&payload),
                Some(CodexEvent::HookToolStarted { tool, .. })
                    if tool == "request_user_input"
            ));
        }
    }

    #[test]
    fn plan_mode_stop_maps_to_a_read_only_plan_event() {
        let event = event_to_codex_event(&json!({
            "hook_event_name":"Stop",
            "session_id":"s1",
            "turn_id":"turn-plan",
            "collaboration_mode":"plan",
            "cwd":"C:/repo",
            "last_assistant_message":"1. Inspect the current flow\n2. Add the plan preview\n3. Verify the interaction"
        }));

        assert!(matches!(
            event,
            Some(CodexEvent::HookPlan {
                request_id,
                thread_id,
                plan,
                cwd,
            }) if request_id == "hook-plan-s1-turn-plan"
                && thread_id == "s1"
                && plan.contains("Add the plan preview")
                && cwd.as_deref() == Some("C:/repo")
        ));
    }

    #[test]
    fn structured_plan_stop_is_detected_without_mode_metadata() {
        let event = event_to_codex_event(&json!({
            "hook_event_name":"Stop",
            "session_id":"s1",
            "last_assistant_message":"## Plan\n\n- Inspect the hook payload\n- Render the plan\n- Run focused tests"
        }));

        assert!(matches!(event, Some(CodexEvent::HookPlan { .. })));
    }

    #[test]
    fn sanitized_plan_payload_round_trips_through_the_inbox_shape() {
        let payload = json!({
            "hook_event_name":"Stop",
            "session_id":"s1",
            "turn_id":"turn-plan",
            "collaboration_mode": { "mode": "plan" },
            "last_assistant_message":"1. Inspect the current flow\n2. Add the plan preview"
        });
        let safe = sanitized_capture(&payload);

        assert_eq!(safe["collaboration_mode"], "plan");
        assert!(matches!(
            event_to_codex_event(&safe),
            Some(CodexEvent::HookPlan { .. })
        ));
    }

    #[test]
    fn codex_rollout_plan_matches_item_completed_and_turn_id() {
        let path = env::temp_dir().join(format!(
            "codecraft-codex-rollout-test-{}-{}.jsonl",
            process::id(),
            now_ms()
        ));
        let lines = [
            json!({
                "type":"event_msg",
                "payload":{
                    "type":"item_completed",
                    "thread_id":"thread-1",
                    "turn_id":"turn-other",
                    "item":{"type":"Plan","text":"wrong turn"}
                }
            }),
            json!({
                "type":"event_msg",
                "payload":{
                    "type":"item_completed",
                    "thread_id":"thread-1",
                    "turn_id":"turn-plan",
                    "item":{"type":"Plan","text":"# 空计划\n\n## 总结\n\n等待实施"}
                }
            }),
            json!({
                "type":"event_msg",
                "payload":{
                    "type":"item_completed",
                    "thread_id":"thread-1",
                    "turn_id":"turn-plan",
                    "item":{"type":"Plan","text":"# 最新计划\n\n1. 保持当前会话"}
                }
            }),
        ];
        let contents = lines
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&path, contents).unwrap();

        assert_eq!(
            plan_text_from_rollout_file(&path, "thread-1", Some("turn-plan")),
            Some("# 最新计划\n\n1. 保持当前会话".to_string())
        );
        assert!(plan_text_from_rollout_file(&path, "thread-1", Some("missing")).is_none());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn captured_codex_plan_takes_precedence_over_stop_explanation() {
        let event = event_to_codex_event(&json!({
            "hook_event_name":"Stop",
            "session_id":"s1",
            "turn_id":"turn-plan",
            "last_assistant_message":"这个计划块就是用来触发实施选择的。",
            "codecraft_plan":"# 空计划\n\n## 总结\n\n这是计划正文。"
        }));

        assert!(matches!(
            event,
            Some(CodexEvent::HookPlan { plan, .. }) if plan == "# 空计划\n\n## 总结\n\n这是计划正文。"
        ));
    }

    #[test]
    fn ordinary_stop_remains_a_stopped_event() {
        let event = event_to_codex_event(&json!({
            "hook_event_name":"Stop",
            "session_id":"s1",
            "last_assistant_message":"The requested change is complete."
        }));

        assert!(matches!(event, Some(CodexEvent::HookStopped { .. })));
    }

    #[test]
    fn merge_preserves_existing_groups_and_is_idempotent() {
        let root = env::temp_dir().join(format!("codecraft-codex-hook-test-{}", process::id()));
        let path = root.join("hooks.json");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(
            &path,
            serde_json::to_vec_pretty(&json!({
                "hooks": {
                    "PreToolUse": [{
                        "matcher": "^Bash$",
                        "hooks": [{"type":"command","command":"existing-policy"}]
                    }]
                }
            }))
            .unwrap(),
        )
        .unwrap();
        merge_hook_file(&path, "codecraft-hook").unwrap();
        merge_hook_file(&path, "codecraft-hook").unwrap();
        let merged: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let pre_tool = merged["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre_tool.len(), 2);
        assert_eq!(pre_tool[0]["matcher"], "^Bash$");
        let codecraft_count = pre_tool
            .iter()
            .flat_map(|group| group["hooks"].as_array().into_iter().flatten())
            .filter(|handler| handler["command"] == "codecraft-hook")
            .count();
        assert_eq!(codecraft_count, 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn merge_installs_synchronous_handlers_for_all_events() {
        let root =
            env::temp_dir().join(format!("codecraft-codex-hook-sync-test-{}", process::id()));
        let path = root.join("hooks.json");
        let _ = fs::remove_dir_all(&root);

        merge_hook_file(&path, "codecraft-hook").unwrap();
        let config: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            config["hooks"]["SessionStart"][0]["hooks"][0]["async"],
            false
        );
        assert_eq!(
            config["hooks"]["PermissionRequest"][0]["hooks"][0]["async"],
            false
        );
        assert_eq!(
            config["hooks"]["PermissionRequest"][0]["hooks"][0]["timeout"],
            APPROVAL_HOOK_TIMEOUT_SECONDS
        );
        let handlers = config["hooks"]["SessionStart"][0]["hooks"]
            .as_array()
            .unwrap();
        assert_eq!(handlers.len(), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn detects_and_removes_only_codecraft_handlers() {
        let mut root = json!({
            "hooks": {
                "PreToolUse": [{
                    "hooks": [{"type": "command", "command": "existing-policy"}]
                }]
            }
        });
        let path = env::temp_dir()
            .join(format!(
                "codecraft-codex-hook-remove-test-{}",
                process::id()
            ))
            .join("hooks.json");
        let _ = fs::remove_file(&path);
        merge_hook_file(&path, "codecraft --codecraft-codex-hook").unwrap();
        root = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        root["hooks"]["PreToolUse"][0]["hooks"] = json!([
            {"type": "command", "command": "existing-policy"}
        ]);
        root["hooks"]["PreToolUse"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "hooks": [{"type": "command", "command": "codecraft --codecraft-codex-hook"}]
            }));
        assert!(hook_file_installed(&root));
        assert!(remove_codecraft_hooks(&mut root).unwrap());
        assert!(!hook_file_installed(&root));
        assert_eq!(
            root["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
            "existing-policy"
        );
        let _ = fs::remove_file(path);
    }
}
