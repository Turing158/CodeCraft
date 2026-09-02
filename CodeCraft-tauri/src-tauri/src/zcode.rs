use std::{
    collections::HashMap,
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use serde_json::Value;

use super::zcode_hook::{
    self, ZCodeApprovalDecision, ZCodeDecisionEnvelope, ZCodeHookEnvelope, ZCodeHookPhase,
    ZCodeQuestionAnswer,
};

const SESSION_TTL_MS: u64 = 24 * 60 * 60 * 1_000;
const MAX_SESSIONS: usize = 100;
const MAX_ACTIVITIES: usize = 100;
const MAX_OUTPUTS: usize = 40;
const MAX_OUTPUT_CHARS: usize = 8_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ZCodeSessionStatus {
    Working,
    WaitingForInput,
    WaitingForApproval,
    ToolRunning,
    ToolFailed,
    Stopped,
    Idle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ZCodeReviewState {
    Pending,
    Submitted,
    ReturnedToZCode,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ZCodeActivity {
    pub(crate) id: String,
    pub(crate) tool: String,
    pub(crate) summary: String,
    pub(crate) status: String,
    pub(crate) started_at: u64,
    pub(crate) updated_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ZCodeOutput {
    pub(crate) id: String,
    pub(crate) text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ZCodeQuestionOption {
    pub(crate) label: String,
    pub(crate) description: Option<String>,
    pub(crate) preview: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ZCodeQuestion {
    pub(crate) header: Option<String>,
    pub(crate) question: String,
    pub(crate) options: Vec<ZCodeQuestionOption>,
    pub(crate) multi_select: bool,
    pub(crate) allow_other: bool,
    pub(crate) allow_chat: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ZCodeQuestionRequest {
    pub(crate) id: String,
    pub(crate) session_id: String,
    pub(crate) questions: Vec<ZCodeQuestion>,
    pub(crate) captured_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ZCodePermissionRequest {
    pub(crate) id: String,
    pub(crate) session_id: String,
    pub(crate) tool_name: String,
    pub(crate) summary: String,
    pub(crate) cwd: Option<String>,
    pub(crate) can_always_allow: bool,
    pub(crate) captured_at: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ZCodePlanRequest {
    pub(crate) id: String,
    pub(crate) session_id: String,
    pub(crate) tool_name: String,
    pub(crate) plan: String,
    pub(crate) allowed_prompts: Option<Value>,
    pub(crate) cwd: Option<String>,
    pub(crate) captured_at: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ZCodeSession {
    pub(crate) id: String,
    pub(crate) status: ZCodeSessionStatus,
    pub(crate) title: String,
    pub(crate) cwd: Option<String>,
    pub(crate) started_at: u64,
    pub(crate) updated_at: u64,
    pub(crate) activities: Vec<ZCodeActivity>,
    pub(crate) outputs: Vec<ZCodeOutput>,
    pub(crate) question: Option<ZCodeQuestionRequest>,
    pub(crate) permission: Option<ZCodePermissionRequest>,
    pub(crate) plan: Option<ZCodePlanRequest>,
    pub(crate) review_state: Option<ZCodeReviewState>,
}

impl ZCodeSession {
    pub(crate) fn is_active(&self) -> bool {
        !matches!(
            self.status,
            ZCodeSessionStatus::Idle | ZCodeSessionStatus::Stopped
        )
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ZCodeCapabilities {
    pub(crate) observation: bool,
    pub(crate) tool_approval: bool,
    pub(crate) question_answer: bool,
    pub(crate) plan_review: bool,
    pub(crate) plan_feedback: bool,
    pub(crate) allow_always: bool,
    pub(crate) streaming_answer: bool,
    pub(crate) question_sound: bool,
}

impl Default for ZCodeCapabilities {
    fn default() -> Self {
        Self {
            observation: true,
            tool_approval: true,
            question_answer: true,
            plan_review: true,
            plan_feedback: true,
            allow_always: false,
            streaming_answer: false,
            question_sound: false,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ZCodeSnapshot {
    pub(crate) connected: bool,
    pub(crate) integration_error: Option<String>,
    pub(crate) detected_path: Option<String>,
    pub(crate) detected_version: Option<String>,
    pub(crate) capabilities: ZCodeCapabilities,
    pub(crate) sessions: Vec<ZCodeSession>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingKind {
    Permission,
    Question,
    Plan,
}

#[derive(Clone, Debug)]
struct PendingRequest {
    session_id: String,
    tool_use_id: Option<String>,
    kind: PendingKind,
}

#[derive(Default)]
pub(crate) struct ZCodeStore {
    sessions: HashMap<String, ZCodeSession>,
    pending: HashMap<String, PendingRequest>,
    integration_error: Option<String>,
}

impl ZCodeStore {
    pub(crate) fn clear(&mut self) {
        self.sessions.clear();
        self.pending.clear();
    }

    pub(crate) fn set_integration_error(&mut self, error: Option<String>) {
        self.integration_error = error;
    }

    pub(crate) fn drain_inbox(&mut self) -> Result<(), String> {
        let inbox = zcode_hook::inbox_dir();
        if inbox.exists() {
            let mut paths = fs::read_dir(&inbox)
                .map_err(|error| error.to_string())?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
                .collect::<Vec<_>>();
            paths.sort();
            for path in paths {
                if let Ok(bytes) = fs::read(&path) {
                    if let Ok(envelope) = serde_json::from_slice::<ZCodeHookEnvelope>(&bytes) {
                        self.apply_envelope(envelope);
                    }
                }
                let _ = fs::remove_file(path);
            }
        }
        let stale_before = now_ms().saturating_sub(SESSION_TTL_MS);
        self.sessions
            .retain(|_, session| session.updated_at >= stale_before);
        self.pending
            .retain(|_, pending| self.sessions.contains_key(&pending.session_id));
        trim_sessions(&mut self.sessions);
        Ok(())
    }

    pub(crate) fn snapshot(
        &self,
        detected_path: Option<String>,
        detected_version: Option<String>,
    ) -> ZCodeSnapshot {
        let mut sessions = self.sessions.values().cloned().collect::<Vec<_>>();
        sessions.sort_by(|left, right| {
            status_rank(left.status)
                .cmp(&status_rank(right.status))
                .then_with(|| right.updated_at.cmp(&left.updated_at))
        });
        ZCodeSnapshot {
            connected: self.integration_error.is_none(),
            integration_error: self.integration_error.clone(),
            detected_path,
            detected_version,
            capabilities: ZCodeCapabilities::default(),
            sessions,
        }
    }

    pub(crate) fn submit_permission(
        &mut self,
        session_id: &str,
        request_id: &str,
        decision: ZCodeApprovalDecision,
        message: Option<String>,
    ) -> Result<(), String> {
        let pending = self.validate_pending(session_id, request_id, PendingKind::Permission)?;
        zcode_hook::submit_decision(&ZCodeDecisionEnvelope::Permission {
            request_id: request_id.to_string(),
            session_id: session_id.to_string(),
            tool_use_id: pending.tool_use_id.clone(),
            decision,
            message: normalized_optional(message),
        })?;
        self.mark_submitted(request_id);
        Ok(())
    }

    pub(crate) fn submit_question(
        &mut self,
        session_id: &str,
        request_id: &str,
        answers: Vec<ZCodeQuestionAnswer>,
        annotations: Option<Value>,
    ) -> Result<(), String> {
        let pending = self.validate_pending(session_id, request_id, PendingKind::Question)?;
        let request = self
            .sessions
            .get(session_id)
            .and_then(|session| session.question.as_ref())
            .ok_or_else(|| "The ZCode question is no longer pending".to_string())?;
        validate_answers(request, &answers)?;
        zcode_hook::submit_decision(&ZCodeDecisionEnvelope::Question {
            request_id: request_id.to_string(),
            session_id: session_id.to_string(),
            tool_use_id: pending.tool_use_id.clone(),
            answers,
            annotations,
        })?;
        self.mark_submitted(request_id);
        Ok(())
    }

    pub(crate) fn submit_plan(
        &mut self,
        session_id: &str,
        request_id: &str,
        approved: bool,
        feedback: Option<String>,
    ) -> Result<(), String> {
        let pending = self.validate_pending(session_id, request_id, PendingKind::Plan)?;
        let feedback = normalized_optional(feedback);
        if !approved && feedback.is_none() {
            return Err("Plan feedback cannot be empty".to_string());
        }
        zcode_hook::submit_decision(&ZCodeDecisionEnvelope::Plan {
            request_id: request_id.to_string(),
            session_id: session_id.to_string(),
            tool_use_id: pending.tool_use_id.clone(),
            approved,
            feedback,
        })?;
        self.mark_submitted(request_id);
        Ok(())
    }

    fn validate_pending(
        &self,
        session_id: &str,
        request_id: &str,
        kind: PendingKind,
    ) -> Result<&PendingRequest, String> {
        let pending = self.pending.get(request_id).ok_or_else(|| {
            match self
                .sessions
                .get(session_id)
                .and_then(|session| session.review_state)
            {
                Some(ZCodeReviewState::ReturnedToZCode) => {
                    "The ZCode request was returned to ZCode".to_string()
                }
                Some(ZCodeReviewState::Submitted) => {
                    "The ZCode request was already submitted".to_string()
                }
                _ => "The ZCode request has expired".to_string(),
            }
        })?;
        if pending.session_id != session_id || pending.kind != kind {
            return Err("The ZCode decision does not belong to this request".to_string());
        }
        Ok(pending)
    }

    fn mark_submitted(&mut self, request_id: &str) {
        let Some(pending) = self.pending.remove(request_id) else {
            return;
        };
        if let Some(session) = self.sessions.get_mut(&pending.session_id) {
            clear_request(session, request_id);
            session.status = ZCodeSessionStatus::Working;
            session.review_state = Some(ZCodeReviewState::Submitted);
            session.updated_at = now_ms();
        }
    }

    fn apply_envelope(&mut self, envelope: ZCodeHookEnvelope) {
        let payload = &envelope.payload;
        let Some(session_id) = string_field(payload, "session_id") else {
            return;
        };
        let Some(event) = string_field(payload, "hook_event_name") else {
            return;
        };
        let captured_at = envelope.captured_at;
        let fallback_title = string_field(payload, "cwd")
            .and_then(directory_title)
            .unwrap_or_else(|| "ZCode 会话".to_string());
        let session = self
            .sessions
            .entry(session_id.to_string())
            .or_insert_with(|| ZCodeSession {
                id: session_id.to_string(),
                status: ZCodeSessionStatus::Idle,
                title: fallback_title,
                cwd: string_field(payload, "cwd").map(str::to_string),
                started_at: captured_at,
                updated_at: captured_at,
                activities: Vec::new(),
                outputs: Vec::new(),
                question: None,
                permission: None,
                plan: None,
                review_state: None,
            });
        session.updated_at = session.updated_at.max(captured_at);
        if let Some(cwd) = string_field(payload, "cwd") {
            session.cwd = Some(cwd.to_string());
        }

        if event == "SessionStart" {
            let matcher = string_field(payload, "source")
                .or_else(|| string_field(payload, "matcher"))
                .unwrap_or("startup");
            match matcher {
                "clear" => {
                    session.status = ZCodeSessionStatus::Idle;
                    session.activities.clear();
                    session.outputs.clear();
                    session.question = None;
                    session.permission = None;
                    session.plan = None;
                    session.review_state = None;
                    self.pending
                        .retain(|_, pending| pending.session_id != session_id);
                }
                "compact" => {
                    push_activity(
                        session,
                        ZCodeActivity {
                            id: format!("compact-{captured_at}"),
                            tool: "Compact".to_string(),
                            summary: "会话上下文已压缩".to_string(),
                            status: "completed".to_string(),
                            started_at: captured_at,
                            updated_at: captured_at,
                        },
                    );
                }
                _ => session.status = ZCodeSessionStatus::Idle,
            }
            return;
        }

        if event == "UserPromptSubmit" {
            session.status = ZCodeSessionStatus::Working;
            session.review_state = None;
            if let Some(prompt) = string_field(payload, "prompt") {
                session.title = truncate(
                    prompt
                        .lines()
                        .find(|line| !line.trim().is_empty())
                        .unwrap_or(prompt),
                    80,
                );
            }
            return;
        }

        match envelope.phase {
            ZCodeHookPhase::Pending if event == "PreToolUse" => {
                let Some(request_id) = envelope.request_id else {
                    return;
                };
                let tool = string_field(payload, "tool_name").unwrap_or("Unknown");
                let tool_use_id = string_field(payload, "tool_use_id").map(str::to_string);
                let kind = if normalized_tool(tool) == "askuserquestion" {
                    let Some(request) = question_request(payload, &request_id, captured_at) else {
                        return;
                    };
                    session.question = Some(request);
                    session.permission = None;
                    session.plan = None;
                    session.status = ZCodeSessionStatus::WaitingForInput;
                    PendingKind::Question
                } else if normalized_tool(tool) == "exitplanmode" {
                    let Some(request) = plan_request(payload, &request_id, captured_at) else {
                        return;
                    };
                    session.plan = Some(request);
                    session.permission = None;
                    session.question = None;
                    session.status = ZCodeSessionStatus::WaitingForApproval;
                    PendingKind::Plan
                } else {
                    session.permission =
                        Some(permission_request(payload, &request_id, captured_at));
                    session.question = None;
                    session.plan = None;
                    session.status = ZCodeSessionStatus::WaitingForApproval;
                    PendingKind::Permission
                };
                session.review_state = Some(ZCodeReviewState::Pending);
                self.pending.insert(
                    request_id,
                    PendingRequest {
                        session_id: session_id.to_string(),
                        tool_use_id,
                        kind,
                    },
                );
                apply_tool_activity(session, payload, captured_at);
            }
            ZCodeHookPhase::ReturnedToZCode => {
                if let Some(request_id) = envelope.request_id {
                    self.pending.remove(&request_id);
                    clear_request(session, &request_id);
                }
                session.status = ZCodeSessionStatus::Working;
                session.review_state = Some(ZCodeReviewState::ReturnedToZCode);
            }
            ZCodeHookPhase::Resolved => {
                if let Some(request_id) = envelope.request_id {
                    self.pending.remove(&request_id);
                    clear_request(session, &request_id);
                }
                session.status = ZCodeSessionStatus::Working;
                session.review_state = Some(ZCodeReviewState::Submitted);
            }
            ZCodeHookPhase::AutoAllowed => {
                session.status = ZCodeSessionStatus::ToolRunning;
                session.review_state = None;
                apply_tool_activity(session, payload, captured_at);
            }
            ZCodeHookPhase::Observed => match event {
                "PreToolUse" => {
                    session.status = ZCodeSessionStatus::ToolRunning;
                    apply_tool_activity(session, payload, captured_at);
                }
                "PostToolUse" => {
                    session.status = ZCodeSessionStatus::Working;
                    apply_tool_activity(session, payload, captured_at);
                    append_tool_output(session, payload, captured_at, false);
                }
                "PostToolUseFailure" => {
                    session.status = ZCodeSessionStatus::ToolFailed;
                    apply_tool_activity(session, payload, captured_at);
                    append_tool_output(session, payload, captured_at, true);
                }
                "Stop" => {
                    session.status = ZCodeSessionStatus::Stopped;
                    session.question = None;
                    session.permission = None;
                    session.plan = None;
                    session.review_state = None;
                    self.pending
                        .retain(|_, pending| pending.session_id != session_id);
                    if let Some(message) = string_field(payload, "last_assistant_message") {
                        push_output(
                            session,
                            ZCodeOutput {
                                id: format!("assistant-{captured_at}"),
                                text: truncate(message, MAX_OUTPUT_CHARS),
                            },
                        );
                    }
                }
                "PermissionRequest" => {}
                _ => {}
            },
            _ => {}
        }
    }
}

fn question_request(
    payload: &Value,
    request_id: &str,
    captured_at: u64,
) -> Option<ZCodeQuestionRequest> {
    let session_id = string_field(payload, "session_id")?.to_string();
    let questions = payload
        .pointer("/tool_input/questions")?
        .as_array()?
        .iter()
        .filter_map(|question| {
            let text = string_field(question, "question")?.to_string();
            let options = question
                .get("options")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|option| {
                    Some(ZCodeQuestionOption {
                        label: string_field(option, "label")?.to_string(),
                        description: string_field(option, "description").map(str::to_string),
                        preview: option.get("preview").cloned(),
                    })
                })
                .collect();
            Some(ZCodeQuestion {
                header: string_field(question, "header").map(str::to_string),
                question: text,
                options,
                multi_select: question
                    .get("multiSelect")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                allow_other: true,
                allow_chat: false,
            })
        })
        .collect::<Vec<_>>();
    if questions.is_empty() || questions.len() > 4 {
        return None;
    }
    Some(ZCodeQuestionRequest {
        id: request_id.to_string(),
        session_id,
        questions,
        captured_at,
    })
}

fn permission_request(
    payload: &Value,
    request_id: &str,
    captured_at: u64,
) -> ZCodePermissionRequest {
    let tool = string_field(payload, "tool_name").unwrap_or("Unknown");
    ZCodePermissionRequest {
        id: request_id.to_string(),
        session_id: string_field(payload, "session_id")
            .unwrap_or_default()
            .to_string(),
        tool_name: tool.to_string(),
        summary: tool_summary(tool, payload.get("tool_input")),
        cwd: string_field(payload, "cwd").map(str::to_string),
        can_always_allow: false,
        captured_at,
    }
}

fn plan_request(payload: &Value, request_id: &str, captured_at: u64) -> Option<ZCodePlanRequest> {
    Some(ZCodePlanRequest {
        id: request_id.to_string(),
        session_id: string_field(payload, "session_id")?.to_string(),
        tool_name: string_field(payload, "tool_name")?.to_string(),
        plan: truncate(string_field(payload.get("tool_input")?, "plan")?, 20_000),
        allowed_prompts: payload.pointer("/tool_input/allowedPrompts").cloned(),
        cwd: string_field(payload, "cwd").map(str::to_string),
        captured_at,
    })
}

fn validate_answers(
    request: &ZCodeQuestionRequest,
    answers: &[ZCodeQuestionAnswer],
) -> Result<(), String> {
    for question in &request.questions {
        let answer = answers
            .iter()
            .find(|answer| answer.question == question.question)
            .ok_or_else(|| format!("No answer was submitted for: {}", question.question))?;
        let has_text = answer
            .extra_text
            .as_deref()
            .is_some_and(|text| !text.trim().is_empty());
        if has_text && !question.allow_other {
            return Err(format!(
                "A custom answer is not allowed for: {}",
                question.question
            ));
        }
        let labels = answer
            .selected_option_labels
            .iter()
            .map(|label| label.trim())
            .filter(|label| !label.is_empty())
            .collect::<Vec<_>>();
        if labels
            .iter()
            .any(|label| !question.options.iter().any(|option| option.label == *label))
        {
            return Err(format!(
                "An unknown option was submitted for: {}",
                question.question
            ));
        }
        if !question.multi_select && labels.len() > 1 {
            return Err(format!(
                "Only one option may be selected for: {}",
                question.question
            ));
        }
        if !has_text && labels.is_empty() {
            return Err(format!("Answer cannot be empty for: {}", question.question));
        }
    }
    Ok(())
}

fn apply_tool_activity(session: &mut ZCodeSession, payload: &Value, captured_at: u64) {
    let event = string_field(payload, "hook_event_name").unwrap_or_default();
    let Some(tool) = string_field(payload, "tool_name") else {
        return;
    };
    if matches!(
        normalized_tool(tool).as_str(),
        "askuserquestion" | "exitplanmode"
    ) {
        return;
    }
    let id = string_field(payload, "tool_use_id")
        .map(str::to_string)
        .unwrap_or_else(|| format!("{tool}-{captured_at}"));
    let status = match event {
        "PostToolUse" => "completed",
        "PostToolUseFailure" => "failed",
        _ => "running",
    };
    if let Some(activity) = session
        .activities
        .iter_mut()
        .find(|activity| activity.id == id)
    {
        activity.status = status.to_string();
        activity.updated_at = captured_at;
        return;
    }
    push_activity(
        session,
        ZCodeActivity {
            id,
            tool: tool.to_string(),
            summary: tool_summary(tool, payload.get("tool_input")),
            status: status.to_string(),
            started_at: captured_at,
            updated_at: captured_at,
        },
    );
}

fn append_tool_output(session: &mut ZCodeSession, payload: &Value, captured_at: u64, failed: bool) {
    let value = if failed {
        payload
            .get("error")
            .or_else(|| payload.get("tool_error"))
            .or_else(|| payload.get("message"))
    } else {
        payload
            .get("tool_response")
            .or_else(|| payload.get("tool_result"))
    };
    let Some(value) = value else {
        return;
    };
    let text = value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string());
    if text.trim().is_empty() {
        return;
    }
    push_output(
        session,
        ZCodeOutput {
            id: format!("tool-output-{captured_at}"),
            text: truncate(&text, MAX_OUTPUT_CHARS),
        },
    );
}

fn clear_request(session: &mut ZCodeSession, request_id: &str) {
    if session
        .question
        .as_ref()
        .is_some_and(|request| request.id == request_id)
    {
        session.question = None;
    }
    if session
        .permission
        .as_ref()
        .is_some_and(|request| request.id == request_id)
    {
        session.permission = None;
    }
    if session
        .plan
        .as_ref()
        .is_some_and(|request| request.id == request_id)
    {
        session.plan = None;
    }
}

fn push_activity(session: &mut ZCodeSession, activity: ZCodeActivity) {
    session.activities.push(activity);
    if session.activities.len() > MAX_ACTIVITIES {
        session
            .activities
            .drain(0..session.activities.len().saturating_sub(MAX_ACTIVITIES));
    }
}

fn push_output(session: &mut ZCodeSession, output: ZCodeOutput) {
    session.outputs.push(output);
    if session.outputs.len() > MAX_OUTPUTS {
        session
            .outputs
            .drain(0..session.outputs.len().saturating_sub(MAX_OUTPUTS));
    }
}

fn trim_sessions(sessions: &mut HashMap<String, ZCodeSession>) {
    if sessions.len() <= MAX_SESSIONS {
        return;
    }
    let mut keys = sessions
        .iter()
        .map(|(key, session)| (key.clone(), session.updated_at))
        .collect::<Vec<_>>();
    keys.sort_by_key(|(_, updated_at)| *updated_at);
    for (key, _) in keys.into_iter().take(sessions.len() - MAX_SESSIONS) {
        sessions.remove(&key);
    }
}

fn tool_summary(tool: &str, input: Option<&Value>) -> String {
    let preferred = match tool.to_ascii_lowercase().as_str() {
        "bash" | "shell" | "exec" | "command" => ["command", "description", "path"],
        "read" | "write" | "edit" | "applypatch" => ["file_path", "path", "command"],
        "grep" | "glob" => ["pattern", "path", "command"],
        "webfetch" => ["url", "prompt", "description"],
        "websearch" => ["query", "description", "prompt"],
        _ => ["description", "prompt", "command"],
    };
    input
        .and_then(|input| {
            preferred
                .iter()
                .find_map(|field| string_field(input, field))
        })
        .map(|value| truncate(value, 240))
        .unwrap_or_else(|| tool.to_string())
}

fn normalized_tool(tool: &str) -> String {
    tool.chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn directory_title(path: &str) -> Option<String> {
    std::path::Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::to_string)
}

fn truncate(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    format!(
        "{}...[truncated]",
        value.chars().take(limit).collect::<String>()
    )
}

fn normalized_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn string_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

fn status_rank(status: ZCodeSessionStatus) -> u8 {
    match status {
        ZCodeSessionStatus::WaitingForInput | ZCodeSessionStatus::WaitingForApproval => 0,
        ZCodeSessionStatus::ToolFailed => 1,
        ZCodeSessionStatus::Working | ZCodeSessionStatus::ToolRunning => 2,
        ZCodeSessionStatus::Stopped => 3,
        ZCodeSessionStatus::Idle => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn envelope(phase: ZCodeHookPhase, payload: Value) -> ZCodeHookEnvelope {
        let request_id =
            (payload["hook_event_name"] == "PreToolUse").then(|| zcode_hook::request_id(&payload));
        ZCodeHookEnvelope {
            captured_at: now_ms(),
            phase,
            request_id,
            payload,
        }
    }

    #[test]
    fn stop_keeps_the_session_and_prompt_wakes_it_again() {
        let mut store = ZCodeStore::default();
        store.apply_envelope(envelope(
            ZCodeHookPhase::Observed,
            json!({"hook_event_name":"Stop", "session_id":"s", "last_assistant_message":"Done"}),
        ));
        assert_eq!(store.sessions["s"].status, ZCodeSessionStatus::Stopped);
        assert_eq!(store.sessions["s"].outputs[0].text, "Done");
        store.apply_envelope(envelope(
            ZCodeHookPhase::Observed,
            json!({"hook_event_name":"UserPromptSubmit", "session_id":"s", "prompt":"Continue"}),
        ));
        assert_eq!(store.sessions["s"].status, ZCodeSessionStatus::Working);
        assert_eq!(store.sessions["s"].title, "Continue");
    }

    #[test]
    fn clear_resets_content_but_preserves_the_session() {
        let mut store = ZCodeStore::default();
        store.apply_envelope(envelope(
            ZCodeHookPhase::Observed,
            json!({"hook_event_name":"Stop", "session_id":"s", "last_assistant_message":"Done"}),
        ));
        store.apply_envelope(envelope(
            ZCodeHookPhase::Observed,
            json!({"hook_event_name":"SessionStart", "session_id":"s", "source":"clear"}),
        ));
        assert!(store.sessions.contains_key("s"));
        assert!(store.sessions["s"].outputs.is_empty());
        assert_eq!(store.sessions["s"].status, ZCodeSessionStatus::Idle);
    }

    #[test]
    fn question_and_plan_do_not_create_permission_cards() {
        let mut store = ZCodeStore::default();
        store.apply_envelope(envelope(
            ZCodeHookPhase::Pending,
            json!({
                "hook_event_name":"PreToolUse",
                "session_id":"question",
                "tool_use_id":"q1",
                "tool_name":"AskUserQuestion",
                "tool_input":{"questions":[{"question":"Choose", "header":"Choice", "options":[{"label":"A","description":"First"}], "multiSelect":false}]}
            }),
        ));
        assert!(store.sessions["question"].question.is_some());
        assert!(store.sessions["question"].permission.is_none());

        store.apply_envelope(envelope(
            ZCodeHookPhase::Pending,
            json!({
                "hook_event_name":"PreToolUse",
                "session_id":"plan",
                "tool_use_id":"p1",
                "tool_name":"ExitPlanMode",
                "tool_input":{"plan":"Implement it"}
            }),
        ));
        assert!(store.sessions["plan"].plan.is_some());
        assert!(store.sessions["plan"].permission.is_none());
    }

    #[test]
    fn returned_interaction_is_immediately_disabled() {
        let payload = json!({
            "hook_event_name":"PreToolUse",
            "session_id":"s",
            "tool_use_id":"q1",
            "tool_name":"AskUserQuestion",
            "tool_input":{"questions":[{"question":"Choose", "options":[], "multiSelect":false}]}
        });
        let request_id = zcode_hook::request_id(&payload);
        let mut store = ZCodeStore::default();
        store.apply_envelope(ZCodeHookEnvelope {
            captured_at: now_ms(),
            phase: ZCodeHookPhase::Pending,
            request_id: Some(request_id.clone()),
            payload: payload.clone(),
        });
        store.apply_envelope(ZCodeHookEnvelope {
            captured_at: now_ms(),
            phase: ZCodeHookPhase::ReturnedToZCode,
            request_id: Some(request_id.clone()),
            payload,
        });
        assert!(!store.pending.contains_key(&request_id));
        assert!(store.sessions["s"].question.is_none());
        assert_eq!(
            store.sessions["s"].review_state,
            Some(ZCodeReviewState::ReturnedToZCode)
        );
        assert_eq!(
            store
                .submit_question(
                    "s",
                    &request_id,
                    vec![ZCodeQuestionAnswer {
                        question: "Choose".to_string(),
                        selected_option_labels: vec!["A".to_string()],
                        extra_text: None,
                    }],
                    None,
                )
                .unwrap_err(),
            "The ZCode request was returned to ZCode"
        );
    }

    #[test]
    fn validates_every_question_has_a_non_empty_answer() {
        let request = ZCodeQuestionRequest {
            id: "r".to_string(),
            session_id: "s".to_string(),
            captured_at: 1,
            questions: vec![ZCodeQuestion {
                header: None,
                question: "Choose".to_string(),
                options: Vec::new(),
                multi_select: false,
                allow_other: true,
                allow_chat: false,
            }],
        };
        assert!(validate_answers(
            &request,
            &[ZCodeQuestionAnswer {
                question: "Choose".to_string(),
                selected_option_labels: Vec::new(),
                extra_text: Some(" ".to_string()),
            }]
        )
        .is_err());
    }

    #[test]
    fn validates_question_option_identity_and_cardinality() {
        let request = ZCodeQuestionRequest {
            id: "r".to_string(),
            session_id: "s".to_string(),
            captured_at: 1,
            questions: vec![ZCodeQuestion {
                header: None,
                question: "Choose".to_string(),
                options: vec![
                    ZCodeQuestionOption {
                        label: "A".to_string(),
                        description: None,
                        preview: None,
                    },
                    ZCodeQuestionOption {
                        label: "B".to_string(),
                        description: None,
                        preview: None,
                    },
                ],
                multi_select: false,
                allow_other: true,
                allow_chat: false,
            }],
        };
        for labels in [vec!["Unknown"], vec!["A", "B"]] {
            assert!(validate_answers(
                &request,
                &[ZCodeQuestionAnswer {
                    question: "Choose".to_string(),
                    selected_option_labels: labels.into_iter().map(str::to_string).collect(),
                    extra_text: None,
                }]
            )
            .is_err());
        }
    }
}
