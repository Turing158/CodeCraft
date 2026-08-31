use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub(crate) const PROTOCOL: &str = "codecraft-dsh-bridge";
pub(crate) const PROTOCOL_VERSION: u32 = 1;
const HEARTBEAT_STALE_MS: u64 = 15_000;
const MAX_SESSIONS: usize = 100;
const MAX_ACTIVITIES: usize = 100;
const MAX_OUTPUTS: usize = 100;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

fn string(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(ToOwned::to_owned)
}

fn truncated(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    format!(
        "{}...[truncated]",
        value.chars().take(limit).collect::<String>()
    )
}

fn session_store_key(plugin_instance_id: &str, session_id: &str) -> String {
    format!("{plugin_instance_id}:{session_id}")
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DshEnvelope {
    pub(crate) protocol: String,
    pub(crate) protocol_version: u32,
    pub(crate) message_type: String,
    pub(crate) message_id: String,
    pub(crate) bridge_instance_id: String,
    pub(crate) plugin_instance_id: String,
    pub(crate) plugin_version: String,
    pub(crate) dsh_version: String,
    pub(crate) dsh_process_id: u32,
    pub(crate) session_id: Option<String>,
    pub(crate) turn_id: Option<u64>,
    pub(crate) step_id: Option<u64>,
    pub(crate) request_id: Option<String>,
    pub(crate) captured_at: u64,
    pub(crate) workspace: Option<String>,
    pub(crate) capabilities: DshCapabilities,
    pub(crate) token: String,
    pub(crate) payload: Value,
}

impl DshEnvelope {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.protocol != PROTOCOL || self.protocol_version != PROTOCOL_VERSION {
            return Err("Unsupported DSH bridge protocol".to_string());
        }
        if self.message_id.is_empty()
            || self.bridge_instance_id.is_empty()
            || self.plugin_instance_id.is_empty()
            || self.plugin_version.is_empty()
            || self.dsh_version.is_empty()
        {
            return Err("Incomplete DSH bridge envelope".to_string());
        }
        if !matches!(
            self.message_type.as_str(),
            "heartbeat" | "event" | "request" | "shutdown"
        ) {
            return Err("Unsupported DSH bridge message type".to_string());
        }
        if self.message_type == "request" && self.request_id.as_deref().unwrap_or("").is_empty() {
            return Err("DSH request is missing requestId".to_string());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct DshCapabilities {
    pub(crate) observation: bool,
    pub(crate) tool_approval: bool,
    pub(crate) question_answer: bool,
    pub(crate) plan_review: bool,
    pub(crate) allow_always: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DshInstance {
    pub(crate) plugin_instance_id: String,
    pub(crate) plugin_version: String,
    pub(crate) dsh_version: String,
    pub(crate) dsh_process_id: u32,
    pub(crate) bridge_instance_id: String,
    pub(crate) workspace: Option<String>,
    pub(crate) heartbeat: u64,
    pub(crate) capabilities: DshCapabilities,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DshSessionStatus {
    Working,
    WaitingForInput,
    WaitingForApproval,
    ToolRunning,
    ToolFailed,
    Stopped,
    Idle,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DshActivity {
    pub(crate) id: String,
    pub(crate) tool: String,
    pub(crate) summary: String,
    pub(crate) status: String,
    pub(crate) started_at: u64,
    pub(crate) updated_at: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DshOutput {
    pub(crate) id: String,
    pub(crate) text: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DshQuestionOption {
    pub(crate) label: String,
    pub(crate) description: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DshQuestion {
    pub(crate) id: String,
    pub(crate) header: Option<String>,
    pub(crate) question: String,
    pub(crate) detail: Option<String>,
    pub(crate) options: Vec<DshQuestionOption>,
    pub(crate) multi_select: bool,
    pub(crate) intent: Option<Value>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DshQuestionRequest {
    pub(crate) id: String,
    pub(crate) bridge_instance_id: String,
    pub(crate) plugin_instance_id: String,
    pub(crate) session_id: String,
    pub(crate) questions: Vec<DshQuestion>,
    pub(crate) captured_at: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DshPermissionRequest {
    pub(crate) id: String,
    pub(crate) bridge_instance_id: String,
    pub(crate) plugin_instance_id: String,
    pub(crate) session_id: String,
    pub(crate) call_id: Option<String>,
    pub(crate) tool_name: String,
    pub(crate) summary: String,
    pub(crate) cwd: Option<String>,
    pub(crate) can_always_allow: bool,
    pub(crate) captured_at: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DshPlanRequest {
    pub(crate) id: String,
    pub(crate) bridge_instance_id: String,
    pub(crate) plugin_instance_id: String,
    pub(crate) session_id: String,
    pub(crate) tool_name: String,
    pub(crate) plan: String,
    pub(crate) approve_label: String,
    pub(crate) decline_label: String,
    pub(crate) cwd: Option<String>,
    pub(crate) captured_at: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DshSession {
    pub(crate) id: String,
    pub(crate) plugin_instance_id: String,
    pub(crate) bridge_instance_id: String,
    pub(crate) status: DshSessionStatus,
    pub(crate) title: String,
    pub(crate) cwd: Option<String>,
    pub(crate) started_at: u64,
    pub(crate) updated_at: u64,
    pub(crate) activities: Vec<DshActivity>,
    pub(crate) outputs: Vec<DshOutput>,
    pub(crate) question: Option<DshQuestionRequest>,
    pub(crate) permission: Option<DshPermissionRequest>,
    pub(crate) plan: Option<DshPlanRequest>,
}

impl DshSession {
    pub(crate) fn is_active(&self) -> bool {
        !matches!(
            self.status,
            DshSessionStatus::Idle | DshSessionStatus::Stopped
        )
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DshSnapshot {
    pub(crate) connected: bool,
    pub(crate) integration_error: Option<String>,
    pub(crate) bridge_instance_id: Option<String>,
    pub(crate) sessions: Vec<DshSession>,
    pub(crate) instances: Vec<DshInstance>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DshApprovalDecision {
    AllowOnce,
    Deny,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DshQuestionAnswer {
    pub(crate) selected_option_labels: Vec<String>,
    pub(crate) extra_text: Option<String>,
}

#[derive(Clone, Debug)]
enum PendingKind {
    Approval(DshPermissionRequest),
    Question(DshQuestionRequest),
    Plan(DshPlanRequest, DshQuestion),
}

#[derive(Clone, Debug)]
struct PendingRequest {
    session_id: String,
    plugin_instance_id: String,
    bridge_instance_id: String,
    kind: PendingKind,
}

#[derive(Default)]
pub(crate) struct DshStore {
    sessions: HashMap<String, DshSession>,
    instances: HashMap<String, DshInstance>,
    pending: HashMap<String, PendingRequest>,
    integration_error: Option<String>,
    bridge_instance_id: Option<String>,
}

impl DshStore {
    pub(crate) fn clear(&mut self) {
        self.sessions.clear();
        self.instances.clear();
        self.pending.clear();
    }

    pub(crate) fn set_integration_error(&mut self, error: Option<String>) {
        self.integration_error = error;
    }

    pub(crate) fn set_bridge_instance_id(&mut self, id: Option<String>) {
        self.bridge_instance_id = id;
    }

    fn session_mut(&mut self, envelope: &DshEnvelope) -> Option<&mut DshSession> {
        let session_id = envelope.session_id.as_ref()?.clone();
        let key = session_store_key(&envelope.plugin_instance_id, &session_id);
        let now = envelope.captured_at.max(now_ms());
        let session = self.sessions.entry(key).or_insert_with(|| DshSession {
            id: session_id,
            plugin_instance_id: envelope.plugin_instance_id.clone(),
            bridge_instance_id: envelope.bridge_instance_id.clone(),
            status: DshSessionStatus::Idle,
            title: "DeepSeek Harness 会话".to_string(),
            cwd: envelope.workspace.clone(),
            started_at: now,
            updated_at: now,
            activities: Vec::new(),
            outputs: Vec::new(),
            question: None,
            permission: None,
            plan: None,
        });
        session.plugin_instance_id = envelope.plugin_instance_id.clone();
        session.bridge_instance_id = envelope.bridge_instance_id.clone();
        session.cwd = envelope.workspace.clone().or_else(|| session.cwd.clone());
        session.updated_at = now;
        Some(session)
    }

    fn ingest_instance(&mut self, envelope: &DshEnvelope) {
        self.instances.insert(
            envelope.plugin_instance_id.clone(),
            DshInstance {
                plugin_instance_id: envelope.plugin_instance_id.clone(),
                plugin_version: envelope.plugin_version.clone(),
                dsh_version: envelope.dsh_version.clone(),
                dsh_process_id: envelope.dsh_process_id,
                bridge_instance_id: envelope.bridge_instance_id.clone(),
                workspace: envelope.workspace.clone(),
                heartbeat: now_ms(),
                capabilities: envelope.capabilities.clone(),
            },
        );
    }

    pub(crate) fn ingest(&mut self, envelope: &DshEnvelope) -> Result<(), String> {
        envelope.validate()?;
        self.ingest_instance(envelope);
        match envelope.message_type.as_str() {
            "heartbeat" => Ok(()),
            "shutdown" => {
                self.instances.remove(&envelope.plugin_instance_id);
                for session in self
                    .sessions
                    .values_mut()
                    .filter(|session| session.plugin_instance_id == envelope.plugin_instance_id)
                {
                    session.status = DshSessionStatus::Stopped;
                    session.updated_at = now_ms();
                }
                Ok(())
            }
            "event" => self.ingest_event(envelope),
            "request" => self.ingest_request(envelope),
            _ => Err("Unsupported DSH message".to_string()),
        }
    }

    fn ingest_event(&mut self, envelope: &DshEnvelope) -> Result<(), String> {
        let event = envelope
            .payload
            .get("event")
            .ok_or_else(|| "DSH event payload is missing event".to_string())?;
        let event_type = string(event, "type").unwrap_or_else(|| "unknown".to_string());
        let data = event.get("data").unwrap_or(&Value::Null);
        let display_text = string(&envelope.payload, "displayText");
        let user_text = string(&envelope.payload, "userText");
        let turn_id = data
            .get("turn")
            .and_then(Value::as_u64)
            .or(envelope.turn_id);
        let step_id = data
            .get("step")
            .and_then(Value::as_u64)
            .or(envelope.step_id);
        let session = self
            .session_mut(envelope)
            .ok_or_else(|| "DSH event is missing sessionId".to_string())?;
        match event_type.as_str() {
            "turn/start" | "step/start" => session.status = DshSessionStatus::Working,
            "turn/end" => session.status = DshSessionStatus::Idle,
            "user/message" => {
                session.status = DshSessionStatus::Working;
                if let Some(text) = user_text.filter(|value| !value.trim().is_empty()) {
                    session.title =
                        truncated(text.trim().lines().next().unwrap_or("DeepSeek Harness"), 96);
                }
            }
            "assistant/chunk" => {
                if let Some(text) = display_text.filter(|value| !value.is_empty()) {
                    let output_id = format!(
                        "assistant-{}-{}",
                        turn_id.unwrap_or_default(),
                        step_id.unwrap_or_default()
                    );
                    if let Some(output) =
                        session.outputs.iter_mut().find(|item| item.id == output_id)
                    {
                        output.text.push_str(&text);
                        output.text = truncated(&output.text, 64 * 1024);
                    } else {
                        session.outputs.push(DshOutput {
                            id: output_id,
                            text,
                        });
                    }
                    if session.outputs.len() > MAX_OUTPUTS {
                        session.outputs.remove(0);
                    }
                }
            }
            "tool/call" => {
                let id = string(data, "callId").unwrap_or_else(|| envelope.message_id.clone());
                let tool = string(data, "name").unwrap_or_else(|| "tool".to_string());
                let summary = string(data, "arguments")
                    .map(|value| truncated(&value, 512))
                    .unwrap_or_else(|| "等待工具结果".to_string());
                session.activities.push(DshActivity {
                    id,
                    tool,
                    summary,
                    status: "running".to_string(),
                    started_at: envelope.captured_at,
                    updated_at: envelope.captured_at,
                });
                if session.activities.len() > MAX_ACTIVITIES {
                    session.activities.remove(0);
                }
                session.status = DshSessionStatus::ToolRunning;
            }
            "tool/result" => {
                let call_id = data
                    .pointer("/message/toolCallId")
                    .or_else(|| data.pointer("/message/callId"))
                    .and_then(Value::as_str);
                if let Some(activity) = call_id.and_then(|id| {
                    session
                        .activities
                        .iter_mut()
                        .rev()
                        .find(|activity| activity.id == id)
                }) {
                    activity.status = if data.get("error").is_some() {
                        "failed"
                    } else {
                        "completed"
                    }
                    .to_string();
                    activity.updated_at = envelope.captured_at;
                }
                session.status = if data.get("error").is_some() {
                    DshSessionStatus::ToolFailed
                } else {
                    DshSessionStatus::Working
                };
            }
            "codecraft/tool-result" => {
                let call_id = string(data, "callId");
                let failed = data
                    .get("isError")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if let Some(activity) = call_id.as_deref().and_then(|id| {
                    session
                        .activities
                        .iter_mut()
                        .rev()
                        .find(|activity| activity.id == id)
                }) {
                    activity.status = if failed { "failed" } else { "completed" }.to_string();
                    activity.updated_at = envelope.captured_at;
                }
                session.status = if failed {
                    DshSessionStatus::ToolFailed
                } else {
                    DshSessionStatus::Working
                };
            }
            "todo/write" | "plan/mode" => {
                session.outputs.push(DshOutput {
                    id: format!(
                        "{}-{}",
                        event_type,
                        event.get("seq").and_then(Value::as_u64).unwrap_or_default()
                    ),
                    text: truncated(&format!("{}: {}", event_type, data), 4096),
                });
            }
            _ => {}
        }
        Ok(())
    }

    fn ingest_request(&mut self, envelope: &DshEnvelope) -> Result<(), String> {
        let request_id = envelope.request_id.clone().unwrap_or_default();
        if self.pending.contains_key(&request_id) {
            return Ok(());
        }
        let session_id = envelope
            .session_id
            .clone()
            .ok_or_else(|| "DSH request is missing sessionId".to_string())?;
        let request_kind = string(&envelope.payload, "requestKind")
            .ok_or_else(|| "DSH request is missing requestKind".to_string())?;
        let pending_kind = match request_kind.as_str() {
            "approval" => {
                let request = DshPermissionRequest {
                    id: request_id.clone(),
                    bridge_instance_id: envelope.bridge_instance_id.clone(),
                    plugin_instance_id: envelope.plugin_instance_id.clone(),
                    session_id: session_id.clone(),
                    call_id: string(&envelope.payload, "callId"),
                    tool_name: string(&envelope.payload, "toolName")
                        .unwrap_or_else(|| "tool".to_string()),
                    summary: string(&envelope.payload, "inputSummary")
                        .or_else(|| string(&envelope.payload, "reason"))
                        .unwrap_or_else(|| "DeepSeek Harness 请求执行工具".to_string()),
                    cwd: envelope.workspace.clone(),
                    can_always_allow: false,
                    captured_at: envelope.captured_at,
                };
                PendingKind::Approval(request)
            }
            "question" | "plan" => {
                let questions: Vec<DshQuestion> = serde_json::from_value(
                    envelope
                        .payload
                        .get("questions")
                        .cloned()
                        .unwrap_or_else(|| json!([])),
                )
                .map_err(|error| format!("Invalid DSH questions: {error}"))?;
                if questions.is_empty() {
                    return Err("DSH question request is empty".to_string());
                }
                if request_kind == "plan" {
                    let question = questions
                        .iter()
                        .find(|question| {
                            question
                                .intent
                                .as_ref()
                                .and_then(|intent| intent.get("kind"))
                                .and_then(Value::as_str)
                                == Some("plan-review")
                        })
                        .cloned()
                        .ok_or_else(|| {
                            "DSH plan request is missing plan-review intent".to_string()
                        })?;
                    let approve_label = question
                        .intent
                        .as_ref()
                        .and_then(|intent| string(intent, "approve"))
                        .ok_or_else(|| "DSH plan request is missing approve label".to_string())?;
                    let decline_label = question
                        .options
                        .iter()
                        .find(|option| option.label != approve_label)
                        .map(|option| option.label.clone())
                        .ok_or_else(|| "DSH plan request is missing decline label".to_string())?;
                    PendingKind::Plan(
                        DshPlanRequest {
                            id: request_id.clone(),
                            bridge_instance_id: envelope.bridge_instance_id.clone(),
                            plugin_instance_id: envelope.plugin_instance_id.clone(),
                            session_id: session_id.clone(),
                            tool_name: "exit_plan_mode".to_string(),
                            plan: question
                                .detail
                                .clone()
                                .unwrap_or_else(|| question.question.clone()),
                            approve_label,
                            decline_label,
                            cwd: envelope.workspace.clone(),
                            captured_at: envelope.captured_at,
                        },
                        question,
                    )
                } else {
                    PendingKind::Question(DshQuestionRequest {
                        id: request_id.clone(),
                        bridge_instance_id: envelope.bridge_instance_id.clone(),
                        plugin_instance_id: envelope.plugin_instance_id.clone(),
                        session_id: session_id.clone(),
                        questions,
                        captured_at: envelope.captured_at,
                    })
                }
            }
            _ => return Err("Unsupported DSH request kind".to_string()),
        };
        self.pending.insert(
            request_id,
            PendingRequest {
                session_id: session_id.clone(),
                plugin_instance_id: envelope.plugin_instance_id.clone(),
                bridge_instance_id: envelope.bridge_instance_id.clone(),
                kind: pending_kind,
            },
        );
        self.refresh_session_pending(&session_id, envelope)?;
        Ok(())
    }

    fn refresh_session_pending(
        &mut self,
        session_id: &str,
        envelope: &DshEnvelope,
    ) -> Result<(), String> {
        let mut approval = None;
        let mut question = None;
        let mut plan = None;
        for pending in self.pending.values().filter(|pending| {
            pending.session_id == session_id
                && pending.plugin_instance_id == envelope.plugin_instance_id
        }) {
            match &pending.kind {
                PendingKind::Approval(request) => approval = Some(request.clone()),
                PendingKind::Question(request) => question = Some(request.clone()),
                PendingKind::Plan(request, _) => plan = Some(request.clone()),
            }
        }
        let session = self
            .session_mut(envelope)
            .ok_or_else(|| "DSH request is missing session".to_string())?;
        session.permission = approval;
        session.question = question;
        session.plan = plan;
        session.status = if session.plan.is_some() || session.permission.is_some() {
            DshSessionStatus::WaitingForApproval
        } else if session.question.is_some() {
            DshSessionStatus::WaitingForInput
        } else {
            DshSessionStatus::Working
        };
        Ok(())
    }

    fn validate_pending(
        &self,
        bridge_instance_id: &str,
        plugin_instance_id: &str,
        session_id: &str,
        request_id: &str,
    ) -> Result<&PendingRequest, String> {
        let request = self
            .pending
            .get(request_id)
            .ok_or_else(|| "DSH request is no longer pending".to_string())?;
        if request.bridge_instance_id != bridge_instance_id
            || request.plugin_instance_id != plugin_instance_id
            || request.session_id != session_id
        {
            return Err("DSH request identity does not match the active bridge".to_string());
        }
        Ok(request)
    }

    pub(crate) fn approval_response(
        &self,
        bridge_instance_id: &str,
        plugin_instance_id: &str,
        session_id: &str,
        request_id: &str,
        decision: DshApprovalDecision,
    ) -> Result<Value, String> {
        let request = self.validate_pending(
            bridge_instance_id,
            plugin_instance_id,
            session_id,
            request_id,
        )?;
        if !matches!(request.kind, PendingKind::Approval(_)) {
            return Err("DSH request is not a tool approval".to_string());
        }
        Ok(json!({
            "decision": match decision {
                DshApprovalDecision::AllowOnce => "allowOnce",
                DshApprovalDecision::Deny => "deny",
            }
        }))
    }

    pub(crate) fn question_response(
        &self,
        bridge_instance_id: &str,
        plugin_instance_id: &str,
        session_id: &str,
        request_id: &str,
        answers: Vec<DshQuestionAnswer>,
    ) -> Result<Value, String> {
        let request = self.validate_pending(
            bridge_instance_id,
            plugin_instance_id,
            session_id,
            request_id,
        )?;
        let PendingKind::Question(question_request) = &request.kind else {
            return Err("DSH request is not a question".to_string());
        };
        if answers.len() != question_request.questions.len() {
            return Err("DSH answer count does not match the question count".to_string());
        }
        let response_answers = question_request
            .questions
            .iter()
            .zip(answers)
            .map(|(question, answer)| {
                for label in &answer.selected_option_labels {
                    if !question.options.iter().any(|option| option.label == *label) {
                        return Err(format!("Unknown DSH answer option: {label}"));
                    }
                }
                Ok(json!({
                    "id": question.id,
                    "selected": answer.selected_option_labels,
                    "custom": answer
                        .extra_text
                        .filter(|value| !value.trim().is_empty())
                        .unwrap_or_default(),
                }))
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(json!({ "decision": "answer", "answers": response_answers }))
    }

    pub(crate) fn plan_response(
        &self,
        bridge_instance_id: &str,
        plugin_instance_id: &str,
        session_id: &str,
        request_id: &str,
        approved: bool,
        feedback: Option<String>,
    ) -> Result<Value, String> {
        if !approved
            && feedback
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
        {
            return Err("DSH plan feedback is required to continue planning".to_string());
        }
        let request = self.validate_pending(
            bridge_instance_id,
            plugin_instance_id,
            session_id,
            request_id,
        )?;
        let PendingKind::Plan(plan, question) = &request.kind else {
            return Err("DSH request is not a plan review".to_string());
        };
        let mut answer = json!({
            "id": question.id,
            "selected": [if approved { &plan.approve_label } else { &plan.decline_label }],
        });
        if !approved {
            answer["custom"] = json!(feedback
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_default());
        }
        Ok(json!({
            "decision": "answer",
            "answers": [answer]
        }))
    }

    pub(crate) fn resolve(&mut self, request_id: &str) {
        let Some(request) = self.pending.remove(request_id) else {
            return;
        };
        let session_id = request.session_id;
        let plugin_instance_id = request.plugin_instance_id;
        let mut approval = None;
        let mut question = None;
        let mut plan = None;
        for pending in self.pending.values().filter(|pending| {
            pending.session_id == session_id && pending.plugin_instance_id == plugin_instance_id
        }) {
            match &pending.kind {
                PendingKind::Approval(request) => approval = Some(request.clone()),
                PendingKind::Question(request) => question = Some(request.clone()),
                PendingKind::Plan(request, _) => plan = Some(request.clone()),
            }
        }
        if let Some(session) = self
            .sessions
            .get_mut(&session_store_key(&plugin_instance_id, &session_id))
        {
            session.permission = approval;
            session.question = question;
            session.plan = plan;
            session.updated_at = now_ms();
            session.status = if session.plan.is_some() || session.permission.is_some() {
                DshSessionStatus::WaitingForApproval
            } else if session.question.is_some() {
                DshSessionStatus::WaitingForInput
            } else {
                DshSessionStatus::Working
            };
        }
    }

    pub(crate) fn snapshot(&mut self) -> DshSnapshot {
        let now = now_ms();
        self.instances
            .retain(|_, instance| now.saturating_sub(instance.heartbeat) <= HEARTBEAT_STALE_MS);
        let active_instances = self.instances.keys().cloned().collect::<Vec<_>>();
        for session in self.sessions.values_mut() {
            if !active_instances.contains(&session.plugin_instance_id)
                && session.is_active()
                && now.saturating_sub(session.updated_at) > HEARTBEAT_STALE_MS
            {
                session.status = DshSessionStatus::Stopped;
            }
        }
        let mut sessions = self.sessions.values().cloned().collect::<Vec<_>>();
        sessions.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        sessions.truncate(MAX_SESSIONS);
        let mut instances = self.instances.values().cloned().collect::<Vec<_>>();
        instances.sort_by(|left, right| right.heartbeat.cmp(&left.heartbeat));
        DshSnapshot {
            connected: self.integration_error.is_none() && !instances.is_empty(),
            integration_error: self.integration_error.clone(),
            bridge_instance_id: self.bridge_instance_id.clone(),
            sessions,
            instances,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(message_type: &str, payload: Value) -> DshEnvelope {
        DshEnvelope {
            protocol: PROTOCOL.to_string(),
            protocol_version: PROTOCOL_VERSION,
            message_type: message_type.to_string(),
            message_id: "message-1".to_string(),
            bridge_instance_id: "bridge-1".to_string(),
            plugin_instance_id: "plugin-1".to_string(),
            plugin_version: "0.1.0".to_string(),
            dsh_version: "0.1.2-alpha.2".to_string(),
            dsh_process_id: 42,
            session_id: Some("session-1".to_string()),
            turn_id: Some(1),
            step_id: Some(1),
            request_id: None,
            captured_at: now_ms(),
            workspace: Some("C:/project".to_string()),
            capabilities: DshCapabilities {
                observation: true,
                tool_approval: true,
                question_answer: true,
                plan_review: true,
                allow_always: false,
            },
            token: "secret".to_string(),
            payload,
        }
    }

    #[test]
    fn records_session_output_and_tool_activity() {
        let mut store = DshStore::default();
        store
            .ingest(&envelope(
                "event",
                json!({"event":{"type":"assistant/chunk","data":{"turn":1,"step":1,"chunk":{"type":"text-delta","text":"hello"}}},"displayText":"hello"}),
            ))
            .unwrap();
        store
            .ingest(&envelope(
                "event",
                json!({"event":{"type":"tool/call","data":{"callId":"call-1","name":"read_file","arguments":"{\"path\":\"a\"}"}}}),
            ))
            .unwrap();
        let snapshot = store.snapshot();
        assert_eq!(snapshot.sessions[0].outputs[0].text, "hello");
        assert_eq!(snapshot.sessions[0].activities[0].id, "call-1");
    }

    #[test]
    fn plan_approval_uses_the_declared_label() {
        let mut store = DshStore::default();
        let mut request = envelope(
            "request",
            json!({
                "requestKind":"plan",
                "questions":[{
                    "id":"plan-question",
                    "header":"Plan",
                    "question":"Review plan",
                    "detail":"# Plan",
                    "options":[{"label":"Continue planning","description":null},{"label":"Approve","description":null}],
                    "multiSelect":false,
                    "intent":{"kind":"plan-review","approve":"Approve"}
                }]
            }),
        );
        request.request_id = Some("request-1".to_string());
        store.ingest(&request).unwrap();
        let response = store
            .plan_response("bridge-1", "plugin-1", "session-1", "request-1", true, None)
            .unwrap();
        assert_eq!(response["answers"][0]["selected"][0], "Approve");
    }

    fn approval_request(request_id: &str, session_id: &str) -> DshEnvelope {
        let mut request = envelope(
            "request",
            json!({
                "requestKind":"approval",
                "toolName":"bash",
                "callId":"call-1",
                "inputSummary":"npm test"
            }),
        );
        request.request_id = Some(request_id.to_string());
        request.session_id = Some(session_id.to_string());
        request
    }

    #[test]
    fn approval_identity_rejects_old_targets_and_consumed_requests() {
        let mut store = DshStore::default();
        store
            .ingest(&approval_request("request-1", "session-1"))
            .unwrap();

        assert!(store
            .approval_response(
                "bridge-old",
                "plugin-1",
                "session-1",
                "request-1",
                DshApprovalDecision::AllowOnce,
            )
            .unwrap_err()
            .contains("identity"));

        store.resolve("request-1");
        assert!(store
            .approval_response(
                "bridge-1",
                "plugin-1",
                "session-1",
                "request-1",
                DshApprovalDecision::Deny,
            )
            .unwrap_err()
            .contains("no longer pending"));
    }

    #[test]
    fn duplicate_request_is_idempotent() {
        let mut store = DshStore::default();
        let request = approval_request("request-1", "session-1");
        store.ingest(&request).unwrap();
        store.ingest(&request).unwrap();
        assert_eq!(store.pending.len(), 1);
        assert_eq!(store.snapshot().sessions.len(), 1);
    }

    #[test]
    fn question_response_validates_count_and_option_labels() {
        let mut store = DshStore::default();
        let mut request = envelope(
            "request",
            json!({
                "requestKind":"question",
                "questions":[{
                    "id":"question-1",
                    "header":"Mode",
                    "question":"Select mode",
                    "detail":null,
                    "options":[{"label":"Safe","description":null}],
                    "multiSelect":false,
                    "intent":null
                }]
            }),
        );
        request.request_id = Some("question-request".to_string());
        store.ingest(&request).unwrap();

        assert!(store
            .question_response(
                "bridge-1",
                "plugin-1",
                "session-1",
                "question-request",
                vec![]
            )
            .unwrap_err()
            .contains("count"));
        assert!(store
            .question_response(
                "bridge-1",
                "plugin-1",
                "session-1",
                "question-request",
                vec![DshQuestionAnswer {
                    selected_option_labels: vec!["Unknown".to_string()],
                    extra_text: None,
                }],
            )
            .unwrap_err()
            .contains("Unknown DSH answer option"));
        let response = store
            .question_response(
                "bridge-1",
                "plugin-1",
                "session-1",
                "question-request",
                vec![DshQuestionAnswer {
                    selected_option_labels: vec!["Safe".to_string()],
                    extra_text: Some("details".to_string()),
                }],
            )
            .unwrap();
        assert_eq!(response["answers"][0]["selected"][0], "Safe");
        assert_eq!(response["answers"][0]["custom"], "details");
    }

    #[test]
    fn plan_rejection_requires_feedback() {
        let mut store = DshStore::default();
        let mut request = envelope(
            "request",
            json!({
                "requestKind":"plan",
                "questions":[{
                    "id":"plan-question",
                    "header":"Plan",
                    "question":"Review plan",
                    "detail":"# Plan",
                    "options":[{"label":"Continue planning","description":null},{"label":"Approve","description":null}],
                    "multiSelect":false,
                    "intent":{"kind":"plan-review","approve":"Approve"}
                }]
            }),
        );
        request.request_id = Some("plan-request".to_string());
        store.ingest(&request).unwrap();
        assert!(store
            .plan_response(
                "bridge-1",
                "plugin-1",
                "session-1",
                "plan-request",
                false,
                Some("  ".to_string()),
            )
            .unwrap_err()
            .contains("feedback is required"));

        let approved = store
            .plan_response(
                "bridge-1",
                "plugin-1",
                "session-1",
                "plan-request",
                true,
                None,
            )
            .unwrap();
        assert_eq!(approved["answers"][0]["selected"][0], "Approve");
        assert!(approved["answers"][0].get("custom").is_none());

        let feedback = store
            .plan_response(
                "bridge-1",
                "plugin-1",
                "session-1",
                "plan-request",
                false,
                Some("调整图标".to_string()),
            )
            .unwrap();
        assert_eq!(feedback["answers"][0]["selected"][0], "Continue planning");
        assert_eq!(feedback["answers"][0]["custom"], "调整图标");
    }

    #[test]
    fn concurrent_sessions_keep_request_identity_separate() {
        let mut store = DshStore::default();
        let first = approval_request("request-1", "session-1");
        let mut second = approval_request("request-2", "session-1");
        second.plugin_instance_id = "plugin-2".to_string();
        store.ingest(&first).unwrap();
        store.ingest(&second).unwrap();

        assert_eq!(store.pending.len(), 2);
        assert_eq!(store.snapshot().sessions.len(), 2);
        assert!(store
            .approval_response(
                "bridge-1",
                "plugin-1",
                "session-1",
                "request-2",
                DshApprovalDecision::AllowOnce,
            )
            .is_err());
        assert!(store
            .approval_response(
                "bridge-1",
                "plugin-2",
                "session-1",
                "request-2",
                DshApprovalDecision::AllowOnce,
            )
            .is_ok());
    }
}
