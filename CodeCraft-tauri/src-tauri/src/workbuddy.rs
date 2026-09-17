//! Read-only WorkBuddy/CodeBuddy session state.
//!
//! Captures observations only. Approvals and answers are handled in WorkBuddy;
//! this store has no decision submission or managed session transport.

use std::collections::{HashMap, HashSet, VecDeque};

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

pub(crate) const PROTOCOL: &str = "workbuddy-codebuddy-hooks";
pub(crate) const PROTOCOL_VERSION: u32 = 1;

const MAX_SESSIONS: usize = 256;
const MAX_INTERACTIONS: usize = 512;
const MAX_SESSION_INTERACTIONS: usize = 64;
const MAX_SEEN_PAYLOADS: usize = 4096;
const MAX_SESSION_ID_CHARS: usize = 160;
const MAX_TITLE_CHARS: usize = 96;
const MAX_PROMPT_CHARS: usize = 2_000;
const MAX_TEXT_CHARS: usize = 8_000;
const INTERACTION_TTL_MS: u64 = 30 * 60 * 1_000;
const MAX_ACTIVITIES: usize = 100;
const MAX_OUTPUTS: usize = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum WorkBuddyRequestKind {
    Tool,
    Question,
    Plan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum WorkBuddyRequestStatus {
    Observed,
    Pending,
    Completed,
    Failed,
    Denied,
    Cancelled,
    Timeout,
    Unavailable,
    Superseded,
}

impl WorkBuddyRequestStatus {
    fn is_open(self) -> bool {
        matches!(self, Self::Observed | Self::Pending | Self::Unavailable)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkBuddyQuestionOption {
    pub(crate) label: String,
    pub(crate) description: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkBuddyQuestion {
    pub(crate) header: Option<String>,
    pub(crate) question: String,
    pub(crate) options: Vec<WorkBuddyQuestionOption>,
    pub(crate) multi_select: bool,
    pub(crate) allow_other: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkBuddyInteraction {
    pub(crate) request_key: String,
    pub(crate) session_id: String,
    pub(crate) kind: WorkBuddyRequestKind,
    pub(crate) status: WorkBuddyRequestStatus,
    pub(crate) native_request_id: Option<String>,
    pub(crate) tool_name: Option<String>,
    pub(crate) tool_input: Option<Value>,
    pub(crate) summary: String,
    pub(crate) questions: Vec<WorkBuddyQuestion>,
    pub(crate) plan: Option<String>,
    pub(crate) plan_hash: Option<String>,
    pub(crate) plan_source: Option<String>,
    pub(crate) payload_hash: String,
    pub(crate) captured_at: u64,
    pub(crate) expires_at: u64,
    pub(crate) answerable: bool,
    pub(crate) reason: String,
    #[serde(skip)]
    elicitation: bool,
    #[serde(skip)]
    notification_only: bool,
    #[serde(skip)]
    generation_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkBuddyActivity {
    pub(crate) id: String,
    pub(crate) tool: String,
    pub(crate) summary: String,
    pub(crate) status: String,
    pub(crate) started_at: u64,
    pub(crate) updated_at: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkBuddyOutput {
    pub(crate) id: String,
    pub(crate) text: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkBuddyCapabilities {
    pub(crate) can_observe: bool,
    pub(crate) can_approve_tools: bool,
    pub(crate) can_answer_questions: bool,
    pub(crate) can_approve_plans: bool,
    pub(crate) can_stream_output: bool,
    pub(crate) protocol_frozen: bool,
    pub(crate) reason: String,
}

impl Default for WorkBuddyCapabilities {
    fn default() -> Self {
        Self {
            can_observe: true,
            can_approve_tools: false,
            can_answer_questions: false,
            can_approve_plans: false,
            can_stream_output: false,
            protocol_frozen: false,
            reason: "WorkBuddy 仅支持只读观察，请前往 WorkBuddy 中处理".to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum WorkBuddyTitleSource {
    Native,
    Prompt,
    Fallback,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkBuddySession {
    pub(crate) id: String,
    pub(crate) workbuddy_session_id: String,
    pub(crate) title: String,
    pub(crate) title_source: WorkBuddyTitleSource,
    pub(crate) identity_status: String,
    pub(crate) source: Option<String>,
    pub(crate) workbuddy_version: Option<String>,
    pub(crate) cli_version: Option<String>,
    pub(crate) plugin_instance_id: Option<String>,
    pub(crate) process_instance_id: Option<String>,
    pub(crate) cwd_hash: Option<String>,
    pub(crate) transcript_hash: Option<String>,
    pub(crate) permission_mode: Option<String>,
    pub(crate) stage: String,
    pub(crate) current_tool: Option<String>,
    pub(crate) last_prompt: Option<String>,
    pub(crate) event_count: u64,
    pub(crate) started_at: u64,
    pub(crate) updated_at: u64,
    pub(crate) ended_at: Option<u64>,
    pub(crate) pending_count: usize,
    pub(crate) activities: Vec<WorkBuddyActivity>,
    pub(crate) outputs: Vec<WorkBuddyOutput>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkBuddySnapshot {
    pub(crate) hook: Option<crate::workbuddy_hook::WorkBuddyHookStatus>,
    pub(crate) connected: bool,
    pub(crate) integration_error: Option<String>,
    pub(crate) version: u64,
    pub(crate) sessions: Vec<WorkBuddySession>,
    pub(crate) interactions: Vec<WorkBuddyInteraction>,
    pub(crate) capabilities: WorkBuddyCapabilities,
    pub(crate) observed_event_count: u64,
    pub(crate) unknown_event_count: u64,
    pub(crate) diagnostics: WorkBuddyDiagnostics,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkBuddyDiagnostics {
    pub(crate) duplicate_events: u64,
    pub(crate) late_events: u64,
    pub(crate) quarantined_events: u64,
    pub(crate) dropped_sessions: u64,
    pub(crate) dropped_interactions: u64,
    pub(crate) ambiguous_completions: u64,
    pub(crate) retired_processes: u64,
}

#[derive(Default)]
pub(crate) struct WorkBuddyStore {
    sessions: HashMap<String, WorkBuddySession>,
    interactions: HashMap<String, WorkBuddyInteraction>,
    seen_payloads: HashSet<String>,
    seen_order: VecDeque<String>,
    diagnostics: WorkBuddyDiagnostics,
    version: u64,
    observed_event_count: u64,
    unknown_event_count: u64,
    integration_error: Option<String>,
}

impl WorkBuddyStore {
    pub(crate) fn ingest(&mut self, payload: &Value, payload_hash: &str, received_at: u64) {
        let unwrapped = unwrap_deferred_tool(payload);
        let payload = unwrapped.as_ref().unwrap_or(payload);
        if !self.seen_payloads.insert(payload_hash.to_string()) {
            self.diagnostics.duplicate_events += 1;
            self.version = self.version.saturating_add(1);
            return;
        }
        self.seen_order.push_back(payload_hash.to_string());
        if self.seen_order.len() > MAX_SEEN_PAYLOADS {
            if let Some(old) = self.seen_order.pop_front() {
                self.seen_payloads.remove(&old);
            }
        }
        self.version = self.version.saturating_add(1);
        self.observed_event_count = self.observed_event_count.saturating_add(1);

        let event = payload
            .get("hook_event_name")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        if !crate::workbuddy_hook::SUPPORTED_EVENTS.contains(&event) {
            self.unknown_event_count = self.unknown_event_count.saturating_add(1);
            self.integration_error = Some(format!("未知 WorkBuddy Hook 事件：{event}"));
            return;
        }

        let Some(native_session_id) = string_field(payload, "session_id") else {
            self.diagnostics.quarantined_events += 1;
            self.integration_error =
                Some("WorkBuddy Hook 事件缺少 session_id，已隔离观察".to_string());
            return;
        };
        let now = received_at.max(1);
        let identity = serde_json::to_string(&[
            Some(native_session_id.as_str()),
            payload.get("plugin_instance_id").and_then(Value::as_str),
            payload.get("process_instance_id").and_then(Value::as_str),
        ])
        .unwrap_or_default();
        let identity_key = digest_text(&identity);
        let identity_complete = ["plugin_instance_id", "process_instance_id", "cwd_hash"]
            .iter()
            .all(|key| string_field(payload, key).is_some());
        if !identity_complete {
            self.diagnostics.quarantined_events += 1;
        }
        if let Some(session) = self.sessions.get(&identity_key) {
            // SessionStart may omit cwd. Enrich missing metadata on later
            // events, but never merge conflicting nonempty identities.
            let conflicts = [
                (&session.cwd_hash, "cwd_hash"),
                (&session.transcript_hash, "transcript_hash"),
            ]
            .iter()
            .any(|(old, key)| {
                old.as_deref()
                    .zip(payload.get(*key).and_then(Value::as_str))
                    .is_some_and(|(old, new)| old != new)
            });
            if conflicts {
                self.diagnostics.quarantined_events += 1;
                self.integration_error =
                    Some("WorkBuddy 会话目录或 transcript 身份冲突，事件已隔离".into());
                return;
            }
            // A native session id is not a restart signal. A different process
            // identity creates a new integration session; ended ones stay ended.
            if now < session.updated_at || session.ended_at.is_some() {
                self.diagnostics.late_events += 1;
                return;
            }
        } else if self.sessions.len() >= MAX_SESSIONS {
            let oldest = self
                .sessions
                .iter()
                .filter(|(_, s)| {
                    matches!(s.stage.as_str(), "idle" | "stopped") && s.pending_count == 0
                })
                .min_by_key(|(_, s)| s.updated_at)
                .map(|(key, _)| key.clone());
            if let Some(key) = oldest {
                if let Some(old) = self.sessions.remove(&key) {
                    self.interactions.retain(|_, i| i.session_id != old.id);
                }
            } else {
                self.diagnostics.dropped_sessions += 1;
                return;
            }
        }
        let session_id = self
            .sessions
            .get(&identity_key)
            .map(|s| s.id.clone())
            .unwrap_or_else(|| format!("wbs_{}", uuid::Uuid::new_v4().simple()));
        let incoming_title = native_session_title(payload, event);
        if matches!(event, "PreToolUse" | "PermissionRequest" | "Elicitation")
            || is_question_permission_notification(payload)
        {
            self.observe_interaction(payload, &session_id, payload_hash, now);
        }
        {
            let session = self
                .sessions
                .entry(identity_key.clone())
                .or_insert_with(|| {
                    let (title, title_source) = incoming_title
                        .clone()
                        .map(|title| (title, WorkBuddyTitleSource::Native))
                        .unwrap_or_else(|| {
                            (
                                fallback_session_title(&native_session_id),
                                WorkBuddyTitleSource::Fallback,
                            )
                        });
                    WorkBuddySession {
                        id: session_id.clone(),
                        workbuddy_session_id: native_session_id,
                        title,
                        title_source,
                        identity_status: if identity_complete {
                            "observedLineage"
                        } else {
                            "quarantined"
                        }
                        .into(),
                        source: string_field(payload, "client"),
                        workbuddy_version: string_field(payload, "workbuddy_version"),
                        cli_version: string_field(payload, "cli_version"),
                        plugin_instance_id: string_field(payload, "plugin_instance_id"),
                        process_instance_id: string_field(payload, "process_instance_id"),
                        cwd_hash: string_field(payload, "cwd_hash"),
                        transcript_hash: string_field(payload, "transcript_hash"),
                        permission_mode: string_field(payload, "permission_mode"),
                        stage: "starting".to_string(),
                        current_tool: None,
                        last_prompt: None,
                        event_count: 0,
                        started_at: now,
                        updated_at: now,
                        ended_at: None,
                        pending_count: 0,
                        activities: Vec::new(),
                        outputs: Vec::new(),
                    }
                });

            session.updated_at = now;
            session.event_count = session.event_count.saturating_add(1);
            if identity_complete {
                session.identity_status = "observedLineage".into();
            }
            session.workbuddy_version =
                string_field(payload, "workbuddy_version").or(session.workbuddy_version.clone());
            session.cli_version =
                string_field(payload, "cli_version").or(session.cli_version.clone());
            session.plugin_instance_id =
                string_field(payload, "plugin_instance_id").or(session.plugin_instance_id.clone());
            session.process_instance_id = string_field(payload, "process_instance_id")
                .or(session.process_instance_id.clone());
            session.cwd_hash = string_field(payload, "cwd_hash").or(session.cwd_hash.clone());
            session.transcript_hash =
                string_field(payload, "transcript_hash").or(session.transcript_hash.clone());
            session.permission_mode =
                string_field(payload, "permission_mode").or(session.permission_mode.clone());
            if let Some(title) = incoming_title {
                session.title = title;
                session.title_source = WorkBuddyTitleSource::Native;
            }

            match event {
                "SessionStart" => {
                    session.stage = "working".to_string();
                    session.ended_at = None;
                }
                "SessionEnd" => {
                    session.stage = "stopped".to_string();
                    session.current_tool = None;
                    session.ended_at = Some(now);
                }
                "PreToolUse" => {
                    let tool = string_field(payload, "tool_name")
                        .map(|value| normalize_tool_name(&value))
                        .unwrap_or_default();
                    session.stage = if matches!(tool.as_str(), "askuserquestion" | "exitplanmode") {
                        "waitingForInput".to_string()
                    } else {
                        "toolRunning".to_string()
                    };
                    session.current_tool = string_field(payload, "tool_name");
                    apply_tool_activity(session, payload, now, "running");
                }
                "PostToolUse" => {
                    let failed = native_tool_failed(payload);
                    session.stage = if failed { "toolFailed" } else { "working" }.to_string();
                    session.current_tool = None;
                    apply_tool_activity(
                        session,
                        payload,
                        now,
                        if failed { "failed" } else { "completed" },
                    );
                    append_tool_output(session, payload, now, failed);
                }
                "PostToolUseFailure" => {
                    session.stage = "toolFailed".to_string();
                    session.current_tool = None;
                    apply_tool_activity(session, payload, now, "failed");
                    append_tool_output(session, payload, now, true);
                }
                "UserPromptSubmit" => {
                    session.stage = "working".to_string();
                    let prompt = payload
                        .get("prompt")
                        .or_else(|| payload.get("user_prompt"))
                        .and_then(Value::as_str)
                        .map(|text| truncate(text, MAX_PROMPT_CHARS));
                    if session.title_source == WorkBuddyTitleSource::Fallback {
                        if let Some(title) = prompt.as_deref().and_then(prompt_session_title) {
                            session.title = title;
                            session.title_source = WorkBuddyTitleSource::Prompt;
                        }
                    }
                    session.last_prompt = prompt;
                }
                "PermissionRequest" | "Elicitation" => {
                    session.stage = "waitingForInput".to_string();
                }
                "PermissionDenied" => {
                    session.stage = "toolFailed".to_string();
                }
                "ElicitationResult" => {
                    session.stage = "working".to_string();
                }
                "Stop" | "SubagentStop" => {
                    session.stage = "idle".to_string();
                    session.current_tool = None;
                    if let Some(text) = payload
                        .get("last_assistant_message")
                        .and_then(Value::as_str)
                    {
                        append_output(session, text, now);
                    }
                }
                "SubagentStart" => session.stage = "working".to_string(),
                _ => {}
            }
        }

        self.resolve_interaction_for_event(payload, &session_id, event, now);
        let pending_count = self
            .interactions
            .values()
            .filter(|interaction| {
                interaction.session_id == session_id
                    && interaction.status == WorkBuddyRequestStatus::Pending
            })
            .count();
        if let Some(session) = self.sessions.get_mut(&identity_key) {
            session.pending_count = pending_count;
            if pending_count > 0 && session.ended_at.is_none() {
                session.stage = "waitingForInput".into();
            }
        }

        self.integration_error = None;
    }

    fn observe_interaction(
        &mut self,
        payload: &Value,
        session_id: &str,
        payload_hash: &str,
        captured_at: u64,
    ) {
        let event = payload["hook_event_name"].as_str().unwrap_or_default();
        let elicitation = event == "Elicitation";
        let notification_only = is_question_permission_notification(payload);
        let tool_name = if notification_only {
            Some("AskUserQuestion".to_string())
        } else {
            string_field(
                payload,
                if elicitation {
                    "mcp_server_name"
                } else {
                    "tool_name"
                },
            )
        };
        let normalized = tool_name
            .as_deref()
            .map(normalize_tool_name)
            .unwrap_or_default();
        let kind = if elicitation || normalized == "askuserquestion" {
            WorkBuddyRequestKind::Question
        } else if normalized == "exitplanmode" {
            WorkBuddyRequestKind::Plan
        } else {
            WorkBuddyRequestKind::Tool
        };
        let native_request_id = native_request_id(payload);
        let generation_id = string_field(payload, "generation_id");
        let awaiting_user = event == "PermissionRequest" || kind != WorkBuddyRequestKind::Tool;
        let status = if awaiting_user {
            WorkBuddyRequestStatus::Pending
        } else {
            WorkBuddyRequestStatus::Unavailable
        };
        let mut existing_key = self
            .interactions
            .values()
            .find(|i| {
                native_request_id.is_some()
                    && i.elicitation == elicitation
                    && i.kind == kind
                    && i.status.is_open()
                    && i.session_id == session_id
                    && i.native_request_id == native_request_id
                    && i.tool_name == tool_name
                    && compatible_optional_id(&i.generation_id, &generation_id)
            })
            .map(|i| i.request_key.clone());
        if existing_key.is_none() && kind == WorkBuddyRequestKind::Question && !elicitation {
            // Desktop asks through its native UI before PreToolUse. Its early
            // permission notification has no call id or question body. Join it
            // only to a unique compatible question, preserving the reminder key.
            let candidates = self
                .interactions
                .values()
                .filter(|i| {
                    i.session_id == session_id
                        && i.kind == WorkBuddyRequestKind::Question
                        && !i.elicitation
                        && i.status.is_open()
                        && (notification_only || i.notification_only)
                        && compatible_optional_id(&i.native_request_id, &native_request_id)
                        && compatible_optional_id(&i.generation_id, &generation_id)
                })
                .map(|i| i.request_key.clone())
                .collect::<Vec<_>>();
            if candidates.len() == 1 {
                existing_key = candidates.into_iter().next();
            } else if notification_only && candidates.len() > 1 {
                // Every candidate already has a question reminder. A notification
                // without an id cannot identify a new request among them.
                return;
            }
        }
        if let Some(existing) = existing_key.and_then(|key| self.interactions.get_mut(&key)) {
            // A PermissionRequest follows PreToolUse for the same native call.
            // Keep its request key so a refresh cannot dismiss/reopen the review.
            if awaiting_user {
                existing.status = status;
                existing.reason = "等待在 WorkBuddy 中处理，完成后此提醒会自动关闭".into();
            }
            if existing.notification_only && !notification_only {
                existing.native_request_id =
                    native_request_id.or(existing.native_request_id.take());
                existing.generation_id = generation_id.or(existing.generation_id.take());
                existing.tool_name = tool_name;
                existing.tool_input = payload
                    .get("tool_input")
                    .or_else(|| payload.get("toolInput"))
                    .cloned();
                let questions = extract_questions(existing.tool_input.as_ref().or(Some(payload)));
                if !questions.is_empty() {
                    existing.questions = questions;
                    existing.summary = "WorkBuddy 问题".into();
                }
                existing.payload_hash = payload_hash.to_string();
                existing.notification_only = false;
            }
            return;
        }
        // Older runtimes omit ids on PermissionRequest. Promote only a unique
        // matching tool observation; never merge concurrent questions/tools.
        if event == "PermissionRequest" && native_request_id.is_none() {
            let matches = self
                .interactions
                .values()
                .filter(|i| {
                    i.session_id == session_id
                        && i.tool_name == tool_name
                        && i.status.is_open()
                        && i.tool_input.as_ref()
                            == payload
                                .get("tool_input")
                                .or_else(|| payload.get("toolInput"))
                })
                .map(|i| i.request_key.clone())
                .collect::<Vec<_>>();
            if matches.len() == 1 {
                let existing = self.interactions.get_mut(&matches[0]).unwrap();
                existing.status = status;
                existing.reason = "等待在 WorkBuddy 中处理，完成后此提醒会自动关闭".into();
                return;
            }
        }
        let request_key = format!("wbk_{}", uuid::Uuid::new_v4().simple());
        let tool_input = payload
            .get("tool_input")
            .or_else(|| payload.get("toolInput"))
            .map(|value| value.clone());
        let questions = if kind == WorkBuddyRequestKind::Question {
            if notification_only {
                vec![WorkBuddyQuestion {
                    header: Some("等待回答".into()),
                    question: "WorkBuddy 正在等待你回答问题，请前往 WorkBuddy 查看并处理。".into(),
                    options: Vec::new(),
                    multi_select: false,
                    allow_other: false,
                }]
            } else if elicitation {
                extract_elicitation_questions(payload)
            } else {
                extract_questions(tool_input.as_ref().or(Some(payload)))
            }
        } else {
            Vec::new()
        };
        let plan = if kind == WorkBuddyRequestKind::Plan {
            extract_plan(tool_input.as_ref().or(Some(payload)))
        } else {
            None
        };
        let summary = if notification_only {
            "WorkBuddy 正在等待回答问题".to_string()
        } else {
            tool_name
                .as_deref()
                .map(|tool| tool_summary(tool, tool_input.as_ref()))
                .unwrap_or_else(|| "WorkBuddy 交互请求".to_string())
        };
        let plan_hash = plan.as_deref().map(digest_text);
        if kind == WorkBuddyRequestKind::Plan {
            for previous in self
                .interactions
                .values_mut()
                .filter(|i| i.session_id == session_id && i.kind == kind && i.status.is_open())
            {
                previous.status = WorkBuddyRequestStatus::Superseded;
                previous.reason = "观察到新的计划请求；旧观察不可提交".into();
            }
        }
        self.interactions.insert(
            request_key.clone(),
            WorkBuddyInteraction {
                request_key: request_key.clone(),
                session_id: session_id.to_string(),
                kind,
                status,
                native_request_id,
                tool_name,
                tool_input,
                summary,
                questions,
                plan_hash,
                plan_source: (kind == WorkBuddyRequestKind::Plan).then(|| {
                    if plan.is_some() {
                        "hookInputUnverified".into()
                    } else {
                        "unavailable".into()
                    }
                }),
                plan,
                payload_hash: payload_hash.to_string(),
                captured_at,
                expires_at: captured_at.saturating_add(INTERACTION_TTL_MS),
                answerable: false,
                reason: if awaiting_user {
                    "等待在 WorkBuddy 中处理，完成后此提醒会自动关闭".into()
                } else {
                    "WorkBuddy 正在调用工具".into()
                },
                elicitation,
                notification_only,
                generation_id,
            },
        );
        self.diagnostics.dropped_interactions +=
            trim_interactions(&mut self.interactions, session_id) as u64;
        // Preserve existing reminders when every slot is waiting for the user.
        // Refuse overflow instead of expiring an older, still-live request.
        if self.interactions.len() > MAX_INTERACTIONS
            || self
                .interactions
                .values()
                .filter(|i| i.session_id == session_id)
                .count()
                > MAX_SESSION_INTERACTIONS
        {
            self.interactions.remove(&request_key);
            self.diagnostics.dropped_interactions += 1;
        }
    }

    fn resolve_interaction_for_event(
        &mut self,
        payload: &Value,
        session_id: &str,
        event: &str,
        captured_at: u64,
    ) {
        if matches!(event, "SessionEnd" | "Stop") {
            // A completed native turn/session cannot still be awaiting an answer.
            // SubagentStop is deliberately excluded: the parent may be waiting.
            for interaction in self
                .interactions
                .values_mut()
                .filter(|i| i.session_id == session_id && i.status.is_open())
            {
                interaction.status = WorkBuddyRequestStatus::Cancelled;
                interaction.reason = "WorkBuddy 已结束本轮交互".into();
                interaction.expires_at = captured_at;
            }
            return;
        }
        if !matches!(
            event,
            "PostToolUse" | "PostToolUseFailure" | "PermissionDenied" | "ElicitationResult"
        ) {
            return;
        }
        let native_id = native_request_id(payload);
        let generation_id = string_field(payload, "generation_id");
        let elicitation = event == "ElicitationResult";
        let tool = string_field(
            payload,
            if elicitation {
                "mcp_server_name"
            } else {
                "tool_name"
            },
        );
        let mut candidates = self
            .interactions
            .values()
            .filter(|interaction| {
                interaction.session_id == session_id
                    && interaction.elicitation == elicitation
                    && interaction.status.is_open()
                    && compatible_optional_id(&interaction.generation_id, &generation_id)
                    && (native_id.is_some()
                        && interaction.native_request_id == native_id
                        && (tool.is_none() || interaction.tool_name == tool)
                        || native_id.is_none() && tool.is_some() && interaction.tool_name == tool)
            })
            .map(|i| i.request_key.clone())
            .collect::<Vec<_>>();
        if candidates.is_empty()
            && !elicitation
            && tool
                .as_deref()
                .is_some_and(|tool| normalize_tool_name(tool) == "askuserquestion")
        {
            // Denial/failure may bypass PreToolUse entirely. A result may close
            // a unique notification reminder even though it now has a call id.
            candidates = self
                .interactions
                .values()
                .filter(|i| {
                    i.session_id == session_id
                        && i.notification_only
                        && i.status.is_open()
                        && compatible_optional_id(&i.native_request_id, &native_id)
                        && compatible_optional_id(&i.generation_id, &generation_id)
                })
                .map(|i| i.request_key.clone())
                .collect();
        }
        // No native id (or a repeated id) must not guess among concurrent tools.
        if candidates.len() > 1 {
            self.diagnostics.ambiguous_completions += 1;
            return;
        }
        if let Some(interaction) = candidates
            .first()
            .and_then(|key| self.interactions.get_mut(key))
        {
            interaction.status = if elicitation {
                match payload["action"].as_str() {
                    Some("accept") => WorkBuddyRequestStatus::Completed,
                    Some("decline") => WorkBuddyRequestStatus::Denied,
                    Some("cancel") => WorkBuddyRequestStatus::Cancelled,
                    _ => return,
                }
            } else if event == "PermissionDenied" {
                WorkBuddyRequestStatus::Denied
            } else if event == "PostToolUseFailure" || native_tool_failed(payload) {
                WorkBuddyRequestStatus::Failed
            } else {
                WorkBuddyRequestStatus::Completed
            };
            interaction.reason = "已观察到 WorkBuddy 原生结果；CodeCraft 未提交决定".into();
            interaction.expires_at = captured_at;
        }
    }

    pub(crate) fn clear(&mut self) {
        let version = self.version.saturating_add(1);
        *self = Self::default();
        self.version = version;
    }

    pub(crate) fn maintain(&mut self, now: u64) {
        let mut changed = false;
        for session in self.sessions.values_mut().filter(|s| s.ended_at.is_none()) {
            if session
                .process_instance_id
                .as_deref()
                .is_some_and(|id| crate::workbuddy_process::is_alive(id) == Some(false))
            {
                session.stage = "stopped".into();
                session.current_tool = None;
                session.ended_at = Some(now);
                session.updated_at = now;
                self.diagnostics.retired_processes += 1;
                changed = true;
            }
        }
        for i in self
            .interactions
            .values_mut()
            .filter(|i| i.status.is_open())
        {
            if i.status != WorkBuddyRequestStatus::Pending && i.expires_at <= now {
                i.status = WorkBuddyRequestStatus::Timeout;
                i.reason = "观察记录已过期；未提交任何决定".into();
                changed = true;
            } else if self
                .sessions
                .values()
                .any(|s| s.id == i.session_id && s.ended_at.is_some())
            {
                i.status = WorkBuddyRequestStatus::Cancelled;
                i.reason = "WorkBuddy 会话已结束；未提交任何决定".into();
                changed = true;
            }
        }
        for session in self.sessions.values_mut() {
            session.pending_count = self
                .interactions
                .values()
                .filter(|i| {
                    i.session_id == session.id && i.status == WorkBuddyRequestStatus::Pending
                })
                .count();
        }
        if changed {
            self.version += 1;
        }
    }

    pub(crate) fn active_session_count(&self) -> usize {
        self.sessions
            .values()
            .filter(|session| !matches!(session.stage.as_str(), "idle" | "stopped"))
            .count()
    }

    pub(crate) fn snapshot(&self) -> WorkBuddySnapshot {
        let mut sessions = self.sessions.values().cloned().collect::<Vec<_>>();
        sessions.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        let mut interactions = self.interactions.values().cloned().collect::<Vec<_>>();
        interactions.sort_by(|left, right| right.captured_at.cmp(&left.captured_at));
        WorkBuddySnapshot {
            hook: None,
            connected: self.integration_error.is_none()
                && sessions.iter().any(|session| session.ended_at.is_none()),
            integration_error: self.integration_error.clone(),
            version: self.version,
            sessions,
            interactions,
            capabilities: WorkBuddyCapabilities::default(),
            observed_event_count: self.observed_event_count,
            unknown_event_count: self.unknown_event_count,
            diagnostics: self.diagnostics.clone(),
        }
    }
}

fn compatible_optional_id(left: &Option<String>, right: &Option<String>) -> bool {
    left.as_ref()
        .zip(right.as_ref())
        .is_none_or(|(left, right)| left == right)
}

fn is_question_permission_notification(payload: &Value) -> bool {
    if payload["hook_event_name"] != "Notification"
        || payload["notification_type"] != "permission_prompt"
    {
        return false;
    }
    let explicit_tool = string_field(payload, "tool_name");
    let tool = explicit_tool.as_deref().or_else(|| {
        payload["message"]
            .as_str()?
            .trim()
            .strip_prefix("needs your permission to use ")
    });
    tool.is_some_and(|tool| {
        tool.len() <= MAX_SESSION_ID_CHARS && normalize_tool_name(tool) == "askuserquestion"
    })
}

fn native_request_id(payload: &Value) -> Option<String> {
    [
        "elicitation_id",
        "tool_use_id",
        "toolUseId",
        "call_id",
        "callId",
        "request_id",
        "requestId",
    ]
    .iter()
    .find_map(|key| string_field(payload, key))
}

fn unwrap_deferred_tool(payload: &Value) -> Option<Value> {
    if normalize_tool_name(payload.get("tool_name")?.as_str()?) != "deferexecutetool" {
        return None;
    }
    let input = payload.get("tool_input")?;
    let name = input.get("toolName")?.as_str()?;
    let mut unwrapped = payload.clone();
    unwrapped["tool_name"] = Value::String(name.to_string());
    unwrapped["tool_input"] = input.get("params").cloned().unwrap_or(Value::Null);
    Some(unwrapped)
}

fn extract_elicitation_questions(payload: &Value) -> Vec<WorkBuddyQuestion> {
    // MCP form schemas and URL prompts are reminders only; no form submission.
    let mut questions = extract_questions(Some(payload));
    if questions.is_empty() {
        questions.push(WorkBuddyQuestion {
            header: string_field(payload, "mcp_server_name"),
            question: payload
                .get("message")
                .and_then(Value::as_str)
                .map(|message| truncate(message, MAX_TEXT_CHARS))
                .unwrap_or_else(|| "WorkBuddy 需要补充信息，请前往对应窗口处理".into()),
            options: Vec::new(),
            multi_select: false,
            allow_other: false,
        });
    }
    questions
}

fn string_field(payload: &Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| value.chars().count() <= MAX_SESSION_ID_CHARS)
        .map(str::to_string)
        .filter(|value: &String| !value.trim().is_empty())
}

fn native_session_title(payload: &Value, event: &str) -> Option<String> {
    let direct = [
        "session_title",
        "sessionTitle",
        "conversation_title",
        "conversationTitle",
        "display_title",
        "displayTitle",
    ]
    .iter()
    .find_map(|key| {
        payload
            .get(*key)
            .and_then(Value::as_str)
            .and_then(compact_session_title)
    });
    let nested = ["session", "conversation"].iter().find_map(|key| {
        payload
            .get(*key)
            .and_then(Value::as_object)
            .and_then(|object| object.get("title"))
            .and_then(Value::as_str)
            .and_then(compact_session_title)
    });
    let session_start_title = (event == "SessionStart")
        .then(|| payload.get("title").and_then(Value::as_str))
        .flatten()
        .and_then(compact_session_title);
    direct.or(nested).or(session_start_title)
}

fn compact_session_title(value: &str) -> Option<String> {
    let safe = crate::workbuddy_redaction::text(value, MAX_PROMPT_CHARS);
    let normalized = safe.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return None;
    }
    if normalized.chars().count() <= MAX_TITLE_CHARS {
        return Some(normalized);
    }
    Some(format!(
        "{}…",
        normalized
            .chars()
            .take(MAX_TITLE_CHARS.saturating_sub(1))
            .collect::<String>()
    ))
}

fn prompt_session_title(value: &str) -> Option<String> {
    value
        .lines()
        .find(|line| !line.trim().is_empty())
        .and_then(compact_session_title)
}

fn fallback_session_title(session_id: &str) -> String {
    let chars = session_id.chars().collect::<Vec<_>>();
    let visible_id = if chars.len() <= 24 {
        session_id.to_string()
    } else {
        format!(
            "{}…{}",
            chars.iter().take(12).collect::<String>(),
            chars.iter().skip(chars.len() - 8).collect::<String>()
        )
    };
    format!("WorkBuddy 会话 · {visible_id}")
}

fn normalize_tool_name(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn apply_tool_activity(
    session: &mut WorkBuddySession,
    payload: &Value,
    captured_at: u64,
    status: &str,
) {
    let Some(tool) = string_field(payload, "tool_name") else {
        return;
    };
    if matches!(
        normalize_tool_name(&tool).as_str(),
        "askuserquestion" | "exitplanmode"
    ) {
        return;
    }
    let id = ["tool_use_id", "toolUseId", "request_id", "requestId"]
        .iter()
        .find_map(|key| string_field(payload, key))
        .unwrap_or_else(|| format!("{tool}-{captured_at}"));
    if let Some(activity) = session
        .activities
        .iter_mut()
        .find(|activity| activity.id == id)
    {
        activity.status = status.to_string();
        activity.updated_at = captured_at;
        return;
    }
    session.activities.push(WorkBuddyActivity {
        id,
        tool: tool.clone(),
        summary: tool_summary(&tool, payload.get("tool_input")),
        status: status.to_string(),
        started_at: captured_at,
        updated_at: captured_at,
    });
    if session.activities.len() > MAX_ACTIVITIES {
        let remove = session.activities.len() - MAX_ACTIVITIES;
        session.activities.drain(0..remove);
    }
}

fn append_tool_output(
    session: &mut WorkBuddySession,
    payload: &Value,
    captured_at: u64,
    failed: bool,
) {
    let value = if failed {
        payload
            .get("error")
            .or_else(|| payload.get("tool_error"))
            .or_else(|| payload.get("message"))
            .or_else(|| payload.get("tool_response"))
    } else {
        payload
            .get("tool_response")
            .or_else(|| payload.get("tool_result"))
    };
    let Some(value) = value else {
        return;
    };
    let text = readable_value(value);
    if text.trim().is_empty() {
        return;
    }
    append_output(session, &text, captured_at);
}

fn native_tool_failed(payload: &Value) -> bool {
    payload.get("tool_response").is_some_and(|response| {
        response
            .get("is_error")
            .or_else(|| response.get("isError"))
            .and_then(Value::as_bool)
            == Some(true)
    })
}

fn append_output(session: &mut WorkBuddySession, text: &str, captured_at: u64) {
    session.outputs.push(WorkBuddyOutput {
        id: format!("output-{captured_at}-{}", digest_text(text)),
        text: truncate(text, MAX_TEXT_CHARS),
    });
    if session.outputs.len() > MAX_OUTPUTS {
        let remove = session.outputs.len() - MAX_OUTPUTS;
        session.outputs.drain(0..remove);
    }
}

fn readable_value(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(values) => values
            .iter()
            .map(readable_value)
            .filter(|value| !value.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(object) => {
            for key in [
                "text", "content", "output", "stdout", "stderr", "message", "error",
            ] {
                if let Some(value) = object.get(key) {
                    let text = readable_value(value);
                    if !text.trim().is_empty() {
                        return text;
                    }
                }
            }
            serde_json::to_string(object).unwrap_or_default()
        }
    }
}

fn truncate(value: &str, limit: usize) -> String {
    crate::workbuddy_redaction::text(value, limit)
}

fn tool_summary(tool: &str, input: Option<&Value>) -> String {
    let preferred = ["command", "description", "path", "file_path", "prompt"];
    input
        .and_then(|input| {
            preferred.iter().find_map(|key| {
                input
                    .get(*key)
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
            })
        })
        .map(|value| truncate(value, 240))
        .unwrap_or_else(|| tool.to_string())
}

fn extract_questions(value: Option<&Value>) -> Vec<WorkBuddyQuestion> {
    let Some(items) = value
        .and_then(|value| value.get("questions").or_else(|| value.get("question")))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    items
        .iter()
        .take(16)
        .filter_map(|item| {
            let object = item.as_object()?;
            let question = object
                .get("question")
                .and_then(Value::as_str)
                .map(|value| truncate(value, MAX_TEXT_CHARS))?;
            let options = object
                .get("options")
                .and_then(Value::as_array)
                .map(|options| {
                    options
                        .iter()
                        .take(32)
                        .filter_map(|option| {
                            let object = option.as_object()?;
                            Some(WorkBuddyQuestionOption {
                                label: object
                                    .get("label")
                                    .and_then(Value::as_str)
                                    .map(|value| truncate(value, 512))?,
                                description: object
                                    .get("description")
                                    .and_then(Value::as_str)
                                    .map(|value| truncate(value, 2_000)),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some(WorkBuddyQuestion {
                header: object
                    .get("header")
                    .and_then(Value::as_str)
                    .map(|value| truncate(value, 160)),
                question,
                options,
                multi_select: object
                    .get("multiSelect")
                    .or_else(|| object.get("multi_select"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                allow_other: object
                    .get("allowOther")
                    .or_else(|| object.get("allow_other"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        })
        .collect()
}

fn extract_plan(value: Option<&Value>) -> Option<String> {
    let plan = value
        .and_then(|value| value.get("plan").or_else(|| value.get("content")))
        .and_then(Value::as_str)?;
    Some(truncate(plan, MAX_TEXT_CHARS))
}

fn trim_interactions(
    interactions: &mut HashMap<String, WorkBuddyInteraction>,
    session_id: &str,
) -> usize {
    let mut removed = 0;
    let mut keys = interactions
        .iter()
        .filter(|(_, i)| i.status != WorkBuddyRequestStatus::Pending)
        .map(|(key, i)| (key.clone(), i.captured_at, i.session_id == session_id))
        .collect::<Vec<_>>();
    keys.sort_by_key(|(_, captured_at, _)| *captured_at);
    let mut session_count = interactions
        .values()
        .filter(|i| i.session_id == session_id)
        .count();
    for (key, _, same_session) in keys {
        if (same_session && session_count > MAX_SESSION_INTERACTIONS)
            || interactions.len() > MAX_INTERACTIONS
        {
            interactions.remove(&key);
            removed += 1;
            if same_session {
                session_count -= 1;
            }
        }
    }
    removed
}

pub(crate) fn digest_text(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest[..12]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn question_notification(session: &str, generation: &str) -> Value {
        json!({
            "hook_event_name": "Notification",
            "notification_type": "permission_prompt",
            "message": "needs your permission to use AskUserQuestion",
            "session_id": session,
            "generation_id": generation,
        })
    }

    #[test]
    fn desktop_question_notification_is_visible_before_native_resolution() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../protocol/workbuddy/5.5.3/fixtures/question-notification.desktop.json"
        ))
        .unwrap();
        let hooks = fixture["hooks"].as_array().unwrap();
        let mut store = WorkBuddyStore::default();
        let notification_at = hooks[0]["receivedAt"].as_u64().unwrap();
        store.ingest(&hooks[0]["input"], "notification", notification_at);
        let waiting = store.snapshot();
        assert_eq!(waiting.sessions[0].stage, "waitingForInput");
        assert_eq!(waiting.sessions[0].pending_count, 1);
        assert_eq!(waiting.interactions.len(), 1);
        let reminder = &waiting.interactions[0];
        assert_eq!(reminder.kind, WorkBuddyRequestKind::Question);
        assert_eq!(reminder.status, WorkBuddyRequestStatus::Pending);
        assert!(!reminder.answerable);
        assert!(reminder.native_request_id.is_none());
        assert!(reminder.questions[0].question.contains("前往 WorkBuddy"));
        assert!(reminder.questions[0].options.is_empty());
        assert!(!waiting.capabilities.can_answer_questions);
        // The old implementation saw only a 168 ms Pre/PostToolUse window.
        // The actual notification must survive every 600 ms UI poll while the
        // native question is open, before any tool hook has arrived.
        let resolved_at = fixture["nativeResolutionAt"].as_u64().unwrap();
        for now in (notification_at + 600..resolved_at).step_by(600) {
            store.maintain(now);
            assert_eq!(
                store.snapshot().interactions[0].status,
                WorkBuddyRequestStatus::Pending
            );
        }
        store.ingest(
            &hooks[1]["input"],
            "pre",
            hooks[1]["receivedAt"].as_u64().unwrap(),
        );
        let enriched = store.snapshot();
        assert_eq!(enriched.interactions.len(), 1);
        assert_eq!(enriched.interactions[0].request_key, reminder.request_key);
        assert_eq!(enriched.interactions[0].captured_at, notification_at);
        assert_eq!(
            enriched.interactions[0].native_request_id.as_deref(),
            Some("native-question-1")
        );
        assert_eq!(
            enriched.interactions[0].questions[0].question,
            "<QUESTION_TEXT>"
        );
        store.ingest(
            &hooks[2]["input"],
            "post",
            hooks[2]["receivedAt"].as_u64().unwrap(),
        );
        let completed = store.snapshot();
        assert_eq!(completed.interactions.len(), 1);
        assert_eq!(completed.interactions[0].request_key, reminder.request_key);
        assert_eq!(
            completed.interactions[0].status,
            WorkBuddyRequestStatus::Completed
        );
        assert_eq!(completed.sessions[0].pending_count, 0);
        assert_eq!(completed.sessions[0].stage, "working");
    }

    #[test]
    fn repeated_question_notifications_preserve_the_reminder_and_later_details() {
        let mut store = WorkBuddyStore::default();
        let notification = question_notification("s", "turn");
        store.ingest(&notification, "notification", 1);
        store.ingest(&notification, "notification-again", 2);
        let key = store.snapshot().interactions[0].request_key.clone();
        let mut tool = json!({"hook_event_name":"PreToolUse", "session_id":"s",
            "generation_id":"turn", "tool_name":"AskUserQuestion", "tool_use_id":"first",
            "tool_input":{"questions":[{"question":"Which mode?", "options":[{"label":"Read"}]}]}});
        store.ingest(&tool, "pre", 3);
        store.ingest(&notification, "notification-after-pre", 4);
        let snapshot = store.snapshot();
        assert_eq!(snapshot.interactions.len(), 1);
        assert_eq!(snapshot.interactions[0].request_key, key);
        assert_eq!(
            snapshot.interactions[0].questions[0].options[0].label,
            "Read"
        );
        // A second real tool call must remain a separate question.
        tool["tool_use_id"] = json!("second");
        store.ingest(&tool, "second-pre", 5);
        store.ingest(&notification, "ambiguous-notification", 6);
        assert_eq!(store.snapshot().interactions.len(), 2);
        tool["hook_event_name"] = json!("PostToolUse");
        tool["tool_use_id"] = json!("first");
        store.ingest(&tool, "first-done", 7);
        assert_eq!(store.snapshot().sessions[0].pending_count, 1);
        assert_eq!(
            store.interactions[&key].status,
            WorkBuddyRequestStatus::Completed
        );
    }

    #[test]
    fn question_notification_does_not_duplicate_an_existing_tool_question() {
        let mut store = WorkBuddyStore::default();
        store.ingest(
            &json!({"hook_event_name":"PreToolUse", "session_id":"s",
            "generation_id":"turn", "tool_name":"AskUserQuestion", "tool_use_id":"call",
            "tool_input":{"questions":[{"question":"Which mode?"}]}}),
            "pre",
            1,
        );
        let key = store.snapshot().interactions[0].request_key.clone();
        store.ingest(&question_notification("s", "turn"), "notification", 2);
        let snapshot = store.snapshot();
        assert_eq!(snapshot.interactions.len(), 1);
        assert_eq!(snapshot.interactions[0].request_key, key);
        assert_eq!(
            snapshot.interactions[0].questions[0].question,
            "Which mode?"
        );
    }

    #[test]
    fn native_results_can_close_a_question_notification_without_pre_tool_use() {
        for (event, expected) in [
            ("PostToolUse", WorkBuddyRequestStatus::Completed),
            ("PermissionDenied", WorkBuddyRequestStatus::Denied),
            ("PostToolUseFailure", WorkBuddyRequestStatus::Failed),
            ("Stop", WorkBuddyRequestStatus::Cancelled),
            ("SessionEnd", WorkBuddyRequestStatus::Cancelled),
        ] {
            let mut store = WorkBuddyStore::default();
            store.ingest(&question_notification("s", "turn"), "notification", 1);
            store.ingest(
                &json!({"hook_event_name":event,"session_id":"s",
                "generation_id":"turn","tool_name":"AskUserQuestion","tool_use_id":"call"}),
                "result",
                2,
            );
            let snapshot = store.snapshot();
            assert_eq!(snapshot.interactions.len(), 1);
            assert_eq!(snapshot.interactions[0].status, expected, "{event}");
            assert_eq!(snapshot.sessions[0].pending_count, 0, "{event}");
        }
    }

    #[test]
    fn notification_matching_preserves_session_generation_and_native_id_boundaries() {
        let mut store = WorkBuddyStore::default();
        for (session, generation) in [("s", "one"), ("s", "two"), ("other", "one")] {
            store.ingest(
                &question_notification(session, generation),
                &format!("{session}-{generation}"),
                1,
            );
        }
        store.ingest(
            &json!({"hook_event_name":"PreToolUse", "session_id":"s",
            "generation_id":"two", "tool_name":"AskUserQuestion", "tool_use_id":"second"}),
            "pre",
            2,
        );
        assert_eq!(store.interactions.len(), 3);
        // An id-less result must use the generation as well as the tool name.
        store.ingest(
            &json!({"hook_event_name":"PermissionDenied", "session_id":"s",
            "generation_id":"one", "tool_name":"AskUserQuestion"}),
            "denied",
            3,
        );
        assert_eq!(
            store
                .interactions
                .values()
                .filter(|i| i.status == WorkBuddyRequestStatus::Denied)
                .count(),
            1
        );
        assert_eq!(
            store
                .interactions
                .values()
                .filter(|i| i.status == WorkBuddyRequestStatus::Pending)
                .count(),
            2
        );

        let mut notification = question_notification("identified", "turn");
        notification["tool_use_id"] = json!("first");
        store.ingest(&notification, "identified-notification", 4);
        store.ingest(
            &json!({"hook_event_name":"PostToolUse", "session_id":"identified",
            "generation_id":"turn", "tool_name":"AskUserQuestion", "tool_use_id":"different"}),
            "unrelated",
            5,
        );
        assert_eq!(
            store
                .interactions
                .values()
                .filter(|i| i.status == WorkBuddyRequestStatus::Pending)
                .count(),
            3
        );
    }

    #[test]
    fn ambiguous_question_notifications_are_not_resolved_by_guessing() {
        let mut store = WorkBuddyStore::default();
        for id in ["one", "two"] {
            let mut notification = question_notification("s", "turn");
            notification["tool_use_id"] = json!(id);
            store.ingest(&notification, id, 1);
        }
        store.ingest(
            &json!({"hook_event_name":"PermissionDenied", "session_id":"s",
            "generation_id":"turn", "tool_name":"AskUserQuestion"}),
            "ambiguous",
            2,
        );
        assert_eq!(store.snapshot().sessions[0].pending_count, 2);
        assert_eq!(store.snapshot().diagnostics.ambiguous_completions, 1);
    }

    #[test]
    fn only_question_permission_notifications_create_question_reminders() {
        let mut store = WorkBuddyStore::default();
        for (kind, message) in [
            (
                "idle_prompt",
                "needs your permission to use AskUserQuestion",
            ),
            (
                "auth_success",
                "needs your permission to use AskUserQuestion",
            ),
            ("permission_prompt", "needs your permission to use Bash"),
            (
                "permission_prompt",
                "needs your permission to use DeferExecuteTool",
            ),
            ("permission_prompt", "AskUserQuestion finished"),
        ] {
            let mut notification = question_notification("s", "turn");
            notification["notification_type"] = json!(kind);
            notification["message"] = json!(message);
            store.ingest(&notification, &format!("{kind}-{message}"), 1);
        }
        assert!(store.snapshot().interactions.is_empty());
        assert_eq!(store.snapshot().sessions[0].pending_count, 0);
        // Honor a structured tool name too, without depending on English text.
        store.ingest(
            &json!({"hook_event_name":"Notification", "notification_type":"permission_prompt",
            "session_id":"s", "tool_name":"AskUserQuestion", "message":"等待回答"}),
            "structured",
            2,
        );
        assert_eq!(store.snapshot().sessions[0].pending_count, 1);
    }

    #[test]
    fn three_review_kinds_wait_for_their_native_results() {
        for (tool, event, kind) in [
            (
                "AskUserQuestion",
                "PreToolUse",
                WorkBuddyRequestKind::Question,
            ),
            ("ExitPlanMode", "PreToolUse", WorkBuddyRequestKind::Plan),
            ("Bash", "PermissionRequest", WorkBuddyRequestKind::Tool),
        ] {
            let mut store = WorkBuddyStore::default();
            store.ingest(
                &json!({"hook_event_name":event,"session_id":"s","tool_name":tool,
                "tool_use_id":"request"}),
                "request",
                1,
            );
            let snapshot = store.snapshot();
            assert_eq!(snapshot.interactions[0].kind, kind);
            assert_eq!(
                snapshot.interactions[0].status,
                WorkBuddyRequestStatus::Pending
            );
            assert!(!snapshot.interactions[0].answerable);
            assert_eq!(snapshot.sessions[0].pending_count, 1);
            assert_eq!(snapshot.sessions[0].stage, "waitingForInput");
            // A long-running user decision must not close on an arbitrary timer.
            store.maintain(INTERACTION_TTL_MS + 2);
            assert_eq!(
                store.snapshot().interactions[0].status,
                WorkBuddyRequestStatus::Pending
            );
            store.ingest(
                &json!({"hook_event_name":"PostToolUse","session_id":"s","tool_name":tool,
                "tool_use_id":"request"}),
                "done",
                INTERACTION_TTL_MS + 3,
            );
            assert_eq!(
                store.snapshot().interactions[0].status,
                WorkBuddyRequestStatus::Completed
            );
            assert_eq!(store.snapshot().sessions[0].pending_count, 0);
        }
    }

    #[test]
    fn ordinary_tools_only_open_reviews_when_permission_is_requested() {
        let mut store = WorkBuddyStore::default();
        let mut payload = json!({"hook_event_name":"PreToolUse","session_id":"s",
            "tool_name":"Bash","tool_use_id":"call","tool_input":{"command":"echo hello"}});
        store.ingest(&payload, "pre", 1);
        let original = store.snapshot().interactions[0].request_key.clone();
        assert_eq!(store.snapshot().sessions[0].pending_count, 0);
        payload["hook_event_name"] = json!("PermissionRequest");
        store.ingest(&payload, "permission", 2);
        store.ingest(&payload, "duplicate", 3);
        let snapshot = store.snapshot();
        assert_eq!(snapshot.interactions.len(), 1);
        assert_eq!(snapshot.interactions[0].request_key, original);
        assert_eq!(
            snapshot.interactions[0].status,
            WorkBuddyRequestStatus::Pending
        );
        store.ingest(
            &json!({"hook_event_name":"PermissionDenied","session_id":"s",
            "tool_use_id":"call","tool_name":"Bash"}),
            "denied",
            4,
        );
        assert_eq!(
            store.snapshot().interactions[0].status,
            WorkBuddyRequestStatus::Denied
        );
        assert_eq!(store.snapshot().sessions[0].pending_count, 0);
    }

    #[test]
    fn legacy_permission_hook_promotes_only_the_matching_tool() {
        let mut store = WorkBuddyStore::default();
        for (index, input) in ["first", "second"].iter().enumerate() {
            store.ingest(
                &json!({"hook_event_name":"PreToolUse","session_id":"s","tool_name":"Bash",
                "tool_input":{"command":input}}),
                input,
                index as u64 + 1,
            );
        }
        store.ingest(
            &json!({"hook_event_name":"PermissionRequest","session_id":"s","tool_name":"Bash",
            "tool_input":{"command":"second"}}),
            "permission",
            3,
        );
        assert_eq!(store.snapshot().interactions.len(), 2);
        assert_eq!(store.snapshot().sessions[0].pending_count, 1);
    }

    #[test]
    fn deferred_question_uses_the_question_page_and_inner_result() {
        let mut store = WorkBuddyStore::default();
        store.ingest(
            &json!({"hook_event_name":"PreToolUse","session_id":"s","tool_name":"DeferExecuteTool",
            "call_id":"call", "tool_input":{"toolName":"AskUserQuestion","params":{
                "questions":[{"question":"Which mode?","options":[{"label":"Read"}]}]}}}),
            "pre",
            1,
        );
        let snapshot = store.snapshot();
        assert_eq!(
            snapshot.interactions[0].kind,
            WorkBuddyRequestKind::Question
        );
        assert_eq!(
            snapshot.interactions[0].questions[0].question,
            "Which mode?"
        );
        store.ingest(
            &json!({"hook_event_name":"PostToolUse","session_id":"s","tool_name":"AskUserQuestion",
            "call_id":"call"}),
            "done",
            2,
        );
        assert_eq!(
            store.snapshot().interactions[0].status,
            WorkBuddyRequestStatus::Completed
        );
    }

    #[test]
    fn elicitation_result_closes_only_its_question_and_preserves_other_sessions() {
        for (action, expected) in [
            ("accept", WorkBuddyRequestStatus::Completed),
            ("decline", WorkBuddyRequestStatus::Denied),
            ("cancel", WorkBuddyRequestStatus::Cancelled),
        ] {
            let mut store = WorkBuddyStore::default();
            for (session, id) in [("s", "one"), ("s", "two"), ("other", "one")] {
                store.ingest(
                    &json!({"hook_event_name":"Elicitation","session_id":session,
                    "mcp_server_name":"forms","elicitation_id":id,"message":"Select a mode"}),
                    &format!("{session}-{id}"),
                    1,
                );
            }
            assert!(store
                .snapshot()
                .interactions
                .iter()
                .all(|i| i.kind == WorkBuddyRequestKind::Question
                    && i.questions[0].question == "Select a mode"));
            store.ingest(
                &json!({"hook_event_name":"ElicitationResult","session_id":"s",
                "mcp_server_name":"forms","elicitation_id":"one","action":action}),
                "result",
                2,
            );
            let snapshot = store.snapshot();
            assert_eq!(
                snapshot
                    .interactions
                    .iter()
                    .filter(|i| i.status == expected)
                    .count(),
                1
            );
            assert_eq!(
                snapshot
                    .interactions
                    .iter()
                    .filter(|i| i.status == WorkBuddyRequestStatus::Pending)
                    .count(),
                2
            );
            assert_eq!(
                snapshot
                    .sessions
                    .iter()
                    .map(|s| s.pending_count)
                    .sum::<usize>(),
                2
            );
        }
    }

    #[test]
    fn concurrent_legacy_elicitations_are_not_closed_by_an_ambiguous_result() {
        let mut store = WorkBuddyStore::default();
        for hash in ["one", "two"] {
            store.ingest(
                &json!({"hook_event_name":"Elicitation","session_id":"s",
                "mcp_server_name":"forms","message":hash}),
                hash,
                1,
            );
        }
        store.ingest(
            &json!({"hook_event_name":"ElicitationResult","session_id":"s",
            "mcp_server_name":"forms","action":"accept"}),
            "result",
            2,
        );
        assert_eq!(store.snapshot().sessions[0].pending_count, 2);
        assert_eq!(store.snapshot().diagnostics.ambiguous_completions, 1);
    }

    #[test]
    fn native_turn_end_clears_reminders_but_subagent_stop_does_not() {
        for event in ["Stop", "SessionEnd"] {
            let mut store = WorkBuddyStore::default();
            store.ingest(
                &json!({"hook_event_name":"PreToolUse","session_id":"s",
                "tool_name":"AskUserQuestion"}),
                "question",
                1,
            );
            store.ingest(
                &json!({"hook_event_name":"SubagentStop","session_id":"s"}),
                "subagent",
                2,
            );
            assert_eq!(store.snapshot().sessions[0].pending_count, 1);
            store.ingest(&json!({"hook_event_name":event,"session_id":"s"}), "end", 3);
            assert_eq!(
                store.snapshot().interactions[0].status,
                WorkBuddyRequestStatus::Cancelled
            );
            assert_eq!(store.snapshot().sessions[0].pending_count, 0);
        }
    }

    #[test]
    fn waiting_requests_remain_bounded_without_evicting_an_older_reminder() {
        let mut store = WorkBuddyStore::default();
        for index in 0..MAX_SESSION_INTERACTIONS + 2 {
            store.ingest(&json!({"hook_event_name":"PreToolUse","session_id":"s","tool_name":"AskUserQuestion",
                "tool_use_id":index.to_string()}), &index.to_string(), index as u64 + 1);
        }
        let snapshot = store.snapshot();
        assert_eq!(snapshot.interactions.len(), MAX_SESSION_INTERACTIONS);
        assert_eq!(snapshot.sessions[0].pending_count, MAX_SESSION_INTERACTIONS);
        assert!(snapshot
            .interactions
            .iter()
            .any(|i| i.native_request_id.as_deref() == Some("0")));
        assert_eq!(snapshot.diagnostics.dropped_interactions, 2);
    }

    #[test]
    fn real_cli_fixture_replays_preserve_native_completion_and_errors() {
        for (text, expected) in [
            (
                include_str!(
                    "../../protocol/workbuddy/5.5.3/fixtures/runtime/tool-allow.print.runtime.json"
                ),
                WorkBuddyRequestStatus::Completed,
            ),
            (
                include_str!(
                    "../../protocol/workbuddy/5.5.3/fixtures/runtime/failure.print.runtime.json"
                ),
                WorkBuddyRequestStatus::Failed,
            ),
        ] {
            let fixture: Value = serde_json::from_str(text).unwrap();
            let mut store = WorkBuddyStore::default();
            for (index, hook) in fixture["hooks"].as_array().unwrap().iter().enumerate() {
                store.ingest(
                    &crate::workbuddy_hook::sanitize_payload(&hook["input"]),
                    &index.to_string(),
                    index as u64 + 1,
                );
            }
            let snapshot = store.snapshot();
            assert_eq!(snapshot.sessions.len(), 1);
            assert_eq!(snapshot.interactions[0].status, expected);
            assert!(!snapshot.capabilities.protocol_frozen);
            assert!(!snapshot.interactions[0].answerable);
        }
    }

    #[test]
    fn identical_native_ids_in_different_processes_are_isolated() {
        let mut store = WorkBuddyStore::default();
        for pid in ["100:1", "101:2"] {
            store.ingest(&json!({"hook_event_name":"PreToolUse","session_id":"same","process_instance_id":pid,
                "plugin_instance_id":"plugin","cwd_hash":"cwd","tool_name":"Read","tool_use_id":"same-call"}),pid,10);
        }
        let snapshot = store.snapshot();
        assert_eq!(snapshot.sessions.len(), 2);
        assert_ne!(snapshot.sessions[0].id, snapshot.sessions[1].id);
        assert_ne!(
            snapshot.interactions[0].session_id,
            snapshot.interactions[1].session_id
        );
        assert!(snapshot
            .sessions
            .iter()
            .all(|s| s.workbuddy_session_id == "same"));
    }

    #[test]
    fn session_start_without_cwd_is_enriched_but_conflicting_cwd_is_quarantined() {
        let mut store = WorkBuddyStore::default();
        store.ingest(
            &json!({"hook_event_name":"SessionStart","session_id":"s"}),
            "start",
            1,
        );
        store.ingest(
            &json!({"hook_event_name":"UserPromptSubmit","session_id":"s","cwd_hash":"first"}),
            "prompt",
            2,
        );
        store.ingest(&json!({"hook_event_name":"PreToolUse","session_id":"s","cwd_hash":"other","tool_name":"Write"}),"wrong",3);
        assert_eq!(store.snapshot().sessions.len(), 1);
        assert_eq!(
            store.snapshot().sessions[0].cwd_hash.as_deref(),
            Some("first")
        );
        assert!(store.snapshot().interactions.is_empty());
    }

    #[test]
    fn native_error_results_and_duplicate_pre_hooks_match_real_runtime() {
        let mut store = WorkBuddyStore::default();
        let payload = json!({"hook_event_name":"PreToolUse","session_id":"s","tool_name":"Read","tool_use_id":"call"});
        store.ingest(&payload, "first", 1);
        store.ingest(&payload, "duplicate-invocation", 2);
        store.ingest(&json!({"hook_event_name":"PostToolUse","session_id":"s","tool_name":"Read","tool_use_id":"call",
            "tool_response":{"is_error":true,"tool_error_code":"1001"}}),"result",3);
        let snapshot = store.snapshot();
        assert_eq!(snapshot.interactions.len(), 1);
        assert_eq!(
            snapshot.interactions[0].status,
            WorkBuddyRequestStatus::Failed
        );
        assert_eq!(snapshot.sessions[0].stage, "toolFailed");
    }

    #[test]
    fn ended_sessions_and_out_of_order_events_cannot_create_interactions() {
        let mut store = WorkBuddyStore::default();
        store.ingest(
            &json!({"hook_event_name":"SessionEnd","session_id":"s"}),
            "end",
            100,
        );
        for (hash, at) in [("late", 101), ("old", 90)] {
            store.ingest(
                &json!({"hook_event_name":"PreToolUse","session_id":"s","tool_name":"Write"}),
                hash,
                at,
            );
        }
        assert!(store.snapshot().interactions.is_empty());
        assert_eq!(store.snapshot().sessions[0].stage, "stopped");
        assert_eq!(store.snapshot().diagnostics.late_events, 2);
    }

    #[test]
    fn all_active_sessions_and_observations_stay_bounded() {
        let mut store = WorkBuddyStore::default();
        for i in 0..MAX_SESSIONS + 12 {
            store.ingest(
                &json!({"hook_event_name":"SessionStart","session_id":format!("s{i}")}),
                &format!("s{i}"),
                1,
            );
        }
        assert_eq!(store.snapshot().sessions.len(), MAX_SESSIONS);
        assert_eq!(store.snapshot().diagnostics.dropped_sessions, 12);
        for i in 0..MAX_SEEN_PAYLOADS + 4 {
            store.ingest(
                &json!({"hook_event_name":"PreToolUse","session_id":"s0","tool_name":"Read"}),
                &format!("t{i}"),
                i as u64 + 2,
            );
        }
        assert_eq!(store.interactions.len(), MAX_SESSION_INTERACTIONS);
        assert_eq!(store.seen_payloads.len(), MAX_SEEN_PAYLOADS);
        assert_eq!(store.seen_order.len(), MAX_SEEN_PAYLOADS);
        assert!(store.snapshot().diagnostics.dropped_interactions > 0);
    }

    #[test]
    fn concurrent_tools_without_native_ids_are_not_guessed() {
        let mut store = WorkBuddyStore::default();
        let payload = json!({"hook_event_name":"PreToolUse","session_id":"s","tool_name":"Read"});
        store.ingest(&payload, "first", 1);
        store.ingest(&payload, "second", 2);
        store.ingest(
            &json!({"hook_event_name":"PostToolUse","session_id":"s","tool_name":"Read"}),
            "done",
            3,
        );
        assert_eq!(store.snapshot().diagnostics.ambiguous_completions, 1);
        assert!(store
            .interactions
            .values()
            .all(|i| i.status == WorkBuddyRequestStatus::Unavailable));
    }

    #[test]
    fn prompts_outputs_and_turn_end_keep_their_true_meaning() {
        let mut store = WorkBuddyStore::default();
        store.ingest(&json!({"hook_event_name":"UserPromptSubmit","session_id":"s","prompt":"p".repeat(400)}),"prompt",1);
        store.ingest(
            &json!({"hook_event_name":"PreToolUse","session_id":"s","tool_name":"Read"}),
            "tool",
            2,
        );
        store.ingest(&json!({"hook_event_name":"Stop","session_id":"s","last_assistant_message":"finished TOKEN=secret-value"}),"stop",3);
        store.maintain(INTERACTION_TTL_MS + 3);
        let snapshot = store.snapshot();
        assert_eq!(
            snapshot.sessions[0].last_prompt.as_ref().unwrap().len(),
            400
        );
        assert!(!snapshot.sessions[0].outputs[0]
            .text
            .contains("secret-value"));
        assert_eq!(
            snapshot.interactions[0].status,
            WorkBuddyRequestStatus::Cancelled
        );
    }

    #[test]
    fn session_titles_are_stable_and_native_metadata_has_priority() {
        let mut store = WorkBuddyStore::default();
        store.ingest(
            &json!({
                "hook_event_name": "UserPromptSubmit",
                "session_id": "native-session",
                "prompt": "Investigate the login failure\nThen run the tests"
            }),
            "prompt-one",
            1,
        );
        let generated = store.snapshot();
        assert_eq!(generated.sessions[0].title, "Investigate the login failure");
        assert_eq!(
            generated.sessions[0].title_source,
            WorkBuddyTitleSource::Prompt
        );

        store.ingest(
            &json!({
                "hook_event_name": "UserPromptSubmit",
                "session_id": "native-session",
                "prompt": "A later prompt must not rename the session"
            }),
            "prompt-two",
            2,
        );
        assert_eq!(
            store.snapshot().sessions[0].title,
            "Investigate the login failure"
        );

        store.ingest(
            &json!({
                "hook_event_name": "SessionStart",
                "session_id": "native-session",
                "session_title": "  WorkBuddy login repair  "
            }),
            "native-title",
            3,
        );
        let native = store.snapshot();
        assert_eq!(native.sessions[0].title, "WorkBuddy login repair");
        assert_eq!(
            native.sessions[0].title_source,
            WorkBuddyTitleSource::Native
        );
    }

    #[test]
    fn session_title_falls_back_to_a_short_native_id() {
        let mut store = WorkBuddyStore::default();
        store.ingest(
            &json!({
                "hook_event_name": "SessionStart",
                "session_id": "123456789012345678901234567890"
            }),
            "session-start",
            1,
        );
        let snapshot = store.snapshot();
        assert_eq!(
            snapshot.sessions[0].title,
            "WorkBuddy 会话 · 123456789012…34567890"
        );
        assert_eq!(
            snapshot.sessions[0].title_source,
            WorkBuddyTitleSource::Fallback
        );
    }

    #[test]
    fn newer_plan_observations_supersede_unverified_older_versions() {
        let mut store = WorkBuddyStore::default();
        for (i, plan) in ["version one", "version two"].iter().enumerate() {
            store.ingest(&json!({"hook_event_name":"PreToolUse","session_id":"s","tool_name":"ExitPlanMode","tool_input":{"plan":plan}}),plan,i as u64 + 1);
        }
        let snapshot = store.snapshot();
        assert_eq!(
            snapshot.interactions[1].status,
            WorkBuddyRequestStatus::Superseded
        );
        assert_ne!(
            snapshot.interactions[0].plan_hash,
            snapshot.interactions[1].plan_hash
        );
        assert!(snapshot.interactions.iter().all(|i| !i.answerable));
        assert_eq!(
            snapshot.interactions[0].plan_source.as_deref(),
            Some("hookInputUnverified")
        );
    }

    #[test]
    fn duplicate_payloads_are_idempotent() {
        let mut store = WorkBuddyStore::default();
        let payload = json!({
            "hook_event_name": "SessionStart",
            "session_id": "session-1",
            "cwd_hash": "abc"
        });
        store.ingest(&payload, "same-hash", 10);
        store.ingest(&payload, "same-hash", 11);
        let snapshot = store.snapshot();
        assert_eq!(snapshot.version, 2); // Diagnostic counter is a snapshot change.
        assert_eq!(snapshot.observed_event_count, 1);
        assert_eq!(snapshot.sessions[0].event_count, 1);
    }

    #[test]
    fn late_events_do_not_regress_session_state() {
        let mut store = WorkBuddyStore::default();
        store.ingest(
            &json!({
                "hook_event_name": "PostToolUse",
                "session_id": "session-1",
                "tool_name": "Read"
            }),
            "new",
            20,
        );
        store.ingest(
            &json!({
                "hook_event_name": "SessionStart",
                "session_id": "session-1"
            }),
            "old",
            10,
        );
        let snapshot = store.snapshot();
        assert_eq!(snapshot.sessions[0].stage, "working");
        assert_eq!(snapshot.sessions[0].updated_at, 20);
    }

    #[test]
    fn pre_tool_use_creates_a_redacted_read_only_interaction() {
        let mut store = WorkBuddyStore::default();
        store.ingest(
            &json!({
                "hook_event_name": "PreToolUse",
                "session_id": "session-1",
                "tool_name": "AskUserQuestion",
                "tool_use_id": "native-1",
                "tool_input": {
                    "questions": [{
                        "header": "范围",
                        "question": "选择范围",
                        "multiSelect": true,
                        "allowOther": true,
                        "options": [{"label": "源代码", "description": "修改实现"}]
                    }]
                }
            }),
            "payload-1",
            100,
        );
        let snapshot = store.snapshot();
        assert_eq!(snapshot.interactions.len(), 1);
        let interaction = &snapshot.interactions[0];
        assert_eq!(interaction.kind, WorkBuddyRequestKind::Question);
        assert!(!interaction.answerable);
        assert_eq!(interaction.questions[0].options[0].label, "源代码");
        assert_eq!(snapshot.sessions[0].stage, "waitingForInput");
    }

    #[test]
    fn post_tool_use_resolves_the_matching_observation() {
        let mut store = WorkBuddyStore::default();
        store.ingest(
            &json!({
                "hook_event_name": "PreToolUse",
                "session_id": "session-1",
                "tool_name": "Read",
                "tool_use_id": "native-1"
            }),
            "payload-1",
            100,
        );
        store.ingest(
            &json!({
                "hook_event_name": "PostToolUse",
                "session_id": "session-1",
                "tool_name": "Read",
                "tool_use_id": "native-1"
            }),
            "payload-2",
            101,
        );
        assert_eq!(
            store.snapshot().interactions[0].status,
            WorkBuddyRequestStatus::Completed
        );
    }

    #[test]
    fn observations_never_offer_approval_capabilities() {
        let mut store = WorkBuddyStore::default();
        store.ingest(
            &json!({
                "hook_event_name": "PreToolUse",
                "session_id": "session-1",
                "tool_name": "Read",
                "tool_use_id": "native-1"
            }),
            "payload-1",
            100,
        );
        let snapshot = store.snapshot();
        assert!(!snapshot.capabilities.can_approve_tools);
        assert!(!snapshot.capabilities.can_answer_questions);
        assert!(!snapshot.capabilities.can_approve_plans);
        assert!(snapshot
            .interactions
            .iter()
            .all(|request| !request.answerable));
    }
}
