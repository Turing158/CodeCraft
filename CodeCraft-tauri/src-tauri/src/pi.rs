use std::{
    collections::HashMap,
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

use super::{approval_policy, inbox_limits, pi_hook};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_INBOX_FILES: usize = 100;
const MAX_INBOX_INSTANCES: usize = 128;
const MAX_INBOX_FILES_PER_INSTANCE: usize = 128;
const MAX_INBOX_FILES_TOTAL: usize = 10_000;
const INBOX_FILE_TTL_MS: u128 = 24 * 60 * 60 * 1_000;
const MAX_SESSIONS: usize = 100;
const IDLE_SESSION_TTL_MS: u64 = 30 * 60 * 1_000;
const SESSION_TTL_MS: u64 = 24 * 60 * 60 * 1_000;
const MAX_ACTIVITIES: usize = 40;
const MAX_OUTPUTS: usize = 80;
const MAX_RESOLVED_REQUESTS: usize = 2048;
const MAX_SESSION_RULES: usize = 2048;
const RESOLVED_TTL_MS: u64 = 10 * 60 * 1000;
const INSTANCE_STALE_MS: u64 = 20_000;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn session_rules_path() -> std::path::PathBuf {
    approval_policy::base_data_dir().join("pi-allow-session-rules.json")
}

fn load_session_rules() -> Vec<PiSessionRule> {
    fs::read(session_rules_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Vec<PiSessionRule>>(&bytes).ok())
        .unwrap_or_default()
}

fn session_rule_matches(
    rules: &[PiSessionRule],
    install_id: &str,
    session_id: &str,
    tool_name: &str,
) -> bool {
    rules.iter().any(|rule| {
        rule.install_id == install_id
            && rule.session_id == session_id
            && rule.tool_name.eq_ignore_ascii_case(tool_name)
    })
}

fn persist_session_rule(install_id: &str, session_id: &str, tool_name: &str) -> Result<(), String> {
    let mut rules = load_session_rules();
    if !session_rule_matches(&rules, install_id, session_id, tool_name) {
        rules.push(PiSessionRule {
            install_id: install_id.to_string(),
            session_id: session_id.to_string(),
            tool_name: tool_name.to_ascii_lowercase(),
        });
    }
    if rules.len() > MAX_SESSION_RULES {
        rules.drain(0..rules.len() - MAX_SESSION_RULES);
    }
    let bytes = serde_json::to_vec_pretty(&rules).map_err(|error| error.to_string())?;
    pi_hook::atomic_write(&session_rules_path(), &bytes)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PiSessionStatus {
    Working,
    WaitingForInput,
    WaitingForApproval,
    ToolRunning,
    Stopped,
    Idle,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PiActivity {
    id: String,
    tool: String,
    summary: String,
    status: String,
    started_at: u64,
    updated_at: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PiOutput {
    id: String,
    text: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PiQuestionOption {
    label: String,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PiQuestion {
    #[serde(default)]
    header: Option<String>,
    question: String,
    #[serde(default)]
    options: Vec<PiQuestionOption>,
    #[serde(default)]
    multi_select: bool,
    #[serde(default = "default_true")]
    allow_other: bool,
    #[serde(default)]
    allow_chat: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PiQuestionRequest {
    pub(crate) id: String,
    pub(crate) questions: Vec<PiQuestion>,
    pub(crate) extension_instance_id: String,
    pub(crate) session_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PiQuestionAnswer {
    pub(crate) question: String,
    pub(crate) selected_option_labels: Vec<String>,
    pub(crate) extra_text: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PiPermissionRequest {
    pub(crate) id: String,
    pub(crate) tool_name: String,
    pub(crate) summary: String,
    pub(crate) cwd: Option<String>,
    pub(crate) can_always_allow: bool,
    pub(crate) captured_at: u64,
    pub(crate) extension_instance_id: String,
    pub(crate) session_id: String,
    pub(crate) run_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PiSession {
    pub(crate) id: String,
    pub(crate) install_id: String,
    pub(crate) extension_instance_id: String,
    pub(crate) status: PiSessionStatus,
    pub(crate) title: String,
    pub(crate) cwd: Option<String>,
    pub(crate) started_at: u64,
    pub(crate) updated_at: u64,
    pub(crate) activities: Vec<PiActivity>,
    pub(crate) outputs: Vec<PiOutput>,
    pub(crate) question: Option<PiQuestionRequest>,
    pub(crate) permission: Option<PiPermissionRequest>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PiInstance {
    pub(crate) extension_instance_id: String,
    pub(crate) install_id: String,
    pub(crate) pi_version: Option<String>,
    pub(crate) protocol_version: String,
    pub(crate) heartbeat: u64,
    pub(crate) capabilities: Option<Value>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PiSnapshot {
    pub(crate) connected: bool,
    pub(crate) integration_error: Option<String>,
    pub(crate) version: u64,
    pub(crate) sessions: Vec<PiSession>,
    pub(crate) instances: Vec<PiInstance>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PiApprovalDecision {
    AllowOnce,
    AllowSession,
    Deny,
}

impl PiApprovalDecision {
    fn as_str(self) -> &'static str {
        match self {
            Self::AllowOnce => "allowOnce",
            Self::AllowSession => "allowSession",
            Self::Deny => "deny",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct PiSessionRule {
    install_id: String,
    session_id: String,
    tool_name: String,
}

#[derive(Default)]
pub(crate) struct PiStore {
    sessions: HashMap<String, PiSession>,
    instances: HashMap<String, PiInstance>,
    requests: HashMap<String, pi_hook::PiEnvelope>,
    resolved_requests: HashMap<String, u64>,
    integration_error: Option<String>,
    version: u64,
}

impl PiStore {
    fn session_key(install_id: &str, session_id: &str) -> String {
        format!("{install_id}:{session_id}")
    }

    fn session_has_pending(session: &PiSession) -> bool {
        session.permission.is_some() || session.question.is_some()
    }

    fn clear_request_from_sessions(&mut self, request_id: &str, denied: bool) {
        for session in self.sessions.values_mut() {
            let permission_matches = session
                .permission
                .as_ref()
                .is_some_and(|request| request.id == request_id);
            let question_matches = session
                .question
                .as_ref()
                .is_some_and(|request| request.id == request_id);
            if permission_matches {
                session.permission = None;
            }
            if question_matches {
                session.question = None;
            }
            if permission_matches || question_matches {
                session.status = if denied {
                    PiSessionStatus::Idle
                } else {
                    PiSessionStatus::Working
                };
                session.updated_at = now_ms();
            }
        }
    }

    fn cleanup_resolved(&mut self) {
        let now = now_ms();
        self.resolved_requests
            .retain(|_, timestamp| now.saturating_sub(*timestamp) <= RESOLVED_TTL_MS);
        while self.resolved_requests.len() > MAX_RESOLVED_REQUESTS {
            let Some(oldest) = self
                .resolved_requests
                .iter()
                .min_by_key(|(_, timestamp)| *timestamp)
                .map(|(id, _)| id.clone())
            else {
                break;
            };
            self.resolved_requests.remove(&oldest);
        }
    }

    fn mark_resolved(&mut self, request_id: &str) {
        self.cleanup_resolved();
        self.resolved_requests
            .insert(request_id.to_string(), now_ms());
        self.requests.remove(request_id);
    }

    fn cleanup_pending(&mut self) {
        let now = now_ms();
        let expired = self
            .requests
            .iter()
            .filter(|(_, envelope)| {
                envelope.expires_at != pi_hook::NO_EXPIRY && envelope.expires_at < now
            })
            .map(|(request_id, _)| request_id.clone())
            .collect::<Vec<_>>();
        for request_id in expired {
            self.requests.remove(&request_id);
            self.clear_request_from_sessions(&request_id, true);
        }
    }

    fn session_mut(
        &mut self,
        install_id: &str,
        extension_instance_id: &str,
        session_id: &str,
        captured_at: u64,
        cwd: Option<String>,
    ) -> &mut PiSession {
        let key = Self::session_key(install_id, session_id);
        let session = self.sessions.entry(key).or_insert_with(|| PiSession {
            id: session_id.to_string(),
            install_id: install_id.to_string(),
            extension_instance_id: extension_instance_id.to_string(),
            status: PiSessionStatus::Working,
            title: "PI 会话".to_string(),
            cwd: cwd.clone(),
            started_at: captured_at,
            updated_at: captured_at,
            activities: Vec::new(),
            outputs: Vec::new(),
            question: None,
            permission: None,
        });
        session.extension_instance_id = extension_instance_id.to_string();
        if cwd.is_some() {
            session.cwd = cwd;
        }
        session
    }

    fn process_heartbeat(&mut self, envelope: &pi_hook::PiEnvelope) {
        let payload = &envelope.payload;
        self.instances.insert(
            envelope.extension_instance_id.clone(),
            PiInstance {
                extension_instance_id: envelope.extension_instance_id.clone(),
                install_id: envelope.install_id.clone(),
                pi_version: string_field(payload, "piVersion").map(str::to_string),
                protocol_version: envelope.protocol_version.clone(),
                heartbeat: envelope.created_at,
                capabilities: payload.get("capabilities").cloned(),
            },
        );
    }

    fn process_event(&mut self, envelope: &pi_hook::PiEnvelope) {
        let payload = &envelope.payload;
        let event_type = string_field(payload, "eventType").unwrap_or_default();
        let event = payload.get("event").unwrap_or(&Value::Null);
        let session_id = envelope
            .session_id
            .as_deref()
            .or_else(|| string_field(payload, "sessionId"));
        let Some(session_id) = session_id else {
            return;
        };
        let cwd = envelope
            .cwd
            .clone()
            .or_else(|| string_field(payload, "cwd").map(str::to_string));
        let captured_at = envelope.created_at;
        let session = self.session_mut(
            &envelope.install_id,
            &envelope.extension_instance_id,
            session_id,
            captured_at,
            cwd,
        );
        session.updated_at = captured_at;
        if let Some(title) = string_field(payload, "sessionTitle") {
            session.title = title.chars().take(256).collect();
        }
        if let (Some(id), Some(text)) = (
            string_field(payload, "outputId"),
            string_field(payload, "displayText"),
        ) {
            if let Some(output) = session.outputs.iter_mut().find(|output| output.id == id) {
                output.text = text.to_string();
            } else {
                session.outputs.push(PiOutput {
                    id: id.to_string(),
                    text: text.to_string(),
                });
                cap_vec(&mut session.outputs, MAX_OUTPUTS);
            }
        }
        match event_type {
            "session_start" => {
                session.status = PiSessionStatus::Working;
                if let Some(title) = string_field(event, "title") {
                    session.title = title.chars().take(256).collect();
                }
            }
            "session_shutdown" => {
                session.status = PiSessionStatus::Stopped;
                session.question = None;
                session.permission = None;
            }
            "agent_start" | "before_agent_start" | "turn_start" => {
                session.status = PiSessionStatus::Working;
            }
            "agent_end" | "agent_settled" | "turn_end" => {
                if !Self::session_has_pending(session) {
                    session.status = PiSessionStatus::Idle;
                }
            }
            "ui_prompt_start" => {
                if !Self::session_has_pending(session) {
                    session.status = PiSessionStatus::WaitingForInput;
                }
            }
            "ui_prompt_end" => {
                if !Self::session_has_pending(session) {
                    session.status = PiSessionStatus::Working;
                }
            }
            "tool_execution_start" => {
                let id = string_field(event, "toolCallId")
                    .unwrap_or("tool-call")
                    .to_string();
                let tool = string_field(event, "toolName")
                    .unwrap_or("tool")
                    .to_string();
                session.status = PiSessionStatus::ToolRunning;
                session.activities.push(PiActivity {
                    id,
                    tool,
                    summary: "调用工具中".to_string(),
                    status: "running".to_string(),
                    started_at: captured_at,
                    updated_at: captured_at,
                });
                cap_vec(&mut session.activities, MAX_ACTIVITIES);
            }
            "tool_execution_update" => {
                if let Some(id) = string_field(event, "toolCallId") {
                    if let Some(activity) = session.activities.iter_mut().find(|item| item.id == id)
                    {
                        activity.updated_at = captured_at;
                        if let Some(update) = string_field(event, "update") {
                            activity.summary = update.chars().take(1024).collect();
                        }
                    }
                }
            }
            "tool_execution_end" | "tool_result" => {
                if let Some(id) = string_field(event, "toolCallId") {
                    if let Some(activity) = session.activities.iter_mut().find(|item| item.id == id)
                    {
                        activity.status = "completed".to_string();
                        activity.updated_at = captured_at;
                    }
                }
                if !Self::session_has_pending(session) {
                    let terminated = event
                        .get("result")
                        .and_then(|result| result.get("terminate"))
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    session.status = if terminated {
                        PiSessionStatus::Idle
                    } else {
                        PiSessionStatus::Working
                    };
                }
            }
            _ => {}
        }
    }

    fn process_request(&mut self, envelope: pi_hook::PiEnvelope) -> Result<(), String> {
        let payload = &envelope.payload;
        let Some(request_id) = string_field(payload, "requestId") else {
            return Err("PI request is missing requestId".to_string());
        };
        if self.resolved_requests.contains_key(request_id) {
            return Ok(());
        }
        let Some(session_id) = envelope.session_id.as_deref() else {
            return Err("PI request is missing sessionId".to_string());
        };
        let request_kind = string_field(payload, "requestKind").unwrap_or("toolCall");
        if !matches!(
            request_kind,
            "toolCall" | "fileChange" | "commandRisk" | "questionnaire"
        ) {
            return Ok(());
        }
        if request_kind == "questionnaire" {
            let questions = payload
                .get("questions")
                .cloned()
                .ok_or_else(|| "PI questionnaire request is missing questions".to_string())
                .and_then(|value| {
                    serde_json::from_value::<Vec<PiQuestion>>(value)
                        .map_err(|error| format!("Invalid PI questionnaire: {error}"))
                })?;
            if questions.is_empty() {
                return Err("PI questionnaire request has no questions".to_string());
            }
            let request = PiQuestionRequest {
                id: request_id.to_string(),
                questions,
                extension_instance_id: envelope.extension_instance_id.clone(),
                session_id: session_id.to_string(),
            };
            let session = self.session_mut(
                &envelope.install_id,
                &envelope.extension_instance_id,
                session_id,
                envelope.created_at,
                envelope.cwd.clone(),
            );
            session.status = PiSessionStatus::WaitingForInput;
            session.updated_at = envelope.created_at;
            session.question = Some(request);
            self.requests.insert(request_id.to_string(), envelope);
            return Ok(());
        }
        let tool_name = string_field(payload, "toolName").unwrap_or("tool");
        let input_summary = string_field(payload, "inputSummary").unwrap_or("工具调用");
        let risk = match string_field(payload, "risk") {
            Some("low") => approval_policy::ApprovalRisk::Low,
            Some("high") => approval_policy::ApprovalRisk::High,
            _ => approval_policy::ApprovalRisk::Elevated,
        };
        if session_rule_matches(
            &load_session_rules(),
            &envelope.install_id,
            session_id,
            tool_name,
        ) {
            pi_hook::write_decision(
                &envelope,
                request_id,
                "allowSession",
                "persisted-session-rule",
            )?;
            self.mark_resolved(request_id);
            return Ok(());
        }
        let settings = approval_policy::load_settings();
        if !approval_policy::requires_user_decision(tool_name)
            && approval_policy::should_auto_approve(settings.mode, risk)
        {
            pi_hook::write_decision(&envelope, request_id, "allowOnce", "policy")?;
            self.mark_resolved(request_id);
            return Ok(());
        }

        let request = PiPermissionRequest {
            id: request_id.to_string(),
            tool_name: tool_name.to_string(),
            summary: input_summary.to_string(),
            cwd: envelope.cwd.clone(),
            can_always_allow: !matches!(risk, approval_policy::ApprovalRisk::High),
            captured_at: envelope.created_at,
            extension_instance_id: envelope.extension_instance_id.clone(),
            session_id: session_id.to_string(),
            run_id: envelope.run_id.clone(),
        };
        let session = self.session_mut(
            &envelope.install_id,
            &envelope.extension_instance_id,
            session_id,
            envelope.created_at,
            envelope.cwd.clone(),
        );
        session.status = PiSessionStatus::WaitingForApproval;
        session.updated_at = envelope.created_at;
        session.permission = Some(request);
        self.requests.insert(request_id.to_string(), envelope);
        Ok(())
    }

    fn process_envelope(&mut self, envelope: pi_hook::PiEnvelope) -> Result<(), String> {
        envelope.validate()?;
        if envelope.expires_at != pi_hook::NO_EXPIRY && envelope.expires_at < now_ms() {
            return Ok(());
        }
        match envelope.message_type.as_str() {
            "heartbeat" => self.process_heartbeat(&envelope),
            "event" => self.process_event(&envelope),
            "request" => self.process_request(envelope)?,
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn drain_inbox(&mut self) -> Result<usize, String> {
        let root = pi_hook::hook_dir().join("inbox");
        fs::create_dir_all(&root).map_err(|error| error.to_string())?;
        let mut files = Vec::new();
        for instance in fs::read_dir(&root)
            .map_err(|error| error.to_string())?
            .flatten()
            .take(MAX_INBOX_INSTANCES)
        {
            if !instance.path().is_dir() {
                continue;
            }
            for entry in fs::read_dir(instance.path())
                .map_err(|error| error.to_string())?
                .flatten()
                .take(MAX_INBOX_FILES_PER_INSTANCE)
            {
                if entry.path().extension().and_then(|value| value.to_str()) == Some("json") {
                    files.push(entry.path());
                }
            }
        }
        files.sort();
        let now = std::time::SystemTime::now();
        files.retain(|path| {
            let stale = fs::metadata(path)
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(|modified| now.duration_since(modified).ok())
                .is_some_and(|age| age.as_millis() > INBOX_FILE_TTL_MS);
            if stale {
                let _ = fs::remove_file(path);
            }
            !stale
        });
        let files = inbox_limits::limit_paths(files, MAX_INBOX_FILES_TOTAL, |path| {
            let Ok(bytes) = fs::read(path) else {
                return false;
            };
            let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
                return false;
            };
            value.get("messageType").and_then(Value::as_str) == Some("request")
        });
        let mut processed = 0;
        for path in files.into_iter().take(MAX_INBOX_FILES) {
            let result = fs::read(&path)
                .map_err(|error| error.to_string())
                .and_then(|bytes| pi_hook::PiEnvelope::from_bytes(&bytes))
                .and_then(|envelope| self.process_envelope(envelope));
            let _ = fs::remove_file(&path);
            processed += 1;
            if let Err(error) = result {
                self.integration_error = Some(error);
            }
        }
        if processed > 0 {
            self.version = self.version.wrapping_add(1);
        }
        Ok(processed)
    }

    pub(crate) fn clear(&mut self) {
        self.sessions.clear();
        self.instances.clear();
        self.requests.clear();
        self.resolved_requests.clear();
        self.integration_error = None;
    }

    pub(crate) fn set_integration_error(&mut self, error: Option<String>) {
        self.integration_error = error;
    }

    pub(crate) fn snapshot(&mut self) -> Result<PiSnapshot, String> {
        let previous_session_count = self.sessions.len();
        let previous_instance_count = self.instances.len();
        self.drain_inbox()?;
        self.cleanup_pending();
        self.trim_sessions();
        let now = now_ms();
        let active_ids = self
            .instances
            .values()
            .filter(|instance| now.saturating_sub(instance.heartbeat) <= INSTANCE_STALE_MS)
            .map(|instance| instance.extension_instance_id.clone())
            .collect::<Vec<_>>();
        for session in self.sessions.values_mut() {
            if !active_ids.contains(&session.extension_instance_id) {
                session.status = PiSessionStatus::Stopped;
                session.question = None;
                session.permission = None;
            }
        }
        self.instances
            .retain(|_, instance| now.saturating_sub(instance.heartbeat) <= INSTANCE_STALE_MS);
        if previous_session_count != self.sessions.len()
            || previous_instance_count != self.instances.len()
        {
            self.version = self.version.wrapping_add(1);
        }
        let mut sessions = self.sessions.values().cloned().collect::<Vec<_>>();
        sessions.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        let mut instances = self.instances.values().cloned().collect::<Vec<_>>();
        instances.sort_by(|left, right| right.heartbeat.cmp(&left.heartbeat));
        Ok(PiSnapshot {
            connected: self.integration_error.is_none() && !instances.is_empty(),
            integration_error: self.integration_error.clone(),
            version: self.version,
            sessions,
            instances,
        })
    }

    pub(crate) fn active_session_count(&self) -> usize {
        self.sessions
            .values()
            .filter(|session| {
                !matches!(
                    session.status,
                    PiSessionStatus::Idle | PiSessionStatus::Stopped
                )
            })
            .count()
    }

    fn trim_sessions(&mut self) {
        let now = now_ms();
        self.sessions.retain(|_, session| {
            if Self::session_has_pending(session) {
                return true;
            }
            let age = now.saturating_sub(session.updated_at);
            age <= SESSION_TTL_MS
                && (!matches!(
                    session.status,
                    PiSessionStatus::Idle | PiSessionStatus::Stopped
                ) || age <= IDLE_SESSION_TTL_MS)
        });
        if self.sessions.len() <= MAX_SESSIONS {
            return;
        }
        let excess = self.sessions.len() - MAX_SESSIONS;
        let mut candidates = self
            .sessions
            .iter()
            .filter(|(_, session)| !Self::session_has_pending(session))
            .map(|(key, session)| (key.clone(), session.updated_at))
            .collect::<Vec<_>>();
        candidates.sort_by_key(|(_, updated_at)| *updated_at);
        for (key, _) in candidates.into_iter().take(excess) {
            self.sessions.remove(&key);
        }
    }

    pub(crate) fn submit_approval(
        &mut self,
        extension_instance_id: &str,
        session_id: &str,
        request_id: &str,
        decision: PiApprovalDecision,
    ) -> Result<(), String> {
        if self.resolved_requests.contains_key(request_id) {
            return Ok(());
        }
        let request = self
            .requests
            .get(request_id)
            .ok_or_else(|| "PI approval request is no longer pending".to_string())?;
        if request.extension_instance_id != extension_instance_id
            || request.session_id.as_deref() != Some(session_id)
        {
            return Err("PI approval request identity does not match".to_string());
        }
        if matches!(decision, PiApprovalDecision::AllowSession) {
            let permission = self
                .sessions
                .get(&Self::session_key(&request.install_id, session_id))
                .and_then(|session| session.permission.as_ref())
                .filter(|permission| permission.id == request_id)
                .ok_or_else(|| "PI approval request is no longer pending".to_string())?;
            if !permission.can_always_allow {
                return Err("PI approval request cannot be allowed for the session".to_string());
            }
            persist_session_rule(&request.install_id, session_id, &permission.tool_name)?;
        }
        pi_hook::write_decision(request, request_id, decision.as_str(), "desktop")?;
        self.mark_resolved(request_id);
        self.clear_request_from_sessions(request_id, matches!(decision, PiApprovalDecision::Deny));
        Ok(())
    }

    pub(crate) fn submit_question(
        &mut self,
        extension_instance_id: &str,
        session_id: &str,
        request_id: &str,
        answers: Vec<PiQuestionAnswer>,
    ) -> Result<(), String> {
        if self.resolved_requests.contains_key(request_id) {
            return Ok(());
        }
        let request = self
            .requests
            .get(request_id)
            .ok_or_else(|| "PI questionnaire is no longer pending".to_string())?;
        if request.extension_instance_id != extension_instance_id
            || request.session_id.as_deref() != Some(session_id)
        {
            return Err("PI questionnaire identity does not match".to_string());
        }
        let details = serde_json::json!({ "answers": answers });
        pi_hook::write_review_decision(request, request_id, "answer", "desktop", details)?;
        self.mark_resolved(request_id);
        self.clear_request_from_sessions(request_id, false);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request_envelope() -> pi_hook::PiEnvelope {
        pi_hook::PiEnvelope {
            schema_version: pi_hook::SCHEMA_VERSION.to_string(),
            protocol_version: pi_hook::PROTOCOL_VERSION.to_string(),
            message_type: "request".to_string(),
            message_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            install_id: "test-install-550e8400-e29b-41d4-a716-446655440000".to_string(),
            extension_instance_id: "instance-1".to_string(),
            endpoint_epoch: "instance-1:generation:1".to_string(),
            stream_id: "session:session-1".to_string(),
            stream_epoch: "epoch-1".to_string(),
            session_id: Some("session-1".to_string()),
            run_id: Some("run-1".to_string()),
            cwd: Some("C:/project".to_string()),
            origin: Some("pi".to_string()),
            event_type: Some("request".to_string()),
            sequence: 1,
            created_at: now_ms(),
            expires_at: pi_hook::NO_EXPIRY,
            payload: json!({
                "requestId": "request-1",
                "toolCallId": "call-1",
                "toolName": "question",
                "requestKind": "toolCall",
                "inputSummary": "{\"path\":\"src/main.ts\"}",
                "risk": "elevated"
            }),
        }
    }

    #[test]
    fn request_becomes_a_pending_permission_for_the_matching_session() {
        let mut store = PiStore::default();
        store.process_request(request_envelope()).unwrap();
        let session = store
            .sessions
            .get(&PiStore::session_key(
                "test-install-550e8400-e29b-41d4-a716-446655440000",
                "session-1",
            ))
            .unwrap();
        assert_eq!(session.status, PiSessionStatus::WaitingForApproval);
        assert_eq!(session.permission.as_ref().unwrap().id, "request-1");
        assert!(store.requests.contains_key("request-1"));
    }

    #[test]
    fn never_expiring_approval_remains_pending_during_cleanup() {
        let mut store = PiStore::default();
        store.process_request(request_envelope()).unwrap();
        store.cleanup_pending();

        assert!(store.requests.contains_key("request-1"));
        assert!(store.sessions.values().any(|session| session
            .permission
            .as_ref()
            .is_some_and(|request| request.id == "request-1")));
    }

    #[test]
    fn terminated_tool_result_keeps_a_denied_session_idle() {
        let mut store = PiStore::default();
        store.process_request(request_envelope()).unwrap();
        let key = PiStore::session_key(
            "test-install-550e8400-e29b-41d4-a716-446655440000",
            "session-1",
        );
        let session = store.sessions.get_mut(&key).unwrap();
        session.permission = None;
        session.status = PiSessionStatus::Idle;

        let mut event = request_envelope();
        event.message_type = "event".to_string();
        event.event_type = Some("event".to_string());
        event.payload = json!({
            "eventType": "tool_execution_end",
            "event": {
                "toolCallId": "call-1",
                "result": {"terminate": true},
                "isError": true
            }
        });
        store.process_event(&event);
        assert_eq!(
            store.sessions.get(&key).unwrap().status,
            PiSessionStatus::Idle
        );
    }

    #[test]
    fn unknown_decision_values_are_rejected_before_writing() {
        assert_eq!(PiApprovalDecision::AllowOnce.as_str(), "allowOnce");
        assert_eq!(PiApprovalDecision::AllowSession.as_str(), "allowSession");
        assert_eq!(PiApprovalDecision::Deny.as_str(), "deny");
    }

    #[test]
    fn durable_session_key_does_not_change_when_pi_restarts() {
        assert_eq!(
            PiStore::session_key("install-1", "session-1"),
            PiStore::session_key("install-1", "session-1")
        );
        assert_ne!(
            PiStore::session_key("install-1", "session-1"),
            PiStore::session_key("install-2", "session-1")
        );
    }

    #[test]
    fn questionnaire_request_uses_the_question_review_slot() {
        let mut envelope = request_envelope();
        envelope.payload = json!({
            "requestId": "question-1",
            "requestKind": "questionnaire",
            "questions": [{
                "header": "范围",
                "question": "选择范围",
                "options": [{"label": "当前模块", "description": "只改当前模块"}],
                "multiSelect": false,
                "allowOther": true
            }]
        });
        let mut store = PiStore::default();
        store.process_request(envelope).unwrap();
        let session = store
            .sessions
            .get(&PiStore::session_key(
                "test-install-550e8400-e29b-41d4-a716-446655440000",
                "session-1",
            ))
            .unwrap();
        assert_eq!(session.status, PiSessionStatus::WaitingForInput);
        assert_eq!(session.question.as_ref().unwrap().id, "question-1");
        assert!(session.permission.is_none());
    }

    #[test]
    fn unsupported_review_request_does_not_create_a_pending_session() {
        let mut envelope = request_envelope();
        envelope.payload = json!({
            "requestId": "unsupported-1",
            "requestKind": "customReview",
            "toolName": "custom-review"
        });
        let mut store = PiStore::default();

        store.process_request(envelope).unwrap();

        assert!(store.sessions.is_empty());
        assert!(store.requests.is_empty());
    }
}

fn string_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn cap_vec<T>(items: &mut Vec<T>, max: usize) {
    if items.len() > max {
        let excess = items.len() - max;
        items.drain(0..excess);
    }
}

fn default_true() -> bool {
    true
}
