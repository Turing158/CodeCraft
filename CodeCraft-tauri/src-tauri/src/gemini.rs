//! Read-only Gemini CLI session observation state.
//!
//! Gemini hooks do not expose a stable approval identifier.  Every identifier
//! in this module therefore has observation semantics only and is never used
//! to submit a decision back to Gemini.

use std::{
    collections::{HashMap, HashSet},
    env,
    fs,
    path::{Path, PathBuf},
};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;

const MAX_OUTPUT_ENTRIES: usize = 24;
const MAX_OUTPUT_CHARS: usize = 8_000;
const MAX_ACTIVITIES: usize = 48;
const MAX_INTERACTIONS: usize = 32;
const MAX_SESSIONS: usize = 100;
const SESSION_TTL_MS: u64 = 24 * 60 * 60 * 1_000;
pub(crate) const MAX_PLAN_CHARS: usize = 512 * 1024;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

fn cap_text(text: &str, limit: usize) -> String {
    let mut chars = text.chars();
    let mut result: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        result.push('\u{2026}');
    }
    result
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn session_id(value: &Value) -> String {
    string_field(value, "session_id")
        .filter(|id| !id.trim().is_empty())
        .unwrap_or_else(|| format!("unknown-session-{}", hash_observation(value)))
}

fn event_name(value: &Value) -> Option<&str> {
    value.get("hook_event_name").and_then(Value::as_str)
}

fn tool_name(value: &Value) -> String {
    string_field(value, "tool_name").unwrap_or_else(|| "unknown".to_string())
}

fn tool_id(value: &Value) -> String {
    string_field(value, "tool_call_id")
        .or_else(|| string_field(value, "tool_use_id"))
        .or_else(|| string_field(value, "call_id"))
        .unwrap_or_else(|| {
            let mut key = serde_json::Map::new();
            key.insert("tool_name".to_string(), Value::String(tool_name(value)));
            if let Some(input) = value.get("tool_input") {
                key.insert("tool_input".to_string(), input.clone());
            }
            hash_observation(&Value::Object(key))
        })
}

fn summary(value: &Value) -> String {
    let input = value.get("tool_input");
    let text = string_field(value, "prompt")
        .or_else(|| string_field(value, "message"))
        .or_else(|| input.and_then(|input| string_field(input, "command")))
        .or_else(|| input.and_then(|input| string_field(input, "description")))
        .or_else(|| string_field(value, "reason"))
        .unwrap_or_else(|| format!("{} 工具活动", tool_name(value)));
    cap_text(&text, 320)
}

fn hash_observation(value: &Value) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    let serialized = serde_json::to_string(value).unwrap_or_default();
    for byte in serialized.bytes() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
    }
    format!("gemini-{hash:016x}")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum GeminiSessionStatus {
    Working,
    WaitingForInput,
    ToolRunning,
    ToolCompleted,
    ToolFailed,
    Idle,
    Stopped,
}

impl GeminiSessionStatus {
    pub(crate) fn is_active(self) -> bool {
        !matches!(self, Self::Idle | Self::Stopped)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum GeminiInteractionKind {
    AskUser,
    ToolPermission,
    FileChange,
    Shell,
    Mcp,
    SandboxExpansion,
    ExitPlanMode,
    Notification,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum GeminiObservationStatus {
    Observed,
    Updated,
    ToolCompleted,
    SessionEnded,
    Stale,
    NavigationAvailable,
    NavigationUnavailable,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GeminiActivity {
    pub id: String,
    pub tool: String,
    pub summary: String,
    pub status: String,
    pub started_at: u64,
    pub updated_at: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GeminiOutputEntry {
    pub id: String,
    pub text: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GeminiInteraction {
    pub observation_id: String,
    pub interaction_key: String,
    pub kind: GeminiInteractionKind,
    pub status: GeminiObservationStatus,
    pub title: String,
    pub detail: String,
    pub tool_name: Option<String>,
    pub tool_call_id: Option<String>,
    pub tool_input: Option<Value>,
    pub questions: Vec<Value>,
    pub plan_filename: Option<String>,
    pub plan: Option<String>,
    pub plan_read_error: Option<String>,
    pub captured_at: u64,
    pub truncated: bool,
    pub navigation_available: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GeminiTerminalBinding {
    pub pid: Option<u32>,
    pub parent_pid: Option<u32>,
    pub process_created_at: Option<u64>,
    pub console_window: Option<String>,
    pub captured_at: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GeminiSession {
    pub id: String,
    pub title: String,
    pub cwd: Option<String>,
    pub status: GeminiSessionStatus,
    pub started_at: u64,
    pub updated_at: u64,
    pub ended_at: Option<u64>,
    pub activities: Vec<GeminiActivity>,
    pub outputs: Vec<GeminiOutputEntry>,
    pub pending_interactions: Vec<GeminiInteraction>,
    pub terminal_binding: Option<GeminiTerminalBinding>,
    pub integration_status: String,
}

impl GeminiSession {
    pub(crate) fn is_active(&self) -> bool {
        self.status.is_active()
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GeminiSnapshot {
    pub connected: bool,
    pub integration_error: Option<String>,
    pub version: u64,
    pub sessions: Vec<GeminiSession>,
    pub interactions: Vec<GeminiInteraction>,
    pub navigation_capability: String,
    pub fallback_action: String,
}

#[derive(Clone, Debug)]
pub(crate) struct GeminiEvent {
    pub payload: Value,
    pub captured_at: u64,
    pub observation_id: String,
}

#[derive(Default)]
pub(crate) struct GeminiStore {
    connected: bool,
    error: Option<String>,
    version: u64,
    sessions: HashMap<String, GeminiSession>,
    seen: HashSet<String>,
}

impl GeminiStore {
    pub(crate) fn clear(&mut self) {
        self.sessions.clear();
        self.seen.clear();
        self.bump();
    }

    pub(crate) fn set_integration_error(&mut self, error: Option<String>) {
        if self.connected == error.is_none() && self.error == error {
            return;
        }
        self.connected = error.is_none();
        self.error = error;
        self.bump();
    }

    pub(crate) fn apply(&mut self, event: GeminiEvent) {
        if !self.seen.insert(event.observation_id.clone()) {
            return;
        }
        let payload = event.payload;
        let id = session_id(&payload);
        let name = event_name(&payload).unwrap_or("");
        let cwd = string_field(&payload, "cwd");
        let now = event.captured_at.max(now_ms());
        let session = self.sessions.entry(id.clone()).or_insert_with(|| GeminiSession {
            id: id.clone(),
            title: "Gemini 会话".to_string(),
            cwd: cwd.clone(),
            status: GeminiSessionStatus::Working,
            started_at: now,
            updated_at: now,
            ended_at: None,
            activities: Vec::new(),
            outputs: Vec::new(),
            pending_interactions: Vec::new(),
            terminal_binding: terminal_binding_from_payload(&payload, now),
            integration_status: "running".to_string(),
        });
        if session.cwd.is_none() {
            session.cwd = cwd;
        }
        session.updated_at = now;

        match name {
            "SessionStart" => {
                session.status = GeminiSessionStatus::Working;
                session.integration_status = "running".to_string();
                if let Some(title) = string_field(&payload, "title") {
                    session.title = cap_text(&title, 256);
                }
            }
            "SessionEnd" => {
                session.status = GeminiSessionStatus::Stopped;
                session.ended_at = Some(now);
                session.integration_status = "sessionEnded".to_string();
                for interaction in &mut session.pending_interactions {
                    if !matches!(interaction.status, GeminiObservationStatus::ToolCompleted | GeminiObservationStatus::SessionEnded) {
                        interaction.status = GeminiObservationStatus::SessionEnded;
                    }
                }
            }
            "BeforeAgent" => {
                session.status = GeminiSessionStatus::Working;
                if let Some(prompt) = string_field(&payload, "prompt") {
                    push_output(&mut session.outputs, &format!("prompt-{}", now), &format!("用户：{}", cap_text(&prompt, MAX_OUTPUT_CHARS)), now);
                }
            }
            "AfterAgent" => {
                session.status = GeminiSessionStatus::Idle;
                if let Some(text) = string_field(&payload, "response")
                    .or_else(|| string_field(&payload, "prompt_response"))
                    .or_else(|| string_field(&payload, "agent_response"))
                    .or_else(|| string_field(&payload, "last_assistant_message"))
                {
                    push_output(&mut session.outputs, &format!("response-{}", now), &cap_text(&text, MAX_OUTPUT_CHARS), now);
                }
            }
            "BeforeTool" => {
                let tool = tool_name(&payload);
                let activity_id = tool_id(&payload);
                let detail = summary(&payload);
                let (kind, questions, plan_filename, plan_hint) = classify_tool(&tool, &payload);
                let (plan, plan_read_error) = if kind == GeminiInteractionKind::ExitPlanMode {
                    resolve_plan(
                        session.cwd.as_deref(),
                        plan_filename.as_deref(),
                        plan_hint.as_deref(),
                    )
                } else {
                    (plan_hint.map(|value| cap_text(&value, MAX_PLAN_CHARS)), None)
                };
                session.status = if kind == GeminiInteractionKind::AskUser {
                    GeminiSessionStatus::WaitingForInput
                } else {
                    GeminiSessionStatus::ToolRunning
                };
                session.activities.push(GeminiActivity {
                    id: activity_id.clone(),
                    tool: tool.clone(),
                    summary: detail.clone(),
                    status: "running".to_string(),
                    started_at: now,
                    updated_at: now,
                });
                let interaction = GeminiInteraction {
                    observation_id: event.observation_id,
                    interaction_key: format!("{}:{}:{}", id, activity_id, kind_label(kind)),
                    kind,
                    status: GeminiObservationStatus::Observed,
                    title: interaction_title(kind, &tool),
                    detail,
                    tool_name: Some(tool),
                    tool_call_id: Some(activity_id),
                    tool_input: payload.get("tool_input").cloned(),
                    questions,
                    plan_filename,
                    plan,
                    plan_read_error,
                    captured_at: now,
                    truncated: payload.get("truncated").and_then(Value::as_bool).unwrap_or(false),
                    navigation_available: session.terminal_binding.is_some(),
                };
                session.pending_interactions.push(interaction);
            }
            "AfterTool" => {
                let activity_id = tool_id(&payload);
                let failed = tool_failed(payload.get("tool_response"));
                if let Some(activity) = session.activities.iter_mut().rev().find(|activity| activity.id == activity_id) {
                    activity.status = if failed { "failed" } else { "completed" }.to_string();
                    activity.updated_at = now;
                }
                if let Some(interaction) = session.pending_interactions.iter_mut().rev().find(|interaction| {
                    interaction.tool_call_id.as_deref() == Some(activity_id.as_str())
                        || (interaction.tool_call_id.is_none()
                            && interaction.tool_name.as_deref() == string_field(&payload, "tool_name").as_deref())
                }) {
                    interaction.status = GeminiObservationStatus::ToolCompleted;
                    interaction.detail = summary(&payload);
                }
                session.status = if failed { GeminiSessionStatus::ToolFailed } else { GeminiSessionStatus::ToolCompleted };
            }
            "Notification" => {
                let detail = summary(&payload);
                session.pending_interactions.push(GeminiInteraction {
                    observation_id: event.observation_id,
                    interaction_key: format!("{}:notification:{}", id, now),
                    kind: GeminiInteractionKind::Notification,
                    status: GeminiObservationStatus::Observed,
                    title: string_field(&payload, "title").unwrap_or_else(|| "Gemini 通知".to_string()),
                    detail,
                    tool_name: None,
                    tool_call_id: None,
                    tool_input: None,
                    questions: Vec::new(),
                    plan_filename: None,
                    plan: None,
                    plan_read_error: None,
                    captured_at: now,
                    truncated: false,
                    navigation_available: session.terminal_binding.is_some(),
                });
            }
            "PreCompress" => {
                session.integration_status = "capturingLimited".to_string();
            }
            _ => {}
        }
        trim_session(session);
        self.prune(now);
        self.bump();
    }

    pub(crate) fn active_session_count(&self) -> usize {
        self.sessions.values().filter(|session| session.is_active()).count()
    }

    pub(crate) fn snapshot(&self) -> GeminiSnapshot {
        let mut sessions: Vec<_> = self.sessions.values().cloned().collect();
        sessions.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        let interactions = sessions
            .iter()
            .flat_map(|session| session.pending_interactions.clone())
            .collect();
        GeminiSnapshot {
            connected: self.connected,
            integration_error: self.error.clone(),
            version: self.version,
            sessions,
            interactions,
            navigation_capability: if cfg!(windows) { "desktopFocus" } else { "unsupported" }.to_string(),
            fallback_action: "openGeminiOnHost".to_string(),
        }
    }

    fn bump(&mut self) {
        self.version = self.version.wrapping_add(1);
    }

    fn prune(&mut self, now: u64) {
        self.seen = self
            .seen
            .drain()
            .take(10_000)
            .collect();
        self.sessions.retain(|_, session| {
            session.is_active() || now.saturating_sub(session.updated_at) <= SESSION_TTL_MS
        });
        if self.sessions.len() > MAX_SESSIONS {
            let mut ids: Vec<_> = self.sessions.values().map(|session| (session.updated_at, session.id.clone())).collect();
            ids.sort_by_key(|item| item.0);
            for (_, id) in ids.into_iter().take(self.sessions.len() - MAX_SESSIONS) {
                self.sessions.remove(&id);
            }
        }
    }
}

fn trim_session(session: &mut GeminiSession) {
    if session.activities.len() > MAX_ACTIVITIES {
        let keep_from = session.activities.len() - MAX_ACTIVITIES;
        session.activities.drain(..keep_from);
    }
    if session.outputs.len() > MAX_OUTPUT_ENTRIES {
        let keep_from = session.outputs.len() - MAX_OUTPUT_ENTRIES;
        session.outputs.drain(..keep_from);
    }
    if session.pending_interactions.len() > MAX_INTERACTIONS {
        let keep_from = session.pending_interactions.len() - MAX_INTERACTIONS;
        session.pending_interactions.drain(..keep_from);
    }
}

fn push_output(outputs: &mut Vec<GeminiOutputEntry>, id: &str, text: &str, _now: u64) {
    outputs.push(GeminiOutputEntry { id: id.to_string(), text: cap_text(text, MAX_OUTPUT_CHARS) });
}

fn kind_label(kind: GeminiInteractionKind) -> &'static str {
    match kind {
        GeminiInteractionKind::AskUser => "ask_user",
        GeminiInteractionKind::ToolPermission => "permission",
        GeminiInteractionKind::FileChange => "file",
        GeminiInteractionKind::Shell => "shell",
        GeminiInteractionKind::Mcp => "mcp",
        GeminiInteractionKind::SandboxExpansion => "sandbox",
        GeminiInteractionKind::ExitPlanMode => "plan",
        GeminiInteractionKind::Notification => "notification",
    }
}

fn interaction_title(kind: GeminiInteractionKind, tool: &str) -> String {
    match kind {
        GeminiInteractionKind::AskUser => "Gemini 请求输入".to_string(),
        GeminiInteractionKind::ExitPlanMode => "Gemini 计划".to_string(),
        GeminiInteractionKind::FileChange => "Gemini 文件变更".to_string(),
        GeminiInteractionKind::Shell => "Gemini Shell 活动".to_string(),
        GeminiInteractionKind::Mcp => "Gemini MCP 活动".to_string(),
        GeminiInteractionKind::SandboxExpansion => "Gemini 沙箱权限请求".to_string(),
        GeminiInteractionKind::Notification => "Gemini 通知".to_string(),
        GeminiInteractionKind::ToolPermission => format!("Gemini 工具活动：{tool}"),
    }
}

fn classify_tool(tool: &str, payload: &Value) -> (GeminiInteractionKind, Vec<Value>, Option<String>, Option<String>) {
    let normalized = tool.to_ascii_lowercase();
    let input = payload.get("tool_input");
    let questions: Vec<Value> = input
        .and_then(|value| value.get("questions"))
        .and_then(Value::as_array)
        .map(|items| items.iter().take(8).cloned().collect())
        .unwrap_or_default();
    let plan_filename = input.and_then(|value| string_field(value, "plan_filename")).or_else(|| string_field(payload, "plan_filename"));
    let plan = input.and_then(|value| string_field(value, "plan")).or_else(|| string_field(payload, "plan"));
    if normalized == "ask_user" || normalized == "askuser" || !questions.is_empty() {
        return (GeminiInteractionKind::AskUser, questions, plan_filename, plan);
    }
    if normalized.contains("exit_plan") || normalized == "plan" {
        return (GeminiInteractionKind::ExitPlanMode, questions, plan_filename, plan);
    }
    if normalized.contains("sandbox") {
        return (GeminiInteractionKind::SandboxExpansion, questions, plan_filename, plan);
    }
    if normalized.contains("mcp") || payload.get("mcp_context").is_some() {
        return (GeminiInteractionKind::Mcp, questions, plan_filename, plan);
    }
    if normalized.contains("shell") || normalized.contains("bash") || normalized.contains("exec") {
        return (GeminiInteractionKind::Shell, questions, plan_filename, plan);
    }
    if normalized.contains("write") || normalized.contains("edit") || normalized.contains("replace") || normalized.contains("file") {
        return (GeminiInteractionKind::FileChange, questions, plan_filename, plan);
    }
    (GeminiInteractionKind::ToolPermission, questions, plan_filename, plan)
}

fn codecraft_data_dir() -> PathBuf {
    if let Some(local) = env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local).join("CodeCraft");
    }
    env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
        .map(|path| path.join(".codecraft"))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn path_key(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

fn is_within(path: &Path, root: &Path) -> bool {
    let path = path_key(path);
    let root = path_key(root);
    path == root || path.starts_with(&(root + "\\"))
}

fn canonical_plan_roots(cwd: &Path) -> Vec<PathBuf> {
    let configured = env::var_os("CODECRAFT_GEMINI_PLAN_DIR")
        .map(PathBuf::from)
        .or_else(|| env::var_os("GEMINI_PLANS_DIR").map(PathBuf::from))
        .unwrap_or_else(|| codecraft_data_dir().join("gemini-plans"));
    [cwd.to_path_buf(), configured]
        .into_iter()
        .filter_map(|root| fs::canonicalize(root).ok())
        .collect()
}

fn resolve_plan(
    cwd: Option<&str>,
    filename: Option<&str>,
    fallback: Option<&str>,
) -> (Option<String>, Option<String>) {
    let Some(filename) = filename.filter(|value| !value.trim().is_empty()) else {
        return (fallback.map(|value| cap_text(value, MAX_PLAN_CHARS)), None);
    };
    let Some(cwd) = cwd.filter(|value| !value.trim().is_empty()) else {
        return (None, Some("缺少会话 cwd，无法安全读取计划文件".to_string()));
    };
    let cwd_path = PathBuf::from(cwd);
    let Ok(canonical_cwd) = fs::canonicalize(&cwd_path) else {
        return (None, Some("会话 cwd 不存在或不可解析".to_string()));
    };
    let roots = canonical_plan_roots(&canonical_cwd);
    let candidate = Path::new(filename);
    let candidates: Vec<PathBuf> = if candidate.is_absolute() {
        vec![candidate.to_path_buf()]
    } else {
        roots.iter().map(|root| root.join(candidate)).collect()
    };
    let mut real_path = None;
    for candidate in candidates {
        if let Ok(path) = fs::canonicalize(candidate) {
            if roots.iter().any(|root| is_within(&path, root)) {
                real_path = Some(path);
                break;
            }
        }
    }
    let Some(real_path) = real_path else {
        return (
            None,
            Some(if candidate.is_absolute() {
                "计划文件路径超出允许目录或不可读取".to_string()
            } else {
                "计划文件不存在、超出允许目录或不可读取".to_string()
            }),
        );
    };
    let Ok(metadata) = fs::metadata(&real_path) else {
        return (None, Some("计划文件不可读取".to_string()));
    };
    if !metadata.is_file() {
        return (None, Some("计划路径不是文件".to_string()));
    }
    if metadata.len() > (MAX_PLAN_CHARS as u64) * 4 {
        return (None, Some("计划文件超过 512 KiB 限制".to_string()));
    }
    match fs::read_to_string(&real_path) {
        Ok(content) => (Some(cap_text(&content, MAX_PLAN_CHARS)), None),
        Err(_) => (None, Some("计划文件不是可读取的 UTF-8 文本".to_string())),
    }
}

fn tool_failed(response: Option<&Value>) -> bool {
    response.is_some_and(|response| {
        response.get("isError").and_then(Value::as_bool) == Some(true)
            || response.get("is_error").and_then(Value::as_bool) == Some(true)
            || response.get("success").and_then(Value::as_bool) == Some(false)
            || response.get("exitCode").and_then(Value::as_i64).is_some_and(|code| code != 0)
            || response.get("exit_code").and_then(Value::as_i64).is_some_and(|code| code != 0)
    })
}

fn terminal_binding_from_payload(payload: &Value, captured_at: u64) -> Option<GeminiTerminalBinding> {
    let pid = payload.get("pid").and_then(Value::as_u64).map(|value| value as u32);
    let parent_pid = payload.get("parent_pid").and_then(Value::as_u64).map(|value| value as u32);
    let console_window = string_field(payload, "console_window");
    if pid.is_none() && parent_pid.is_none() && console_window.is_none() {
        return None;
    }
    Some(GeminiTerminalBinding {
        pid,
        parent_pid,
        process_created_at: payload.get("process_created_at").and_then(Value::as_u64),
        console_window,
        captured_at,
    })
}

pub(crate) fn event_from_payload(payload: Value, captured_at: u64) -> Option<GeminiEvent> {
    event_name(&payload)?;
    let observation_id = string_field(&payload, "observation_id").unwrap_or_else(|| hash_observation(&payload));
    Some(GeminiEvent { payload, captured_at, observation_id })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    #[test]
    fn before_tool_is_read_only_and_classified() {
        let mut store = GeminiStore::default();
        store.apply(event_from_payload(json!({
            "hook_event_name": "BeforeTool",
            "session_id": "s1",
            "tool_name": "run_shell",
            "tool_call_id": "t1",
            "tool_input": {"command": "echo hello"}
        }), 1).unwrap());
        let snapshot = store.snapshot();
        assert_eq!(snapshot.interactions[0].kind, GeminiInteractionKind::Shell);
        assert!(!snapshot.interactions[0].navigation_available);
    }

    #[test]
    fn duplicate_observation_does_not_duplicate_activity() {
        let payload = json!({"hook_event_name":"SessionStart","session_id":"s1"});
        let event = event_from_payload(payload, 1).unwrap();
        let duplicate = event.clone();
        let mut store = GeminiStore::default();
        store.apply(event);
        store.apply(duplicate);
        assert_eq!(store.snapshot().sessions.len(), 1);
        assert_eq!(store.snapshot().version, 1);
    }

    #[test]
    fn after_tool_matches_the_same_call_id() {
        let mut store = GeminiStore::default();
        store.apply(event_from_payload(json!({
            "hook_event_name": "BeforeTool",
            "session_id": "s1",
            "tool_name": "run_shell",
            "tool_call_id": "first",
            "tool_input": {"command": "echo first"}
        }), 1).unwrap());
        store.apply(event_from_payload(json!({
            "hook_event_name": "BeforeTool",
            "session_id": "s1",
            "tool_name": "run_shell",
            "tool_call_id": "second",
            "tool_input": {"command": "echo second"}
        }), 2).unwrap());
        store.apply(event_from_payload(json!({
            "hook_event_name": "AfterTool",
            "session_id": "s1",
            "tool_name": "run_shell",
            "tool_call_id": "first",
            "tool_response": {"success": true}
        }), 3).unwrap());
        let interactions = &store.snapshot().sessions[0].pending_interactions;
        assert_eq!(interactions.iter().filter(|item| item.status == GeminiObservationStatus::ToolCompleted).count(), 1);
        assert_eq!(interactions.iter().filter(|item| item.status == GeminiObservationStatus::Observed).count(), 1);
    }

    #[test]
    fn plan_path_is_read_only_and_rejects_escape() {
        let root = env::temp_dir().join(format!("codecraft-gemini-plan-{}", now_ms()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("plan.md"), "# plan\ncontent").unwrap();
        let (plan, error) = resolve_plan(Some(root.to_string_lossy().as_ref()), Some("plan.md"), None);
        assert_eq!(plan.as_deref(), Some("# plan\ncontent"));
        assert!(error.is_none());
        let (plan, error) = resolve_plan(Some(root.to_string_lossy().as_ref()), Some("..\\outside.md"), None);
        assert!(plan.is_none());
        assert!(error.is_some());
        let _ = fs::remove_dir_all(root);
    }
}
