use std::{
    collections::{HashMap, HashSet},
    env, fs,
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process, thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{approval_policy, inbox_limits};

const HOOK_ARGUMENT: &str = "--codecraft-claude-hook";
const REGISTERED_HOOKS: [(&str, Option<&str>); 10] = [
    ("SessionStart", None),
    ("UserPromptSubmit", None),
    ("Notification", None),
    ("Stop", None),
    ("SessionEnd", None),
    ("PreToolUse", None),
    ("PostToolUse", None),
    ("PostToolUseFailure", None),
    ("PermissionRequest", None),
    ("PermissionDenied", None),
];
const SESSION_TTL_MS: u64 = 24 * 60 * 60 * 1_000;
const IDLE_SESSION_TTL_MS: u64 = 30 * 60 * 1_000;
const MAX_SESSIONS: usize = 100;
const MAX_INBOX_FILES_PER_PASS: usize = 100;
const MAX_INBOX_FILES_TOTAL: usize = 10_000;
const INBOX_FILE_TTL_MS: u128 = 24 * 60 * 60 * 1_000;
const MAX_ACTIVITIES: usize = 40;
const MAX_TRANSCRIPT_BYTES: u64 = 512 * 1_024;
const MAX_OUTPUT_ENTRIES: usize = 24;
const MAX_OUTPUT_ENTRY_CHARS: usize = 8_000;
const HOOK_TIMEOUT_SECONDS: u64 = 330;
// Claude's synchronous hook timeout is the hard upper bound for reviews.
const REVIEW_WAIT_TIMEOUT: Duration = Duration::from_secs(HOOK_TIMEOUT_SECONDS);
const QUESTION_POLL_INTERVAL: Duration = Duration::from_millis(200);
const HEARTBEAT_STALE_SECONDS: u64 = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ClaudeSessionStatus {
    Working,
    Waiting,
    Attention,
    ToolFailed,
    Stopped,
    Idle,
}

impl ClaudeSessionStatus {
    fn is_active(self) -> bool {
        !matches!(self, Self::Idle | Self::Stopped)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ClaudeActivityStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClaudeActivity {
    id: String,
    tool: String,
    summary: String,
    status: ClaudeActivityStatus,
    started_at: u64,
    updated_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClaudeOutputEntry {
    id: String,
    text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClaudeQuestionOption {
    label: String,
    description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClaudeQuestion {
    header: Option<String>,
    question: String,
    options: Vec<ClaudeQuestionOption>,
    multi_select: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClaudeQuestionRequest {
    id: String,
    questions: Vec<ClaudeQuestion>,
    #[serde(skip)]
    tool_use_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClaudePermissionRequest {
    id: String,
    tool_name: String,
    summary: String,
    cwd: Option<String>,
    can_always_allow: bool,
    captured_at: u64,
    #[serde(skip)]
    tool_use_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClaudePlanRequest {
    id: String,
    tool_name: String,
    plan: String,
    cwd: Option<String>,
    captured_at: u64,
    #[serde(skip)]
    tool_use_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClaudeQuestionAnswer {
    pub(crate) question: String,
    pub(crate) selected_option_labels: Vec<String>,
    pub(crate) extra_text: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct QuestionAnswerEnvelope {
    request_id: String,
    answers: Vec<ClaudeQuestionAnswer>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PermissionDecisionEnvelope {
    request_id: String,
    decision: PermissionDecision,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PermissionDecision {
    Allow,
    AllowAlways,
    Deny,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanDecisionEnvelope {
    request_id: String,
    mode: PlanExecutionMode,
    note: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PlanExecutionMode {
    Auto,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClaudeSession {
    id: String,
    status: ClaudeSessionStatus,
    title: String,
    started_at: u64,
    updated_at: u64,
    question: Option<ClaudeQuestionRequest>,
    permission: Option<ClaudePermissionRequest>,
    plan: Option<ClaudePlanRequest>,
    activities: Vec<ClaudeActivity>,
    outputs: Vec<ClaudeOutputEntry>,
    #[serde(skip)]
    transcript_path: Option<PathBuf>,
    #[serde(skip)]
    transcript_signature: Option<(SystemTime, u64)>,
}

impl ClaudeSession {
    pub(crate) fn is_active(&self) -> bool {
        self.status.is_active()
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct HookEnvelope {
    captured_at: u64,
    payload: Value,
}

#[derive(Default)]
struct TranscriptSnapshot {
    outputs: Vec<ClaudeOutputEntry>,
    resolved_tool_use_ids: HashSet<String>,
    request_tool_use_ids: HashMap<String, String>,
}

#[derive(Clone, Debug)]
struct NativeResolution {
    transcript_path: PathBuf,
    request_id: String,
    tool_use_id: Option<String>,
}

trait PendingReview {
    fn request_id(&self) -> &str;
    fn tool_use_id(&self) -> Option<&str>;
    fn set_tool_use_id(&mut self, tool_use_id: String);
    fn preserve_duplicate_identity(&mut self, existing: &Self);
}

impl PendingReview for ClaudeQuestionRequest {
    fn request_id(&self) -> &str {
        &self.id
    }

    fn tool_use_id(&self) -> Option<&str> {
        self.tool_use_id.as_deref()
    }

    fn set_tool_use_id(&mut self, tool_use_id: String) {
        self.tool_use_id = Some(tool_use_id);
    }

    fn preserve_duplicate_identity(&mut self, existing: &Self) {
        if self.id == existing.id {
            self.tool_use_id = existing.tool_use_id.clone();
        }
    }
}

impl PendingReview for ClaudePermissionRequest {
    fn request_id(&self) -> &str {
        &self.id
    }

    fn tool_use_id(&self) -> Option<&str> {
        self.tool_use_id.as_deref()
    }

    fn set_tool_use_id(&mut self, tool_use_id: String) {
        self.tool_use_id = Some(tool_use_id);
    }

    fn preserve_duplicate_identity(&mut self, existing: &Self) {
        if self.tool_name == existing.tool_name
            && self.summary == existing.summary
            && self.cwd == existing.cwd
        {
            self.id = existing.id.clone();
            self.tool_use_id = existing.tool_use_id.clone();
        }
    }
}

impl PendingReview for ClaudePlanRequest {
    fn request_id(&self) -> &str {
        &self.id
    }

    fn tool_use_id(&self) -> Option<&str> {
        self.tool_use_id.as_deref()
    }

    fn set_tool_use_id(&mut self, tool_use_id: String) {
        self.tool_use_id = Some(tool_use_id);
    }

    fn preserve_duplicate_identity(&mut self, existing: &Self) {
        if self.tool_name == existing.tool_name
            && self.plan == existing.plan
            && self.cwd == existing.cwd
        {
            self.id = existing.id.clone();
            self.tool_use_id = existing.tool_use_id.clone();
        }
    }
}

fn merge_pending_request<T: PendingReview>(slot: &mut Option<T>, mut incoming: Option<T>) {
    let Some(incoming_request) = incoming.as_mut() else {
        return;
    };
    if incoming_request.tool_use_id().is_none() {
        if let Some(existing) = slot.as_ref() {
            incoming_request.preserve_duplicate_identity(existing);
        }
    }
    *slot = incoming;
}

fn pending_tool_use_id<T: PendingReview>(request: &Option<T>) -> Option<&str> {
    request.as_ref().and_then(PendingReview::tool_use_id)
}

fn bind_pending_tool_use_id<T: PendingReview>(
    request: &mut Option<T>,
    request_tool_use_ids: &HashMap<String, String>,
) {
    let Some(request) = request.as_mut() else {
        return;
    };
    if request.tool_use_id().is_some() {
        return;
    }
    if let Some(tool_use_id) = request_tool_use_ids.get(request.request_id()) {
        request.set_tool_use_id(tool_use_id.clone());
    }
}

fn clear_resolved_request_id<T: PendingReview>(
    request: &mut Option<T>,
    request_id: &str,
    tool_use_id: &str,
) {
    let Some(request_value) = request.as_mut() else {
        return;
    };
    if request_value.request_id() != request_id {
        return;
    }
    if request_value.tool_use_id().is_none() {
        request_value.set_tool_use_id(tool_use_id.to_string());
    }
    if request_value.tool_use_id() == Some(tool_use_id) {
        *request = None;
    }
}

#[derive(Default)]
pub(crate) struct ClaudeSessionStore {
    sessions: HashMap<String, ClaudeSession>,
    version: u64,
}

impl ClaudeSessionStore {
    pub(crate) fn drain_inbox(&mut self) -> Result<(), String> {
        let mut changed = false;
        let inbox = hook_inbox_dir();
        if inbox.exists() {
            let mut event_paths = fs::read_dir(&inbox)
                .map_err(|error| error.to_string())?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
                .collect::<Vec<_>>();
            let now = SystemTime::now();
            event_paths.retain(|path| {
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
            event_paths = inbox_limits::limit_paths(event_paths, MAX_INBOX_FILES_TOTAL, |path| {
                let Ok(contents) = fs::read_to_string(path) else {
                    return false;
                };
                let Ok(value) = serde_json::from_str::<Value>(&contents) else {
                    return false;
                };
                let payload = value.get("payload").unwrap_or(&value);
                let event = string_field(payload, "hook_event_name");
                event.as_deref() == Some("PermissionRequest")
                    || (event.as_deref() == Some("PreToolUse")
                        && payload
                            .get("tool_name")
                            .and_then(Value::as_str)
                            .is_some_and(|tool| {
                                tool.eq_ignore_ascii_case("askuserquestion")
                                    || tool.eq_ignore_ascii_case("exitplanmode")
                            }))
                    || (event.as_deref() == Some("Stop")
                        && plan_request_from_payload(payload).is_some())
            });

            for path in event_paths.into_iter().take(MAX_INBOX_FILES_PER_PASS) {
                if let Ok(contents) = fs::read_to_string(&path) {
                    if let Ok(envelope) = serde_json::from_str::<HookEnvelope>(&contents) {
                        self.apply_event(&envelope.payload, envelope.captured_at);
                        changed = true;
                    }
                }

                let _ = fs::remove_file(path);
            }
        }

        for session in self.sessions.values_mut() {
            if session.transcript_signature.is_none()
                || session.is_active()
                || session.question.is_some()
                || session.permission.is_some()
                || session.plan.is_some()
            {
                changed |= session.refresh_transcript();
            }
        }

        let stale_before = unix_time_ms().saturating_sub(SESSION_TTL_MS);
        let mut removable = self
            .sessions
            .iter()
            .filter(|(_, session)| {
                if session.question.is_some() || session.permission.is_some() || session.plan.is_some() {
                    return false;
                }
                let age = unix_time_ms().saturating_sub(session.updated_at);
                session.updated_at < stale_before
                    || (matches!(session.status, ClaudeSessionStatus::Idle | ClaudeSessionStatus::Stopped)
                        && age > IDLE_SESSION_TTL_MS)
            })
            .map(|(id, session)| (id.clone(), session.updated_at))
            .collect::<Vec<_>>();
        for (id, _) in removable.drain(..) {
            self.sessions.remove(&id);
        }
        if self.sessions.len() > MAX_SESSIONS {
            let excess = self.sessions.len() - MAX_SESSIONS;
            let mut candidates = self
                .sessions
                .iter()
                .filter(|(_, session)| {
                    session.question.is_none()
                        && session.permission.is_none()
                        && session.plan.is_none()
                })
                .map(|(id, session)| (id.clone(), session.updated_at))
                .collect::<Vec<_>>();
            candidates.sort_by_key(|(_, updated_at)| *updated_at);
            for (id, _) in candidates.into_iter().take(excess) {
                self.sessions.remove(&id);
            }
        }

        if changed {
            self.version = self.version.wrapping_add(1);
        }

        Ok(())
    }

    pub(crate) fn version(&self) -> u64 {
        self.version
    }

    pub(crate) fn sessions(&self) -> Vec<ClaudeSession> {
        let mut sessions = self.sessions.values().cloned().collect::<Vec<_>>();
        sessions.sort_by(|left, right| {
            status_rank(left.status)
                .cmp(&status_rank(right.status))
                .then_with(|| right.updated_at.cmp(&left.updated_at))
        });
        sessions
    }

    pub(crate) fn active_session_count(&self) -> usize {
        self.sessions
            .values()
            .filter(|session| session.is_active())
            .count()
    }

    pub(crate) fn clear_permission(&mut self, request_id: &str) {
        let mut changed = false;
        for session in self.sessions.values_mut() {
            if session
                .permission
                .as_ref()
                .map(|permission| permission.id == request_id)
                .unwrap_or(false)
            {
                session.permission = None;
                changed = true;
            }
        }
        if changed {
            self.version = self.version.wrapping_add(1);
        }
    }

    pub(crate) fn clear_plan(&mut self, request_id: &str) {
        let mut changed = false;
        for session in self.sessions.values_mut() {
            if session
                .plan
                .as_ref()
                .map(|plan| plan.id == request_id)
                .unwrap_or(false)
            {
                session.plan = None;
                changed = true;
            }
        }
        if changed {
            self.version = self.version.wrapping_add(1);
        }
    }

    fn apply_event(&mut self, payload: &Value, captured_at: u64) {
        let Some(session_id) = string_field(payload, "session_id") else {
            return;
        };
        let Some(event_name) = string_field(payload, "hook_event_name") else {
            return;
        };

        if event_name == "SessionEnd" {
            self.sessions.remove(session_id);
            return;
        }

        let incoming_question = question_request_from_payload(payload);
        let incoming_permission = permission_request_from_payload(payload);
        let incoming_plan = plan_request_from_payload(payload);
        let incoming_transcript_path = string_field(payload, "transcript_path").map(PathBuf::from);
        let status = match event_name {
            "SessionStart" => ClaudeSessionStatus::Idle,
            "PreToolUse" if incoming_question.is_some() => ClaudeSessionStatus::Waiting,
            "PermissionRequest" if incoming_question.is_some() => ClaudeSessionStatus::Waiting,
            "PermissionRequest" => ClaudeSessionStatus::Waiting,
            "UserPromptSubmit" | "PreToolUse" | "PostToolUse" => ClaudeSessionStatus::Working,
            "Notification" | "PermissionDenied" => ClaudeSessionStatus::Attention,
            "PostToolUseFailure" => ClaudeSessionStatus::ToolFailed,
            "Stop" => ClaudeSessionStatus::Stopped,
            _ => return,
        };

        let fallback_title = string_field(payload, "cwd")
            .and_then(directory_title)
            .unwrap_or_else(|| "Claude Code 会话".to_string());
        let incoming_title = event_title(payload);
        let session = self
            .sessions
            .entry(session_id.to_string())
            .or_insert_with(|| ClaudeSession {
                id: session_id.to_string(),
                status,
                title: incoming_title
                    .clone()
                    .unwrap_or_else(|| fallback_title.clone()),
                started_at: captured_at,
                updated_at: captured_at,
                question: None,
                permission: None,
                plan: None,
                activities: Vec::new(),
                outputs: Vec::new(),
                transcript_path: incoming_transcript_path.clone(),
                transcript_signature: None,
            });

        session.status = status;
        session.updated_at = captured_at;
        merge_pending_request(&mut session.question, incoming_question);
        merge_pending_request(&mut session.permission, incoming_permission);
        merge_pending_request(&mut session.plan, incoming_plan);

        if matches!(
            event_name,
            "PostToolUse" | "PostToolUseFailure" | "PermissionDenied"
        ) {
            if let Some(tool_use_id) = string_field(payload, "tool_use_id") {
                session.clear_resolved_tool_use_id(tool_use_id);
                if let Some(request_id) = completion_request_id(payload) {
                    session.clear_resolved_request_id(&request_id, tool_use_id);
                }
            }
        }

        if event_name == "Stop" {
            session.clear_pending_reviews();
        }
        if let Some(title) = incoming_title {
            session.title = title;
        }
        if let Some(path) = incoming_transcript_path {
            if session.transcript_path.as_ref() != Some(&path) {
                session.transcript_signature = None;
            }
            session.transcript_path = Some(path);
        }
        session.apply_activity(payload, captured_at);
    }
}

impl ClaudeSession {
    fn apply_activity(&mut self, payload: &Value, captured_at: u64) {
        let Some(event_name) = string_field(payload, "hook_event_name") else {
            return;
        };
        if !matches!(
            event_name,
            "PreToolUse" | "PostToolUse" | "PostToolUseFailure"
        ) {
            return;
        }
        let Some(tool) = string_field(payload, "tool_name") else {
            return;
        };
        if tool == "AskUserQuestion" {
            return;
        }

        let id = string_field(payload, "tool_use_id")
            .map(str::to_string)
            .unwrap_or_else(|| format!("{tool}-{captured_at}"));
        let status = match event_name {
            "PreToolUse" => ClaudeActivityStatus::Running,
            "PostToolUseFailure" => ClaudeActivityStatus::Failed,
            _ => ClaudeActivityStatus::Completed,
        };

        if let Some(activity) = self.activities.iter_mut().rev().find(|item| item.id == id) {
            activity.status = status;
            activity.updated_at = captured_at;
            return;
        }

        self.activities.push(ClaudeActivity {
            id,
            tool: tool.to_string(),
            summary: tool_summary(tool, payload.get("tool_input")),
            status,
            started_at: captured_at,
            updated_at: captured_at,
        });
        if self.activities.len() > MAX_ACTIVITIES {
            self.activities
                .drain(0..self.activities.len().saturating_sub(MAX_ACTIVITIES));
        }
    }

    fn refresh_transcript(&mut self) -> bool {
        let Some(path) = self.transcript_path.as_deref() else {
            return false;
        };
        let Ok(metadata) = fs::metadata(path) else {
            return false;
        };
        let signature = (
            metadata.modified().unwrap_or(UNIX_EPOCH),
            metadata.len(),
        );
        if self.transcript_signature == Some(signature) {
            return false;
        }
        if let Ok(snapshot) = read_transcript_snapshot(path) {
            self.outputs = snapshot.outputs;
            bind_pending_tool_use_id(&mut self.question, &snapshot.request_tool_use_ids);
            bind_pending_tool_use_id(&mut self.permission, &snapshot.request_tool_use_ids);
            bind_pending_tool_use_id(&mut self.plan, &snapshot.request_tool_use_ids);
            for tool_use_id in snapshot.resolved_tool_use_ids {
                self.clear_resolved_tool_use_id(&tool_use_id);
            }
            self.transcript_signature = Some(signature);
            return true;
        }
        false
    }

    fn clear_resolved_tool_use_id(&mut self, tool_use_id: &str) {
        if pending_tool_use_id(&self.question) == Some(tool_use_id) {
            self.question = None;
        }
        if pending_tool_use_id(&self.permission) == Some(tool_use_id) {
            self.permission = None;
        }
        if pending_tool_use_id(&self.plan) == Some(tool_use_id) {
            self.plan = None;
        }
    }

    fn clear_resolved_request_id(&mut self, request_id: &str, tool_use_id: &str) {
        clear_resolved_request_id(&mut self.question, request_id, tool_use_id);
        clear_resolved_request_id(&mut self.permission, request_id, tool_use_id);
        clear_resolved_request_id(&mut self.plan, request_id, tool_use_id);
    }

    fn clear_pending_reviews(&mut self) {
        self.question = None;
        self.permission = None;
        self.plan = None;
    }
}

pub fn capture_claude_hook() -> Result<(), String> {
    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .map_err(|error| error.to_string())?;
    let payload = serde_json::from_str::<Value>(&input).map_err(|error| error.to_string())?;

    if string_field(&payload, "hook_event_name") == Some("PermissionRequest") {
        return capture_permission_request(&payload);
    }

    write_hook_envelope(&payload)
}

fn capture_permission_request(payload: &Value) -> Result<(), String> {
    if string_field(payload, "tool_name") != Some("AskUserQuestion") {
        if is_plan_tool(payload) {
            return capture_plan_request(payload);
        }
        return capture_tool_permission_request(payload);
    }

    capture_question_permission_request(payload)
}

fn capture_plan_request(payload: &Value) -> Result<(), String> {
    let Some(request_id) = plan_request_id(payload) else {
        print_permission_decision("ask");
        return Ok(());
    };

    remove_stale_decision_file(&plan_decision_path(&request_id))?;
    write_hook_envelope(payload)?;
    if !app_is_running() {
        print_permission_decision("ask");
        return Ok(());
    }

    match wait_for_plan_decision(&request_id, native_resolution(payload, &request_id)) {
        Ok(Some(envelope)) => print_plan_decision(payload, envelope.note.as_deref()),
        Ok(None) => print_permission_decision("ask"),
        Err(error) => {
            eprintln!("CodeCraft could not read the plan decision: {error}");
            print_permission_decision("ask");
        }
    }
    Ok(())
}

fn capture_question_permission_request(payload: &Value) -> Result<(), String> {
    let Some(request_id) = question_request_id(payload) else {
        print_permission_decision("ask");
        return Ok(());
    };

    remove_stale_decision_file(&question_answer_path(&request_id))?;
    write_hook_envelope(payload)?;
    if !app_is_running() {
        print_permission_decision("ask");
        return Ok(());
    }

    match wait_for_question_answer(&request_id, native_resolution(payload, &request_id)) {
        Ok(Some(envelope)) => {
            let response = build_permission_answer_response(payload, &envelope);
            println!(
                "{}",
                serde_json::to_string(&response).map_err(|error| error.to_string())?
            );
        }
        Ok(None) => print_permission_decision("ask"),
        Err(error) => {
            eprintln!("CodeCraft could not read the question answer: {error}");
            print_permission_decision("ask");
        }
    }
    Ok(())
}

fn capture_tool_permission_request(payload: &Value) -> Result<(), String> {
    let Some(request_id) = permission_request_id(payload) else {
        print_permission_decision("ask");
        return Ok(());
    };

    if env::var_os("CODECRAFT_HOOK_DEBUG").is_some() {
        let suggestions_count = payload
            .get("permission_suggestions")
            .and_then(Value::as_array)
            .map(|items| items.len())
            .unwrap_or(0);
        eprintln!(
            "[codecraft-hook] PermissionRequest tool={} permission_suggestions={suggestions_count}",
            string_field(payload, "tool_name").unwrap_or("?")
        );
    }

    let approval_settings = approval_policy::load_settings();
    let risk = approval_policy::risk_for_claude_permission(payload);
    let tool = string_field(payload, "tool_name").unwrap_or_default();
    if !approval_policy::requires_user_decision(tool)
        && approval_policy::should_auto_approve(approval_settings.mode, risk)
    {
        print_permission_decision("allow");
        return Ok(());
    }
    remove_stale_decision_file(&permission_decision_path(&request_id))?;
    write_hook_envelope(payload)?;
    if !app_is_running() {
        print_permission_decision("ask");
        return Ok(());
    }

    match wait_for_permission_decision(&request_id, native_resolution(payload, &request_id)) {
        Ok(Some(envelope)) => match envelope.decision {
            PermissionDecision::Allow => print_permission_decision("allow"),
            PermissionDecision::AllowAlways => print_permission_decision_allow_always(payload),
            PermissionDecision::Deny => print_permission_decision("deny"),
        },
        Ok(None) => print_permission_decision("ask"),
        Err(error) => {
            eprintln!("CodeCraft could not read the permission decision: {error}");
            print_permission_decision("ask");
        }
    }
    Ok(())
}

fn write_hook_envelope(payload: &Value) -> Result<(), String> {
    let envelope = HookEnvelope {
        captured_at: unix_time_ms(),
        payload: payload.clone(),
    };

    let inbox = hook_inbox_dir();
    fs::create_dir_all(&inbox).map_err(|error| error.to_string())?;
    let serialized = serde_json::to_vec(&envelope).map_err(|error| error.to_string())?;

    for attempt in 0..10 {
        let stem = format!("{:020}-{}-{attempt}", envelope.captured_at, process::id());
        let temporary_path = inbox.join(format!("{stem}.tmp"));
        let event_path = inbox.join(format!("{stem}.json"));
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path);
        let Ok(mut file) = file else {
            continue;
        };

        file.write_all(&serialized)
            .and_then(|_| file.flush())
            .map_err(|error| error.to_string())?;
        drop(file);
        fs::rename(&temporary_path, &event_path).map_err(|error| error.to_string())?;
        return Ok(());
    }

    Err("Unable to allocate a unique Claude Code hook event file".to_string())
}

fn print_permission_decision(behavior: &str) {
    println!("{}", permission_decision_response(behavior));
}

fn permission_decision_response(behavior: &str) -> Value {
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PermissionRequest",
            "decision": {
                "behavior": behavior
            }
        }
    })
}

fn print_plan_decision(payload: &Value, note: Option<&str>) {
    let note = note.map(str::trim).filter(|value| !value.is_empty());
    let mut decision = serde_json::Map::new();
    decision.insert("behavior".to_string(), Value::String("allow".to_string()));
    if let Some(note) = note {
        let mut updated_input = payload
            .get("tool_input")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        updated_input.insert("note".to_string(), Value::String(note.to_string()));
        decision.insert("updatedInput".to_string(), Value::Object(updated_input));
    }

    println!(
        "{}",
        json!({
            "hookSpecificOutput": {
                "hookEventName": "PermissionRequest",
                "decision": Value::Object(decision)
            }
        })
    );
}

fn print_permission_decision_allow_always(payload: &Value) {
    println!(
        "{}",
        json!({
            "hookSpecificOutput": {
                "hookEventName": "PermissionRequest",
                "decision": { "behavior": "allow" },
                "updatedPermissions": permission_update_rules(payload)
            }
        })
    );
}

fn permission_update_rules(payload: &Value) -> Vec<Value> {
    payload
        .get("permission_suggestions")
        .and_then(Value::as_array)
        .filter(|suggestions| !suggestions.is_empty())
        .cloned()
        .unwrap_or_else(|| fallback_permission_rules(payload))
}

fn fallback_permission_rules(payload: &Value) -> Vec<Value> {
    let Some(tool_name) = string_field(payload, "tool_name") else {
        return Vec::new();
    };
    let input = payload.get("tool_input").unwrap_or(&Value::Null);
    let Some(rule_content) = permission_rule_content(tool_name, input) else {
        return Vec::new();
    };

    vec![json!({
        "type": "addRules",
        "rules": [{
            "toolName": tool_name,
            "ruleContent": rule_content
        }],
        "behavior": "allow",
        "destination": "localSettings"
    })]
}

fn permission_rule_content(tool: &str, input: &Value) -> Option<String> {
    let preferred_fields: &[&str] = match tool {
        "Bash" => &["command"],
        "Read" | "Write" | "Edit" | "NotebookEdit" => &["file_path", "notebook_path"],
        "Glob" | "Grep" => &["pattern", "path"],
        "WebFetch" => &["url", "prompt"],
        "WebSearch" => &["query"],
        "Task" => &["description", "prompt"],
        _ => &[
            "description",
            "file_path",
            "path",
            "command",
            "query",
            "url",
            "prompt",
        ],
    };

    preferred_fields
        .iter()
        .find_map(|field| string_field(input, field).map(str::to_string))
}

fn build_permission_answer_response(payload: &Value, envelope: &QuestionAnswerEnvelope) -> Value {
    let questions = payload
        .pointer("/tool_input/questions")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    let mut answers = serde_json::Map::new();
    for answer in &envelope.answers {
        answers.insert(answer.question.clone(), question_answer_value(answer));
    }

    json!({
        "hookSpecificOutput": {
            "hookEventName": "PermissionRequest",
            "decision": {
                "behavior": "allow",
                "updatedInput": {
                    "questions": questions,
                    "answers": Value::Object(answers)
                }
            }
        }
    })
}

fn question_answer_value(answer: &ClaudeQuestionAnswer) -> Value {
    let extra = answer
        .extra_text
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(text) = extra {
        return Value::String(text.to_string());
    }

    match answer.selected_option_labels.as_slice() {
        [] => Value::String(String::new()),
        [label] => Value::String(label.clone()),
        labels => Value::Array(labels.iter().cloned().map(Value::String).collect()),
    }
}

fn native_resolution(payload: &Value, request_id: &str) -> Option<NativeResolution> {
    Some(NativeResolution {
        transcript_path: PathBuf::from(string_field(payload, "transcript_path")?),
        request_id: request_id.to_string(),
        tool_use_id: string_field(payload, "tool_use_id").map(str::to_string),
    })
}

fn native_review_resolved(resolution: Option<&NativeResolution>) -> bool {
    let Some(resolution) = resolution else {
        return false;
    };
    read_transcript_snapshot(&resolution.transcript_path)
        .map(|snapshot| {
            let tool_use_id = resolution
                .tool_use_id
                .as_ref()
                .or_else(|| snapshot.request_tool_use_ids.get(&resolution.request_id));
            tool_use_id
                .map(|tool_use_id| snapshot.resolved_tool_use_ids.contains(tool_use_id))
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

fn wait_for_question_answer(
    request_id: &str,
    native_resolution: Option<NativeResolution>,
) -> Result<Option<QuestionAnswerEnvelope>, String> {
    let path = question_answer_path(request_id);
    let started_at = Instant::now();

    loop {
        if path.exists() {
            let contents = fs::read_to_string(&path).map_err(|error| error.to_string())?;
            let _ = fs::remove_file(&path);
            return serde_json::from_str(&contents)
                .map(Some)
                .map_err(|error| error.to_string());
        }
        if native_review_resolved(native_resolution.as_ref())
            || started_at.elapsed() >= REVIEW_WAIT_TIMEOUT
        {
            return Ok(None);
        }
        thread::sleep(QUESTION_POLL_INTERVAL);
    }
}

fn wait_for_permission_decision(
    request_id: &str,
    native_resolution: Option<NativeResolution>,
) -> Result<Option<PermissionDecisionEnvelope>, String> {
    wait_for_decision_file(permission_decision_path(request_id), native_resolution).and_then(
        |envelope| match envelope {
            Some(value) => serde_json::from_value(value)
                .map(Some)
                .map_err(|error| error.to_string()),
            None => Ok(None),
        },
    )
}

fn wait_for_plan_decision(
    request_id: &str,
    native_resolution: Option<NativeResolution>,
) -> Result<Option<PlanDecisionEnvelope>, String> {
    wait_for_decision_file(plan_decision_path(request_id), native_resolution).and_then(|envelope| {
        match envelope {
            Some(value) => serde_json::from_value(value)
                .map(Some)
                .map_err(|error| error.to_string()),
            None => Ok(None),
        }
    })
}

fn wait_for_decision_file(
    path: PathBuf,
    native_resolution: Option<NativeResolution>,
) -> Result<Option<Value>, String> {
    let started_at = Instant::now();

    loop {
        if path.exists() {
            let contents = fs::read_to_string(&path).map_err(|error| error.to_string())?;
            let _ = fs::remove_file(&path);
            return serde_json::from_str(&contents)
                .map(Some)
                .map_err(|error| error.to_string());
        }
        if native_review_resolved(native_resolution.as_ref())
            || started_at.elapsed() >= REVIEW_WAIT_TIMEOUT
        {
            return Ok(None);
        }
        thread::sleep(QUESTION_POLL_INTERVAL);
    }
}

fn app_is_running() -> bool {
    let Ok(metadata) = fs::metadata(heartbeat_path()) else {
        return false;
    };
    let Ok(modified) = metadata.modified() else {
        return false;
    };
    modified
        .elapsed()
        .map(|elapsed| elapsed.as_secs() <= HEARTBEAT_STALE_SECONDS)
        .unwrap_or(false)
}

pub(crate) fn touch_app_heartbeat() -> Result<(), String> {
    let path = heartbeat_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(path, unix_time_ms().to_string()).map_err(|error| error.to_string())
}

pub(crate) fn submit_question_answer(
    request_id: &str,
    answers: &[ClaudeQuestionAnswer],
) -> Result<(), String> {
    write_decision_file(
        question_answer_path(request_id),
        &QuestionAnswerEnvelope {
            request_id: request_id.to_string(),
            answers: answers.to_vec(),
        },
    )
}

pub(crate) fn submit_permission_decision(
    request_id: &str,
    decision: PermissionDecision,
) -> Result<(), String> {
    write_decision_file(
        permission_decision_path(request_id),
        &PermissionDecisionEnvelope {
            request_id: request_id.to_string(),
            decision,
        },
    )
}

pub(crate) fn submit_plan_decision(
    request_id: &str,
    mode: PlanExecutionMode,
    note: Option<String>,
) -> Result<(), String> {
    write_decision_file(
        plan_decision_path(request_id),
        &PlanDecisionEnvelope {
            request_id: request_id.to_string(),
            mode,
            note: note
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
        },
    )
}

fn write_decision_file(path: PathBuf, envelope: &impl Serialize) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let serialized = serde_json::to_vec(envelope).map_err(|error| error.to_string())?;
    let temporary_path = path.with_extension("tmp");
    fs::write(&temporary_path, serialized).map_err(|error| error.to_string())?;
    let _ = fs::remove_file(&path);
    fs::rename(&temporary_path, &path).map_err(|error| error.to_string())
}

fn remove_stale_decision_file(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

pub(crate) fn install_claude_hooks(executable: &Path) -> Result<(), String> {
    let settings_path = claude_settings_path()?;
    let mut settings = if settings_path.exists() {
        let contents = fs::read_to_string(&settings_path).map_err(|error| error.to_string())?;
        serde_json::from_str::<Value>(&contents).map_err(|error| error.to_string())?
    } else {
        json!({})
    };
    let command = hook_command(executable);

    if !merge_hook_settings(&mut settings, &command)? {
        return Ok(());
    }

    if let Some(parent) = settings_path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let mut serialized =
        serde_json::to_string_pretty(&settings).map_err(|error| error.to_string())?;
    serialized.push('\n');
    fs::write(settings_path, serialized).map_err(|error| error.to_string())
}

#[cfg(windows)]
fn hook_command(executable: &Path) -> String {
    // Claude already executes command hooks through a shell. Nesting another
    // `cmd /c` makes that inner shell consume the hook JSON as commands instead
    // of forwarding it to CodeCraft's stdin.
    let executable = executable.display().to_string();
    let escaped = executable.replace('"', "\"\"");
    format!("\"{escaped}\" {HOOK_ARGUMENT}")
}

#[cfg(not(windows))]
fn hook_command(executable: &Path) -> String {
    let escaped = executable.display().to_string().replace('\'', "'\\''");
    format!("'{escaped}' {HOOK_ARGUMENT}")
}

fn claude_settings_path() -> Result<PathBuf, String> {
    env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
        .map(|home| home.join(".claude").join("settings.json"))
        .ok_or_else(|| "Unable to locate the user profile".to_string())
}

fn group_has_codecraft_hook(group: &Value) -> bool {
    group
        .get("hooks")
        .and_then(Value::as_array)
        .is_some_and(|commands| {
            commands.iter().any(|hook| {
                hook.get("command")
                    .and_then(Value::as_str)
                    .is_some_and(|command| command.contains(HOOK_ARGUMENT))
            })
        })
}

fn hook_settings_installed(settings: &Value) -> bool {
    let Some(hooks) = settings.get("hooks").and_then(Value::as_object) else {
        return false;
    };

    REGISTERED_HOOKS.iter().all(|(event_name, _)| {
        hooks
            .get(*event_name)
            .and_then(Value::as_array)
            .is_some_and(|groups| groups.iter().any(group_has_codecraft_hook))
    })
}

pub(crate) fn claude_hooks_installed() -> Result<bool, String> {
    let path = claude_settings_path()?;
    if !path.exists() {
        return Ok(false);
    }
    let contents = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let settings = serde_json::from_str::<Value>(&contents).map_err(|error| error.to_string())?;
    Ok(hook_settings_installed(&settings))
}

fn remove_codecraft_hooks(settings: &mut Value) -> Result<bool, String> {
    let before = settings.clone();
    let root = settings
        .as_object_mut()
        .ok_or_else(|| "Claude settings must contain a JSON object".to_string())?;
    let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) else {
        return Ok(false);
    };

    for (event_name, _) in REGISTERED_HOOKS {
        let remove_event =
            if let Some(groups) = hooks.get_mut(event_name).and_then(Value::as_array_mut) {
                for group in groups.iter_mut() {
                    if let Some(commands) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                        commands.retain(|hook| {
                            !hook
                                .get("command")
                                .and_then(Value::as_str)
                                .is_some_and(|command| command.contains(HOOK_ARGUMENT))
                        });
                    }
                }
                groups.retain(|group| {
                    group
                        .get("hooks")
                        .and_then(Value::as_array)
                        .is_none_or(|commands| !commands.is_empty())
                });
                groups.is_empty()
            } else {
                false
            };
        if remove_event {
            hooks.remove(event_name);
        }
    }

    Ok(*settings != before)
}

pub(crate) fn uninstall_claude_hooks() -> Result<(), String> {
    let path = claude_settings_path()?;
    if !path.exists() {
        return Ok(());
    }
    let contents = fs::read_to_string(&path).map_err(|error| error.to_string())?;
    let mut settings =
        serde_json::from_str::<Value>(&contents).map_err(|error| error.to_string())?;
    if !remove_codecraft_hooks(&mut settings)? {
        return Ok(());
    }
    let mut serialized =
        serde_json::to_string_pretty(&settings).map_err(|error| error.to_string())?;
    serialized.push('\n');
    fs::write(path, serialized).map_err(|error| error.to_string())
}

fn merge_hook_settings(settings: &mut Value, command: &str) -> Result<bool, String> {
    let before = settings.clone();
    let root = settings
        .as_object_mut()
        .ok_or_else(|| "Claude settings must contain a JSON object".to_string())?;
    let hooks = root.entry("hooks").or_insert_with(|| json!({}));
    let hooks = hooks
        .as_object_mut()
        .ok_or_else(|| "Claude settings hooks must contain a JSON object".to_string())?;

    for (event_name, matcher) in REGISTERED_HOOKS {
        let groups = hooks.entry(event_name).or_insert_with(|| json!([]));
        let groups = groups
            .as_array_mut()
            .ok_or_else(|| format!("Claude hook {event_name} must contain an array"))?;

        for group in groups.iter_mut() {
            if let Some(commands) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                commands.retain(|hook| {
                    !hook
                        .get("command")
                        .and_then(Value::as_str)
                        .map(|value| value.contains(HOOK_ARGUMENT))
                        .unwrap_or(false)
                });
            }
        }
        groups.retain(|group| {
            group
                .get("hooks")
                .and_then(Value::as_array)
                .map(|commands| !commands.is_empty())
                .unwrap_or(true)
        });
        let is_permission_request = event_name == "PermissionRequest";
        let mut command_config = json!({
            "type": "command",
            "command": command,
            "timeout": if is_permission_request {
                HOOK_TIMEOUT_SECONDS
            } else {
                5
            }
        });
        if !is_permission_request {
            command_config["async"] = Value::Bool(true);
        }
        let mut group = json!({
            "hooks": [command_config]
        });
        if let Some(matcher) = matcher {
            group["matcher"] = Value::String(matcher.to_string());
        }
        groups.push(group);
    }

    Ok(*settings != before)
}

fn hook_inbox_dir() -> PathBuf {
    app_data_dir().join("claude-hooks")
}

fn app_data_dir() -> PathBuf {
    env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir)
        .join("CodeCraft")
}

fn answer_dir() -> PathBuf {
    app_data_dir().join("claude-answers")
}

fn permission_dir() -> PathBuf {
    app_data_dir().join("claude-permissions")
}

fn question_answer_path(request_id: &str) -> PathBuf {
    answer_dir().join(format!("{}.json", safe_answer_stem(request_id)))
}

fn permission_decision_path(request_id: &str) -> PathBuf {
    permission_dir().join(format!("{}.json", safe_answer_stem(request_id)))
}

fn plan_dir() -> PathBuf {
    app_data_dir().join("claude-plans")
}

fn plan_decision_path(request_id: &str) -> PathBuf {
    plan_dir().join(format!("{}.json", safe_answer_stem(request_id)))
}

fn safe_answer_stem(request_id: &str) -> String {
    request_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect()
}

pub(crate) fn heartbeat_path() -> PathBuf {
    app_data_dir().join("app-running")
}

fn string_field<'a>(payload: &'a Value, field: &str) -> Option<&'a str> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

fn question_from_value(prompt: &Value) -> Option<ClaudeQuestion> {
    let question = string_field(prompt, "question")?.trim().to_string();
    let header = string_field(prompt, "header").map(|value| value.trim().to_string());
    let multi_select = prompt
        .get("multiSelect")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let options = prompt
        .get("options")
        .and_then(Value::as_array)
        .map(|options| {
            options
                .iter()
                .filter_map(|option| {
                    let label = string_field(option, "label")?.trim().to_string();
                    let description =
                        string_field(option, "description").map(|value| value.trim().to_string());
                    Some(ClaudeQuestionOption { label, description })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Some(ClaudeQuestion {
        header,
        question,
        options,
        multi_select,
    })
}

fn question_request_from_payload(payload: &Value) -> Option<ClaudeQuestionRequest> {
    let id = question_request_id(payload)?;
    let questions = payload
        .get("tool_input")?
        .get("questions")?
        .as_array()?
        .iter()
        .filter_map(question_from_value)
        .collect::<Vec<_>>();
    if questions.is_empty() {
        return None;
    }

    Some(ClaudeQuestionRequest {
        id,
        questions,
        tool_use_id: string_field(payload, "tool_use_id").map(str::to_string),
    })
}

fn question_request_id(payload: &Value) -> Option<String> {
    if !matches!(
        string_field(payload, "hook_event_name"),
        Some("PreToolUse") | Some("PermissionRequest")
    ) {
        return None;
    }
    if string_field(payload, "tool_name") != Some("AskUserQuestion") {
        return None;
    }
    let questions = payload.get("tool_input")?.get("questions")?.as_array()?;
    if questions.is_empty() {
        return None;
    }

    Some(scoped_request_id(
        "question",
        payload,
        &question_content_hash(questions),
    ))
}

fn question_content_hash(questions: &[Value]) -> String {
    let serialized = serde_json::to_string(questions).unwrap_or_default();
    stable_hash(&serialized)
}

fn stable_hash(serialized: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in serialized.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn session_id(payload: &Value) -> Option<&str> {
    string_field(payload, "session_id").or_else(|| string_field(payload, "sessionId"))
}

fn scoped_request_id(prefix: &str, payload: &Value, content_hash: &str) -> String {
    match session_id(payload) {
        Some(session_id) => format!("{prefix}-{}-{content_hash}", stable_hash(session_id)),
        None => format!("{prefix}-{content_hash}"),
    }
}

fn permission_content_hash(tool: &str, input: &Value) -> String {
    let serialized =
        serde_json::to_string(&json!({ "tool": tool, "input": input })).unwrap_or_default();
    stable_hash(&serialized)
}

fn permission_request_id(payload: &Value) -> Option<String> {
    if string_field(payload, "hook_event_name") != Some("PermissionRequest") {
        return None;
    }
    if string_field(payload, "tool_name") == Some("AskUserQuestion") {
        return None;
    }
    let tool = string_field(payload, "tool_name")?;
    if let Some(tool_use_id) = string_field(payload, "tool_use_id") {
        return Some(format!("permission-{tool_use_id}"));
    }

    let input = payload.get("tool_input").cloned().unwrap_or(Value::Null);
    Some(scoped_request_id(
        "permission",
        payload,
        &permission_content_hash(tool, &input),
    ))
}

fn plan_request_id(payload: &Value) -> Option<String> {
    permission_request_id(payload).map(|id| id.replacen("permission-", "plan-", 1))
}

fn is_plan_tool(payload: &Value) -> bool {
    let Some(tool_name) = string_field(payload, "tool_name") else {
        return false;
    };
    is_plan_tool_name(tool_name)
}

fn is_plan_tool_name(tool_name: &str) -> bool {
    let normalized = tool_name.to_ascii_lowercase();
    normalized == "plan" || normalized.starts_with("exitplan")
}

fn plan_request_from_payload(payload: &Value) -> Option<ClaudePlanRequest> {
    if !is_plan_tool(payload) {
        return None;
    }
    let id = plan_request_id(payload)?;
    let tool_name = string_field(payload, "tool_name")?.to_string();
    let plan = plan_text(payload);
    let cwd = string_field(payload, "cwd").map(str::to_string);

    Some(ClaudePlanRequest {
        id,
        tool_name,
        plan,
        cwd,
        captured_at: unix_time_ms(),
        tool_use_id: string_field(payload, "tool_use_id").map(str::to_string),
    })
}

fn plan_text(payload: &Value) -> String {
    let input = payload.get("tool_input").unwrap_or(&Value::Null);
    if let Some(plan) = input.get("plan").and_then(Value::as_str) {
        return plan.trim().to_string();
    }

    match input {
        Value::Null => "计划内容为空".to_string(),
        value => serde_json::to_string_pretty(value)
            .unwrap_or_else(|error| format!("无法读取计划内容：{error}")),
    }
}

fn permission_request_from_payload(payload: &Value) -> Option<ClaudePermissionRequest> {
    if is_plan_tool(payload) {
        return None;
    }
    let id = permission_request_id(payload)?;
    let tool_name = string_field(payload, "tool_name")?.to_string();
    let summary = tool_summary(&tool_name, payload.get("tool_input"));
    let cwd = string_field(payload, "cwd").map(str::to_string);
    let can_always_allow = payload
        .get("permission_suggestions")
        .and_then(Value::as_array)
        .map(|suggestions| !suggestions.is_empty())
        .unwrap_or(false);

    Some(ClaudePermissionRequest {
        id,
        tool_name,
        summary,
        cwd,
        can_always_allow,
        captured_at: unix_time_ms(),
        tool_use_id: string_field(payload, "tool_use_id").map(str::to_string),
    })
}

fn event_title(payload: &Value) -> Option<String> {
    ["session_name", "title", "prompt"]
        .iter()
        .find_map(|field| string_field(payload, field))
        .map(normalize_title)
        .filter(|value| !value.is_empty())
}

fn normalize_title(value: &str) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut characters = normalized.chars();
    let title = characters.by_ref().take(64).collect::<String>();

    if characters.next().is_some() {
        format!("{title}...")
    } else {
        title
    }
}

fn directory_title(value: &str) -> Option<String> {
    Path::new(value)
        .file_name()
        .and_then(|name| name.to_str())
        .map(normalize_title)
        .filter(|title| !title.is_empty())
}

fn tool_summary(tool: &str, input: Option<&Value>) -> String {
    let input = input.unwrap_or(&Value::Null);
    let preferred_fields: &[&str] = match tool {
        "Bash" => &["command", "description"],
        "Read" | "Write" | "Edit" | "NotebookEdit" => &["file_path", "notebook_path"],
        "Glob" | "Grep" => &["pattern", "path"],
        "WebFetch" => &["url", "prompt"],
        "WebSearch" => &["query"],
        "Task" => &["description", "prompt"],
        _ => &[
            "description",
            "file_path",
            "path",
            "command",
            "query",
            "url",
            "prompt",
        ],
    };

    let summary = preferred_fields
        .iter()
        .find_map(|field| string_field(input, field))
        .map(str::to_string)
        .or_else(|| {
            if input.is_null() {
                None
            } else {
                serde_json::to_string(input).ok()
            }
        })
        .unwrap_or_else(|| "正在执行工具".to_string());

    truncate_text(&summary, 180)
}

#[cfg(test)]
fn read_transcript_outputs(path: &Path) -> Result<Vec<ClaudeOutputEntry>, String> {
    Ok(read_transcript_snapshot(path)?.outputs)
}

fn read_transcript_snapshot(path: &Path) -> Result<TranscriptSnapshot, String> {
    let mut file = fs::File::open(path).map_err(|error| error.to_string())?;
    let length = file.metadata().map_err(|error| error.to_string())?.len();
    let start = length.saturating_sub(MAX_TRANSCRIPT_BYTES);
    file.seek(SeekFrom::Start(start))
        .map_err(|error| error.to_string())?;
    let mut reader = BufReader::new(file);

    if start > 0 {
        let mut partial_line = String::new();
        reader
            .read_line(&mut partial_line)
            .map_err(|error| error.to_string())?;
    }

    let mut snapshot = TranscriptSnapshot::default();
    for line in reader.lines().map_while(Result::ok) {
        let Ok(record) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        collect_tool_result_ids(&record, &mut snapshot.resolved_tool_use_ids);
        collect_request_tool_use_ids(&record, &record, &mut snapshot.request_tool_use_ids);
        let Some(message) = record.get("message") else {
            continue;
        };
        if string_field(message, "role") != Some("assistant") {
            continue;
        }

        let base_id = string_field(&record, "uuid")
            .or_else(|| string_field(message, "id"))
            .unwrap_or("assistant");
        let mut append_text = |index: usize, text: &str| {
            let normalized = text.trim();
            if normalized.is_empty()
                || snapshot
                    .outputs
                    .last()
                    .map(|entry: &ClaudeOutputEntry| entry.text == normalized)
                    .unwrap_or(false)
            {
                return;
            }
            snapshot.outputs.push(ClaudeOutputEntry {
                id: format!("{base_id}-{index}"),
                text: truncate_text(normalized, MAX_OUTPUT_ENTRY_CHARS),
            });
        };

        match message.get("content") {
            Some(Value::String(text)) => append_text(0, text),
            Some(Value::Array(blocks)) => {
                for (index, block) in blocks.iter().enumerate() {
                    if string_field(block, "type") == Some("text") {
                        if let Some(text) = string_field(block, "text") {
                            append_text(index, text);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    if snapshot.outputs.len() > MAX_OUTPUT_ENTRIES {
        snapshot
            .outputs
            .drain(0..snapshot.outputs.len() - MAX_OUTPUT_ENTRIES);
    }
    Ok(snapshot)
}

fn collect_tool_result_ids(value: &Value, resolved_tool_use_ids: &mut HashSet<String>) {
    match value {
        Value::Object(object) => {
            if object.get("type").and_then(Value::as_str) == Some("tool_result") {
                if let Some(tool_use_id) = object.get("tool_use_id").and_then(Value::as_str) {
                    if !tool_use_id.trim().is_empty() {
                        resolved_tool_use_ids.insert(tool_use_id.to_string());
                    }
                }
            }
            for child in object.values() {
                collect_tool_result_ids(child, resolved_tool_use_ids);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_tool_result_ids(item, resolved_tool_use_ids);
            }
        }
        _ => {}
    }
}

fn collect_request_tool_use_ids(
    record: &Value,
    value: &Value,
    request_tool_use_ids: &mut HashMap<String, String>,
) {
    match value {
        Value::Object(object) => {
            if object.get("type").and_then(Value::as_str) == Some("tool_use") {
                let tool_use_id = object.get("id").and_then(Value::as_str);
                let tool_name = object.get("name").and_then(Value::as_str);
                let input = object.get("input").unwrap_or(&Value::Null);
                if let (Some(tool_use_id), Some(tool_name)) = (tool_use_id, tool_name) {
                    if let Some(request_id) = transcript_review_request_id(record, tool_name, input)
                    {
                        request_tool_use_ids.insert(request_id, tool_use_id.to_string());
                    }
                }
            }
            for child in object.values() {
                collect_request_tool_use_ids(record, child, request_tool_use_ids);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_request_tool_use_ids(record, item, request_tool_use_ids);
            }
        }
        _ => {}
    }
}

fn transcript_review_request_id(record: &Value, tool_name: &str, input: &Value) -> Option<String> {
    if tool_name == "AskUserQuestion" {
        let questions = input.get("questions")?.as_array()?;
        if questions.is_empty() {
            return None;
        }
        return Some(scoped_request_id(
            "question",
            record,
            &question_content_hash(questions),
        ));
    }

    let prefix = if is_plan_tool_name(tool_name) {
        "plan"
    } else {
        "permission"
    };
    Some(scoped_request_id(
        prefix,
        record,
        &permission_content_hash(tool_name, input),
    ))
}

fn completion_request_id(payload: &Value) -> Option<String> {
    let tool_name = string_field(payload, "tool_name")?;
    let input = payload.get("tool_input")?;
    transcript_review_request_id(payload, tool_name, input)
}

fn truncate_text(value: &str, max_characters: usize) -> String {
    let mut characters = value.chars();
    let truncated = characters.by_ref().take(max_characters).collect::<String>();
    if characters.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

fn status_rank(status: ClaudeSessionStatus) -> u8 {
    match status {
        ClaudeSessionStatus::Working => 0,
        ClaudeSessionStatus::ToolFailed => 1,
        ClaudeSessionStatus::Attention => 2,
        ClaudeSessionStatus::Waiting => 3,
        ClaudeSessionStatus::Stopped => 4,
        ClaudeSessionStatus::Idle => 5,
    }
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn windows_hook_command_relies_on_claudes_shell_and_quotes_the_executable() {
        let command = hook_command(Path::new(r"C:\Program Files\CodeCraft\codecraft-tauri.exe"));

        assert_eq!(
            command,
            r#""C:\Program Files\CodeCraft\codecraft-tauri.exe" --codecraft-claude-hook"#
        );
        assert!(!command.to_ascii_lowercase().contains("cmd.exe"));
    }

    #[test]
    fn marks_a_session_as_stopped_after_stop() {
        let mut store = ClaudeSessionStore::default();
        store.apply_event(
            &json!({
                "session_id": "session-1",
                "hook_event_name": "SessionStart",
                "cwd": "C:\\work\\CodeCraft"
            }),
            1_000,
        );
        store.apply_event(
            &json!({
                "session_id": "session-1",
                "hook_event_name": "UserPromptSubmit",
                "prompt": "Add a compact session list"
            }),
            2_000,
        );
        store.apply_event(
            &json!({
                "session_id": "session-1",
                "hook_event_name": "Stop"
            }),
            3_000,
        );

        assert_eq!(
            store.sessions(),
            vec![ClaudeSession {
                id: "session-1".to_string(),
                status: ClaudeSessionStatus::Stopped,
                title: "Add a compact session list".to_string(),
                started_at: 1_000,
                updated_at: 3_000,
                question: None,
                permission: None,
                plan: None,
                activities: Vec::new(),
                outputs: Vec::new(),
                transcript_path: None,
                transcript_signature: None,
            }]
        );
    }

    #[test]
    fn removes_a_session_when_claude_ends_it() {
        let mut store = ClaudeSessionStore::default();
        store.apply_event(
            &json!({
                "session_id": "session-1",
                "hook_event_name": "SessionStart"
            }),
            1_000,
        );
        store.apply_event(
            &json!({
                "session_id": "session-1",
                "hook_event_name": "SessionEnd"
            }),
            2_000,
        );

        assert!(store.sessions().is_empty());
    }

    #[test]
    fn merges_codecraft_hooks_without_replacing_existing_hooks() {
        let mut settings = json!({
            "hooks": {
                "Stop": [{
                    "hooks": [{
                        "type": "command",
                        "command": "existing-hook"
                    }]
                }]
            }
        });

        assert!(merge_hook_settings(
            &mut settings,
            "\"C:\\CodeCraft.exe\" --codecraft-claude-hook"
        )
        .unwrap());
        assert!(!merge_hook_settings(
            &mut settings,
            "\"C:\\CodeCraft.exe\" --codecraft-claude-hook"
        )
        .unwrap());

        let stop_groups = settings["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop_groups.len(), 2);
        assert_eq!(
            stop_groups[0]["hooks"][0]["command"],
            Value::String("existing-hook".to_string())
        );
        assert_eq!(stop_groups[1]["hooks"][0]["async"], Value::Bool(true));

        let pre_tool_groups = settings["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre_tool_groups.len(), 1);
        assert!(pre_tool_groups[0].get("matcher").is_none());
        assert_eq!(
            settings["hooks"]["PostToolUseFailure"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn detects_and_removes_only_codecraft_hooks() {
        let mut settings = json!({
            "hooks": {
                "Stop": [{"hooks": [{"type": "command", "command": "existing-hook"}]}]
            }
        });
        merge_hook_settings(&mut settings, "codecraft --codecraft-claude-hook").unwrap();
        assert!(hook_settings_installed(&settings));
        assert!(remove_codecraft_hooks(&mut settings).unwrap());
        assert!(!hook_settings_installed(&settings));
        assert_eq!(
            settings["hooks"]["Stop"][0]["hooks"][0]["command"],
            "existing-hook"
        );
    }

    #[test]
    fn migrates_the_old_permission_hook_and_preserves_third_party_hooks() {
        let mut settings = json!({
            "hooks": {
                "PermissionRequest": [
                    {
                        "hooks": [{
                            "type": "command",
                            "command": "third-party-hook",
                            "timeout": 12
                        }]
                    },
                    {
                        "hooks": [{
                            "type": "command",
                            "command": "\"C:\\OldCodeCraft.exe\" --codecraft-claude-hook",
                            "timeout": 330
                        }]
                    }
                ]
            }
        });

        merge_hook_settings(
            &mut settings,
            "\"C:\\CodeCraft.exe\" --codecraft-claude-hook",
        )
        .unwrap();

        let groups = settings["hooks"]["PermissionRequest"].as_array().unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0]["hooks"][0]["command"], "third-party-hook");
        assert_eq!(groups[1]["hooks"][0]["timeout"], 330);
        assert!(groups[1]["hooks"][0].get("async").is_none());
        assert_eq!(
            settings["hooks"]["PermissionDenied"][0]["hooks"][0]["async"],
            true
        );
    }

    #[test]
    fn permission_hook_can_fall_back_to_claude_code() {
        assert_eq!(
            permission_decision_response("ask")["hookSpecificOutput"]["decision"]["behavior"],
            "ask"
        );
        assert_eq!(HOOK_TIMEOUT_SECONDS, 330);
        assert_eq!(REVIEW_WAIT_TIMEOUT.as_secs(), HOOK_TIMEOUT_SECONDS);
    }

    #[test]
    fn tracks_tool_activity_from_start_to_completion() {
        let mut store = ClaudeSessionStore::default();
        store.apply_event(
            &json!({
                "session_id": "session-1",
                "hook_event_name": "PreToolUse",
                "tool_name": "Read",
                "tool_use_id": "tool-1",
                "tool_input": { "file_path": "C:\\work\\src\\main.rs" }
            }),
            1_000,
        );

        let sessions = store.sessions();
        assert_eq!(sessions[0].activities.len(), 1);
        assert_eq!(sessions[0].activities[0].tool, "Read");
        assert_eq!(sessions[0].activities[0].summary, "C:\\work\\src\\main.rs");
        assert_eq!(
            sessions[0].activities[0].status,
            ClaudeActivityStatus::Running
        );

        store.apply_event(
            &json!({
                "session_id": "session-1",
                "hook_event_name": "PostToolUse",
                "tool_name": "Read",
                "tool_use_id": "tool-1"
            }),
            2_000,
        );

        let sessions = store.sessions();
        assert_eq!(sessions[0].activities.len(), 1);
        assert_eq!(
            sessions[0].activities[0].status,
            ClaudeActivityStatus::Completed
        );
        assert_eq!(sessions[0].activities[0].updated_at, 2_000);
    }

    #[test]
    fn marks_a_session_and_activity_when_a_tool_call_fails() {
        let mut store = ClaudeSessionStore::default();
        store.apply_event(
            &json!({
                "session_id": "session-1",
                "hook_event_name": "PreToolUse",
                "tool_name": "Bash",
                "tool_use_id": "tool-1",
                "tool_input": { "command": "exit 1" }
            }),
            1_000,
        );
        store.apply_event(
            &json!({
                "session_id": "session-1",
                "hook_event_name": "PostToolUseFailure",
                "tool_name": "Bash",
                "tool_use_id": "tool-1",
                "tool_input": { "command": "exit 1" }
            }),
            2_000,
        );

        let sessions = store.sessions();
        assert_eq!(sessions[0].status, ClaudeSessionStatus::ToolFailed);
        assert_eq!(sessions[0].activities.len(), 1);
        assert_eq!(
            sessions[0].activities[0].status,
            ClaudeActivityStatus::Failed
        );
        assert_eq!(sessions[0].activities[0].updated_at, 2_000);
    }

    #[test]
    fn reads_assistant_text_from_a_transcript() {
        let path = env::temp_dir().join(format!(
            "codecraft-transcript-{}-{}.jsonl",
            process::id(),
            unix_time_ms()
        ));
        fs::write(
            &path,
            concat!(
                "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}\n",
                "{\"uuid\":\"answer-1\",\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"正在检查代码\"},{\"type\":\"tool_use\",\"name\":\"Read\"}]}}\n",
                "{\"uuid\":\"answer-2\",\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":\"检查完成\"}}\n"
            ),
        )
        .unwrap();

        let outputs = read_transcript_outputs(&path).unwrap();
        let _ = fs::remove_file(path);

        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0].text, "正在检查代码");
        assert_eq!(outputs[1].text, "检查完成");
    }

    #[test]
    fn transcript_tool_results_resolve_exact_review_ids() {
        let path = env::temp_dir().join(format!(
            "codecraft-results-{}-{}.jsonl",
            process::id(),
            unix_time_ms()
        ));
        let transcript_path = path.to_string_lossy().to_string();
        let mut store = ClaudeSessionStore::default();
        store.apply_event(
            &json!({
                "session_id": "session-1",
                "hook_event_name": "PreToolUse",
                "tool_name": "AskUserQuestion",
                "tool_use_id": "question-tool",
                "transcript_path": transcript_path,
                "tool_input": {
                    "questions": [{ "question": "Choose", "options": [] }]
                }
            }),
            1_000,
        );
        store.apply_event(
            &json!({
                "session_id": "session-1",
                "hook_event_name": "PermissionRequest",
                "tool_name": "Bash",
                "tool_use_id": "permission-tool",
                "tool_input": { "command": "git status" }
            }),
            1_100,
        );
        store.apply_event(
            &json!({
                "session_id": "session-1",
                "hook_event_name": "PermissionRequest",
                "tool_name": "ExitPlanMode",
                "tool_use_id": "plan-tool",
                "tool_input": { "plan": "Run the plan" }
            }),
            1_200,
        );

        fs::write(
            &path,
            concat!(
                "{\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"question-tool\",\"content\":\"answered\"}]}}\n",
                "{\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"unrelated-tool\",\"content\":\"done\"}]}}\n",
                "{\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"plan-tool\",\"content\":\"approved\"}]}}\n",
                "{\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"permission-tool\""
            ),
        )
        .unwrap();

        store
            .sessions
            .get_mut("session-1")
            .unwrap()
            .refresh_transcript();
        let session = store.sessions.get("session-1").unwrap();
        assert!(session.question.is_none());
        assert!(session.plan.is_none());
        assert!(session.permission.is_some());

        fs::write(
            &path,
            "{\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"permission-tool\",\"is_error\":true,\"content\":\"denied\"}]}}\n",
        )
        .unwrap();
        store
            .sessions
            .get_mut("session-1")
            .unwrap()
            .refresh_transcript();
        let _ = fs::remove_file(path);

        assert!(store
            .sessions
            .get("session-1")
            .unwrap()
            .permission
            .is_none());
    }

    #[test]
    fn binds_permission_requests_without_hook_ids_to_transcript_tool_uses() {
        let path = env::temp_dir().join(format!(
            "codecraft-fallback-id-{}-{}.jsonl",
            process::id(),
            unix_time_ms()
        ));
        let transcript_path = path.to_string_lossy().to_string();
        let mut store = ClaudeSessionStore::default();
        store.apply_event(
            &json!({
                "session_id": "fallback-session",
                "hook_event_name": "PermissionRequest",
                "tool_name": "Bash",
                "transcript_path": transcript_path,
                "tool_input": { "command": "git status" }
            }),
            1_000,
        );

        let request_id = {
            let sessions = store.sessions();
            let permission = sessions[0].permission.as_ref().unwrap();
            assert!(permission.tool_use_id.is_none());
            permission.id.clone()
        };

        fs::write(
            &path,
            concat!(
                "{\"sessionId\":\"fallback-session\",\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"tool_use\",\"id\":\"call-fallback\",\"name\":\"Bash\",\"input\":{\"command\":\"git status\"}}]}}\n",
                "{\"sessionId\":\"fallback-session\",\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"call-fallback\",\"is_error\":true,\"content\":\"denied\"}]}}\n"
            ),
        )
        .unwrap();

        store
            .sessions
            .get_mut("fallback-session")
            .unwrap()
            .refresh_transcript();

        assert!(store.sessions()[0].permission.is_none());
        assert!(native_review_resolved(Some(&NativeResolution {
            transcript_path: path.clone(),
            request_id,
            tool_use_id: None,
        })));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn binds_all_review_types_without_hook_ids_and_ignores_unrelated_results() {
        let path = env::temp_dir().join(format!(
            "codecraft-all-fallback-ids-{}-{}.jsonl",
            process::id(),
            unix_time_ms()
        ));
        let transcript_path = path.to_string_lossy().to_string();
        let mut store = ClaudeSessionStore::default();

        for (tool_name, tool_input) in [
            (
                "AskUserQuestion",
                json!({ "questions": [{ "question": "Choose", "options": [] }] }),
            ),
            ("Bash", json!({ "command": "git status" })),
            ("ExitPlanMode", json!({ "plan": "Run the plan" })),
        ] {
            store.apply_event(
                &json!({
                    "session_id": "all-fallback-session",
                    "hook_event_name": "PermissionRequest",
                    "tool_name": tool_name,
                    "transcript_path": transcript_path,
                    "tool_input": tool_input
                }),
                1_000,
            );
        }

        fs::write(
            &path,
            concat!(
                "{\"sessionId\":\"all-fallback-session\",\"message\":{\"role\":\"assistant\",\"content\":[",
                "{\"type\":\"tool_use\",\"id\":\"question-call\",\"name\":\"AskUserQuestion\",\"input\":{\"questions\":[{\"question\":\"Choose\",\"options\":[]}]}},",
                "{\"type\":\"tool_use\",\"id\":\"permission-call\",\"name\":\"Bash\",\"input\":{\"command\":\"git status\"}},",
                "{\"type\":\"tool_use\",\"id\":\"plan-call\",\"name\":\"ExitPlanMode\",\"input\":{\"plan\":\"Run the plan\"}}]}}\n",
                "{\"sessionId\":\"all-fallback-session\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"unrelated-call\"}]}}\n"
            ),
        )
        .unwrap();

        let session = store.sessions.get_mut("all-fallback-session").unwrap();
        session.refresh_transcript();
        assert_eq!(
            session.question.as_ref().unwrap().tool_use_id.as_deref(),
            Some("question-call")
        );
        assert_eq!(
            session.permission.as_ref().unwrap().tool_use_id.as_deref(),
            Some("permission-call")
        );
        assert_eq!(
            session.plan.as_ref().unwrap().tool_use_id.as_deref(),
            Some("plan-call")
        );

        let mut transcript = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(
            transcript,
            "{}",
            json!({
                "sessionId": "all-fallback-session",
                "message": {
                    "role": "user",
                    "content": [
                        { "type": "tool_result", "tool_use_id": "question-call" },
                        { "type": "tool_result", "tool_use_id": "plan-call" }
                    ]
                }
            })
        )
        .unwrap();
        drop(transcript);

        session.refresh_transcript();
        assert!(session.question.is_none());
        assert!(session.permission.is_some());
        assert!(session.plan.is_none());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn completion_hooks_bind_missing_ids_from_request_content() {
        let mut store = ClaudeSessionStore::default();
        store.apply_event(
            &json!({
                "session_id": "content-fallback-session",
                "hook_event_name": "PermissionRequest",
                "tool_name": "Bash",
                "tool_input": { "command": "git status" }
            }),
            1_000,
        );
        assert!(store.sessions()[0].permission.is_some());

        store.apply_event(
            &json!({
                "session_id": "content-fallback-session",
                "hook_event_name": "PermissionDenied",
                "tool_name": "Bash",
                "tool_use_id": "permission-call",
                "tool_input": { "command": "git status" }
            }),
            2_000,
        );

        assert!(store.sessions()[0].permission.is_none());
    }

    #[test]
    fn captures_all_ask_user_questions_until_the_tool_finishes() {
        let mut store = ClaudeSessionStore::default();
        store.apply_event(
            &json!({
                "session_id": "session-1",
                "hook_event_name": "PreToolUse",
                "tool_name": "AskUserQuestion",
                "tool_use_id": "tool-1",
                "tool_input": {
                    "questions": [
                        {
                            "header": "实现方式",
                            "question": "你希望使用哪一种实现方式？",
                            "multiSelect": false,
                            "options": [{
                                "label": "方案 A",
                                "description": "保持当前结构"
                            }]
                        },
                        {
                            "header": "验证范围",
                            "question": "需要执行哪些验证？",
                            "multiSelect": true,
                            "options": [{
                                "label": "单元测试",
                                "description": "运行自动化测试"
                            }]
                        },
                        {
                            "header": "无效项",
                            "options": []
                        }
                    ]
                }
            }),
            1_000,
        );

        let sessions = store.sessions();
        assert_eq!(sessions[0].status, ClaudeSessionStatus::Waiting);
        assert_eq!(
            sessions[0].question,
            Some(ClaudeQuestionRequest {
                id: "question-1688595dcf49184d-16f9e3874d66e40a".to_string(),
                questions: vec![
                    ClaudeQuestion {
                        header: Some("实现方式".to_string()),
                        question: "你希望使用哪一种实现方式？".to_string(),
                        options: vec![ClaudeQuestionOption {
                            label: "方案 A".to_string(),
                            description: Some("保持当前结构".to_string()),
                        }],
                        multi_select: false,
                    },
                    ClaudeQuestion {
                        header: Some("验证范围".to_string()),
                        question: "需要执行哪些验证？".to_string(),
                        options: vec![ClaudeQuestionOption {
                            label: "单元测试".to_string(),
                            description: Some("运行自动化测试".to_string()),
                        }],
                        multi_select: true,
                    },
                ],
                tool_use_id: Some("tool-1".to_string()),
            })
        );

        store.apply_event(
            &json!({
                "session_id": "session-1",
                "hook_event_name": "PostToolUse",
                "tool_name": "AskUserQuestion",
                "tool_use_id": "tool-1"
            }),
            2_000,
        );

        let sessions = store.sessions();
        assert_eq!(sessions[0].status, ClaudeSessionStatus::Working);
        assert_eq!(sessions[0].question, None);
    }

    #[test]
    fn duplicate_review_events_preserve_existing_tool_use_ids() {
        let mut store = ClaudeSessionStore::default();
        let question_input = json!({
            "questions": [{ "question": "Choose", "options": [] }]
        });
        store.apply_event(
            &json!({
                "session_id": "question-session",
                "hook_event_name": "PreToolUse",
                "tool_name": "AskUserQuestion",
                "tool_use_id": "question-tool",
                "tool_input": question_input.clone()
            }),
            1_000,
        );
        store.apply_event(
            &json!({
                "session_id": "question-session",
                "hook_event_name": "PermissionRequest",
                "tool_name": "AskUserQuestion",
                "tool_input": question_input
            }),
            1_100,
        );

        for (session_id, tool_name, tool_use_id, tool_input) in [
            (
                "permission-session",
                "Bash",
                "permission-tool",
                json!({ "command": "git status" }),
            ),
            (
                "plan-session",
                "ExitPlanMode",
                "plan-tool",
                json!({ "plan": "Run the plan" }),
            ),
        ] {
            store.apply_event(
                &json!({
                    "session_id": session_id,
                    "hook_event_name": "PermissionRequest",
                    "tool_name": tool_name,
                    "tool_use_id": tool_use_id,
                    "tool_input": tool_input.clone()
                }),
                1_000,
            );
            store.apply_event(
                &json!({
                    "session_id": session_id,
                    "hook_event_name": "PermissionRequest",
                    "tool_name": tool_name,
                    "tool_input": tool_input
                }),
                1_100,
            );
        }

        let sessions = store.sessions();
        let question = sessions
            .iter()
            .find(|session| session.id == "question-session")
            .unwrap()
            .question
            .as_ref()
            .unwrap();
        assert_eq!(question.tool_use_id.as_deref(), Some("question-tool"));
        let permission = sessions
            .iter()
            .find(|session| session.id == "permission-session")
            .unwrap()
            .permission
            .as_ref()
            .unwrap();
        assert_eq!(permission.id, "permission-permission-tool");
        assert_eq!(permission.tool_use_id.as_deref(), Some("permission-tool"));
        let plan = sessions
            .iter()
            .find(|session| session.id == "plan-session")
            .unwrap()
            .plan
            .as_ref()
            .unwrap();
        assert_eq!(plan.id, "plan-plan-tool");
        assert_eq!(plan.tool_use_id.as_deref(), Some("plan-tool"));
    }

    #[test]
    fn internal_tool_use_ids_are_not_serialized_to_the_frontend() {
        let request = ClaudeQuestionRequest {
            id: "question-1".to_string(),
            questions: vec![ClaudeQuestion {
                header: None,
                question: "Choose".to_string(),
                options: Vec::new(),
                multi_select: false,
            }],
            tool_use_id: Some("tool-1".to_string()),
        };

        let value = serde_json::to_value(request).unwrap();
        assert_eq!(value["id"], "question-1");
        assert!(value.get("toolUseId").is_none());
    }

    #[test]
    fn registers_permission_request_as_a_synchronous_hook_for_all_tools() {
        let mut settings = json!({});
        merge_hook_settings(
            &mut settings,
            "\"C:\\CodeCraft.exe\" --codecraft-claude-hook",
        )
        .unwrap();

        let groups = settings["hooks"]["PermissionRequest"].as_array().unwrap();
        assert_eq!(groups.len(), 1);
        assert!(groups[0].get("matcher").is_none());
        assert!(groups[0]["hooks"][0].get("async").is_none());
        assert_eq!(groups[0]["hooks"][0]["timeout"], json!(330));
        assert_eq!(
            settings["hooks"]["PreToolUse"][0]["hooks"][0]["async"],
            json!(true)
        );
        assert_eq!(
            settings["hooks"]["PermissionDenied"][0]["hooks"][0]["async"],
            json!(true)
        );
    }

    #[test]
    fn captures_a_tool_permission_request_until_the_tool_finishes() {
        let mut store = ClaudeSessionStore::default();
        store.apply_event(
            &json!({
                "session_id": "session-1",
                "hook_event_name": "PermissionRequest",
                "tool_name": "Bash",
                "tool_use_id": "tool-bash-1",
                "cwd": "C:\\work\\CodeCraft",
                "tool_input": {
                    "command": "git push --force"
                }
            }),
            1_000,
        );

        let sessions = store.sessions();
        assert_eq!(sessions[0].status, ClaudeSessionStatus::Waiting);
        let permission = sessions[0].permission.as_ref().unwrap();
        assert_eq!(permission.id, "permission-tool-bash-1");
        assert_eq!(permission.tool_name, "Bash");
        assert_eq!(permission.summary, "git push --force");
        assert_eq!(permission.cwd.as_deref(), Some("C:\\work\\CodeCraft"));
        assert!(!permission.can_always_allow);
        assert!(permission.captured_at > 0);

        store.apply_event(
            &json!({
                "session_id": "session-1",
                "hook_event_name": "PostToolUse",
                "tool_name": "Bash",
                "tool_use_id": "tool-bash-1"
            }),
            2_000,
        );
        assert_eq!(store.sessions()[0].permission, None);
    }

    #[test]
    fn captures_a_plan_request_separately_from_permissions() {
        let mut store = ClaudeSessionStore::default();
        store.apply_event(
            &json!({
                "session_id": "session-1",
                "hook_event_name": "PermissionRequest",
                "tool_name": "ExitPlanMode",
                "tool_use_id": "tool-plan-1",
                "cwd": "C:\\work\\CodeCraft",
                "tool_input": {
                    "plan": "# 计划\n\n1. 读取代码\n2. 修改文件"
                }
            }),
            1_000,
        );

        let sessions = store.sessions();
        assert_eq!(sessions[0].status, ClaudeSessionStatus::Waiting);
        assert_eq!(sessions[0].permission, None);
        let plan = sessions[0].plan.as_ref().unwrap();
        assert_eq!(plan.id, "plan-tool-plan-1");
        assert_eq!(plan.tool_name, "ExitPlanMode");
        assert_eq!(plan.plan, "# 计划\n\n1. 读取代码\n2. 修改文件");
        assert_eq!(plan.cwd.as_deref(), Some("C:\\work\\CodeCraft"));
        assert!(plan.captured_at > 0);

        store.apply_event(
            &json!({
                "session_id": "session-1",
                "hook_event_name": "PostToolUse",
                "tool_name": "ExitPlanMode",
                "tool_use_id": "tool-plan-1"
            }),
            2_000,
        );
        assert_eq!(store.sessions()[0].plan, None);
    }

    #[test]
    fn completion_hooks_clear_only_the_matching_review_id() {
        let mut store = ClaudeSessionStore::default();
        store.apply_event(
            &json!({
                "session_id": "question-session",
                "hook_event_name": "PreToolUse",
                "tool_name": "AskUserQuestion",
                "tool_use_id": "question-tool",
                "tool_input": {
                    "questions": [{ "question": "Choose", "options": [] }]
                }
            }),
            1_000,
        );
        store.apply_event(
            &json!({
                "session_id": "permission-session",
                "hook_event_name": "PermissionRequest",
                "tool_name": "Bash",
                "tool_use_id": "permission-tool",
                "tool_input": { "command": "git status" }
            }),
            1_000,
        );
        store.apply_event(
            &json!({
                "session_id": "plan-session",
                "hook_event_name": "PermissionRequest",
                "tool_name": "ExitPlanMode",
                "tool_use_id": "plan-tool",
                "tool_input": { "plan": "Run the plan" }
            }),
            1_000,
        );

        store.apply_event(
            &json!({
                "session_id": "question-session",
                "hook_event_name": "PostToolUse",
                "tool_name": "AskUserQuestion",
                "tool_use_id": "unrelated-tool"
            }),
            2_000,
        );
        assert!(store
            .sessions()
            .iter()
            .find(|session| session.id == "question-session")
            .unwrap()
            .question
            .is_some());

        for (session_id, event_name, tool_use_id) in [
            ("question-session", "PostToolUse", "question-tool"),
            ("permission-session", "PermissionDenied", "permission-tool"),
            ("plan-session", "PostToolUseFailure", "plan-tool"),
        ] {
            store.apply_event(
                &json!({
                    "session_id": session_id,
                    "hook_event_name": event_name,
                    "tool_use_id": tool_use_id
                }),
                3_000,
            );
        }

        for session in store.sessions() {
            assert!(session.question.is_none());
            assert!(session.permission.is_none());
            assert!(session.plan.is_none());
        }
    }

    #[test]
    fn requests_without_tool_ids_wait_for_a_session_lifecycle_event() {
        let mut store = ClaudeSessionStore::default();
        store.apply_event(
            &json!({
                "session_id": "session-1",
                "hook_event_name": "PermissionRequest",
                "tool_name": "Bash",
                "tool_input": { "command": "git status" }
            }),
            1_000,
        );
        store.apply_event(
            &json!({
                "session_id": "session-1",
                "hook_event_name": "PostToolUse",
                "tool_name": "Bash"
            }),
            2_000,
        );
        assert!(store.sessions()[0].permission.is_some());

        store.apply_event(
            &json!({
                "session_id": "session-1",
                "hook_event_name": "Stop"
            }),
            3_000,
        );
        assert!(store.sessions()[0].permission.is_none());
    }

    #[test]
    fn plan_text_falls_back_to_a_readable_pretty_json() {
        let payload = json!({
            "hook_event_name": "PermissionRequest",
            "tool_name": "Plan",
            "tool_use_id": "tool-plan-fallback",
            "tool_input": {
                "steps": ["step-1", "step-2"]
            }
        });

        let plan = plan_request_from_payload(&payload).unwrap();
        assert_eq!(plan.tool_name, "Plan");
        assert!(plan.plan.contains("\"steps\""));
        assert!(plan.plan.contains('\n'));
        assert_eq!(plan_request_from_payload(&payload).unwrap().id, plan.id);
    }

    #[test]
    fn plan_tools_do_not_create_permission_requests() {
        let payload = json!({
            "hook_event_name": "PermissionRequest",
            "tool_name": "ExitPlanMode",
            "tool_use_id": "tool-plan-2",
            "tool_input": { "plan": "Plan text" }
        });

        assert!(permission_request_from_payload(&payload).is_none());
        assert!(plan_request_from_payload(&payload).is_some());
    }

    #[test]
    fn permission_request_id_falls_back_to_a_stable_content_hash() {
        let payload = json!({
            "hook_event_name": "PermissionRequest",
            "tool_name": "Write",
            "tool_input": {
                "file_path": "src/main.rs",
                "content": "fn main() {}"
            }
        });
        assert_eq!(
            permission_request_id(&payload),
            permission_request_id(&payload)
        );
        assert!(permission_request_id(&payload)
            .unwrap()
            .starts_with("permission-"));

        let pre_tool_use = json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "Write",
            "tool_input": {
                "file_path": "src/main.rs"
            }
        });
        assert_eq!(permission_request_id(&pre_tool_use), None);
        assert_eq!(
            permission_request_id(&json!({ "tool_name": "AskUserQuestion" })),
            None
        );
    }

    #[test]
    fn fallback_request_ids_are_scoped_to_the_claude_session() {
        let first = json!({
            "session_id": "session-a",
            "hook_event_name": "PermissionRequest",
            "tool_name": "Bash",
            "tool_input": { "command": "git status" }
        });
        let second = json!({
            "session_id": "session-b",
            "hook_event_name": "PermissionRequest",
            "tool_name": "Bash",
            "tool_input": { "command": "git status" }
        });

        assert_ne!(
            permission_request_id(&first),
            permission_request_id(&second)
        );
    }

    #[test]
    fn stale_decision_files_are_removed_before_reusing_a_request_id() {
        let path = env::temp_dir().join(format!(
            "codecraft-stale-decision-{}-{}.json",
            process::id(),
            unix_time_ms()
        ));
        fs::write(&path, b"stale").unwrap();
        remove_stale_decision_file(&path).unwrap();
        assert!(!path.exists());
        remove_stale_decision_file(&path).unwrap();
    }

    #[test]
    fn permission_request_reports_when_claude_can_offer_always_allow() {
        let mut store = ClaudeSessionStore::default();
        store.apply_event(
            &json!({
                "session_id": "session-1",
                "hook_event_name": "PermissionRequest",
                "tool_name": "Bash",
                "tool_use_id": "tool-bash-2",
                "tool_input": {
                    "command": "git fetch origin develop"
                },
                "permission_suggestions": [
                    {
                        "type": "addRules",
                        "rules": [
                            {
                                "toolName": "Bash",
                                "ruleContent": "git fetch origin develop"
                            }
                        ],
                        "behavior": "allow",
                        "destination": "localSettings"
                    }
                ]
            }),
            1_000,
        );

        let sessions = store.sessions();
        let permission = sessions[0].permission.as_ref().unwrap();
        assert!(permission.can_always_allow);
    }

    #[test]
    fn resolves_stable_question_request_ids_across_hook_events() {
        let pre_tool_use = json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "AskUserQuestion",
            "tool_use_id": "tool-1",
            "tool_input": {
                "questions": [{ "question": "Choose", "options": [] }]
            }
        });
        let permission_request = json!({
            "hook_event_name": "PermissionRequest",
            "tool_name": "AskUserQuestion",
            "tool_input": {
                "questions": [{ "question": "Choose", "options": [] }]
            }
        });
        assert_eq!(
            question_request_id(&pre_tool_use),
            question_request_id(&permission_request)
        );
        assert!(question_request_id(&pre_tool_use).is_some());

        let different_question = json!({
            "hook_event_name": "PermissionRequest",
            "tool_name": "AskUserQuestion",
            "tool_input": {
                "questions": [{ "question": "Pick", "options": [] }]
            }
        });
        assert_ne!(
            question_request_id(&pre_tool_use),
            question_request_id(&different_question)
        );
    }
}
