use std::{
    collections::hash_map::DefaultHasher,
    env, fs,
    hash::{Hash, Hasher},
    io::{self, BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{self, Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Mutex, OnceLock,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
use windows::Win32::{
    Foundation::CloseHandle,
    System::Threading::{OpenProcess, SetProcessAffinityMask, PROCESS_SET_INFORMATION},
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::approval_policy;

const HOOK_ARGUMENT: &str = "--codecraft-zcode-hook";
const REGISTERED_HOOKS: [&str; 7] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PermissionRequest",
    "PostToolUse",
    "PostToolUseFailure",
    "Stop",
];
const MAX_MESSAGE_BYTES: u64 = 256 * 1_024;
pub(crate) const TOOL_WAIT_TIMEOUT_MS: u64 = 120_000;
pub(crate) const INTERACTION_WAIT_TIMEOUT_MS: u64 = 150_000;
pub(crate) const TIMEOUT_SAFETY_WINDOW_MS: u64 = 30_000;
pub(crate) const OUTER_TIMEOUT_MS: u64 = INTERACTION_WAIT_TIMEOUT_MS + TIMEOUT_SAFETY_WINDOW_MS;
const POLL_INTERVAL: Duration = Duration::from_millis(200);
const HEARTBEAT_STALE_SECONDS: u64 = 20;
const DETECTION_CACHE_TTL: Duration = Duration::from_secs(30);
const APP_SERVER_START_DELAY: Duration = Duration::from_millis(750);
const APP_SERVER_RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);
const APP_SERVER_TURN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const APP_SERVER_SEND_RETRIES: usize = 20;
const APP_SERVER_RETRY_DELAY: Duration = Duration::from_millis(500);
const APP_SERVER_DELIVERY_KIND: &str = "desktop-continuous";
const APP_SERVER_RUNTIME_MODEL_BUDGET: &str = "preflight-v1";
static APP_SERVER_REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);
static DETECTION_CACHE: OnceLock<Mutex<Option<(Instant, Option<(PathBuf, Option<String>)>)>>> =
    OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ZCodeHookPhase {
    Observed,
    AutoAllowed,
    Pending,
    Resolved,
    ReturnedToZCode,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ZCodeHookEnvelope {
    pub(crate) captured_at: u64,
    pub(crate) phase: ZCodeHookPhase,
    pub(crate) request_id: Option<String>,
    pub(crate) payload: Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ZCodeApprovalDecision {
    AllowOnce,
    Deny,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ZCodeQuestionAnswer {
    pub(crate) question: String,
    pub(crate) selected_option_labels: Vec<String>,
    pub(crate) extra_text: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum ZCodeDecisionEnvelope {
    Permission {
        request_id: String,
        session_id: String,
        tool_use_id: Option<String>,
        decision: ZCodeApprovalDecision,
        message: Option<String>,
    },
    Question {
        request_id: String,
        session_id: String,
        tool_use_id: Option<String>,
        answers: Vec<ZCodeQuestionAnswer>,
        annotations: Option<Value>,
    },
    Plan {
        request_id: String,
        session_id: String,
        tool_use_id: Option<String>,
        approved: bool,
        feedback: Option<String>,
    },
}

impl ZCodeDecisionEnvelope {
    fn request_id(&self) -> &str {
        match self {
            Self::Permission { request_id, .. }
            | Self::Question { request_id, .. }
            | Self::Plan { request_id, .. } => request_id,
        }
    }

    fn session_id(&self) -> &str {
        match self {
            Self::Permission { session_id, .. }
            | Self::Question { session_id, .. }
            | Self::Plan { session_id, .. } => session_id,
        }
    }

    fn tool_use_id(&self) -> Option<&str> {
        match self {
            Self::Permission { tool_use_id, .. }
            | Self::Question { tool_use_id, .. }
            | Self::Plan { tool_use_id, .. } => tool_use_id.as_deref(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ZCodeHookInstallState {
    NotInstalled,
    Installed,
    Modified,
    Conflict,
    Incompatible,
    Error,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ZCodeHookStatus {
    pub(crate) state: ZCodeHookInstallState,
    pub(crate) install_path: String,
    pub(crate) detected_path: Option<String>,
    pub(crate) detected_version: Option<String>,
    pub(crate) error: Option<String>,
}

impl ZCodeHookStatus {
    pub(crate) fn installed(&self) -> bool {
        self.state == ZCodeHookInstallState::Installed
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
struct InstallRecord {
    enabled_existed: bool,
    enabled_value: Option<bool>,
    timeout_existed: bool,
    timeout_value: Option<u64>,
}

pub fn capture_zcode_hook() -> Result<(), String> {
    let mut reader = io::stdin().take(MAX_MESSAGE_BYTES + 1);
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_MESSAGE_BYTES {
        return Err("ZCode hook input exceeded 256 KiB".to_string());
    }
    let payload: Value = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    let Some(event_name) = string_field(&payload, "hook_event_name") else {
        return Ok(());
    };
    if !REGISTERED_HOOKS.contains(&event_name) {
        return Ok(());
    }

    if event_name != "PreToolUse" {
        write_hook_envelope(&payload, ZCodeHookPhase::Observed, None)?;
        return Ok(());
    }

    capture_pre_tool_use(&payload)
}

fn capture_pre_tool_use(payload: &Value) -> Result<(), String> {
    let tool = string_field(payload, "tool_name").unwrap_or_default();
    let interactive = approval_policy::requires_user_decision(tool);
    let request_id = request_id(payload);

    if !interactive {
        let settings = approval_policy::load_settings();
        let risk = approval_policy::risk_for_tool(tool, payload.get("tool_input"));
        if approval_policy::should_auto_approve(settings.mode, risk) {
            write_hook_envelope(payload, ZCodeHookPhase::AutoAllowed, Some(&request_id))?;
            print_pre_tool_decision("allow", None, None)?;
            return Ok(());
        }
    }

    remove_stale_decision_file(&decision_path(&request_id))?;
    write_hook_envelope(payload, ZCodeHookPhase::Pending, Some(&request_id))?;
    if !app_is_running() {
        write_hook_envelope(payload, ZCodeHookPhase::ReturnedToZCode, Some(&request_id))?;
        if !interactive {
            print_pre_tool_decision(
                "ask",
                Some("CodeCraft is not running; continue in ZCode"),
                None,
            )?;
        }
        return Ok(());
    }

    let wait_ms = if interactive {
        INTERACTION_WAIT_TIMEOUT_MS
    } else {
        TOOL_WAIT_TIMEOUT_MS
    };
    match wait_for_decision(payload, &request_id, Duration::from_millis(wait_ms)) {
        Ok(Some(decision)) => {
            write_hook_envelope(payload, ZCodeHookPhase::Resolved, Some(&request_id))?;
            print_decision(payload, decision)
        }
        Ok(None) => {
            write_hook_envelope(payload, ZCodeHookPhase::ReturnedToZCode, Some(&request_id))?;
            if !interactive {
                print_pre_tool_decision(
                    "ask",
                    Some("CodeCraft review timed out; continue in ZCode"),
                    None,
                )?;
            }
            Ok(())
        }
        Err(error) => {
            eprintln!("CodeCraft could not read the ZCode decision: {error}");
            write_hook_envelope(payload, ZCodeHookPhase::ReturnedToZCode, Some(&request_id))?;
            if !interactive {
                print_pre_tool_decision(
                    "ask",
                    Some("CodeCraft review failed; continue in ZCode"),
                    None,
                )?;
            }
            Ok(())
        }
    }
}

fn wait_for_decision(
    payload: &Value,
    request_id: &str,
    timeout: Duration,
) -> Result<Option<ZCodeDecisionEnvelope>, String> {
    let deadline = Instant::now() + timeout;
    let session_id = string_field(payload, "session_id").unwrap_or_default();
    let tool_use_id = string_field(payload, "tool_use_id");
    let path = decision_path(request_id);
    loop {
        if path.exists() {
            let bytes = fs::read(&path).map_err(|error| error.to_string())?;
            let _ = fs::remove_file(&path);
            let envelope: ZCodeDecisionEnvelope =
                serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
            if envelope.request_id() != request_id
                || envelope.session_id() != session_id
                || envelope.tool_use_id() != tool_use_id
            {
                return Err("ZCode decision did not match the active request".to_string());
            }
            return Ok(Some(envelope));
        }
        if !app_is_running() || Instant::now() >= deadline {
            return Ok(None);
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn print_decision(payload: &Value, decision: ZCodeDecisionEnvelope) -> Result<(), String> {
    let tool = string_field(payload, "tool_name").unwrap_or_default();
    match decision {
        ZCodeDecisionEnvelope::Permission {
            decision, message, ..
        } if !approval_policy::requires_user_decision(tool) => match decision {
            ZCodeApprovalDecision::AllowOnce => {
                print_pre_tool_decision("allow", message.as_deref(), None)
            }
            ZCodeApprovalDecision::Deny => print_pre_tool_decision(
                "deny",
                Some(
                    message
                        .as_deref()
                        .unwrap_or("User denied this tool call in CodeCraft"),
                ),
                None,
            ),
        },
        ZCodeDecisionEnvelope::Question {
            answers,
            annotations,
            ..
        } if normalized_tool(tool) == "askuserquestion" => {
            let updated_input = question_updated_input(payload, &answers, annotations)?;
            print_pre_tool_decision("allow", Some("Answered in CodeCraft"), Some(updated_input))
        }
        ZCodeDecisionEnvelope::Plan {
            approved, feedback, ..
        } if normalized_tool(tool) == "exitplanmode" => {
            if approved {
                print_pre_tool_decision("allow", Some("Plan approved in CodeCraft"), None)
            } else {
                let feedback = feedback
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| "Plan feedback cannot be empty".to_string())?;
                print_pre_tool_decision("deny", Some(feedback), None)
            }
        }
        _ => Err("ZCode decision kind did not match the active tool".to_string()),
    }
}

fn question_updated_input(
    payload: &Value,
    submitted: &[ZCodeQuestionAnswer],
    annotations: Option<Value>,
) -> Result<Value, String> {
    let mut input = payload
        .get("tool_input")
        .and_then(Value::as_object)
        .cloned()
        .ok_or_else(|| "AskUserQuestion input must be an object".to_string())?;
    let questions = input
        .get("questions")
        .and_then(Value::as_array)
        .ok_or_else(|| "AskUserQuestion input is missing questions".to_string())?;
    let mut answers = Map::new();
    for question in questions {
        let text = string_field(question, "question")
            .ok_or_else(|| "AskUserQuestion contains an empty question".to_string())?;
        let submitted = submitted
            .iter()
            .find(|answer| answer.question == text)
            .ok_or_else(|| format!("No answer was submitted for: {text}"))?;
        let labels = submitted
            .selected_option_labels
            .iter()
            .map(|label| label.trim())
            .filter(|label| !label.is_empty())
            .collect::<Vec<_>>();
        let allowed_labels = question
            .get("options")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|option| string_field(option, "label"))
            .collect::<Vec<_>>();
        if labels
            .iter()
            .any(|label| !allowed_labels.iter().any(|allowed| allowed == label))
        {
            return Err(format!("Unknown option submitted for: {text}"));
        }
        let multi_select = question
            .get("multiSelect")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !multi_select && labels.len() > 1 {
            return Err(format!("Only one option may be selected for: {text}"));
        }
        let extra_text = submitted
            .extra_text
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let value = match (labels.is_empty(), extra_text) {
            (false, Some(extra_text)) => format!("{}, {extra_text}", labels.join(", ")),
            (false, None) => labels.join(", "),
            (true, Some(extra_text)) => extra_text.to_string(),
            (true, None) => return Err(format!("Answer cannot be empty for: {text}")),
        };
        answers.insert(text.to_string(), Value::String(value));
    }
    input.insert("answers".to_string(), Value::Object(answers));
    if let Some(annotations) = annotations.filter(Value::is_object) {
        input.insert("annotations".to_string(), annotations);
    }
    Ok(Value::Object(input))
}

fn print_pre_tool_decision(
    decision: &str,
    reason: Option<&str>,
    updated_input: Option<Value>,
) -> Result<(), String> {
    let mut output = Map::new();
    output.insert(
        "hookEventName".to_string(),
        Value::String("PreToolUse".to_string()),
    );
    output.insert(
        "permissionDecision".to_string(),
        Value::String(decision.to_string()),
    );
    if let Some(reason) = reason.map(str::trim).filter(|value| !value.is_empty()) {
        output.insert(
            "permissionDecisionReason".to_string(),
            Value::String(reason.to_string()),
        );
    }
    if let Some(updated_input) = updated_input {
        output.insert("updatedInput".to_string(), updated_input);
    }
    println!(
        "{}",
        serde_json::to_string(&json!({ "hookSpecificOutput": Value::Object(output) }))
            .map_err(|error| error.to_string())?
    );
    Ok(())
}

pub(crate) fn submit_decision(decision: &ZCodeDecisionEnvelope) -> Result<(), String> {
    write_decision_file(decision_path(decision.request_id()), decision)
}

pub(crate) fn queue_plan_feedback(session_id: &str, feedback: &str) {
    let session_id = session_id.to_string();
    let feedback = feedback.trim().to_string();
    if feedback.is_empty() {
        return;
    }
    thread::spawn(move || {
        thread::sleep(APP_SERVER_START_DELAY);
        if let Err(error) = send_plan_feedback(&session_id, &feedback) {
            eprintln!("CodeCraft could not return ZCode plan feedback: {error}");
        }
    });
}

#[derive(Debug)]
enum AppServerError {
    Io(String),
    Protocol(String),
    Rpc { code: i64, message: String },
    Timeout(&'static str),
}

impl std::fmt::Display for AppServerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(message) | Self::Protocol(message) => formatter.write_str(message),
            Self::Rpc { code, message } => {
                write!(formatter, "ZCode app-server error {code}: {message}")
            }
            Self::Timeout(stage) => {
                write!(formatter, "timed out waiting for ZCode app-server {stage}")
            }
        }
    }
}

struct AppServerClient {
    child: Child,
    stdin: ChildStdin,
    incoming: mpsc::Receiver<Result<String, String>>,
    terminal_turns: Vec<(String, AppServerTurnOutcome)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AppServerTurnOutcome {
    Completed,
    Failed,
}

impl Drop for AppServerClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl AppServerClient {
    fn spawn(executable: &Path) -> Result<Self, AppServerError> {
        let mut command = Command::new(executable);
        command
            .env("ELECTRON_RUN_AS_NODE", "1")
            .arg("resources/glm/zcode.cjs")
            .arg("app-server")
            .arg("--stdio")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(parent) = executable.parent() {
            command.current_dir(parent);
        }
        #[cfg(windows)]
        command.creation_flags(0x0800_0000);

        let mut child = command.spawn().map_err(|error| {
            AppServerError::Io(format!("unable to start ZCode app-server: {error}"))
        })?;
        #[cfg(windows)]
        if let Err(error) = limit_app_server_affinity(child.id()) {
            eprintln!("CodeCraft could not limit ZCode app-server affinity: {error}");
        }
        let stdin = child.stdin.take().ok_or_else(|| {
            let _ = child.kill();
            AppServerError::Protocol("ZCode app-server did not expose stdin".to_string())
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            let _ = child.kill();
            AppServerError::Protocol("ZCode app-server did not expose stdout".to_string())
        })?;
        let (sender, incoming) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let line = line.map_err(|error| error.to_string());
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            stdin,
            incoming,
            terminal_turns: Vec::new(),
        })
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, AppServerError> {
        let id = Value::String(format!(
            "codecraft-{}-{}",
            process::id(),
            APP_SERVER_REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        self.write_message(&json!({
            "id": id,
            "method": method,
            "params": params,
        }))?;
        self.wait_for_response(&id)
    }

    fn write_message(&mut self, value: &Value) -> Result<(), AppServerError> {
        let bytes = serde_json::to_vec(value)
            .map_err(|error| AppServerError::Protocol(error.to_string()))?;
        self.stdin
            .write_all(&bytes)
            .and_then(|_| self.stdin.write_all(b"\n"))
            .and_then(|_| self.stdin.flush())
            .map_err(|error| AppServerError::Io(error.to_string()))
    }

    fn wait_for_response(&mut self, request_id: &Value) -> Result<Value, AppServerError> {
        let deadline = Instant::now() + APP_SERVER_RESPONSE_TIMEOUT;
        loop {
            let value = self.next_message(deadline, "a response")?;
            if value.get("id") == Some(request_id) {
                if let Some(error) = value.get("error") {
                    return Err(AppServerError::Rpc {
                        code: error.get("code").and_then(Value::as_i64).unwrap_or(-32000),
                        message: error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("Unknown ZCode app-server error")
                            .to_string(),
                    });
                }
                return value.get("result").cloned().ok_or_else(|| {
                    AppServerError::Protocol("ZCode app-server response had no result".to_string())
                });
            }
            self.observe_async_message(&value)?;
        }
    }

    fn wait_for_turn(&mut self, session_id: &str) -> Result<(), AppServerError> {
        if let Some(outcome) = self.take_terminal_turn(session_id) {
            return terminal_turn_result(outcome);
        }
        let deadline = Instant::now() + APP_SERVER_TURN_TIMEOUT;
        loop {
            let value = self.next_message(deadline, "the ZCode turn to complete")?;
            self.observe_async_message(&value)?;
            if let Some(outcome) = self.take_terminal_turn(session_id) {
                return terminal_turn_result(outcome);
            }
        }
    }

    fn observe_async_message(&mut self, value: &Value) -> Result<(), AppServerError> {
        if let Some((session_id, outcome)) = terminal_turn_event(value) {
            self.terminal_turns.push((session_id, outcome));
        }
        self.handle_server_request(value)
    }

    fn take_terminal_turn(&mut self, session_id: &str) -> Option<AppServerTurnOutcome> {
        let position = self
            .terminal_turns
            .iter()
            .position(|(candidate, _)| candidate == session_id)?;
        Some(self.terminal_turns.remove(position).1)
    }

    fn clear_terminal_turns(&mut self) {
        self.terminal_turns.clear();
    }

    fn next_message(
        &self,
        deadline: Instant,
        stage: &'static str,
    ) -> Result<Value, AppServerError> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(AppServerError::Timeout(stage));
        }
        let line = self
            .incoming
            .recv_timeout(remaining)
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => AppServerError::Timeout(stage),
                mpsc::RecvTimeoutError::Disconnected => AppServerError::Protocol(
                    "ZCode app-server closed its output stream".to_string(),
                ),
            })?
            .map_err(AppServerError::Io)?;
        if line.trim().is_empty() {
            return self.next_message(deadline, stage);
        }
        serde_json::from_str(line.trim()).map_err(|error| {
            AppServerError::Protocol(format!("invalid ZCode app-server message: {error}"))
        })
    }

    fn handle_server_request(&mut self, value: &Value) -> Result<(), AppServerError> {
        let Some(method) = value.get("method").and_then(Value::as_str) else {
            return Ok(());
        };
        let Some(id) = value.get("id") else {
            return Ok(());
        };
        if method == "session/requestRuntimePreferences" {
            return self.write_message(&json!({
                "id": id,
                "result": runtime_preferences(),
            }));
        }
        eprintln!("CodeCraft received unsupported ZCode app-server request: {method}");
        self.write_message(&json!({
            "id": id,
            "error": {
                "code": -32601,
                "message": format!("Unsupported ZCode app-server request: {method}"),
            },
        }))
    }
}

fn send_plan_feedback(session_id: &str, feedback: &str) -> Result<(), AppServerError> {
    let executable = detect_zcode_installation()
        .map(|(path, _)| path)
        .ok_or_else(|| {
            AppServerError::Protocol("ZCode installation was not detected".to_string())
        })?;
    let mut client = AppServerClient::spawn(&executable)?;
    client.request("session/resume", json!({"sessionId": session_id}))?;
    client.request(
        "session/subscribe",
        json!({
            "sessionId": session_id,
            "deliveryKind": APP_SERVER_DELIVERY_KIND,
            "includeSnapshot": false,
        }),
    )?;
    client.clear_terminal_turns();

    let mut sent = false;
    for attempt in 0..APP_SERVER_SEND_RETRIES {
        match client.request(
            "session/send",
            json!({"sessionId": session_id, "content": feedback}),
        ) {
            Ok(_) => {
                sent = true;
                break;
            }
            Err(AppServerError::Rpc { code: -32010, .. })
                if attempt + 1 < APP_SERVER_SEND_RETRIES =>
            {
                client.clear_terminal_turns();
                thread::sleep(APP_SERVER_RETRY_DELAY);
            }
            Err(error) => return Err(error),
        }
    }
    if !sent {
        return Err(AppServerError::Timeout("the active ZCode prompt to finish"));
    }
    client.wait_for_turn(session_id)
}

fn runtime_preferences() -> Value {
    json!({
        "nativeSearchEnhancementsEnabled": true,
        "memoryEnabled": false,
        "askUserQuestionAutoResolutionEnabled": true,
        "modelContextBudgetStrategy": APP_SERVER_RUNTIME_MODEL_BUDGET,
    })
}

fn terminal_turn_event(value: &Value) -> Option<(String, AppServerTurnOutcome)> {
    if value.get("method").and_then(Value::as_str) != Some("session/event") {
        return None;
    }
    let params = value.get("params")?;
    let session_id = params.get("sessionId")?.as_str()?.to_string();
    let outcome = match params.get("type")?.as_str()? {
        "turn.completed" => AppServerTurnOutcome::Completed,
        "turn.failed" => AppServerTurnOutcome::Failed,
        _ => return None,
    };
    Some((session_id, outcome))
}

fn terminal_turn_result(outcome: AppServerTurnOutcome) -> Result<(), AppServerError> {
    match outcome {
        AppServerTurnOutcome::Completed => Ok(()),
        AppServerTurnOutcome::Failed => Err(AppServerError::Protocol(
            "the ZCode plan feedback turn failed".to_string(),
        )),
    }
}

#[cfg(windows)]
fn limit_app_server_affinity(process_id: u32) -> Result<(), String> {
    unsafe {
        let handle = OpenProcess(PROCESS_SET_INFORMATION, false, process_id)
            .map_err(|error| error.to_string())?;
        let result = SetProcessAffinityMask(handle, 0b11usize).map_err(|error| error.to_string());
        let _ = CloseHandle(handle);
        result
    }
}

fn write_decision_file(path: PathBuf, envelope: &impl Serialize) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let temporary = path.with_extension(format!("{}.tmp", process::id()));
    let bytes = serde_json::to_vec(envelope).map_err(|error| error.to_string())?;
    fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
    if path.exists() {
        let _ = fs::remove_file(&temporary);
        return Err("The ZCode request has already been decided".to_string());
    }
    fs::rename(&temporary, &path).map_err(|error| error.to_string())
}

fn write_hook_envelope(
    payload: &Value,
    phase: ZCodeHookPhase,
    request_id: Option<&str>,
) -> Result<(), String> {
    let envelope = ZCodeHookEnvelope {
        captured_at: now_ms(),
        phase,
        request_id: request_id.map(str::to_string),
        payload: payload.clone(),
    };
    let inbox = inbox_dir();
    fs::create_dir_all(&inbox).map_err(|error| error.to_string())?;
    let bytes = serde_json::to_vec(&envelope).map_err(|error| error.to_string())?;
    for attempt in 0..10 {
        let stem = format!("{:020}-{}-{attempt}", envelope.captured_at, process::id());
        let temporary = inbox.join(format!("{stem}.tmp"));
        let destination = inbox.join(format!("{stem}.json"));
        if destination.exists() {
            continue;
        }
        let Ok(mut file) = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        else {
            continue;
        };
        file.write_all(&bytes)
            .and_then(|_| file.flush())
            .map_err(|error| error.to_string())?;
        drop(file);
        fs::rename(temporary, destination).map_err(|error| error.to_string())?;
        return Ok(());
    }
    Err("Unable to allocate a unique ZCode hook event file".to_string())
}

pub(crate) fn inbox_dir() -> PathBuf {
    approval_policy::base_data_dir()
        .join("zcode-hook")
        .join("inbox")
}

fn decisions_dir() -> PathBuf {
    approval_policy::base_data_dir()
        .join("zcode-hook")
        .join("decisions")
}

fn decision_path(request_id: &str) -> PathBuf {
    decisions_dir().join(format!("{}.json", safe_stem(request_id)))
}

fn safe_stem(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .take(96)
        .collect::<String>();
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    format!("{sanitized}-{:016x}", hasher.finish())
}

fn remove_stale_decision_file(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

pub(crate) fn request_id(payload: &Value) -> String {
    let session_id = string_field(payload, "session_id").unwrap_or("unknown-session");
    let event_name = string_field(payload, "hook_event_name").unwrap_or("unknown-event");
    let tool_use_id = string_field(payload, "tool_use_id")
        .map(str::to_string)
        .unwrap_or_else(|| {
            let mut hasher = DefaultHasher::new();
            payload.get("tool_name").hash(&mut hasher);
            payload
                .get("tool_input")
                .map(Value::to_string)
                .hash(&mut hasher);
            format!("fallback-{:016x}", hasher.finish())
        });
    format!("{session_id}:{tool_use_id}:{event_name}")
}

fn normalized_tool(tool: &str) -> String {
    tool.chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn app_is_running() -> bool {
    let path = super::claude_hook::heartbeat_path();
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = metadata.modified() else {
        return false;
    };
    SystemTime::now()
        .duration_since(modified)
        .map(|elapsed| elapsed.as_secs() <= HEARTBEAT_STALE_SECONDS)
        .unwrap_or(false)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

fn string_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn zcode_config_path() -> Result<PathBuf, String> {
    let home = env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .ok_or_else(|| "Unable to locate the user profile for ZCode".to_string())?;
    Ok(PathBuf::from(home)
        .join(".zcode")
        .join("cli")
        .join("config.json"))
}

fn install_record_path() -> PathBuf {
    approval_policy::base_data_dir()
        .join("zcode-hook")
        .join("installation.json")
}

fn expected_hook(executable: &Path) -> Value {
    json!({
        "type": "process",
        "command": executable.to_string_lossy(),
        "args": [HOOK_ARGUMENT],
        "timeoutMs": OUTER_TIMEOUT_MS,
    })
}

fn is_own_hook(value: &Value) -> bool {
    value
        .get("args")
        .and_then(Value::as_array)
        .is_some_and(|args| args.iter().any(|arg| arg.as_str() == Some(HOOK_ARGUMENT)))
        || value
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(|command| command.contains(HOOK_ARGUMENT))
}

fn own_hook_is_modified(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return true;
    };
    let allowed = ["type", "command", "args", "timeoutMs"];
    object.keys().any(|key| !allowed.contains(&key.as_str()))
        || value.get("type").and_then(Value::as_str) != Some("process")
        || value
            .get("args")
            .and_then(Value::as_array)
            .is_none_or(|args| {
                args.len() != 1 || args.first().and_then(Value::as_str) != Some(HOOK_ARGUMENT)
            })
        || value.get("timeoutMs").and_then(Value::as_u64) != Some(OUTER_TIMEOUT_MS)
        || string_field(value, "command").is_none()
}

fn hook_targets_executable(value: &Value, executable: &Path) -> bool {
    let Some(command) = string_field(value, "command") else {
        return false;
    };
    paths_match(Path::new(command), executable)
}

#[cfg(windows)]
fn paths_match(left: &Path, right: &Path) -> bool {
    left.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .eq_ignore_ascii_case(
            right
                .to_string_lossy()
                .replace('/', "\\")
                .trim_end_matches('\\'),
        )
}

#[cfg(not(windows))]
fn paths_match(left: &Path, right: &Path) -> bool {
    left == right
}

pub(crate) fn install(executable: &Path) -> Result<ZCodeHookStatus, String> {
    let path = zcode_config_path()?;
    let mut root = read_config(&path)?;
    let record = installation_record(&root, read_install_record());
    merge_hooks(&mut root, executable, true)?;
    backup_and_atomic_write(&path, &root)?;
    write_install_record(&record)?;
    status()
}

pub(crate) fn uninstall() -> Result<(), String> {
    let path = zcode_config_path()?;
    if !path.exists() {
        return Ok(());
    }
    let mut root = read_config(&path)?;
    let record = read_install_record().unwrap_or_default();
    remove_own_hooks(&mut root);
    if !has_any_config_hooks(&root) {
        restore_field(
            &mut root,
            "enabled",
            record.enabled_existed,
            record.enabled_value.map(Value::Bool),
        );
        restore_field(
            &mut root,
            "timeoutMs",
            record.timeout_existed,
            record.timeout_value.map(Value::from),
        );
    }
    backup_and_atomic_write(&path, &root)?;
    let _ = fs::remove_file(install_record_path());
    Ok(())
}

fn install_record_for(root: &Value) -> InstallRecord {
    InstallRecord {
        enabled_existed: root.pointer("/hooks/enabled").is_some(),
        enabled_value: root.pointer("/hooks/enabled").and_then(Value::as_bool),
        timeout_existed: root.pointer("/hooks/timeoutMs").is_some(),
        timeout_value: root.pointer("/hooks/timeoutMs").and_then(Value::as_u64),
    }
}

fn installation_record(root: &Value, existing: Option<InstallRecord>) -> InstallRecord {
    existing.unwrap_or_else(|| install_record_for(root))
}

fn read_install_record() -> Option<InstallRecord> {
    fs::read(install_record_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
}

pub(crate) fn status() -> Result<ZCodeHookStatus, String> {
    let path = zcode_config_path()?;
    let installation = detect_zcode_installation();
    let (detected_path, detected_version) = installation
        .as_ref()
        .map(|(path, version)| (Some(path.to_string_lossy().to_string()), version.clone()))
        .unwrap_or((None, None));
    if !path.exists() {
        return Ok(ZCodeHookStatus {
            state: ZCodeHookInstallState::NotInstalled,
            install_path: path.to_string_lossy().to_string(),
            detected_path,
            detected_version,
            error: None,
        });
    }
    let root = match read_config(&path) {
        Ok(root) => root,
        Err(error) => {
            return Ok(ZCodeHookStatus {
                state: ZCodeHookInstallState::Error,
                install_path: path.to_string_lossy().to_string(),
                detected_path,
                detected_version,
                error: Some(error),
            })
        }
    };
    let mut count = 0usize;
    let mut modified = false;
    let mut conflict = false;
    let current_executable = env::current_exe().ok();
    for event in REGISTERED_HOOKS {
        let hooks = event_hooks(&root, event);
        let own = hooks
            .iter()
            .filter(|hook| is_own_hook(hook))
            .collect::<Vec<_>>();
        count += own.len();
        conflict |= own.len() > 1;
        modified |= own.iter().any(|hook| own_hook_is_modified(hook));
        if let Some(executable) = current_executable.as_deref() {
            modified |= own
                .iter()
                .any(|hook| !hook_targets_executable(hook, executable));
        }
    }
    let enabled = root.pointer("/hooks/enabled").and_then(Value::as_bool) == Some(true);
    let state = if count == 0 {
        ZCodeHookInstallState::NotInstalled
    } else if conflict {
        ZCodeHookInstallState::Conflict
    } else if count != REGISTERED_HOOKS.len() || modified || !enabled {
        ZCodeHookInstallState::Modified
    } else if !version_is_compatible(detected_version.as_deref()) {
        ZCodeHookInstallState::Incompatible
    } else {
        ZCodeHookInstallState::Installed
    };
    Ok(ZCodeHookStatus {
        state,
        install_path: path.to_string_lossy().to_string(),
        detected_path,
        detected_version,
        error: None,
    })
}

fn merge_hooks(root: &mut Value, executable: &Path, reject_modified: bool) -> Result<(), String> {
    if !root.is_object() {
        return Err("ZCode config root must be a JSON object".to_string());
    }
    let hooks = root
        .as_object_mut()
        .expect("checked object")
        .entry("hooks")
        .or_insert_with(|| json!({}));
    if !hooks.is_object() {
        return Err("ZCode hooks config must be a JSON object".to_string());
    }
    let hooks_object = hooks.as_object_mut().expect("checked object");
    hooks_object.insert("enabled".to_string(), Value::Bool(true));
    hooks_object.insert("timeoutMs".to_string(), Value::from(OUTER_TIMEOUT_MS));
    let events = hooks_object
        .entry("events")
        .or_insert_with(|| Value::Object(Map::new()));
    let events = events
        .as_object_mut()
        .ok_or_else(|| "ZCode hooks.events must be a JSON object".to_string())?;
    for event in REGISTERED_HOOKS {
        let groups = events
            .entry(event)
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or_else(|| format!("ZCode hooks.events.{event} must be an array"))?;
        let mut positions = Vec::new();
        for (group_index, group) in groups.iter().enumerate() {
            if let Some(items) = group.get("hooks").and_then(Value::as_array) {
                for (hook_index, hook) in items.iter().enumerate() {
                    if is_own_hook(hook) {
                        positions.push((group_index, hook_index));
                    }
                }
            }
        }
        if positions.len() > 1 {
            return Err(format!(
                "Conflicting CodeCraft ZCode hooks exist for {event}"
            ));
        }
        if let Some((group_index, hook_index)) = positions.first().copied() {
            let hook = groups[group_index]
                .get_mut("hooks")
                .and_then(Value::as_array_mut)
                .and_then(|items| items.get_mut(hook_index))
                .expect("position came from this structure");
            if reject_modified && own_hook_is_modified(hook) {
                return Err(format!("The CodeCraft ZCode hook for {event} was modified"));
            }
            *hook = expected_hook(executable);
        } else {
            groups.push(json!({ "hooks": [expected_hook(executable)] }));
        }
    }
    Ok(())
}

fn remove_own_hooks(root: &mut Value) {
    let Some(events) = root
        .pointer_mut("/hooks/events")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    for event in REGISTERED_HOOKS {
        let Some(groups) = events.get_mut(event).and_then(Value::as_array_mut) else {
            continue;
        };
        for group in groups.iter_mut() {
            if let Some(hooks) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                hooks.retain(|hook| !is_own_hook(hook));
            }
        }
        groups.retain(|group| {
            group
                .get("hooks")
                .and_then(Value::as_array)
                .is_some_and(|hooks| !hooks.is_empty())
        });
    }
    events.retain(|_, groups| groups.as_array().is_none_or(|groups| !groups.is_empty()));
}

fn restore_field(root: &mut Value, field: &str, existed: bool, value: Option<Value>) {
    let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) else {
        return;
    };
    if existed {
        if let Some(value) = value {
            hooks.insert(field.to_string(), value);
        }
    } else {
        hooks.remove(field);
    }
}

fn has_any_config_hooks(root: &Value) -> bool {
    root.pointer("/hooks/events")
        .and_then(Value::as_object)
        .is_some_and(|events| {
            events.values().any(|groups| {
                groups.as_array().is_some_and(|groups| {
                    groups.iter().any(|group| {
                        group
                            .get("hooks")
                            .and_then(Value::as_array)
                            .is_some_and(|hooks| !hooks.is_empty())
                    })
                })
            })
        })
}

fn event_hooks<'a>(root: &'a Value, event: &str) -> Vec<&'a Value> {
    root.pointer(&format!("/hooks/events/{event}"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|group| group.get("hooks").and_then(Value::as_array))
        .flatten()
        .collect()
}

fn read_config(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(json!({}));
    }
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

fn write_install_record(record: &InstallRecord) -> Result<(), String> {
    let path = install_record_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(
        path,
        serde_json::to_vec_pretty(record).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())
}

fn backup_and_atomic_write(path: &Path, root: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    if path.exists() {
        let backup = PathBuf::from(format!("{}.bak", path.to_string_lossy()));
        if !backup.exists() {
            fs::copy(path, backup).map_err(|error| error.to_string())?;
        }
    }
    let temporary = path.with_extension(format!("{}.tmp", process::id()));
    let bytes = serde_json::to_vec_pretty(root).map_err(|error| error.to_string())?;
    fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
    replace_file(&temporary, path)
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        core::PCWSTR,
        Win32::Storage::FileSystem::{
            MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        },
    };

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    unsafe {
        MoveFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(destination.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
        .map_err(|error| error.to_string())
    }
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> Result<(), String> {
    if destination.exists() {
        fs::remove_file(destination).map_err(|error| error.to_string())?;
    }
    fs::rename(source, destination).map_err(|error| error.to_string())
}

pub(crate) fn detect_zcode_installation() -> Option<(PathBuf, Option<String>)> {
    let cache = DETECTION_CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(mut cache) = cache.lock() {
        if let Some((checked_at, installation)) = cache.as_ref() {
            if checked_at.elapsed() < DETECTION_CACHE_TTL {
                return installation.clone();
            }
        }
        let installation = detect_zcode_installation_uncached();
        *cache = Some((Instant::now(), installation.clone()));
        return installation;
    }
    detect_zcode_installation_uncached()
}

fn detect_zcode_installation_uncached() -> Option<(PathBuf, Option<String>)> {
    let mut candidates = Vec::new();
    if let Some(path) = running_zcode_path() {
        candidates.push(path);
    }
    if let Some(path) = uninstall_registry_path() {
        candidates.push(path);
    }
    if let Some(local) = env::var_os("LOCALAPPDATA") {
        candidates.push(
            PathBuf::from(local)
                .join("Programs")
                .join("ZCode")
                .join("ZCode.exe"),
        );
    }
    candidates.push(PathBuf::from(r"C:\Program Files\ZCode\ZCode.exe"));
    candidates.push(PathBuf::from(r"D:\software\ZCode\ZCode.exe"));
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .map(|path| {
            let version = file_version(&path);
            (path, version)
        })
}

#[cfg(windows)]
fn running_zcode_path() -> Option<PathBuf> {
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "Get-Process -Name ZCode -ErrorAction SilentlyContinue | Where-Object Path | Select-Object -First 1 -ExpandProperty Path",
        ])
        .output()
        .ok()?;
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

#[cfg(not(windows))]
fn running_zcode_path() -> Option<PathBuf> {
    None
}

#[cfg(windows)]
fn uninstall_registry_path() -> Option<PathBuf> {
    let script = "$roots=@('HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall','HKLM:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall','HKLM:\\Software\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall'); $app=$roots | ForEach-Object { Get-ChildItem $_ -ErrorAction SilentlyContinue } | ForEach-Object { Get-ItemProperty $_.PSPath } | Where-Object { $_.DisplayName -like 'ZCode*' } | Select-Object -First 1; if ($app.InstallLocation) { Join-Path $app.InstallLocation 'ZCode.exe' } elseif ($app.DisplayIcon) { $app.DisplayIcon -replace '^\"|\",?\\d*$|,\\d*$','' }";
    let output = Command::new("powershell")
        .args(["-NoProfile", "-Command", script])
        .output()
        .ok()?;
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!path.is_empty()).then(|| PathBuf::from(path.trim_matches('"')))
}

#[cfg(not(windows))]
fn uninstall_registry_path() -> Option<PathBuf> {
    None
}

#[cfg(windows)]
fn file_version(path: &Path) -> Option<String> {
    let escaped = path.to_string_lossy().replace('\'', "''");
    let script = format!("(Get-Item -LiteralPath '{escaped}').VersionInfo.ProductVersion");
    let output = Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .output()
        .ok()?;
    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!version.is_empty()).then_some(version)
}

#[cfg(not(windows))]
fn file_version(_path: &Path) -> Option<String> {
    None
}

fn version_is_compatible(version: Option<&str>) -> bool {
    let Some(version) = version else {
        return false;
    };
    let parts = version
        .split('.')
        .take(3)
        .map(|part| part.parse::<u64>().unwrap_or_default())
        .collect::<Vec<_>>();
    parts.as_slice() >= &[3, 10, 1]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_only_the_seven_supported_events() {
        assert_eq!(REGISTERED_HOOKS.len(), 7);
        assert!(REGISTERED_HOOKS.contains(&"PreToolUse"));
        assert!(!REGISTERED_HOOKS.contains(&"Notification"));
        assert!(!REGISTERED_HOOKS.contains(&"SessionEnd"));
    }

    #[test]
    fn decision_file_stems_do_not_collide_after_sanitizing() {
        assert_ne!(safe_stem("session:a/b"), safe_stem("session:a:b"));
        assert!(safe_stem(&"a".repeat(1_000)).len() <= 113);
    }

    #[test]
    fn timeout_budget_preserves_a_safety_window() {
        assert!(INTERACTION_WAIT_TIMEOUT_MS >= TOOL_WAIT_TIMEOUT_MS);
        assert!(OUTER_TIMEOUT_MS >= INTERACTION_WAIT_TIMEOUT_MS + TIMEOUT_SAFETY_WINDOW_MS);
    }

    #[test]
    fn app_server_runtime_preferences_match_the_protocol_schema() {
        let preferences = runtime_preferences();
        assert_eq!(preferences["nativeSearchEnhancementsEnabled"], true);
        assert_eq!(preferences["memoryEnabled"], false);
        assert_eq!(preferences["askUserQuestionAutoResolutionEnabled"], true);
        assert_eq!(preferences["modelContextBudgetStrategy"], "preflight-v1");
    }

    #[test]
    fn terminal_turn_events_are_filtered_by_method_and_type() {
        let completed = json!({
            "method": "session/event",
            "params": {"sessionId": "s", "type": "turn.completed"}
        });
        assert_eq!(
            terminal_turn_event(&completed),
            Some(("s".to_string(), AppServerTurnOutcome::Completed))
        );
        assert_eq!(
            terminal_turn_event(&json!({
                "method": "session/event",
                "params": {"sessionId": "s", "type": "turn.failed"}
            })),
            Some(("s".to_string(), AppServerTurnOutcome::Failed))
        );
        assert_eq!(
            terminal_turn_event(&json!({
                "method": "session/event",
                "params": {"sessionId": "s", "type": "message.upserted"}
            })),
            None
        );
        assert_eq!(
            terminal_turn_event(&json!({
                "method": "other/event",
                "params": {"sessionId": "s", "type": "turn.completed"}
            })),
            None
        );
    }

    #[test]
    fn automatic_mode_never_auto_approves_interaction_tools() {
        for tool in ["AskUserQuestion", "ExitPlanMode"] {
            let interactive = approval_policy::requires_user_decision(tool);
            assert!(interactive);
            assert!(
                !(!interactive
                    && approval_policy::should_auto_approve(
                        approval_policy::ApprovalMode::Automatic,
                        approval_policy::ApprovalRisk::Elevated,
                    ))
            );
        }
    }

    #[test]
    fn question_response_preserves_unknown_input_fields() {
        let payload = json!({
            "tool_input": {
                "questions": [{
                    "question":"Choose",
                    "options":[{"label":"A","description":"First"}],
                    "multiSelect":false
                }],
                "metadata": {"keep": true}
            }
        });
        let updated = question_updated_input(
            &payload,
            &[ZCodeQuestionAnswer {
                question: "Choose".to_string(),
                selected_option_labels: vec!["A".to_string()],
                extra_text: None,
            }],
            None,
        )
        .unwrap();
        assert_eq!(updated["metadata"]["keep"], true);
        assert_eq!(updated["answers"]["Choose"], "A");
    }

    #[test]
    fn question_response_combines_selected_options_with_other_text() {
        let payload = json!({
            "tool_input": {
                "questions": [{
                    "question":"Choose",
                    "options":[{"label":"A"},{"label":"B"}],
                    "multiSelect":true
                }]
            }
        });

        let selected = |selected_option_labels: Vec<&str>, extra_text: Option<&str>| {
            question_updated_input(
                &payload,
                &[ZCodeQuestionAnswer {
                    question: "Choose".to_string(),
                    selected_option_labels: selected_option_labels
                        .into_iter()
                        .map(str::to_string)
                        .collect(),
                    extra_text: extra_text.map(str::to_string),
                }],
                None,
            )
            .unwrap()["answers"]["Choose"]
                .as_str()
                .unwrap()
                .to_string()
        };

        assert_eq!(selected(vec!["A", "B"], None), "A, B");
        assert_eq!(selected(Vec::new(), Some(" custom ")), "custom");
        assert_eq!(selected(vec!["A", "B"], Some(" custom ")), "A, B, custom");
    }

    #[test]
    fn question_response_rejects_partial_or_empty_answers() {
        let payload = json!({
            "tool_input": {
                "questions": [
                    {"question":"First", "options":[], "multiSelect":false},
                    {"question":"Second", "options":[], "multiSelect":false}
                ]
            }
        });
        assert!(question_updated_input(
            &payload,
            &[ZCodeQuestionAnswer {
                question: "First".to_string(),
                selected_option_labels: Vec::new(),
                extra_text: Some(" ".to_string()),
            }],
            None,
        )
        .is_err());
    }

    #[test]
    fn question_response_rejects_unknown_or_excess_options() {
        let payload = json!({
            "tool_input": {
                "questions": [{
                    "question":"Choose",
                    "options":[{"label":"A"},{"label":"B"}],
                    "multiSelect":false
                }]
            }
        });
        for labels in [vec!["Unknown"], vec!["A", "B"]] {
            assert!(question_updated_input(
                &payload,
                &[ZCodeQuestionAnswer {
                    question: "Choose".to_string(),
                    selected_option_labels: labels.into_iter().map(str::to_string).collect(),
                    extra_text: None,
                }],
                None,
            )
            .is_err());
        }
    }

    #[test]
    fn merge_preserves_user_hooks_and_migrates_our_executable_path() {
        let mut root = json!({
            "mcp": {"keep": true},
            "hooks": {
                "enabled": false,
                "events": {
                    "PreToolUse": [{"hooks": [
                        {"type":"command", "command":"user-hook"},
                        {"type":"process", "command":"C:\\old\\CodeCraft.exe", "args":[HOOK_ARGUMENT], "timeoutMs": OUTER_TIMEOUT_MS}
                    ]}]
                }
            }
        });
        merge_hooks(&mut root, Path::new(r"C:\new\CodeCraft.exe"), true).unwrap();
        assert_eq!(root["mcp"]["keep"], true);
        assert_eq!(root["hooks"]["enabled"], true);
        assert_eq!(
            root["hooks"]["events"]["PreToolUse"][0]["hooks"][0]["command"],
            "user-hook"
        );
        assert_eq!(
            root["hooks"]["events"]["PreToolUse"][0]["hooks"][1]["command"],
            r"C:\new\CodeCraft.exe"
        );
        for event in REGISTERED_HOOKS {
            assert_eq!(
                event_hooks(&root, event)
                    .iter()
                    .filter(|hook| is_own_hook(hook))
                    .count(),
                1
            );
        }
    }

    #[test]
    fn reinstall_keeps_the_first_install_record() {
        let original = json!({"hooks":{"enabled":false,"timeoutMs":7}});
        let first = install_record_for(&original);
        let already_installed = json!({"hooks":{"enabled":true,"timeoutMs":OUTER_TIMEOUT_MS}});
        let selected = installation_record(&already_installed, Some(first));
        assert_eq!(selected.enabled_value, Some(false));
        assert_eq!(selected.timeout_value, Some(7));
    }

    #[test]
    fn uninstall_merge_removes_only_codecraft_hooks() {
        let mut root = json!({
            "plugins": {"keep": true},
            "hooks": {
                "enabled": true,
                "events": {
                    "PreToolUse": [{"hooks": [
                        {"type":"command", "command":"user-hook"},
                        expected_hook(Path::new("CodeCraft.exe"))
                    ]}]
                }
            }
        });
        merge_hooks(&mut root, Path::new("CodeCraft.exe"), true).unwrap();
        remove_own_hooks(&mut root);
        assert_eq!(root["plugins"]["keep"], true);
        assert_eq!(
            event_hooks(&root, "PreToolUse")
                .iter()
                .filter(|hook| !is_own_hook(hook))
                .count(),
            1
        );
        assert!(REGISTERED_HOOKS
            .iter()
            .all(|event| event_hooks(&root, event)
                .iter()
                .all(|hook| !is_own_hook(hook))));
    }

    #[test]
    fn atomic_config_write_creates_a_bak_before_replacement() {
        let directory = env::temp_dir().join(format!(
            "codecraft-zcode-config-test-{}-{}",
            process::id(),
            now_ms()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("config.json");
        fs::write(&path, br#"{"keep":"original"}"#).unwrap();

        backup_and_atomic_write(&path, &json!({"keep":"updated"})).unwrap();

        let backup = PathBuf::from(format!("{}.bak", path.to_string_lossy()));
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(&backup).unwrap()).unwrap(),
            json!({"keep":"original"})
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(&path).unwrap()).unwrap(),
            json!({"keep":"updated"})
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn modified_owned_hook_is_not_overwritten() {
        let mut root = json!({
            "hooks": {"events": {"PreToolUse": [{"hooks": [{
                "type":"process",
                "command":"CodeCraft.exe",
                "args":[HOOK_ARGUMENT],
                "timeoutMs":OUTER_TIMEOUT_MS,
                "custom":true
            }]}]}}
        });
        assert!(merge_hooks(&mut root, Path::new("CodeCraft.exe"), true).is_err());
    }

    #[test]
    fn owned_hook_timeout_changes_are_treated_as_modified() {
        let mut hook = expected_hook(Path::new("CodeCraft.exe"));
        hook["timeoutMs"] = Value::from(OUTER_TIMEOUT_MS - 1);
        assert!(own_hook_is_modified(&hook));
    }

    #[test]
    fn output_uses_the_zcode_pre_tool_shape() {
        let output = json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "allow",
                "updatedInput": {}
            }
        });
        assert_eq!(output["hookSpecificOutput"]["permissionDecision"], "allow");
        assert!(output["hookSpecificOutput"].get("decision").is_none());
    }

    #[test]
    fn supported_version_starts_at_3_10_1() {
        assert!(!version_is_compatible(Some("3.10.0")));
        assert!(version_is_compatible(Some("3.10.1")));
        assert!(version_is_compatible(Some("3.11.0.1")));
        assert!(!version_is_compatible(None));
    }
}
