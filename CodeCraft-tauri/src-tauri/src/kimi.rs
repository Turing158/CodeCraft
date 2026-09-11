//! Read-only state for the Kimi Code Hook protocol.
//!
//! Kimi hooks are synchronous callbacks.  Version 0.41.0 does not expose an
//! asynchronous decision id that CodeCraft can safely answer, so this module
//! deliberately records observations only.  Identifiers emitted here are
//! integration identifiers and must never be sent back to Kimi as decisions.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    env, fs,
    path::{Path, PathBuf},
};

use serde::Serialize;
use serde_json::{Map, Value};

const MAX_OUTPUT_ENTRIES: usize = 24;
const MAX_OUTPUT_CHARS: usize = 8_000;
const MAX_ACTIVITIES: usize = 48;
const MAX_INTERACTIONS: usize = 32;
const MAX_SESSIONS: usize = 100;
const MAX_QUARANTINE: usize = 128;
const SESSION_TTL_MS: u64 = 24 * 60 * 60 * 1_000;
pub(crate) const MAX_PLAN_CHARS: usize = 512 * 1024;

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

fn hash_observation(value: &Value) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    let serialized = serde_json::to_string(value).unwrap_or_default();
    for byte in serialized.bytes() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
    }
    format!("kimi-{hash:016x}")
}

fn event_name(value: &Value) -> Option<&str> {
    value.get("hook_event_name").and_then(Value::as_str)
}

fn native_session_id(value: &Value) -> Option<String> {
    string_field(value, "session_id").filter(|value| !value.trim().is_empty())
}

fn process_instance_id(value: &Value) -> Option<String> {
    let mut identity = serde_json::Map::new();
    for key in ["pid", "process_created_at"] {
        if let Some(field) = value.get(key) {
            identity.insert(key.to_string(), field.clone());
        }
    }
    if identity.is_empty() {
        None
    } else {
        Some(hash_observation(&Value::Object(identity)))
    }
}

/// The native id may be reused after a Kimi restart.  Include the process
/// instance and cwd, and include the hook installation generation when it is
/// present, to keep old events out of a new session.
fn integration_session_key(value: &Value) -> String {
    let mut identity = serde_json::Map::new();
    for key in [
        "session_id",
        "cwd",
        "pid",
        "process_created_at",
        "hook_install_id",
    ] {
        if let Some(field) = value.get(key) {
            identity.insert(key.to_string(), field.clone());
        }
    }
    format!("kimi:{}", hash_observation(&Value::Object(identity)))
}

fn tool_name(value: &Value) -> String {
    string_field(value, "tool_name").unwrap_or_else(|| "unknown".to_string())
}

fn tool_call_id(value: &Value) -> Option<String> {
    string_field(value, "tool_call_id")
        .or_else(|| string_field(value, "tool_use_id"))
        .or_else(|| string_field(value, "call_id"))
}

fn observation_id(value: &Value, captured_at: u64) -> String {
    let mut key = serde_json::Map::new();
    key.insert("payload".to_string(), value.clone());
    key.insert("captured_at".to_string(), Value::from(captured_at));
    hash_observation(&Value::Object(key))
}

fn summary(value: &Value) -> String {
    let input = value.get("tool_input");
    let output = value.get("tool_output");
    let display = value.get("display");
    let text = string_field(value, "prompt")
        .or_else(|| string_field(value, "message"))
        .or_else(|| input.and_then(|input| string_field(input, "path")))
        .or_else(|| input.and_then(|input| string_field(input, "command")))
        .or_else(|| input.and_then(|input| string_field(input, "description")))
        .or_else(|| display.and_then(|display| string_field(display, "command")))
        .or_else(|| display.and_then(|display| string_field(display, "path")))
        .or_else(|| display.and_then(|display| string_field(display, "summary")))
        .or_else(|| output.and_then(Value::as_str).map(str::to_string))
        .or_else(|| string_field(value, "reason"))
        .unwrap_or_else(|| format!("{} 工具活动", tool_name(value)));
    cap_text(&text, 320)
}

fn prompt_text(value: &Value) -> Option<String> {
    match value.get("prompt") {
        Some(Value::String(text)) => Some(cap_text(text, MAX_OUTPUT_CHARS)),
        Some(Value::Array(parts)) => {
            let text = parts
                .iter()
                .filter_map(|part| {
                    if part.get("type").and_then(Value::as_str) == Some("text") {
                        part.get("text").and_then(Value::as_str)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            (!text.is_empty()).then(|| cap_text(&text, MAX_OUTPUT_CHARS))
        }
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum KimiSessionStatus {
    Working,
    WaitingForInput,
    ToolRunning,
    ToolCompleted,
    ToolFailed,
    Idle,
    Stopped,
}

impl KimiSessionStatus {
    pub(crate) fn is_active(self) -> bool {
        !matches!(self, Self::Idle | Self::Stopped)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum KimiInteractionKind {
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
pub(crate) enum KimiObservationStatus {
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
pub(crate) struct KimiActivity {
    pub id: String,
    pub tool: String,
    pub summary: String,
    pub status: String,
    pub started_at: u64,
    pub updated_at: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KimiOutputEntry {
    pub id: String,
    pub text: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KimiInteraction {
    pub observation_id: String,
    pub interaction_key: String,
    pub native_interaction_id: Option<String>,
    pub wire_session_id: Option<String>,
    pub agent_id: Option<String>,
    pub resolved: bool,
    pub kind: KimiInteractionKind,
    pub status: KimiObservationStatus,
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
pub(crate) struct KimiTerminalBinding {
    pub pid: Option<u32>,
    pub parent_pid: Option<u32>,
    pub process_created_at: Option<u64>,
    pub console_window: Option<String>,
    pub window_process_id: Option<u32>,
    pub window_process_created_at: Option<u64>,
    pub shared_terminal: bool,
    pub captured_at: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KimiSession {
    pub id: String,
    pub kimi_session_id: Option<String>,
    pub integration_session_key: String,
    pub hook_install_id: Option<String>,
    pub client_type: Option<String>,
    pub process_instance_id: Option<String>,
    pub kimi_version: Option<String>,
    pub title: String,
    pub cwd: Option<String>,
    pub status: KimiSessionStatus,
    pub started_at: u64,
    pub updated_at: u64,
    pub ended_at: Option<u64>,
    pub activities: Vec<KimiActivity>,
    pub outputs: Vec<KimiOutputEntry>,
    pub pending_interactions: Vec<KimiInteraction>,
    pub terminal_binding: Option<KimiTerminalBinding>,
    pub integration_status: String,
    // ToolFailed also describes recoverable tool errors inside an active turn.
    #[serde(skip)]
    turn_ended: bool,
}

impl KimiSession {
    pub(crate) fn is_active(&self) -> bool {
        !self.turn_ended && self.status.is_active()
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KimiSnapshot {
    pub connected: bool,
    pub integration_error: Option<String>,
    pub version: u64,
    pub sessions: Vec<KimiSession>,
    pub interactions: Vec<KimiInteraction>,
    pub quarantined_events: usize,
    pub dropped_events: usize,
    pub navigation_capability: String,
    pub fallback_action: String,
    pub capabilities: KimiCapabilities,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KimiCapabilities {
    pub can_observe: bool,
    pub can_approve_tools: bool,
    pub can_answer_questions: bool,
    pub can_approve_plans: bool,
    pub can_stream_output: bool,
    pub read_only: bool,
    pub reason: String,
}

#[derive(Clone, Debug)]
pub(crate) struct KimiEvent {
    pub payload: Value,
    pub captured_at: u64,
    pub observation_id: String,
}

#[derive(Default)]
pub(crate) struct KimiStore {
    connected: bool,
    error: Option<String>,
    version: u64,
    sessions: HashMap<String, KimiSession>,
    seen: HashSet<String>,
    quarantine: VecDeque<KimiEvent>,
    dropped_events: usize,
}

impl KimiStore {
    pub(crate) fn clear(&mut self) {
        self.sessions.clear();
        self.seen.clear();
        self.quarantine.clear();
        self.dropped_events = 0;
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

    pub(crate) fn apply(&mut self, event: KimiEvent) {
        if !self.seen.insert(event.observation_id.clone()) {
            return;
        }
        let payload = event.payload;
        let Some(native_id) = native_session_id(&payload) else {
            self.quarantine_event(KimiEvent { payload, ..event });
            return;
        };
        let name = event_name(&payload).unwrap_or("");
        let now = event.captured_at;
        let computed_id = integration_session_key(&payload);
        let id = self
            .sessions
            .get(&computed_id)
            .map(|_| computed_id.clone())
            .or_else(|| {
                let incoming_cwd = string_field(&payload, "cwd");
                let incoming_process = process_instance_id(&payload);
                let mut candidates = self
                    .sessions
                    .values()
                    .filter(|session| {
                        session.kimi_session_id.as_deref() == Some(native_id.as_str())
                    })
                    .filter(|session| {
                        incoming_cwd
                            .as_deref()
                            .is_none_or(|cwd| session.cwd.as_deref() == Some(cwd))
                    })
                    .filter(|session| {
                        incoming_process.as_ref().map_or(true, |process| {
                            session.process_instance_id.as_ref() == Some(process)
                        })
                    })
                    .filter(|session| {
                        session.hook_install_id == string_field(&payload, "hook_install_id")
                    })
                    .map(|session| session.id.clone());
                let first = candidates.next()?;
                candidates.next().is_none().then_some(first)
            })
            .unwrap_or(computed_id);

        // SessionEnd is authoritative.  Do not resurrect a stopped process
        // from a late tool event with the same native session id.
        if (name != "SessionStart" && !self.sessions.contains_key(&id))
            || self.sessions.get(&id).is_some_and(|session| {
                session.status == KimiSessionStatus::Stopped || now < session.updated_at
            })
        {
            self.quarantine_event(KimiEvent { payload, ..event });
            return;
        }

        let cwd = string_field(&payload, "cwd");
        let session = self
            .sessions
            .entry(id.clone())
            .or_insert_with(|| KimiSession {
                id: id.clone(),
                kimi_session_id: Some(native_id.clone()),
                integration_session_key: id.clone(),
                hook_install_id: string_field(&payload, "hook_install_id"),
                client_type: string_field(&payload, "client_type"),
                process_instance_id: process_instance_id(&payload),
                kimi_version: string_field(&payload, "kimi_version")
                    .or_else(|| string_field(&payload, "version")),
                title: "Kimi Code 会话".to_string(),
                cwd: cwd.clone(),
                status: KimiSessionStatus::Working,
                started_at: now,
                updated_at: now,
                ended_at: None,
                activities: Vec::new(),
                outputs: Vec::new(),
                pending_interactions: Vec::new(),
                terminal_binding: terminal_binding_from_payload(&payload, now),
                integration_status: "running".to_string(),
                turn_ended: false,
            });
        if session.cwd.is_none() {
            session.cwd = cwd;
        }
        if let Some(binding) = terminal_binding_from_payload(&payload, now) {
            if binding.console_window.is_some() || session.terminal_binding.is_none() {
                session.terminal_binding = Some(binding);
            }
        }
        session.updated_at = now;

        match name {
            "SessionStart" => {
                session.turn_ended = false;
                session.status = KimiSessionStatus::Working;
                session.ended_at = None;
                session.integration_status = "running".to_string();
                if let Some(title) = string_field(&payload, "session_title")
                    .or_else(|| string_field(&payload, "title"))
                {
                    session.title = cap_text(&title, 256);
                }
            }
            "SessionEnd" => {
                session.turn_ended = true;
                session.status = KimiSessionStatus::Stopped;
                session.ended_at = Some(now);
                session.integration_status = "sessionEnded".to_string();
                for interaction in &mut session.pending_interactions {
                    if !matches!(
                        interaction.status,
                        KimiObservationStatus::ToolCompleted | KimiObservationStatus::SessionEnded
                    ) {
                        interaction.status = KimiObservationStatus::SessionEnded;
                    }
                }
            }
            "UserPromptSubmit" | "UserPromptQueued" => {
                session.turn_ended = false;
                session.status = KimiSessionStatus::Working;
                if let Some(prompt) = prompt_text(&payload) {
                    push_output(
                        &mut session.outputs,
                        &format!("prompt-{now}"),
                        &format!("用户：{prompt}"),
                    );
                }
            }
            "TurnStarted" | "TaskStarted" | "SubagentStart" => {
                if session.is_active()
                    && !session.pending_interactions.iter().any(|interaction| {
                        matches!(
                            interaction.status,
                            KimiObservationStatus::Observed | KimiObservationStatus::Updated
                        )
                    })
                {
                    session.status = KimiSessionStatus::Working;
                }
            }
            "Stop" | "StopFailure" | "Interrupt" => {
                session.turn_ended = true;
                session.status = if name == "StopFailure" {
                    KimiSessionStatus::ToolFailed
                } else {
                    KimiSessionStatus::Idle
                };
                for interaction in &mut session.pending_interactions {
                    if interaction.native_interaction_id.is_none()
                        && matches!(
                            interaction.status,
                            KimiObservationStatus::Observed | KimiObservationStatus::Updated
                        )
                    {
                        interaction.status = KimiObservationStatus::Stale;
                    }
                }
                if let Some(text) = string_field(&payload, "response")
                    .or_else(|| string_field(&payload, "prompt_response"))
                    .or_else(|| string_field(&payload, "agent_response"))
                    .or_else(|| string_field(&payload, "last_assistant_message"))
                {
                    push_output(&mut session.outputs, &format!("response-{now}"), &text);
                }
            }
            "PreToolUse" | "PermissionRequest" => {
                observe_tool(session, &payload, &event.observation_id, now, id.as_str());
            }
            "PostToolUse" | "PostToolUseFailure" => {
                let status = session.status;
                let was_active = session.is_active();
                if tool_call_id(&payload).is_some()
                    && !session
                        .pending_interactions
                        .iter()
                        .any(|interaction| interaction.tool_call_id == tool_call_id(&payload))
                {
                    observe_tool(session, &payload, &event.observation_id, now, id.as_str());
                }
                complete_tool(session, &payload, now, name == "PostToolUseFailure");
                if !was_active {
                    session.status = status;
                }
            }
            "PermissionResult" => {
                session.integration_status = "permissionObserved".to_string();
                if let Some(call_id) = tool_call_id(&payload) {
                    let status = session.status;
                    if !session.pending_interactions.iter().any(|interaction| {
                        interaction.tool_call_id.as_deref() == Some(call_id.as_str())
                    }) {
                        observe_tool(session, &payload, &event.observation_id, now, id.as_str());
                    }
                    for interaction in &mut session.pending_interactions {
                        if interaction.tool_call_id.as_deref() == Some(call_id.as_str())
                            && matches!(
                                interaction.status,
                                KimiObservationStatus::Observed | KimiObservationStatus::Updated
                            )
                        {
                            if interaction.native_interaction_id.is_none() {
                                interaction.status = KimiObservationStatus::Stale;
                            }
                        }
                    }
                    session.status = status;
                }
            }
            "Notification" => {
                session.pending_interactions.push(KimiInteraction {
                    observation_id: event.observation_id,
                    interaction_key: format!("{id}:notification:{now}"),
                    native_interaction_id: None,
                    wire_session_id: None,
                    agent_id: None,
                    resolved: false,
                    kind: KimiInteractionKind::Notification,
                    status: KimiObservationStatus::Observed,
                    title: string_field(&payload, "title")
                        .unwrap_or_else(|| "Kimi 通知".to_string()),
                    detail: summary(&payload),
                    tool_name: None,
                    tool_call_id: None,
                    tool_input: None,
                    questions: Vec::new(),
                    plan_filename: None,
                    plan: None,
                    plan_read_error: None,
                    captured_at: now,
                    truncated: false,
                    navigation_available: false,
                });
            }
            "SessionHeartbeat" => session.integration_status = "running".to_string(),
            "PreCompact" | "PostCompact" => {
                session.integration_status = "capturingLimited".to_string()
            }
            _ => {}
        }
        trim_session(session);
        self.prune(now);
        self.bump();
    }

    pub(crate) fn active_session_count(&self) -> usize {
        self.sessions
            .values()
            .filter(|session| session.is_active())
            .count()
    }

    /// Merge interactive requests observed in Kimi's read-only wire stream.
    /// Hook events remain authoritative for tool activity; wire records only
    /// create or settle the review shown in CodeCraft.
    pub(crate) fn sync_wire_interactions(
        &mut self,
        wire_interactions: &[crate::kimi_wire::KimiWireInteraction],
    ) {
        let mut changed = false;
        let mut active_ids = HashSet::new();
        for wire in wire_interactions {
            let Some(session_id) = self.find_wire_session(wire) else {
                continue;
            };
            let Some(session) = self.sessions.get_mut(&session_id) else {
                continue;
            };
            // SessionEnd and a failed turn are terminal for the integration.
            // Do not let a still-open wire record resurrect that session.
            if session.status == KimiSessionStatus::Stopped
                || (session.status == KimiSessionStatus::ToolFailed && session.turn_ended)
            {
                continue;
            }
            let mut payload = Map::new();
            let request = &wire.request;
            let tool = request
                .get("toolName")
                .or_else(|| request.get("tool_name"))
                .and_then(Value::as_str)
                .unwrap_or(if wire.kind == "question" {
                    "AskUserQuestion"
                } else {
                    "Kimi approval"
                })
                .to_string();
            payload.insert("tool_name".to_string(), Value::String(tool.clone()));
            if let Some(input) = request
                .get("toolInput")
                .or_else(|| request.get("tool_input"))
                .or_else(|| request.get("args"))
                .or_else(|| request.get("arguments"))
            {
                payload.insert("tool_input".to_string(), input.clone());
            }
            if let Some(questions) = request.get("questions") {
                payload.insert("tool_input".to_string(), {
                    let mut input = Map::new();
                    input.insert("questions".to_string(), questions.clone());
                    Value::Object(input)
                });
            }
            if let Some(display) = request.get("display") {
                payload.insert("display".to_string(), display.clone());
            }
            for (target, keys) in [
                ("plan", ["plan", "planText"].as_slice()),
                ("plan_filename", ["planFilename", "plan_filename"].as_slice()),
            ] {
                if let Some(value) = keys.iter().find_map(|key| request.get(*key)) {
                    payload.insert(target.to_string(), value.clone());
                }
            }
            let payload = Value::Object(payload);
            let (kind, questions, plan_filename, plan_hint) = classify_tool(&tool, &payload);
            let detail = wire_detail(wire, &tool, &questions);
            let plan = if kind == KimiInteractionKind::ExitPlanMode {
                plan_hint.map(|value| cap_text(&value, MAX_PLAN_CHARS))
            } else {
                None
            };
            let interaction_key = format!(
                "{}:{}:{}",
                session.integration_session_key,
                wire.tool_call_id
                    .as_deref()
                    .unwrap_or(wire.native_interaction_id.as_str()),
                kind_label(kind)
            );
            active_ids.insert((session.id.clone(), wire.native_interaction_id.clone()));
            let existing = session.pending_interactions.iter().position(|item| {
                item.native_interaction_id.as_deref() == Some(wire.native_interaction_id.as_str())
                    || (wire.tool_call_id.is_some()
                        && item.tool_call_id == wire.tool_call_id
                        && item.kind == kind)
                    || item.interaction_key == interaction_key
            });
            let existing_resolved = existing
                .and_then(|index| session.pending_interactions.get(index))
                .is_some_and(|item| item.resolved);
            if existing_resolved {
                continue;
            }
            let interaction = KimiInteraction {
                observation_id: wire.native_interaction_id.clone(),
                interaction_key,
                native_interaction_id: Some(wire.native_interaction_id.clone()),
                wire_session_id: Some(wire.native_session_id.clone()),
                agent_id: wire.agent_id.clone(),
                resolved: false,
                kind,
                status: KimiObservationStatus::Observed,
                title: interaction_title(kind, &tool),
                detail,
                tool_name: Some(tool),
                tool_call_id: wire.tool_call_id.clone(),
                tool_input: request
                    .get("toolInput")
                    .or_else(|| request.get("tool_input"))
                    .or_else(|| request.get("args"))
                    .or_else(|| request.get("arguments"))
                    .cloned(),
                questions,
                plan_filename,
                plan,
                plan_read_error: None,
                captured_at: wire.captured_at,
                truncated: false,
                navigation_available: session
                    .terminal_binding
                    .as_ref()
                    .is_some_and(|binding| binding.console_window.is_some()),
            };
            if let Some(index) = existing {
                let current = &session.pending_interactions[index];
                let differs = current.detail != interaction.detail
                    || current.questions != interaction.questions
                    || current.plan != interaction.plan
                    || current.plan_filename != interaction.plan_filename
                    || current.tool_input != interaction.tool_input
                    || current.captured_at != interaction.captured_at
                    || current.status != interaction.status
                    || current.resolved;
                if differs {
                    session.pending_interactions[index] = interaction;
                    changed = true;
                }
            } else {
                session.pending_interactions.push(interaction);
                changed = true;
            }
            let expected_status = if kind == KimiInteractionKind::AskUser {
                KimiSessionStatus::WaitingForInput
            } else {
                KimiSessionStatus::ToolRunning
            };
            if session.turn_ended || session.status != expected_status {
                changed = true;
            }
            session.turn_ended = false;
            session.status = expected_status;
        }

        for session in self.sessions.values_mut() {
            for interaction in &mut session.pending_interactions {
                let Some(native_id) = interaction.native_interaction_id.as_ref() else {
                    continue;
                };
                if !active_ids.contains(&(session.id.clone(), native_id.clone()))
                    && !interaction.resolved
                {
                    interaction.resolved = true;
                    interaction.status = KimiObservationStatus::ToolCompleted;
                    changed = true;
                }
            }
            trim_session(session);
        }
        self.prune(wire_interactions.iter().map(|item| item.captured_at).max().unwrap_or(0));
        if changed {
            self.bump();
        }
    }

    fn find_wire_session(&self, wire: &crate::kimi_wire::KimiWireInteraction) -> Option<String> {
        let mut candidates: Vec<_> = self
            .sessions
            .values()
            .filter(|session| session.kimi_session_id.as_deref() == Some(wire.native_session_id.as_str()))
            .collect();
        if candidates.len() > 1 {
            let cwd = wire
                .request
                .get("display")
                .and_then(|display| display.get("cwd"))
                .and_then(Value::as_str);
            if let Some(cwd) = cwd {
                let matching: Vec<_> = candidates
                    .iter()
                    .copied()
                    .filter(|session| session.cwd.as_deref() == Some(cwd))
                    .collect();
                if matching.len() == 1 {
                    return Some(matching[0].id.clone());
                }
            }
            candidates.sort_by_key(|session| std::cmp::Reverse(session.updated_at));
        }
        candidates.first().map(|session| session.id.clone())
    }

    pub(crate) fn snapshot(&self) -> KimiSnapshot {
        let mut sessions: Vec<_> = self.sessions.values().cloned().collect();
        sessions.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        let interactions = sessions
            .iter()
            .flat_map(|session| session.pending_interactions.clone())
            .collect();
        KimiSnapshot {
            connected: self.connected,
            integration_error: self.error.clone(),
            version: self.version,
            sessions,
            interactions,
            quarantined_events: self.quarantine.len(),
            dropped_events: self.dropped_events,
            navigation_capability: if self.sessions.values().any(|session| {
                session
                    .terminal_binding
                    .as_ref()
                    .is_some_and(|binding| binding.console_window.is_some())
            }) {
                "bestEffort"
            } else {
                "unsupported"
            }
            .to_string(),
            fallback_action: "openKimiOnHost".to_string(),
            capabilities: KimiCapabilities {
                can_observe: true,
                can_approve_tools: false,
                can_answer_questions: false,
                can_approve_plans: false,
                can_stream_output: false,
                read_only: true,
                reason: "Kimi Code 0.41.0 Hook 与 wire.jsonl 仅提供只读观察，CodeCraft 不会写回审批决定"
                    .to_string(),
            },
        }
    }

    fn quarantine_event(&mut self, event: KimiEvent) {
        if self.seen.len() > 10_000 {
            self.seen.clear();
        }
        if self.quarantine.len() >= MAX_QUARANTINE {
            self.quarantine.pop_front();
            self.dropped_events += 1;
        }
        self.quarantine.push_back(event);
        self.bump();
    }

    fn bump(&mut self) {
        self.version = self.version.wrapping_add(1);
    }

    fn prune(&mut self, now: u64) {
        if self.seen.len() > 10_000 {
            let keep = self
                .seen
                .iter()
                .skip(self.seen.len() - 5_000)
                .cloned()
                .collect();
            self.seen = keep;
        }
        self.sessions.retain(|_, session| {
            session.is_active() || now.saturating_sub(session.updated_at) <= SESSION_TTL_MS
        });
        if self.sessions.len() > MAX_SESSIONS {
            let mut ids: Vec<_> = self
                .sessions
                .values()
                .map(|session| (session.updated_at, session.id.clone()))
                .collect();
            ids.sort_by_key(|item| item.0);
            for (_, id) in ids.into_iter().take(self.sessions.len() - MAX_SESSIONS) {
                self.sessions.remove(&id);
            }
        }
    }
}

fn observe_tool(
    session: &mut KimiSession,
    payload: &Value,
    observation_id: &str,
    now: u64,
    session_key: &str,
) {
    let call_id = tool_call_id(payload);
    let previous = session
        .pending_interactions
        .iter()
        .position(|interaction| call_id.is_some() && interaction.tool_call_id == call_id);
    // Observation-only hooks can arrive after their result or the turn's Stop.
    // A late notification may enrich the review, but cannot reopen it.
    let settled_status = previous
        .map(|index| session.pending_interactions[index].status)
        .filter(|status| {
            !matches!(
                status,
                KimiObservationStatus::Observed | KimiObservationStatus::Updated
            )
        })
        .or_else(|| (!session.is_active()).then_some(KimiObservationStatus::Stale));
    // PermissionRequest may omit arguments already captured by PreToolUse.
    let mut merged = payload.clone();
    if let (Some(index), Some(fields)) = (previous, merged.as_object_mut()) {
        let previous = &session.pending_interactions[index];
        for (key, value) in [
            ("tool_name", previous.tool_name.clone().map(Value::String)),
            ("tool_input", previous.tool_input.clone()),
            (
                "plan_filename",
                previous.plan_filename.clone().map(Value::String),
            ),
            ("plan", previous.plan.clone().map(Value::String)),
        ] {
            if fields.get(key).is_none_or(Value::is_null) {
                if let Some(value) = value {
                    fields.insert(key.to_string(), value);
                }
            }
        }
    }
    let payload = &merged;
    let tool = tool_name(payload);
    let activity_id = call_id.clone().unwrap_or_else(|| hash_observation(payload));
    let detail = summary(payload);
    let (kind, questions, plan_filename, plan_hint) = classify_tool(&tool, payload);
    let (plan, plan_read_error) = if kind == KimiInteractionKind::ExitPlanMode {
        resolve_plan(
            session.cwd.as_deref(),
            plan_filename.as_deref(),
            plan_hint.as_deref(),
        )
    } else {
        (
            plan_hint.map(|value| cap_text(&value, MAX_PLAN_CHARS)),
            None,
        )
    };
    if settled_status.is_none() {
        session.status = if kind == KimiInteractionKind::AskUser {
            KimiSessionStatus::WaitingForInput
        } else {
            KimiSessionStatus::ToolRunning
        };
    }
    if !session
        .activities
        .iter()
        .any(|activity| activity.id == activity_id)
    {
        session.activities.push(KimiActivity {
            id: activity_id.clone(),
            tool: tool.clone(),
            summary: detail.clone(),
            status: "running".to_string(),
            started_at: now,
            updated_at: now,
        });
    }
    // PreToolUse and PermissionRequest describe the same in-flight call.
    // Keep one review so a second notification cannot leave a stale prompt.
    let interaction = KimiInteraction {
        observation_id: observation_id.to_string(),
        interaction_key: format!("{session_key}:{activity_id}:{}", kind_label(kind)),
        native_interaction_id: None,
        wire_session_id: None,
        agent_id: None,
        resolved: false,
        kind,
        status: settled_status.unwrap_or(KimiObservationStatus::Observed),
        title: interaction_title(kind, &tool),
        detail,
        tool_name: Some(tool),
        tool_call_id: call_id,
        tool_input: payload.get("tool_input").cloned(),
        questions,
        plan_filename,
        plan,
        plan_read_error,
        captured_at: now,
        truncated: payload
            .get("truncated")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        navigation_available: session
            .terminal_binding
            .as_ref()
            .is_some_and(|binding| binding.console_window.is_some()),
    };
    if let Some(index) = previous {
        session.pending_interactions[index] = interaction;
    } else {
        session.pending_interactions.push(interaction);
    }
}

fn complete_tool(session: &mut KimiSession, payload: &Value, now: u64, failed: bool) {
    let call_id = tool_call_id(payload);
    let matched = call_id.as_deref().and_then(|id| {
        session
            .activities
            .iter_mut()
            .rev()
            .find(|activity| activity.id == id)
    });
    if let Some(activity) = matched {
        activity.status = if failed { "failed" } else { "completed" }.to_string();
        activity.updated_at = now;
        activity.summary = summary(payload);
    }
    for interaction in session
        .pending_interactions
        .iter_mut()
        .filter(|item| call_id.is_some() && item.tool_call_id == call_id)
    {
        interaction.status = KimiObservationStatus::ToolCompleted;
        if interaction.native_interaction_id.is_some() {
            interaction.resolved = true;
        }
        interaction.detail = summary(payload);
    }
    if let Some(output) = payload.get("tool_output") {
        let text = output
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| output.to_string());
        push_output(&mut session.outputs, &format!("tool-{now}"), &text);
    }
    session.status = if failed {
        KimiSessionStatus::ToolFailed
    } else {
        KimiSessionStatus::ToolCompleted
    };
}

fn trim_session(session: &mut KimiSession) {
    if session.activities.len() > MAX_ACTIVITIES {
        let keep = session.activities.len() - MAX_ACTIVITIES;
        session.activities.drain(..keep);
    }
    if session.outputs.len() > MAX_OUTPUT_ENTRIES {
        let keep = session.outputs.len() - MAX_OUTPUT_ENTRIES;
        session.outputs.drain(..keep);
    }
    if session.pending_interactions.len() > MAX_INTERACTIONS {
        let keep = session.pending_interactions.len() - MAX_INTERACTIONS;
        session.pending_interactions.drain(..keep);
    }
}

fn push_output(outputs: &mut Vec<KimiOutputEntry>, id: &str, text: &str) {
    outputs.push(KimiOutputEntry {
        id: id.to_string(),
        text: cap_text(text, MAX_OUTPUT_CHARS),
    });
}

fn kind_label(kind: KimiInteractionKind) -> &'static str {
    match kind {
        KimiInteractionKind::AskUser => "ask_user",
        KimiInteractionKind::ToolPermission => "permission",
        KimiInteractionKind::FileChange => "file",
        KimiInteractionKind::Shell => "shell",
        KimiInteractionKind::Mcp => "mcp",
        KimiInteractionKind::SandboxExpansion => "sandbox",
        KimiInteractionKind::ExitPlanMode => "plan",
        KimiInteractionKind::Notification => "notification",
    }
}

fn interaction_title(kind: KimiInteractionKind, tool: &str) -> String {
    match kind {
        KimiInteractionKind::AskUser => "Kimi 请求输入".to_string(),
        KimiInteractionKind::ExitPlanMode => "Kimi 计划观察".to_string(),
        KimiInteractionKind::FileChange => "Kimi 文件变更观察".to_string(),
        KimiInteractionKind::Shell => "Kimi Shell 活动".to_string(),
        KimiInteractionKind::Mcp => "Kimi MCP 活动".to_string(),
        KimiInteractionKind::SandboxExpansion => "Kimi 沙箱活动".to_string(),
        KimiInteractionKind::Notification => "Kimi 通知".to_string(),
        KimiInteractionKind::ToolPermission => format!("Kimi 工具活动：{tool}"),
    }
}

fn wire_detail(
    wire: &crate::kimi_wire::KimiWireInteraction,
    tool: &str,
    questions: &[Value],
) -> String {
    if wire.kind == "question" {
        if let Some(question) = questions.first() {
            if let Some(text) = question.as_str() {
                return cap_text(text, 320);
            }
            if let Some(text) = question.get("question").and_then(Value::as_str) {
                return cap_text(text, 320);
            }
        }
        return "Kimi 正在等待你的回答".to_string();
    }
    let display = wire.request.get("display");
    let detail = wire
        .request
        .get("action")
        .and_then(Value::as_str)
        .or_else(|| display.and_then(|value| value.get("command")).and_then(Value::as_str))
        .or_else(|| display.and_then(|value| value.get("path")).and_then(Value::as_str))
        .or_else(|| display.and_then(|value| value.get("summary")).and_then(Value::as_str))
        .or_else(|| display.and_then(|value| value.get("description")).and_then(Value::as_str))
        .unwrap_or("Kimi 请求确认此操作");
    cap_text(&format!("{tool}: {detail}"), 320)
}

fn classify_tool(
    tool: &str,
    payload: &Value,
) -> (
    KimiInteractionKind,
    Vec<Value>,
    Option<String>,
    Option<String>,
) {
    let normalized = tool.to_ascii_lowercase().replace(['_', '-'], "");
    let input = payload.get("tool_input");
    let questions: Vec<Value> = input
        .and_then(|value| value.get("questions"))
        .and_then(Value::as_array)
        .map(|items| items.iter().take(8).cloned().collect())
        .unwrap_or_default();
    let plan_display = payload
        .get("display")
        .filter(|display| display.get("kind").and_then(Value::as_str) == Some("plan_review"));
    let plan_filename = plan_display
        .and_then(|display| string_field(display, "path"))
        .or_else(|| input.and_then(|value| string_field(value, "plan_filename")))
        .or_else(|| string_field(payload, "plan_filename"));
    let plan = plan_display
        .and_then(|display| string_field(display, "plan"))
        .or_else(|| input.and_then(|value| string_field(value, "plan")))
        .or_else(|| string_field(payload, "plan"));
    if normalized == "askuser" || normalized == "askuserquestion" || !questions.is_empty() {
        return (KimiInteractionKind::AskUser, questions, plan_filename, plan);
    }
    if normalized.contains("exitplan") || normalized == "plan" || plan_display.is_some() {
        return (
            KimiInteractionKind::ExitPlanMode,
            questions,
            plan_filename,
            plan,
        );
    }
    if normalized.contains("sandbox") {
        return (
            KimiInteractionKind::SandboxExpansion,
            questions,
            plan_filename,
            plan,
        );
    }
    if normalized.contains("mcp") || payload.get("mcp_context").is_some() {
        return (KimiInteractionKind::Mcp, questions, plan_filename, plan);
    }
    if normalized.contains("shell") || normalized.contains("bash") || normalized.contains("exec") {
        return (KimiInteractionKind::Shell, questions, plan_filename, plan);
    }
    if normalized.contains("write")
        || normalized.contains("edit")
        || normalized.contains("replace")
        || normalized.contains("file")
    {
        return (
            KimiInteractionKind::FileChange,
            questions,
            plan_filename,
            plan,
        );
    }
    (
        KimiInteractionKind::ToolPermission,
        questions,
        plan_filename,
        plan,
    )
}

fn codecraft_data_dir() -> PathBuf {
    env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|path| path.join("CodeCraft"))
        .or_else(|| {
            env::var_os("USERPROFILE")
                .or_else(|| env::var_os("HOME"))
                .map(PathBuf::from)
                .map(|path| path.join(".codecraft"))
        })
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
    let configured = env::var_os("CODECRAFT_KIMI_PLAN_DIR")
        .map(PathBuf::from)
        .or_else(|| env::var_os("KIMI_PLANS_DIR").map(PathBuf::from))
        .unwrap_or_else(|| codecraft_data_dir().join("kimi-plans"));
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
    // The Hook carries the revision shown in Kimi, even when its plan file
    // lives outside the workspace or has changed since the request.
    if let Some(plan) = fallback.filter(|value| !value.trim().is_empty()) {
        return (
            Some(crate::kimi_hook::redact_text(&cap_text(
                plan,
                MAX_PLAN_CHARS,
            ))),
            None,
        );
    }
    let Some(filename) = filename.filter(|value| !value.trim().is_empty()) else {
        return (None, None);
    };
    let Some(cwd) = cwd.filter(|value| !value.trim().is_empty()) else {
        return (None, Some("缺少会话 cwd，无法安全读取计划文件".to_string()));
    };
    let Ok(canonical_cwd) = fs::canonicalize(cwd) else {
        return (None, Some("会话 cwd 不存在或不可解析".to_string()));
    };
    let roots = canonical_plan_roots(&canonical_cwd);
    let candidate = Path::new(filename);
    let candidates: Vec<PathBuf> = if candidate.is_absolute() {
        vec![candidate.to_path_buf()]
    } else {
        roots.iter().map(|root| root.join(candidate)).collect()
    };
    let real_path = candidates
        .into_iter()
        .filter_map(|candidate| fs::canonicalize(candidate).ok())
        .find(|path| roots.iter().any(|root| is_within(path, root)));
    let Some(real_path) = real_path else {
        return (
            None,
            Some("计划文件不存在、超出允许目录或不可读取".to_string()),
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
    match fs::read_to_string(real_path) {
        Ok(content) => (
            Some(crate::kimi_hook::redact_text(&cap_text(
                &content,
                MAX_PLAN_CHARS,
            ))),
            None,
        ),
        Err(_) => (None, Some("计划文件不是可读取的 UTF-8 文本".to_string())),
    }
}

fn terminal_binding_from_payload(payload: &Value, captured_at: u64) -> Option<KimiTerminalBinding> {
    let pid = payload
        .get("pid")
        .and_then(Value::as_u64)
        .map(|value| value as u32);
    let parent_pid = payload
        .get("parent_pid")
        .and_then(Value::as_u64)
        .map(|value| value as u32);
    let console_window = string_field(payload, "console_window").or_else(|| {
        payload
            .get("console_window")
            .and_then(Value::as_i64)
            .map(|value| value.to_string())
    });
    if pid.is_none() && parent_pid.is_none() && console_window.is_none() {
        return None;
    }
    Some(KimiTerminalBinding {
        pid,
        parent_pid,
        process_created_at: payload.get("process_created_at").and_then(Value::as_u64),
        console_window,
        window_process_id: payload
            .get("window_process_id")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok()),
        window_process_created_at: payload
            .get("window_process_created_at")
            .and_then(Value::as_u64),
        shared_terminal: payload
            .get("shared_terminal")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        captured_at,
    })
}

pub(crate) fn event_from_payload(payload: Value, captured_at: u64) -> Option<KimiEvent> {
    let name = event_name(&payload)?;
    if !crate::kimi_hook::HOOK_EVENTS.contains(&name) {
        return None;
    }
    Some(KimiEvent {
        observation_id: observation_id(&payload, captured_at),
        payload,
        captured_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(payload: Value, at: u64) -> KimiEvent {
        event_from_payload(payload, at).unwrap()
    }

    #[test]
    fn real_fixture_prompt_array_and_tool_output_are_observed() {
        let mut store = KimiStore::default();
        store.apply(event(json!({"hook_event_name":"SessionStart","session_id":"s1","cwd":"C:\\work","client_type":"kimi_code_cli"}), 1));
        store.apply(event(json!({"hook_event_name":"UserPromptSubmit","session_id":"s1","cwd":"C:\\work","prompt":[{"type":"text","text":"Read once"}]}), 2));
        store.apply(event(json!({"hook_event_name":"PreToolUse","session_id":"s1","cwd":"C:\\work","tool_name":"Read","tool_call_id":"t1","tool_input":{"path":"C:\\work\\a.txt"}}), 3));
        store.apply(event(json!({"hook_event_name":"PostToolUse","session_id":"s1","cwd":"C:\\work","tool_name":"Read","tool_call_id":"t1","tool_output":"contents"}), 4));
        let snapshot = store.snapshot();
        assert_eq!(snapshot.sessions.len(), 1);
        assert_eq!(snapshot.sessions[0].outputs[0].text, "用户：Read once");
        assert_eq!(
            snapshot.sessions[0].pending_interactions[0].status,
            KimiObservationStatus::ToolCompleted
        );
        assert!(snapshot.capabilities.read_only);
    }

    #[test]
    fn duplicate_payload_with_same_capture_time_is_ignored() {
        let payload = json!({"hook_event_name":"SessionStart","session_id":"s1"});
        let mut store = KimiStore::default();
        store.apply(event(payload.clone(), 1));
        store.apply(event(payload, 1));
        assert_eq!(store.snapshot().version, 1);
    }

    #[test]
    fn replays_normalized_production_capture() {
        let captures: Vec<Value> = serde_json::from_str(include_str!(
            "../../protocol/kimi/0.41.0/read-tool-stop.json"
        ))
        .unwrap();
        let mut store = KimiStore::default();
        for capture in captures {
            store.apply(event(
                capture["payload"].clone(),
                capture["captured_at"].as_u64().unwrap(),
            ));
        }
        let snapshot = store.snapshot();
        assert_eq!(snapshot.sessions.len(), 1);
        assert_eq!(snapshot.quarantined_events, 0);
        assert_eq!(snapshot.sessions[0].activities[0].status, "completed");
        assert!(snapshot.sessions[0]
            .outputs
            .iter()
            .any(|output| output.text.contains("codecraft-kimi-fixture")));
        assert_eq!(snapshot.sessions[0].status, KimiSessionStatus::Idle);
        assert!(snapshot.sessions[0].outputs[0]
            .text
            .contains("Read fixture.txt"));
        assert!(!snapshot.capabilities.can_approve_tools);
    }

    #[test]
    fn replays_production_plan_permission_capture_without_reopening_the_review() {
        let captures: Vec<Value> = serde_json::from_str(include_str!(
            "../../protocol/kimi/0.41.0/plan-permission-stop.json"
        ))
        .unwrap();
        let mut store = KimiStore::default();
        for capture in captures {
            let name = capture["payload"]["hook_event_name"].as_str().unwrap();
            store.apply(event(
                capture["payload"].clone(),
                capture["captured_at"].as_u64().unwrap(),
            ));
            if name == "PermissionRequest" {
                let snapshot = store.snapshot();
                let review = snapshot
                    .interactions
                    .iter()
                    .find(|interaction| interaction.kind == KimiInteractionKind::ExitPlanMode)
                    .unwrap();
                assert_eq!(review.status, KimiObservationStatus::Stale);
                assert_eq!(
                    review.plan.as_deref(),
                    Some("# Fixture plan\n\n1. Read fixture.txt.\n2. Report the result.\n")
                );
                assert!(review.plan_read_error.is_none());
            }
        }
        let snapshot = store.snapshot();
        assert_eq!(snapshot.sessions.len(), 1);
        assert_eq!(snapshot.quarantined_events, 0);
        assert_eq!(snapshot.sessions[0].status, KimiSessionStatus::Idle);
        assert_eq!(snapshot.sessions[0].activities.len(), 3);
        assert!(snapshot
            .interactions
            .iter()
            .all(|interaction| interaction.status == KimiObservationStatus::ToolCompleted));
        assert!(!snapshot.capabilities.can_approve_plans);
    }

    #[test]
    fn replays_production_question_capture_until_native_failure_closes_it() {
        let captures: Vec<Value> = serde_json::from_str(include_str!(
            "../../protocol/kimi/0.41.0/question-failure-stop.json"
        ))
        .unwrap();
        let mut store = KimiStore::default();
        for capture in captures {
            let name = capture["payload"]["hook_event_name"].as_str().unwrap();
            store.apply(event(
                capture["payload"].clone(),
                capture["captured_at"].as_u64().unwrap(),
            ));
            if matches!(name, "PreToolUse" | "TurnStarted") {
                let snapshot = store.snapshot();
                assert_eq!(
                    snapshot.sessions[0].status,
                    KimiSessionStatus::WaitingForInput
                );
                assert_eq!(snapshot.interactions[0].kind, KimiInteractionKind::AskUser);
                assert_eq!(
                    snapshot.interactions[0].questions[0]["question"],
                    "Which output format?"
                );
                assert_eq!(
                    snapshot.interactions[0].questions[0]["options"][0]["label"],
                    "JSON"
                );
            }
        }
        let snapshot = store.snapshot();
        assert_eq!(snapshot.sessions.len(), 1);
        assert_eq!(snapshot.quarantined_events, 0);
        assert_eq!(snapshot.sessions[0].status, KimiSessionStatus::Idle);
        assert_eq!(snapshot.sessions[0].activities[0].status, "failed");
        assert_eq!(
            snapshot.interactions[0].status,
            KimiObservationStatus::ToolCompleted
        );
        assert!(!snapshot.capabilities.can_answer_questions);
    }

    #[test]
    fn a_stopped_session_quarantines_late_events() {
        let mut store = KimiStore::default();
        store.apply(event(
            json!({"hook_event_name":"SessionStart","session_id":"s1"}),
            1,
        ));
        store.apply(event(
            json!({"hook_event_name":"SessionEnd","session_id":"s1"}),
            2,
        ));
        store.apply(event(json!({"hook_event_name":"PostToolUse","session_id":"s1","tool_name":"Read","tool_call_id":"late"}), 3));
        assert_eq!(store.snapshot().quarantined_events, 1);
        assert_eq!(store.snapshot().sessions.len(), 1);
    }

    #[test]
    fn concurrent_processes_with_same_native_id_remain_separate() {
        let mut store = KimiStore::default();
        for pid in [100, 200] {
            store.apply(event(json!({"hook_event_name":"SessionStart", "session_id":"same", "cwd":"C:/work", "pid":pid, "process_created_at":pid}), 1));
        }
        store.apply(event(json!({"hook_event_name":"PreToolUse", "session_id":"same", "cwd":"C:/work", "pid":100, "process_created_at":100, "tool_name":"Read", "tool_call_id":"a"}), 2));
        store.apply(event(json!({"hook_event_name":"PreToolUse", "session_id":"same", "tool_name":"Read", "tool_call_id":"ambiguous"}), 3));
        let snapshot = store.snapshot();
        assert_eq!(snapshot.sessions.len(), 2);
        assert_eq!(
            snapshot
                .sessions
                .iter()
                .map(|s| s.activities.len())
                .sum::<usize>(),
            1
        );
        assert_eq!(snapshot.quarantined_events, 1);
    }

    #[test]
    fn restart_and_old_events_do_not_share_state() {
        let mut store = KimiStore::default();
        for (at, name, created) in [
            (1, "SessionStart", 10),
            (2, "SessionEnd", 10),
            (3, "SessionStart", 20),
            (4, "PreToolUse", 10),
        ] {
            store.apply(event(json!({"hook_event_name":name, "session_id":"s", "pid":100, "process_created_at":created}), at));
        }
        assert_eq!(store.snapshot().sessions.len(), 2);
        assert_eq!(store.snapshot().quarantined_events, 1);
        assert_eq!(store.active_session_count(), 1);
    }

    #[test]
    fn orphan_and_out_of_order_events_are_quarantined() {
        let mut store = KimiStore::default();
        store.apply(event(
            json!({"hook_event_name":"PreToolUse", "session_id":"s"}),
            1,
        ));
        store.apply(event(
            json!({"hook_event_name":"SessionStart", "session_id":"s"}),
            10,
        ));
        store.apply(event(
            json!({"hook_event_name":"Stop", "session_id":"s"}),
            5,
        ));
        assert_eq!(store.snapshot().quarantined_events, 2);
        assert_eq!(
            store.snapshot().sessions[0].status,
            KimiSessionStatus::Working
        );
    }

    #[test]
    fn stop_expires_observations_without_ending_the_session() {
        let mut store = KimiStore::default();
        for (at, name) in [(1, "SessionStart"), (2, "PreToolUse"), (3, "Stop")] {
            store.apply(event(
                json!({"hook_event_name":name, "session_id":"s", "tool_name":"AskUser"}),
                at,
            ));
        }
        let session = &store.snapshot().sessions[0];
        assert_eq!(session.status, KimiSessionStatus::Idle);
        assert!(session.ended_at.is_none());
        assert_eq!(
            session.pending_interactions[0].status,
            KimiObservationStatus::Stale
        );
    }

    #[test]
    fn real_tool_names_route_to_question_and_plan_observations() {
        for tool in ["AskUserQuestion", "AskUser", "ask_user"] {
            assert_eq!(
                classify_tool(tool, &json!({})).0,
                KimiInteractionKind::AskUser
            );
        }
        for tool in ["ExitPlanMode", "exit_plan_mode", "Plan"] {
            assert_eq!(
                classify_tool(tool, &json!({})).0,
                KimiInteractionKind::ExitPlanMode
            );
        }
    }

    #[test]
    fn permission_notification_updates_and_resolves_the_existing_tool_review() {
        let mut store = KimiStore::default();
        store.apply(event(
            json!({"hook_event_name":"SessionStart", "session_id":"s"}),
            1,
        ));
        for (at, name) in [(2, "PreToolUse"), (3, "PermissionRequest")] {
            store.apply(event(json!({"hook_event_name":name, "session_id":"s", "tool_name":"Shell", "tool_call_id":"call"}), at));
        }
        let snapshot = store.snapshot();
        assert_eq!(snapshot.sessions[0].activities.len(), 1);
        assert_eq!(snapshot.sessions[0].pending_interactions.len(), 1);
        store.apply(event(
            json!({"hook_event_name":"PermissionResult", "session_id":"s", "tool_call_id":"call"}),
            4,
        ));
        assert_eq!(
            store.snapshot().sessions[0].pending_interactions[0].status,
            KimiObservationStatus::Stale
        );
    }

    #[test]
    fn permission_notification_preserves_pre_tool_arguments() {
        for (tool, input, kind) in [
            (
                "Shell",
                json!({"command":"echo test"}),
                KimiInteractionKind::Shell,
            ),
            (
                "AskUserQuestion",
                json!({"questions":[{"question":"Which file?", "options":["a", "b"]}]}),
                KimiInteractionKind::AskUser,
            ),
            (
                "ExitPlanMode",
                json!({"plan":"# Proposed plan"}),
                KimiInteractionKind::ExitPlanMode,
            ),
        ] {
            let mut store = KimiStore::default();
            store.apply(event(
                json!({"hook_event_name":"SessionStart", "session_id":"s"}),
                1,
            ));
            store.apply(event(json!({"hook_event_name":"PreToolUse", "session_id":"s", "tool_name":tool, "tool_call_id":"call", "tool_input":input}), 2));
            let initial = store.snapshot().sessions[0].pending_interactions[0].clone();
            store.apply(event(json!({"hook_event_name":"PermissionRequest", "session_id":"s", "tool_call_id":"call"}), 3));
            let snapshot = store.snapshot();
            let observations = &snapshot.sessions[0].pending_interactions;
            assert_eq!(observations.len(), 1);
            assert_eq!(observations[0].kind, kind);
            assert_eq!(observations[0].interaction_key, initial.interaction_key);
            assert_eq!(observations[0].tool_input, initial.tool_input);
            assert_eq!(observations[0].questions, initial.questions);
            assert_eq!(observations[0].plan, initial.plan);
            assert_eq!(observations[0].detail, initial.detail);
        }
    }

    #[test]
    fn permission_display_updates_the_plan_review_with_the_captured_revision() {
        let mut store = KimiStore::default();
        store.apply(event(
            json!({"hook_event_name":"SessionStart", "session_id":"s"}),
            1,
        ));
        store.apply(event(
            json!({
                "hook_event_name":"PreToolUse", "session_id":"s",
                "tool_name":"ExitPlanMode", "tool_call_id":"plan", "tool_input":{}
            }),
            2,
        ));
        let initial = store.snapshot().sessions[0].pending_interactions[0].clone();
        for (at, plan) in [(3, "# First revision"), (4, "# Revised plan")] {
            store.apply(event(json!({
                "hook_event_name":"PermissionRequest", "session_id":"s",
                "tool_name":"ExitPlanMode", "tool_call_id":"plan", "tool_input":{},
                "display":{"kind":"plan_review", "path":"outside-workspace/plan.md", "plan":plan}
            }), at));
            let snapshot = store.snapshot();
            let reviews = &snapshot.sessions[0].pending_interactions;
            assert_eq!(reviews.len(), 1);
            assert_eq!(reviews[0].interaction_key, initial.interaction_key);
            assert_eq!(reviews[0].kind, KimiInteractionKind::ExitPlanMode);
            assert_eq!(reviews[0].plan.as_deref(), Some(plan));
            assert_eq!(
                reviews[0].plan_filename.as_deref(),
                Some("outside-workspace/plan.md")
            );
            assert!(reviews[0].plan_read_error.is_none());
        }
        store.apply(event(
            json!({
                "hook_event_name":"PermissionRequest", "session_id":"s", "tool_call_id":"plan"
            }),
            5,
        ));
        assert_eq!(
            store.snapshot().sessions[0].pending_interactions[0]
                .plan
                .as_deref(),
            Some("# Revised plan")
        );
    }

    #[test]
    fn command_display_supplies_a_missing_tool_summary() {
        assert_eq!(
            summary(&json!({
                "tool_name":"Shell", "tool_input":{},
                "display":{"kind":"command", "command":"echo test"}
            })),
            "echo test"
        );
    }

    #[test]
    fn late_permission_requests_cannot_reopen_resolved_reviews() {
        for initial in ["PreToolUse", "PermissionResult", "PostToolUse"] {
            let mut store = KimiStore::default();
            store.apply(event(
                json!({"hook_event_name":"SessionStart", "session_id":"s"}),
                1,
            ));
            for (at, name) in [
                (2, initial),
                (3, "PermissionResult"),
                (4, "PermissionRequest"),
            ] {
                store.apply(event(
                    json!({
                        "hook_event_name":name, "session_id":"s",
                        "tool_name":"ExitPlanMode", "tool_call_id":"plan", "tool_input":{},
                        "display":{"kind":"plan_review", "plan":"# Captured plan"}
                    }),
                    at,
                ));
            }
            let snapshot = store.snapshot();
            let reviews = &snapshot.sessions[0].pending_interactions;
            assert_eq!(reviews.len(), 1);
            assert!(!matches!(
                reviews[0].status,
                KimiObservationStatus::Observed | KimiObservationStatus::Updated
            ));
            assert_eq!(reviews[0].plan.as_deref(), Some("# Captured plan"));
        }
    }

    #[test]
    fn late_tool_and_turn_events_cannot_reactivate_a_stopped_turn() {
        let mut store = KimiStore::default();
        for (at, name) in [
            (1, "SessionStart"),
            (2, "Stop"),
            (3, "PostToolUse"),
            (4, "PreToolUse"),
            (5, "PermissionRequest"),
            (6, "TurnStarted"),
        ] {
            store.apply(event(
                json!({
                    "hook_event_name":name, "session_id":"s",
                    "tool_name":"AskUserQuestion", "tool_call_id":"call"
                }),
                at,
            ));
        }
        let snapshot = store.snapshot();
        assert_eq!(snapshot.sessions[0].status, KimiSessionStatus::Idle);
        assert_eq!(
            snapshot.sessions[0].pending_interactions[0].status,
            KimiObservationStatus::ToolCompleted
        );
        store.apply(event(
            json!({"hook_event_name":"UserPromptSubmit", "session_id":"s", "prompt":"Next turn"}),
            7,
        ));
        store.apply(event(json!({"hook_event_name":"PreToolUse", "session_id":"s", "tool_name":"AskUserQuestion", "tool_call_id":"next-call"}), 8));
        store.apply(event(
            json!({"hook_event_name":"TurnStarted", "session_id":"s"}),
            9,
        ));
        assert_eq!(
            store.snapshot().sessions[0].status,
            KimiSessionStatus::WaitingForInput
        );
    }

    #[test]
    fn failed_turn_stays_closed_until_the_next_prompt() {
        let mut store = KimiStore::default();
        for (at, name) in [
            (1, "SessionStart"),
            (2, "StopFailure"),
            (3, "PreToolUse"),
            (4, "PermissionRequest"),
            (5, "PostToolUse"),
            (6, "TurnStarted"),
        ] {
            store.apply(event(
                json!({
                    "hook_event_name":name, "session_id":"s",
                    "tool_name":"AskUserQuestion", "tool_call_id":"late-call"
                }),
                at,
            ));
            if at >= 2 {
                let snapshot = store.snapshot();
                assert_eq!(snapshot.sessions[0].status, KimiSessionStatus::ToolFailed);
                assert_eq!(store.active_session_count(), 0);
                assert!(snapshot.interactions.iter().all(|interaction| !matches!(
                    interaction.status,
                    KimiObservationStatus::Observed | KimiObservationStatus::Updated
                )));
            }
        }
        store.apply(event(
            json!({
                "hook_event_name":"UserPromptSubmit", "session_id":"s", "prompt":"Retry"
            }),
            7,
        ));
        store.apply(event(
            json!({
                "hook_event_name":"PostToolUseFailure", "session_id":"s",
                "tool_name":"Read", "tool_call_id":"recoverable-error"
            }),
            8,
        ));
        assert_eq!(store.active_session_count(), 1);
        store.apply(event(
            json!({
                "hook_event_name":"PreToolUse", "session_id":"s",
                "tool_name":"AskUserQuestion", "tool_call_id":"new-question"
            }),
            9,
        ));
        assert_eq!(
            store.snapshot().sessions[0].status,
            KimiSessionStatus::WaitingForInput
        );
    }

    fn wire_interaction(kind: &str, id: &str, tool_call_id: &str, request: Value) -> crate::kimi_wire::KimiWireInteraction {
        crate::kimi_wire::KimiWireInteraction {
            wire_path: std::path::PathBuf::from("wire.jsonl"),
            native_session_id: "s".to_string(),
            agent_id: Some("main".to_string()),
            native_interaction_id: id.to_string(),
            tool_call_id: Some(tool_call_id.to_string()),
            kind: kind.to_string(),
            request,
            captured_at: 2,
        }
    }

    #[test]
    fn wire_requests_are_visible_until_resolved() {
        let mut store = KimiStore::default();
        store.apply(event(
            json!({"hook_event_name":"SessionStart", "session_id":"s", "cwd":"C:/work"}),
            1,
        ));
        let request = wire_interaction(
            "question",
            "question_1",
            "call_1",
            json!({"toolName":"AskUserQuestion","questions":[{"question":"Which file?","options":[{"label":"a"}]}]}),
        );
        store.sync_wire_interactions(&[request.clone()]);
        let snapshot = store.snapshot();
        assert_eq!(snapshot.sessions[0].status, KimiSessionStatus::WaitingForInput);
        assert_eq!(snapshot.interactions.len(), 1);
        assert_eq!(snapshot.interactions[0].kind, KimiInteractionKind::AskUser);
        assert_eq!(snapshot.interactions[0].native_interaction_id.as_deref(), Some("question_1"));
        let version = snapshot.version;
        store.sync_wire_interactions(&[request]);
        assert_eq!(store.snapshot().version, version);
        store.sync_wire_interactions(&[]);
        let resolved = &store.snapshot().interactions[0];
        assert!(resolved.resolved);
        assert_eq!(resolved.status, KimiObservationStatus::ToolCompleted);
    }

    #[test]
    fn wire_plan_and_approval_requests_use_the_existing_review_kinds() {
        let mut store = KimiStore::default();
        store.apply(event(
            json!({"hook_event_name":"SessionStart", "session_id":"s"}),
            1,
        ));
        let plan = wire_interaction(
            "approval",
            "approval_plan",
            "plan_call",
            json!({"toolName":"ExitPlanMode","display":{"kind":"plan_review","path":"plan.md","plan":"# Plan"}}),
        );
        store.sync_wire_interactions(&[plan]);
        assert_eq!(store.snapshot().interactions[0].kind, KimiInteractionKind::ExitPlanMode);
        let approval = wire_interaction(
            "approval",
            "approval_shell",
            "shell_call",
            json!({"toolName":"Bash","action":"Running: echo test","display":{"kind":"command","command":"echo test"}}),
        );
        store.sync_wire_interactions(&[approval]);
        let kinds: Vec<_> = store.snapshot().interactions.iter().map(|item| item.kind).collect();
        assert!(kinds.contains(&KimiInteractionKind::Shell));
    }

    #[test]
    fn session_end_cannot_be_reopened_by_an_unresolved_wire_record() {
        let mut store = KimiStore::default();
        store.apply(event(
            json!({"hook_event_name":"SessionStart", "session_id":"s"}),
            1,
        ));
        let request = wire_interaction(
            "question",
            "question_ended",
            "call_ended",
            json!({"toolName":"AskUserQuestion","questions":[{"question":"Continue?"}]}),
        );
        store.sync_wire_interactions(&[request]);
        store.apply(event(
            json!({"hook_event_name":"SessionEnd", "session_id":"s"}),
            3,
        ));
        store.sync_wire_interactions(&[wire_interaction(
            "question",
            "question_ended",
            "call_ended",
            json!({"toolName":"AskUserQuestion","questions":[{"question":"Continue?"}]}),
        )]);
        let snapshot = store.snapshot();
        assert_eq!(snapshot.sessions[0].status, KimiSessionStatus::Stopped);
        assert!(snapshot.interactions.iter().all(|item| item.resolved));
    }

    #[test]
    fn window_binding_accepts_old_numeric_handles_and_new_verified_metadata() {
        let binding = terminal_binding_from_payload(
            &json!({
                "pid":42, "process_created_at":10, "console_window":123,
                "window_process_id":43, "window_process_created_at":9, "shared_terminal":true,
            }),
            20,
        )
        .unwrap();
        assert_eq!(binding.console_window.as_deref(), Some("123"));
        assert_eq!(binding.window_process_id, Some(43));
        assert_eq!(binding.window_process_created_at, Some(9));
        assert!(binding.shared_terminal);
    }
}
