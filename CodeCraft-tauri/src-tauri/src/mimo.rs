use std::{
    collections::HashMap,
    fs,
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use super::{inbox_limits, mimo_hook};

const MAX_ENVELOPE_BYTES: u64 = 256 * 1024;
const MAX_INBOX_FILES: usize = 100;
const MAX_INBOX_FILES_TOTAL: usize = 10_000;
const INBOX_FILE_TTL_MS: u128 = 24 * 60 * 60 * 1_000;
const MAX_SESSIONS: usize = 100;
const MAX_PENDING_REVIEWS: usize = 64;
const IDLE_SESSION_TTL_MS: u64 = 30 * 60 * 1_000;
const SESSION_TTL_MS: u64 = 24 * 60 * 60 * 1_000;
const INSTANCE_STALE_MS: u64 = 20_000;
const MAX_ACTIVITIES: usize = 40;
const MAX_OUTPUTS: usize = 24;
const RESOLVED_REVIEW_TTL_MS: u64 = 10 * 60 * 1000;
const MAX_RESOLVED_REVIEWS: usize = 2048;
const ACTIVE_INSTANCES_CACHE_TTL_MS: u64 = 1_000;
static ACTIVE_INSTANCES_CACHE: OnceLock<Mutex<Option<(std::time::Instant, Vec<MimoInstance>)>>> =
    OnceLock::new();

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum MimoSessionStatus {
    Working,
    WaitingForInput,
    WaitingForApproval,
    ToolRunning,
    Stopped,
    Idle,
}

impl MimoSessionStatus {
    fn is_active(self) -> bool {
        !matches!(self, Self::Idle | Self::Stopped)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MimoActivity {
    id: String,
    tool: String,
    summary: String,
    status: String,
    started_at: u64,
    updated_at: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MimoOutput {
    id: String,
    text: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MimoQuestionOption {
    label: String,
    description: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MimoQuestion {
    header: Option<String>,
    question: String,
    options: Vec<MimoQuestionOption>,
    multi_select: bool,
    allow_other: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(
    tag = "reviewType",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum MimoReview {
    Question {
        review_id: String,
        plugin_instance_id: String,
        session_id: String,
        request_id: String,
        call_id: Option<String>,
        questions: Vec<MimoQuestion>,
        captured_at: u64,
        submitting: bool,
        submission_error: Option<String>,
    },
    NativePermission {
        review_id: String,
        plugin_instance_id: String,
        session_id: String,
        request_id: String,
        call_id: Option<String>,
        permission: String,
        patterns: Vec<String>,
        always: Vec<String>,
        metadata: Value,
        captured_at: u64,
        submitting: bool,
        submission_error: Option<String>,
    },
    StrictToolGate {
        review_id: String,
        plugin_instance_id: String,
        session_id: String,
        call_id: Option<String>,
        tool: String,
        summary: String,
        rule_key: Option<String>,
        captured_at: u64,
        submitting: bool,
        submission_error: Option<String>,
    },
    Plan {
        review_id: String,
        plugin_instance_id: String,
        session_id: String,
        request_id: String,
        call_id: Option<String>,
        questions: Vec<MimoQuestion>,
        plan_path: Option<String>,
        plan_body: Option<String>,
        captured_at: u64,
        submitting: bool,
        submission_error: Option<String>,
    },
}

impl MimoReview {
    fn review_id(&self) -> &str {
        match self {
            Self::Question { review_id, .. }
            | Self::NativePermission { review_id, .. }
            | Self::StrictToolGate { review_id, .. }
            | Self::Plan { review_id, .. } => review_id,
        }
    }

    fn request_id(&self) -> Option<&str> {
        match self {
            Self::Question { request_id, .. } | Self::NativePermission { request_id, .. } => {
                Some(request_id)
            }
            Self::Plan { request_id, .. } => Some(request_id),
            Self::StrictToolGate { .. } => None,
        }
    }

    fn set_submitting(&mut self, value: bool) {
        match self {
            Self::Question { submitting, .. }
            | Self::NativePermission { submitting, .. }
            | Self::StrictToolGate { submitting, .. }
            | Self::Plan { submitting, .. } => *submitting = value,
        }
    }

    fn set_submission_error(&mut self, error: Option<String>) {
        match self {
            Self::Question {
                submission_error, ..
            }
            | Self::NativePermission {
                submission_error, ..
            }
            | Self::StrictToolGate {
                submission_error, ..
            }
            | Self::Plan {
                submission_error, ..
            } => *submission_error = error,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MimoSession {
    pub(crate) source: &'static str,
    id: String,
    plugin_instance_id: String,
    status: MimoSessionStatus,
    title: String,
    cwd: Option<String>,
    started_at: u64,
    updated_at: u64,
    activities: Vec<MimoActivity>,
    outputs: Vec<MimoOutput>,
    pending_reviews: Vec<MimoReview>,
}

impl MimoSession {
    pub(crate) fn is_active(&self) -> bool {
        self.status.is_active()
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MimoInstance {
    plugin_instance_id: String,
    plugin_version: String,
    protocol_version: String,
    process_id: Option<u32>,
    directory: Option<String>,
    worktree: Option<String>,
    started_at: u64,
    heartbeat: u64,
    #[serde(default)]
    capabilities: Option<Value>,
    restart_required: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MimoSnapshot {
    pub(crate) connected: bool,
    pub(crate) integration_error: Option<String>,
    pub(crate) version: u64,
    pub(crate) sessions: Vec<MimoSession>,
    pub(crate) instances: Vec<MimoInstance>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InboxEnvelope {
    protocol_version: String,
    plugin_version: String,
    #[serde(rename = "pluginInstanceID", alias = "pluginInstanceId")]
    plugin_instance_id: String,
    captured_at: u64,
    kind: String,
    payload: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstanceEnvelope {
    protocol_version: String,
    plugin_version: String,
    #[serde(rename = "pluginInstanceID", alias = "pluginInstanceId")]
    plugin_instance_id: String,
    #[serde(rename = "processID", alias = "processId")]
    process_id: Option<u32>,
    directory: Option<String>,
    worktree: Option<String>,
    started_at: u64,
    heartbeat: u64,
    #[serde(default)]
    capabilities: Option<Value>,
}

#[derive(Default)]
pub(crate) struct MimoStore {
    sessions: HashMap<String, MimoSession>,
    resolved_reviews: HashMap<String, u64>,
    integration_error: Option<String>,
    version: u64,
    last_instance_ids: Vec<String>,
}

impl MimoStore {
    fn session_key(plugin_instance_id: &str, session_id: &str) -> String {
        format!("{plugin_instance_id}:{session_id}")
    }

    fn review_key(plugin_instance_id: &str, review_id: &str) -> String {
        format!("{plugin_instance_id}:{review_id}")
    }

    fn is_resolved(&mut self, plugin_instance_id: &str, review_id: &str) -> bool {
        let now = now_ms();
        self.resolved_reviews
            .retain(|_, resolved_at| now.saturating_sub(*resolved_at) <= RESOLVED_REVIEW_TTL_MS);
        self.resolved_reviews
            .contains_key(&Self::review_key(plugin_instance_id, review_id))
    }

    fn mark_resolved(&mut self, plugin_instance_id: &str, review_id: &str) {
        let now = now_ms();
        self.resolved_reviews
            .retain(|_, resolved_at| now.saturating_sub(*resolved_at) <= RESOLVED_REVIEW_TTL_MS);
        if self.resolved_reviews.len() >= MAX_RESOLVED_REVIEWS {
            if let Some(oldest) = self
                .resolved_reviews
                .iter()
                .min_by_key(|(_, resolved_at)| *resolved_at)
                .map(|(key, _)| key.clone())
            {
                self.resolved_reviews.remove(&oldest);
            }
        }
        self.resolved_reviews
            .insert(Self::review_key(plugin_instance_id, review_id), now);
    }

    fn forget_resolution(&mut self, plugin_instance_id: &str, review_id: &str) {
        self.resolved_reviews
            .remove(&Self::review_key(plugin_instance_id, review_id));
    }

    fn session_mut(
        &mut self,
        plugin_instance_id: &str,
        session_id: &str,
        captured_at: u64,
        cwd: Option<String>,
    ) -> &mut MimoSession {
        let key = Self::session_key(plugin_instance_id, session_id);
        self.sessions.entry(key).or_insert_with(|| MimoSession {
            source: "mimo",
            id: session_id.to_string(),
            plugin_instance_id: plugin_instance_id.to_string(),
            status: MimoSessionStatus::Working,
            title: "Mimo 会话".to_string(),
            cwd,
            started_at: captured_at,
            updated_at: captured_at,
            activities: Vec::new(),
            outputs: Vec::new(),
            pending_reviews: Vec::new(),
        })
    }

    fn process_envelope(&mut self, envelope: InboxEnvelope, instances: &[MimoInstance]) {
        if envelope.protocol_version != mimo_hook::PROTOCOL_VERSION {
            return;
        }
        let _plugin_version = envelope.plugin_version;
        let instance = instances
            .iter()
            .find(|instance| instance.plugin_instance_id == envelope.plugin_instance_id);
        let cwd = instance.and_then(|instance| instance.directory.clone());
        match envelope.kind.as_str() {
            "event" => self.process_event(
                &envelope.plugin_instance_id,
                envelope.captured_at,
                cwd,
                &envelope.payload,
            ),
            "pending" => self.process_gate_pending(
                &envelope.plugin_instance_id,
                envelope.captured_at,
                cwd,
                &envelope.payload,
            ),
            "toolAfter" => self.process_tool_after(
                &envelope.plugin_instance_id,
                envelope.captured_at,
                cwd,
                &envelope.payload,
            ),
            "assistantMessage" => self.process_assistant_message(
                &envelope.plugin_instance_id,
                envelope.captured_at,
                cwd,
                &envelope.payload,
            ),
            "sessionInput" => self.process_session_input(
                &envelope.plugin_instance_id,
                envelope.captured_at,
                cwd,
                &envelope.payload,
            ),
            "receipt" => self.process_receipt(&envelope.plugin_instance_id, &envelope.payload),
            "nativePermissionAutoResolved" => {
                if let Some(request_id) = string_field(&envelope.payload, "requestID") {
                    self.remove_request(&envelope.plugin_instance_id, request_id);
                }
            }
            _ => {}
        }
    }

    fn process_event(
        &mut self,
        plugin_instance_id: &str,
        captured_at: u64,
        cwd: Option<String>,
        payload: &Value,
    ) {
        let Some(event_type) = string_field(payload, "eventType") else {
            return;
        };
        match event_type {
            "question.asked" => {
                let Some(session_id) = string_field(payload, "sessionID") else {
                    return;
                };
                let Some(request_id) = string_field(payload, "requestID") else {
                    return;
                };
                let call_id = payload
                    .get("tool")
                    .and_then(|tool| string_field(tool, "callID"))
                    .map(str::to_string);
                let questions = questions_from_value(payload.get("questions"));
                if self.is_resolved(plugin_instance_id, request_id) {
                    return;
                }
                let key = string_field(payload, "key").or_else(|| {
                    payload
                        .get("questions")
                        .and_then(Value::as_array)
                        .and_then(|items| items.first())
                        .and_then(|question| {
                            string_field(question, "key").or_else(|| string_field(question, "i18nKey"))
                        })
                });
                let plan_path = string_field(payload, "planPath").or_else(|| {
                    payload
                        .get("questions")
                        .and_then(Value::as_array)
                        .and_then(|items| items.first())
                        .and_then(|question| question.get("params"))
                        .and_then(|params| string_field(params, "plan"))
                });
                let review = if key == Some("plan_exit") {
                    MimoReview::Plan {
                        review_id: request_id.to_string(),
                        plugin_instance_id: plugin_instance_id.to_string(),
                        session_id: session_id.to_string(),
                        request_id: request_id.to_string(),
                        call_id,
                        questions,
                        plan_path: plan_path.map(str::to_string),
                        plan_body: string_field(payload, "planBody")
                            .map(|value| value.chars().take(64 * 1024).collect()),
                        captured_at,
                        submitting: false,
                        submission_error: None,
                    }
                } else {
                    MimoReview::Question {
                        review_id: request_id.to_string(),
                        plugin_instance_id: plugin_instance_id.to_string(),
                        session_id: session_id.to_string(),
                        request_id: request_id.to_string(),
                        call_id,
                        questions,
                        captured_at,
                        submitting: false,
                        submission_error: None,
                    }
                };
                let accepted = {
                    let session = self.session_mut(plugin_instance_id, session_id, captured_at, cwd);
                    session.status = MimoSessionStatus::WaitingForInput;
                    session.updated_at = captured_at;
                    upsert_review(session, review)
                };
                if !accepted {
                    self.integration_error = Some("Mimo 待审批请求超过上限，等待用户处理".to_string());
                }
            }
            "question.replied"
            | "question.rejected"
            | "question.cancelled"
            | "question.canceled"
            | "question.timeout"
            | "permission.replied"
            | "permission.cancelled"
            | "permission.canceled"
            | "permission.timeout" => {
                let properties = payload.get("properties").unwrap_or(payload);
                if let Some(request_id) = string_field(properties, "requestID") {
                    self.remove_request(plugin_instance_id, request_id);
                }
            }
            "session.deleted" | "session.ended" | "session.cancelled" | "session.canceled" => {
                let properties = payload.get("properties").unwrap_or(payload);
                let Some(session_id) = string_field(properties, "sessionID")
                    .or_else(|| string_field(properties, "sessionId"))
                    .or_else(|| string_field(properties, "id"))
                else {
                    return;
                };
                self.clear_session_reviews(plugin_instance_id, session_id);
            }
            "permission.asked" => {
                let Some(session_id) = string_field(payload, "sessionID") else {
                    return;
                };
                let Some(request_id) = string_field(payload, "requestID") else {
                    return;
                };
                let call_id = payload
                    .get("tool")
                    .and_then(|tool| string_field(tool, "callID"))
                    .map(str::to_string);
                let review = MimoReview::NativePermission {
                    review_id: request_id.to_string(),
                    plugin_instance_id: plugin_instance_id.to_string(),
                    session_id: session_id.to_string(),
                    request_id: request_id.to_string(),
                    call_id,
                    permission: string_field(payload, "permission")
                        .unwrap_or("permission")
                        .to_string(),
                    patterns: string_array(payload.get("patterns")),
                    always: string_array(payload.get("always")),
                    metadata: payload.get("metadata").cloned().unwrap_or(Value::Null),
                    captured_at,
                    submitting: false,
                    submission_error: None,
                };
                if self.is_resolved(plugin_instance_id, request_id) {
                    return;
                }
                let accepted = {
                    let session = self.session_mut(plugin_instance_id, session_id, captured_at, cwd);
                    session.status = MimoSessionStatus::WaitingForApproval;
                    session.updated_at = captured_at;
                    upsert_review(session, review)
                };
                if !accepted {
                    self.integration_error = Some("Mimo 待审批请求超过上限，等待用户处理".to_string());
                }
            }
            "session.status" => {
                let properties = payload.get("properties").unwrap_or(payload);
                let Some(session_id) = string_field(properties, "sessionID") else {
                    return;
                };
                let status = properties
                    .get("status")
                    .and_then(|value| string_field(value, "type"))
                    .unwrap_or("working");
                if matches!(
                    status,
                    "error" | "stopped" | "terminated" | "cancelled" | "canceled"
                ) {
                    self.clear_session_reviews(plugin_instance_id, session_id);
                    return;
                }
                let session = self.session_mut(plugin_instance_id, session_id, captured_at, cwd);
                session.updated_at = captured_at;
                session.status = if !session.pending_reviews.is_empty() {
                    if session
                        .pending_reviews
                        .iter()
                        .any(|review| matches!(review, MimoReview::Question { .. } | MimoReview::Plan { .. }))
                    {
                        MimoSessionStatus::WaitingForInput
                    } else {
                        MimoSessionStatus::WaitingForApproval
                    }
                } else if status == "idle" {
                    MimoSessionStatus::Idle
                } else {
                    MimoSessionStatus::Working
                };
            }
            "session.idle" => {
                let properties = payload.get("properties").unwrap_or(payload);
                let Some(session_id) = string_field(properties, "sessionID") else {
                    return;
                };
                let session = self.session_mut(plugin_instance_id, session_id, captured_at, cwd);
                session.status = if session.pending_reviews.is_empty() {
                    MimoSessionStatus::Idle
                } else {
                    MimoSessionStatus::WaitingForInput
                };
                session.updated_at = captured_at;
            }
            _ => {}
        }
    }

    fn process_gate_pending(
        &mut self,
        plugin_instance_id: &str,
        captured_at: u64,
        cwd: Option<String>,
        payload: &Value,
    ) {
        if string_field(payload, "reviewType") != Some("strictToolGate") {
            return;
        }
        let Some(session_id) = string_field(payload, "sessionID") else {
            return;
        };
        let Some(review_id) = string_field(payload, "reviewID") else {
            return;
        };
        let tool = string_field(payload, "tool").unwrap_or("tool").to_string();
        let summary = payload
            .get("args")
            .map(safe_summary)
            .unwrap_or_else(|| tool.clone());
        let review = MimoReview::StrictToolGate {
            review_id: review_id.to_string(),
            plugin_instance_id: plugin_instance_id.to_string(),
            session_id: session_id.to_string(),
            call_id: string_field(payload, "callID").map(str::to_string),
            tool: tool.clone(),
            summary: summary.clone(),
            rule_key: string_field(payload, "ruleKey").map(str::to_string),
            captured_at,
            submitting: false,
            submission_error: None,
        };
        self.forget_resolution(plugin_instance_id, review_id);
        let accepted = {
            let session = self.session_mut(plugin_instance_id, session_id, captured_at, cwd);
            session.status = MimoSessionStatus::WaitingForApproval;
            session.updated_at = captured_at;
            upsert_review(session, review)
        };
        if !accepted {
            self.integration_error = Some("Mimo 待审批请求超过上限，等待用户处理".to_string());
        }
        let session = self
            .sessions
            .get_mut(&Self::session_key(plugin_instance_id, session_id))
            .expect("session inserted above");
        session.activities.push(MimoActivity {
            id: string_field(payload, "callID")
                .unwrap_or(review_id)
                .to_string(),
            tool,
            summary,
            status: "waiting".to_string(),
            started_at: captured_at,
            updated_at: captured_at,
        });
        cap_vec(&mut session.activities, MAX_ACTIVITIES);
    }

    fn process_tool_after(
        &mut self,
        plugin_instance_id: &str,
        captured_at: u64,
        cwd: Option<String>,
        payload: &Value,
    ) {
        let Some(session_id) = string_field(payload, "sessionID") else {
            return;
        };
        let call_id = string_field(payload, "callID").unwrap_or("unknown-call");
        let tool = string_field(payload, "tool").unwrap_or("tool");
        let session = self.session_mut(plugin_instance_id, session_id, captured_at, cwd);
        session.updated_at = captured_at;
        session.status = MimoSessionStatus::Working;
        if let Some(activity) = session
            .activities
            .iter_mut()
            .find(|item| item.id == call_id)
        {
            activity.status = "completed".to_string();
            activity.updated_at = captured_at;
        } else {
            session.activities.push(MimoActivity {
                id: call_id.to_string(),
                tool: tool.to_string(),
                summary: string_field(payload, "title").unwrap_or(tool).to_string(),
                status: "completed".to_string(),
                started_at: captured_at,
                updated_at: captured_at,
            });
        }
        cap_vec(&mut session.activities, MAX_ACTIVITIES);
    }

    fn process_assistant_message(
        &mut self,
        plugin_instance_id: &str,
        captured_at: u64,
        cwd: Option<String>,
        payload: &Value,
    ) {
        let Some(session_id) = string_field(payload, "sessionID") else {
            return;
        };
        let Some(message_id) = string_field(payload, "messageID") else {
            return;
        };
        let text = string_field(payload, "text")
            .unwrap_or_default()
            .chars()
            .take(64 * 1024)
            .collect::<String>();
        let agent = string_field(payload, "agent").unwrap_or("unknown");
        let session = self.session_mut(plugin_instance_id, session_id, captured_at, cwd);
        session.updated_at = captured_at;
        if let Some(output) = session
            .outputs
            .iter_mut()
            .find(|output| output.id == message_id)
        {
            output.text = text.clone();
        } else {
            session.outputs.push(MimoOutput {
                id: message_id.to_string(),
                text: text.clone(),
            });
            cap_vec(&mut session.outputs, MAX_OUTPUTS);
        }
        let _ = agent;
    }

    fn process_session_input(
        &mut self,
        plugin_instance_id: &str,
        captured_at: u64,
        cwd: Option<String>,
        payload: &Value,
    ) {
        let Some(session_id) = string_field(payload, "sessionID") else {
            return;
        };
        let session = self.session_mut(plugin_instance_id, session_id, captured_at, cwd);
        session.updated_at = captured_at;
        session.status = MimoSessionStatus::Working;
    }

    fn process_receipt(&mut self, plugin_instance_id: &str, payload: &Value) {
        let review_id = string_field(payload, "reviewID");
        let request_id = string_field(payload, "requestID");
        let result = string_field(payload, "result").unwrap_or("error");
        let session_id = string_field(payload, "sessionID");
        let decision_type = string_field(payload, "decisionType");
        if result == "applied" {
            if let Some(review_id) = review_id {
                self.mark_resolved(plugin_instance_id, review_id);
            }
            if let Some(request_id) = request_id {
                self.mark_resolved(plugin_instance_id, request_id);
            }
        }
        for session in self
            .sessions
            .values_mut()
            .filter(|session| session.plugin_instance_id == plugin_instance_id)
        {
            if result == "applied"
                && decision_type == Some("sessionMessage")
                && session_id == Some(session.id.as_str())
            {
                session.status = MimoSessionStatus::Working;
            }
            if result == "applied" {
                let tool_started = session.pending_reviews.iter().any(|review| {
                    review_id.is_some_and(|id| review.review_id() == id)
                        && matches!(review, MimoReview::StrictToolGate { .. })
                });
                session.pending_reviews.retain(|review| {
                    review_id.is_none_or(|id| review.review_id() != id)
                        && request_id.is_none_or(|id| review.request_id() != Some(id))
                });
                if session.pending_reviews.is_empty() {
                    session.status = if tool_started {
                        MimoSessionStatus::ToolRunning
                    } else {
                        MimoSessionStatus::Working
                    };
                }
            } else {
                for review in &mut session.pending_reviews {
                    if review_id.is_some_and(|id| review.review_id() == id)
                        || request_id.is_some_and(|id| review.request_id() == Some(id))
                    {
                        review.set_submitting(false);
                        review.set_submission_error(
                            string_field(payload, "error")
                                .map(|error| error.chars().take(2000).collect()),
                        );
                    }
                }
                if let Some(error) = string_field(payload, "error") {
                    session.outputs.push(MimoOutput {
                        id: format!("receipt-{}", now_ms()),
                        text: error.chars().take(2000).collect(),
                    });
                    cap_vec(&mut session.outputs, MAX_OUTPUTS);
                }
            }
        }
    }

    fn remove_request(&mut self, plugin_instance_id: &str, request_id: &str) {
        for session in self
            .sessions
            .values_mut()
            .filter(|session| session.plugin_instance_id == plugin_instance_id)
        {
            session.pending_reviews.retain(|review| {
                let matches_request = review.request_id() == Some(request_id);
                !matches_request
            });
            if session.pending_reviews.is_empty() {
                session.status = MimoSessionStatus::Working;
            }
        }
        self.mark_resolved(plugin_instance_id, request_id);
    }

    fn clear_session_reviews(&mut self, plugin_instance_id: &str, session_id: &str) {
        let key = Self::session_key(plugin_instance_id, session_id);
        let review_ids = self
            .sessions
            .get(&key)
            .map(|session| {
                session
                    .pending_reviews
                    .iter()
                    .map(|review| review.review_id().to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for review_id in review_ids {
            self.mark_resolved(plugin_instance_id, &review_id);
        }
        if let Some(session) = self.sessions.get_mut(&key) {
            session.pending_reviews.clear();
            session.status = MimoSessionStatus::Stopped;
            session.updated_at = now_ms();
        }
    }

    pub(crate) fn drain_inbox(&mut self) -> Result<Vec<MimoInstance>, String> {
        let instances = active_instances();
        let directory = mimo_hook::inbox_dir();
        fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        let mut files = fs::read_dir(&directory)
            .map_err(|error| error.to_string())?
            .flatten()
            .take(MAX_INBOX_FILES_TOTAL.saturating_add(MAX_INBOX_FILES))
            .filter(|entry| {
                entry.path().extension().and_then(|value| value.to_str()) == Some("json")
            })
            .collect::<Vec<_>>();
        files.sort_by_key(|entry| entry.file_name());
        let paths = inbox_limits::limit_paths(
            files.into_iter().map(|entry| entry.path()).collect(),
            MAX_INBOX_FILES_TOTAL,
            |path| {
                let Ok(bytes) = fs::read(path) else {
                    return false;
                };
                let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
                    return false;
                };
                value
                    .get("payload")
                    .and_then(|payload| payload.get("eventType"))
                    .and_then(Value::as_str)
                    .is_some_and(|event| event.ends_with(".asked"))
            },
        );
        let mut changed = false;
        let now = SystemTime::now();
        for path in paths.into_iter().take(MAX_INBOX_FILES) {
            let stale = fs::metadata(&path)
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(|modified| now.duration_since(modified).ok())
                .is_some_and(|age| age.as_millis() > INBOX_FILE_TTL_MS);
            if stale {
                let _ = fs::remove_file(&path);
                continue;
            }
            let result = (|| {
                let metadata = fs::metadata(&path).map_err(|error| error.to_string())?;
                if metadata.len() > MAX_ENVELOPE_BYTES {
                    return Err(format!(
                        "Mimo IPC envelope is too large: {}",
                        path.display()
                    ));
                }
                let bytes = fs::read(&path).map_err(|error| error.to_string())?;
                let envelope = serde_json::from_slice::<InboxEnvelope>(&bytes)
                    .map_err(|error| format!("Invalid Mimo IPC envelope: {error}"))?;
                self.process_envelope(envelope, &instances);
                Ok(())
            })();
            let _ = fs::remove_file(&path);
            changed = true;
            if let Err(error) = result {
                self.integration_error = Some(error);
            }
        }
        self.trim_sessions();
        if changed {
            self.version = self.version.wrapping_add(1);
        }
        Ok(instances)
    }

    pub(crate) fn snapshot(
        &mut self,
        hook_error: Option<String>,
    ) -> Result<MimoSnapshot, String> {
        let instances = self.drain_inbox()?;
        let instance_ids = instances
            .iter()
            .map(|instance| instance.plugin_instance_id.clone())
            .collect::<Vec<_>>();
        if self.last_instance_ids != instance_ids {
            self.last_instance_ids = instance_ids;
            self.version = self.version.wrapping_add(1);
        }
        let active_ids = instances
            .iter()
            .map(|instance| instance.plugin_instance_id.as_str())
            .collect::<Vec<_>>();
        for session in self.sessions.values_mut() {
            if !active_ids.contains(&session.plugin_instance_id.as_str()) {
                session.status = MimoSessionStatus::Stopped;
                session.pending_reviews.clear();
            }
        }
        let mut sessions = self.sessions.values().cloned().collect::<Vec<_>>();
        sessions.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        let integration_error = hook_error.or_else(|| self.integration_error.clone());
        Ok(MimoSnapshot {
            connected: integration_error.is_none() && !instances.is_empty(),
            integration_error,
            version: self.version,
            sessions,
            instances,
        })
    }

    pub(crate) fn active_session_count(&self) -> usize {
        self.sessions
            .values()
            .filter(|session| session.is_active())
            .count()
    }

    fn trim_sessions(&mut self) {
        let now = now_ms();
        self.sessions.retain(|_, session| {
            if !session.pending_reviews.is_empty() {
                return true;
            }
            let age = now.saturating_sub(session.updated_at);
            age <= SESSION_TTL_MS
                && (!matches!(
                    session.status,
                    MimoSessionStatus::Idle | MimoSessionStatus::Stopped
                ) || age <= IDLE_SESSION_TTL_MS)
        });
        if self.sessions.len() <= MAX_SESSIONS {
            return;
        }
        let excess = self.sessions.len() - MAX_SESSIONS;
        let mut candidates = self
            .sessions
            .iter()
            .filter(|(_, session)| session.pending_reviews.is_empty())
            .map(|(key, session)| (key.clone(), session.updated_at))
            .collect::<Vec<_>>();
        candidates.sort_by_key(|(_, updated_at)| *updated_at);
        for (key, _) in candidates.into_iter().take(excess) {
            self.sessions.remove(&key);
        }
    }

    fn submit(
        &mut self,
        plugin_instance_id: &str,
        session_id: &str,
        review_id: &str,
        request_id: Option<&str>,
        kind: &str,
        action: &str,
        extra: Value,
    ) -> Result<Option<String>, String> {
        if self
            .resolved_reviews
            .contains_key(&Self::review_key(plugin_instance_id, review_id))
        {
            return Ok(None);
        }
        let key = Self::session_key(plugin_instance_id, session_id);
        let session = self
            .sessions
            .get_mut(&key)
            .ok_or_else(|| "Mimo session is no longer available".to_string())?;
        let review = session
            .pending_reviews
            .iter_mut()
            .find(|review| {
                review.review_id() == review_id
                    && request_id.is_none_or(|request_id| review.request_id() == Some(request_id))
            })
            .ok_or_else(|| "Mimo request is no longer pending".to_string())?;
        if matches!(
            review,
            MimoReview::Question {
                submitting: true,
                ..
            } | MimoReview::NativePermission {
                submitting: true,
                ..
            } | MimoReview::StrictToolGate {
                submitting: true,
                ..
            } | MimoReview::Plan {
                submitting: true,
                ..
            }
        ) {
            return Ok(None);
        }

        let decision_id = Uuid::new_v4().to_string();
        let mut envelope = json!({
            "protocolVersion": mimo_hook::PROTOCOL_VERSION,
            "decisionID": decision_id,
            "pluginInstanceID": plugin_instance_id,
            "sessionID": session_id,
            "reviewID": review_id,
            "type": kind,
            "action": action,
            "createdAt": now_ms()
        });
        if let Some(request_id) = request_id {
            envelope["requestID"] = json!(request_id);
        }
        if let (Some(target), Some(extra)) = (envelope.as_object_mut(), extra.as_object()) {
            target.extend(extra.clone());
        }
        mimo_hook::write_outbox_decision(&envelope)?;
        review.set_submission_error(None);
        review.set_submitting(true);
        Ok(Some(decision_id))
    }

    pub(crate) fn submit_question(
        &mut self,
        plugin_instance_id: &str,
        session_id: &str,
        request_id: &str,
        answers: Vec<Vec<String>>,
    ) -> Result<(), String> {
        self.submit(
            plugin_instance_id,
            session_id,
            request_id,
            Some(request_id),
            "question",
            "reply",
            json!({ "answers": answers }),
        )
        .map(|_| ())
    }

    pub(crate) fn submit_plan(
        &mut self,
        plugin_instance_id: &str,
        session_id: &str,
        request_id: &str,
        approved: bool,
        feedback: Option<String>,
    ) -> Result<(), String> {
        self.submit(
            plugin_instance_id,
            session_id,
            request_id,
            Some(request_id),
            "plan",
            "reply",
            json!({ "approved": approved, "feedback": feedback }),
        )
        .map(|_| ())
    }

    pub(crate) fn reject_question(
        &mut self,
        plugin_instance_id: &str,
        session_id: &str,
        request_id: &str,
    ) -> Result<(), String> {
        self.submit(
            plugin_instance_id,
            session_id,
            request_id,
            Some(request_id),
            "question",
            "reject",
            json!({}),
        )
        .map(|_| ())
    }

    pub(crate) fn submit_permission(
        &mut self,
        plugin_instance_id: &str,
        session_id: &str,
        request_id: &str,
        action: &str,
        message: Option<String>,
    ) -> Result<(), String> {
        if !matches!(action, "once" | "always" | "reject") {
            return Err("Unsupported Mimo permission decision".to_string());
        }
        self.submit(
            plugin_instance_id,
            session_id,
            request_id,
            Some(request_id),
            "permission",
            action,
            json!({ "message": message }),
        )
        .map(|_| ())
    }

    pub(crate) fn submit_gate(
        &mut self,
        plugin_instance_id: &str,
        session_id: &str,
        review_id: &str,
        action: &str,
    ) -> Result<(), String> {
        if !matches!(action, "allowOnce" | "allowSession" | "reject") {
            return Err("Unsupported Mimo tool gate decision".to_string());
        }
        self.submit(
            plugin_instance_id,
            session_id,
            review_id,
            None,
            "strictToolGate",
            action,
            json!({}),
        )
        .map(|_| ())
    }

}

fn active_instances() -> Vec<MimoInstance> {
    let cache = ACTIVE_INSTANCES_CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(cache) = cache.lock() {
        if let Some((created_at, instances)) = cache.as_ref() {
            if created_at.elapsed().as_millis() <= ACTIVE_INSTANCES_CACHE_TTL_MS as u128 {
                return instances.clone();
            }
        }
    }
    let now = now_ms();
    let mut instances = Vec::new();
    let Ok(entries) = fs::read_dir(mimo_hook::instances_dir()) else {
        return instances;
    };
    for entry in entries.flatten().take(128) {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let Ok(metadata) = fs::metadata(&path) else {
            continue;
        };
        if metadata.len() > MAX_ENVELOPE_BYTES {
            continue;
        }
        let Ok(bytes) = fs::read(path) else {
            continue;
        };
        let Ok(value) = serde_json::from_slice::<InstanceEnvelope>(&bytes) else {
            continue;
        };
        if now.saturating_sub(value.heartbeat) > INSTANCE_STALE_MS {
            continue;
        }
        let restart_required = value.protocol_version != mimo_hook::PROTOCOL_VERSION
            || value.plugin_version != mimo_hook::PLUGIN_VERSION;
        instances.push(MimoInstance {
            plugin_instance_id: value.plugin_instance_id,
            plugin_version: value.plugin_version,
            protocol_version: value.protocol_version,
            process_id: value.process_id,
            directory: value.directory,
            worktree: value.worktree,
            started_at: value.started_at,
            heartbeat: value.heartbeat,
            capabilities: value.capabilities,
            restart_required,
        });
    }
    instances.sort_by(|left, right| right.heartbeat.cmp(&left.heartbeat));
    if let Ok(mut cache) = cache.lock() {
        *cache = Some((std::time::Instant::now(), instances.clone()));
    }
    instances
}

fn upsert_review(session: &mut MimoSession, review: MimoReview) -> bool {
    if let Some(existing) = session
        .pending_reviews
        .iter_mut()
        .find(|existing| existing.review_id() == review.review_id())
    {
        *existing = review;
        return true;
    }
    if session.pending_reviews.len() >= MAX_PENDING_REVIEWS {
        return false;
    }
    session.pending_reviews.push(review);
    session.pending_reviews.sort_by_key(|review| match review {
        MimoReview::Question { captured_at, .. }
        | MimoReview::NativePermission { captured_at, .. }
        | MimoReview::StrictToolGate { captured_at, .. }
        | MimoReview::Plan { captured_at, .. } => *captured_at,
    });
    true
}

fn cap_vec<T>(items: &mut Vec<T>, limit: usize) {
    if items.len() > limit {
        items.drain(0..items.len() - limit);
    }
}

fn string_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn first_string_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| string_field(value, key))
        .map(str::to_string)
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .take(64)
                .map(|item| item.chars().take(2000).collect())
                .collect()
        })
        .unwrap_or_default()
}

fn questions_from_value(value: Option<&Value>) -> Vec<MimoQuestion> {
    value
        .and_then(Value::as_array)
        .map(|questions| {
            questions
                .iter()
                .take(16)
                .filter_map(|question| {
                    let text = string_field(question, "question")?;
                    let options = question
                        .get("options")
                        .and_then(Value::as_array)
                        .map(|options| {
                            options
                                .iter()
                                .take(32)
                                .filter_map(|option| {
                                    Some(MimoQuestionOption {
                                        label: string_field(option, "label")?
                                            .chars()
                                            .take(500)
                                            .collect(),
                                        description: string_field(option, "description")
                                            .map(|value| value.chars().take(2000).collect()),
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    Some(MimoQuestion {
                        header: string_field(question, "header")
                            .map(|value| value.chars().take(200).collect()),
                        question: text.chars().take(8000).collect(),
                        options,
                        multi_select: question
                            .get("multiple")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        allow_other: question
                            .get("custom")
                            .and_then(Value::as_bool)
                            .unwrap_or(true),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn safe_summary(value: &Value) -> String {
    if let Some(command) = string_field(value, "command") {
        return command.chars().take(500).collect();
    }
    if let Some(path) = first_string_field(
        value,
        &[
            "filePath",
            "file_path",
            "filepath",
            "notebookPath",
            "notebook_path",
            "path",
        ],
    ) {
        return path.chars().take(500).collect();
    }
    serde_json::to_string(value)
        .unwrap_or_else(|_| "工具调用".to_string())
        .chars()
        .take(500)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plugin_wire_id_fields() {
        let inbox = serde_json::from_value::<InboxEnvelope>(json!({
            "protocolVersion": "0.4",
            "pluginVersion": "0.1.0",
            "pluginInstanceID": "instance-1",
            "capturedAt": 100,
            "kind": "event",
            "payload": {"eventType": "session.idle"}
        }))
        .unwrap();
        assert_eq!(inbox.plugin_instance_id, "instance-1");

        let instance = serde_json::from_value::<InstanceEnvelope>(json!({
            "protocolVersion": "0.4",
            "pluginVersion": "0.1.0",
            "pluginInstanceID": "instance-1",
            "processID": 42,
            "directory": "C:/project",
            "worktree": "C:/project",
            "startedAt": 90,
            "heartbeat": 100
        }))
        .unwrap();
        assert_eq!(instance.plugin_instance_id, "instance-1");
        assert_eq!(instance.process_id, Some(42));
    }

    #[test]
    fn summarizes_snake_case_file_arguments_without_json() {
        assert_eq!(
            safe_summary(&json!({ "file_path": "test.txt" })),
            "test.txt"
        );
    }

    #[test]
    fn keeps_accepting_standard_camel_case_id_fields() {
        let inbox = serde_json::from_value::<InboxEnvelope>(json!({
            "protocolVersion": "0.4",
            "pluginVersion": "0.1.0",
            "pluginInstanceId": "instance-1",
            "capturedAt": 100,
            "kind": "event",
            "payload": {}
        }))
        .unwrap();
        assert_eq!(inbox.plugin_instance_id, "instance-1");

        let instance = serde_json::from_value::<InstanceEnvelope>(json!({
            "protocolVersion": "0.4",
            "pluginVersion": "0.1.0",
            "pluginInstanceId": "instance-1",
            "processId": 42,
            "directory": null,
            "worktree": null,
            "startedAt": 90,
            "heartbeat": 100
        }))
        .unwrap();
        assert_eq!(instance.plugin_instance_id, "instance-1");
        assert_eq!(instance.process_id, Some(42));
    }

    #[test]
    fn maps_question_and_permission_reviews() {
        let mut store = MimoStore::default();
        store.process_event(
            "instance",
            10,
            Some("C:/project".to_string()),
            &json!({
                "eventType": "question.asked",
                "requestID": "que_1",
                "sessionID": "ses_1",
                "questions": [{
                    "header": "Choice",
                    "question": "Continue?",
                    "options": [
                        {"label": "Yes", "description": "Continue"},
                        {"label": "No", "description": "Stop"}
                    ],
                    "multiple": false,
                    "custom": true
                }]
            }),
        );
        store.process_event(
            "instance",
            11,
            None,
            &json!({
                "eventType": "permission.asked",
                "requestID": "per_1",
                "sessionID": "ses_1",
                "permission": "bash",
                "patterns": ["git status"],
                "always": ["git *"]
            }),
        );
        let session = store.sessions.values().next().unwrap();
        assert_eq!(session.pending_reviews.len(), 2);
        assert_eq!(session.status, MimoSessionStatus::WaitingForApproval);
        let MimoReview::Question { questions, .. } = &session.pending_reviews[0] else {
            panic!("expected the first pending review to be a question");
        };
        assert_eq!(questions[0].options.len(), 2);
        assert_eq!(questions[0].options[0].label, "Yes");
        assert_eq!(questions[0].options[1].label, "No");
        assert!(questions[0].allow_other);
    }

    #[test]
    fn maps_plan_exit_questions_and_accepts_nested_key_aliases() {
        let mut store = MimoStore::default();
        store.process_event(
            "instance",
            10,
            Some("C:/project".to_string()),
            &json!({
                "eventType": "question.asked",
                "requestID": "plan_1",
                "sessionID": "session_1",
                "questions": [{
                    "header": "Plan",
                    "question": "Plan is complete",
                    "key": "plan_exit",
                    "params": {"plan": "plans/feature.md"},
                    "options": [{"label": "Yes", "description": "Start"}],
                    "multiple": false,
                    "custom": true
                }]
            }),
        );
        let session = &store.sessions["instance:session_1"];
        assert!(matches!(
            session.pending_reviews.first(),
            Some(MimoReview::Plan {
                request_id,
                plan_path: Some(plan_path),
                plan_body: None,
                ..
            }) if request_id == "plan_1" && plan_path == "plans/feature.md"
        ));

        store.process_event(
            "instance",
            11,
            None,
            &json!({
                "eventType": "question.asked",
                "requestID": "plan_2",
                "sessionID": "session_1",
                "questions": [{
                    "question": "Another plan",
                    "i18nKey": "plan_exit",
                    "options": []
                }]
            }),
        );
        assert!(matches!(
            store.sessions["instance:session_1"].pending_reviews.get(1),
            Some(MimoReview::Plan { request_id, .. }) if request_id == "plan_2"
        ));
    }

    #[test]
    fn clears_a_review_when_mimo_reports_cancellation() {
        let mut store = MimoStore::default();
        store.process_event(
            "instance",
            10,
            Some("C:/project".to_string()),
            &json!({
                "eventType": "permission.asked",
                "requestID": "permission_1",
                "sessionID": "session_1",
                "permission": "bash"
            }),
        );
        store.process_event(
            "instance",
            11,
            None,
            &json!({
                "eventType": "permission.timeout",
                "properties": {
                    "requestID": "permission_1",
                    "sessionID": "session_1"
                }
            }),
        );

        assert!(store.sessions["instance:session_1"]
            .pending_reviews
            .is_empty());
    }

    #[test]
    fn clears_reviews_when_a_mimo_session_ends() {
        let mut store = MimoStore::default();
        store.process_event(
            "instance",
            10,
            Some("C:/project".to_string()),
            &json!({
                "eventType": "permission.asked",
                "requestID": "permission_1",
                "sessionID": "session_1",
                "permission": "bash"
            }),
        );
        store.process_event(
            "instance",
            11,
            None,
            &json!({
                "eventType": "session.deleted",
                "properties": {"sessionID": "session_1"}
            }),
        );

        let session = &store.sessions["instance:session_1"];
        assert!(session.pending_reviews.is_empty());
        assert_eq!(session.status, MimoSessionStatus::Stopped);

        store.process_event(
            "instance",
            12,
            None,
            &json!({
                "eventType": "permission.asked",
                "requestID": "permission_1",
                "sessionID": "session_1",
                "permission": "bash"
            }),
        );
        assert!(store.sessions["instance:session_1"]
            .pending_reviews
            .is_empty());
    }

    #[test]
    fn serializes_review_fields_as_camel_case_for_frontend_commands() {
        let review = MimoReview::StrictToolGate {
            review_id: "review_1".to_string(),
            plugin_instance_id: "plugin_1".to_string(),
            session_id: "session_1".to_string(),
            call_id: Some("call_1".to_string()),
            tool: "bash".to_string(),
            summary: "Run a command".to_string(),
            rule_key: Some("bash:read".to_string()),
            captured_at: 10,
            submitting: false,
            submission_error: None,
        };

        let value = serde_json::to_value(review).unwrap();
        assert_eq!(value["reviewType"], "strictToolGate");
        assert_eq!(value["reviewId"], "review_1");
        assert_eq!(value["pluginInstanceId"], "plugin_1");
        assert_eq!(value["sessionId"], "session_1");
        assert_eq!(value["callId"], "call_1");
        assert_eq!(value["ruleKey"], "bash:read");
        assert!(value.get("plugin_instance_id").is_none());
        assert!(value.get("session_id").is_none());
    }

    #[test]
    fn repeated_submission_after_resolution_is_idempotent() {
        let mut store = MimoStore::default();
        store.mark_resolved("instance", "request_1");

        assert_eq!(
            store
                .submit(
                    "instance",
                    "session",
                    "request_1",
                    Some("request_1"),
                    "question",
                    "reply",
                    json!({ "answers": [["Yes"]] }),
                )
                .unwrap(),
            None
        );
        assert!(store
            .submit(
                "instance",
                "session",
                "unknown",
                Some("unknown"),
                "question",
                "reply",
                json!({ "answers": [["Yes"]] }),
            )
            .is_err());
    }

    #[test]
    fn receipt_error_keeps_review_retryable_with_visible_error() {
        let mut store = MimoStore::default();
        store.process_event(
            "instance",
            10,
            Some("C:/project".to_string()),
            &json!({
                "eventType": "permission.asked",
                "requestID": "permission_1",
                "sessionID": "session_1",
                "permission": "bash",
                "patterns": ["pwd"]
            }),
        );
        let session = store.sessions.get_mut("instance:session_1").unwrap();
        let review = session.pending_reviews.first_mut().unwrap();
        review.set_submitting(true);

        store.process_receipt(
            "instance",
            &json!({
                "reviewID": "permission_1",
                "requestID": "permission_1",
                "result": "error",
                "error": "Mimo HTTP 409: request is no longer pending"
            }),
        );

        let review = &store.sessions["instance:session_1"].pending_reviews[0];
        assert!(
            matches!(review, MimoReview::NativePermission { submitting: false, submission_error: Some(error), .. } if error.contains("409"))
        );
    }
}
