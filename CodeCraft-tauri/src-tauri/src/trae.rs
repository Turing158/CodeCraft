use crate::{files, protocol::*, wire};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub source: String,
    pub session_key: String,
    pub installation_id: String,
    pub trae_instance_id: String,
    pub session_id: String,
    pub cwd: String,
    pub workspace_roots: Vec<String>,
    pub title: String,
    pub status: String,
    pub turn_epoch: u32,
    pub task_id: Option<String>,
    pub native_notice: Option<Value>,
    #[serde(default)]
    pub native_interactions: Vec<Value>,
    pub output: String,
    #[serde(default)]
    pub messages: Vec<SessionMessage>,
    #[serde(default)]
    pub history_truncated: bool,
    #[serde(default)]
    pub started_at: String,
    pub updated_at: String,
    pub activities: Vec<Value>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMessage {
    pub id: String,
    pub role: String,
    pub text: String,
    pub at: String,
}

impl Session {
    fn remember_output(&mut self) {
        // Preserve the last reply from snapshots written before message history existed.
        if self.messages.is_empty() && !self.output.is_empty() {
            self.messages.push(SessionMessage {
                id: id(),
                role: "assistant".into(),
                text: self.output.clone(),
                at: self.updated_at.clone(),
            });
        }
    }

    fn record_message(&mut self, role: &str, text: &str) {
        self.remember_output();
        let body: String = text.chars().take(32768).collect();
        self.history_truncated |= body.len() < text.len();
        if body.is_empty()
            || (role == "assistant"
                && self
                    .messages
                    .last()
                    .is_some_and(|m| m.role == role && m.text == body))
        {
            return;
        }
        self.messages.push(SessionMessage {
            id: id(),
            role: role.into(),
            text: body,
            at: stamp(0),
        });
        let mut bytes: usize = self.messages.iter().map(|m| m.text.len()).sum();
        while self.messages.len() > 256 || bytes > 512 * 1024 {
            bytes -= self.messages.remove(0).text.len();
            self.history_truncated = true;
        }
    }

    fn finish_activities(&mut self) {
        for activity in &mut self.activities {
            if activity["status"] == "running" {
                activity["status"] = json!("unknown");
                activity["at"] = json!(stamp(0));
            }
        }
    }

    fn record_activity(&mut self, input: &Value) {
        let post = input["hook_event_name"] == "PostToolUse";
        let index = self.activities.iter().position(|a| {
            a["toolUseId"] == input["tool_use_id"]
                && a["tool"] == input["tool_name"]
                && a["turnEpoch"] == self.turn_epoch
        });
        // Repeated PreToolUse must not turn a finished call back into a running one.
        if !post && index.is_some() {
            return;
        }
        let now = stamp(0);
        let arguments = redacted_activity(&input["tool_input"]);
        let response = &input["tool_response"];
        let has_result = !response.is_null()
            && match response {
                Value::String(s) => !s.is_empty(),
                Value::Object(o) => !o.is_empty(),
                Value::Array(a) => !a.is_empty(),
                _ => true,
            };
        let failed = response["is_error"] == true
            || response["isError"] == true
            || response["success"] == false;
        let status = if !post {
            "running"
        } else if !has_result {
            "unknown"
        } else if failed {
            "failed"
        } else {
            "completed"
        };
        let mut activity = json!({
            "id":format!("{}:{}", self.turn_epoch, input["tool_use_id"].as_str().unwrap_or_default()),
            "turnEpoch":self.turn_epoch, "tool":input["tool_name"], "toolUseId":input["tool_use_id"],
            "status":status, "arguments":arguments, "at":now, "startedAt":now,
        });
        if post && has_result {
            activity["result"] = redacted_activity(response);
        }
        if let Some(index) = index {
            activity["startedAt"] = self.activities[index]["startedAt"].clone();
            self.activities[index] = activity;
        } else {
            self.activities.push(activity);
        }
        let mut bytes: usize = self.activities.iter().map(|a| a.to_string().len()).sum();
        while self.activities.len() > 200 || bytes > 512 * 1024 {
            bytes -= self.activities.remove(0).to_string().len();
            self.history_truncated = true;
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub task_id: String,
    pub plan_id: String,
    pub version: u32,
    pub title: String,
    pub primary_root: String,
    pub workspace_roots: Vec<String>,
    pub document_path: String,
    pub state: String,
    pub session_key: Option<String>,
    pub turn_epoch: u32,
    pub revision: u32,
    pub content_hash: String,
    pub file_identity: String,
    pub approved: bool,
    pub expires_at: String,
    #[serde(default)]
    pub closed_at: Option<String>,
    launch_hash: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub target: Target,
    pub kind: String,
    pub channel: String,
    pub tool_use_id: String,
    pub tool_name: String,
    pub arguments: Value,
    pub state: String,
    pub expires_at: String,
    pub plan: Option<Value>,
    pub result: Option<ToolResult>,
    pub error: Option<ApiError>,
    decision: Option<Action>,
    connection: Option<String>,
    lease: Option<String>,
    parameter_hash: String,
}
impl Request {
    fn transition(&mut self, state: &str) {
        if self.state != state {
            self.state = state.into();
            self.target.request_version = self
                .target
                .request_version
                .saturating_add(1)
                .min(MAX_VERSION);
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Grant {
    pub grant_id: String,
    pub version: u32,
    pub session_key: String,
    pub task_id: Option<String>,
    pub turn_epoch: u32,
    pub revision: u32,
    pub content_hash: String,
    pub parameter_hash: String,
    pub tool_use_id: String,
    pub tool_name: String,
    pub state: String,
    pub resolution: Option<String>,
    #[serde(default)]
    pub closed_at: Option<String>,
}
#[derive(Clone)]
struct Ticket {
    token: String,
    session: String,
    task: Option<String>,
    turn: u32,
    call: String,
    tool: String,
    hash: String,
    operation: Option<String>,
}
#[derive(Clone, Default)]
struct Runtime {
    plan_files: HashMap<String, crate::native::PlanFiles>,
    deadlines: HashMap<String, Instant>,
    tickets: HashMap<String, Ticket>,
    calls: HashMap<String, (String, String)>,
    controls: HashMap<String, Value>,
    last_cleanup: Option<Instant>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Ledger {
    hash: String,
    epoch: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Store {
    pub epoch: String,
    pub version: u64,
    pub capabilities: Capabilities,
    pub sessions: HashMap<String, Session>,
    pub tasks: HashMap<String, Task>,
    pub requests: HashMap<String, Request>,
    pub grants: HashMap<String, Grant>,
    ledger: HashMap<String, Ledger>,
    #[serde(skip)]
    runtime: Runtime,
    #[serde(skip)]
    root: PathBuf,
}
fn err(code: ErrorCode, message: &str) -> ApiError {
    ApiError::new(code, message)
}
fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| invalid(format!("Missing {key}")))
}
fn secret() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| err(ErrorCode::StateUnavailable, &e.to_string()))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
fn digest(value: &Value) -> Result<String> {
    wire::hash(value).map_err(invalid)
}
fn decoded<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T> {
    serde_json::from_value(value.clone()).map_err(|e| invalid(e.to_string()))
}
fn read_arguments_known(tool: &str, args: &Value) -> bool {
    let Some(object) = args.as_object() else {
        return false;
    };
    let allowed: &[&str] = match tool {
        "Read" => &["file_path", "offset", "limit"],
        "Glob" => &["pattern", "path"],
        "Grep" => &[
            "pattern",
            "path",
            "glob",
            "output_mode",
            "head_limit",
            "multiline",
            "-i",
            "-n",
            "-A",
            "-B",
            "-C",
        ],
        "LS" => &["path", "ignore"],
        _ => return false,
    };
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return false;
    }
    match tool {
        "Read" => args["file_path"].as_str().is_some_and(|p| !p.is_empty()),
        "Glob" | "Grep" => args["pattern"].is_string(),
        "LS" => args["path"].is_string(),
        _ => false,
    }
}
fn terminal(state: &str) -> bool {
    matches!(
        state,
        "delivered" | "cancelled" | "expired" | "failed" | "returned_to_trae"
    )
}

impl Store {
    pub fn new(root: PathBuf, capabilities: Capabilities) -> Self {
        Self {
            root,
            epoch: id(),
            version: 1,
            capabilities,
            sessions: HashMap::new(),
            tasks: HashMap::new(),
            requests: HashMap::new(),
            grants: HashMap::new(),
            ledger: HashMap::new(),
            runtime: Runtime::default(),
        }
    }
    fn deadline(&mut self, key: String, seconds: u64) {
        self.runtime
            .deadlines
            .insert(key, Instant::now() + Duration::from_secs(seconds));
    }
    fn live(&self, key: &str) -> bool {
        self.runtime
            .deadlines
            .get(key)
            .is_some_and(|d| *d > Instant::now())
    }
    fn revoke(&mut self, task: &str) {
        if let Some(t) = self.tasks.get_mut(task) {
            if t.approved {
                t.version = t.version.saturating_add(1).min(MAX_VERSION);
            }
            t.approved = false;
            if t.state == "approved" {
                t.state = "awaiting_plan".into();
            }
        }
        self.runtime.deadlines.remove(&format!("approval:{task}"));
        for r in self
            .requests
            .values_mut()
            .filter(|r| r.target.task_id.as_deref() == Some(task) && !terminal(&r.state))
        {
            r.transition("cancelled");
            r.error = Some(err(
                ErrorCode::PlanChanged,
                "Plan permission was invalidated; issue a new tool call",
            ));
        }
    }
    fn invalidate_session(&mut self, key: &str) -> Result<()> {
        self.runtime.plan_files.remove(key);
        let s = self
            .sessions
            .get_mut(key)
            .ok_or_else(|| err(ErrorCode::NotFound, "Session not found"))?;
        s.native_notice = None;
        s.native_interactions.clear();
        s.turn_epoch = s
            .turn_epoch
            .checked_add(1)
            .filter(|n| *n <= MAX_VERSION)
            .ok_or_else(|| err(ErrorCode::TaskChanged, "Turn version exhausted"))?;
        let task = s.task_id.clone().filter(|task| {
            self.tasks
                .get(task)
                .is_some_and(|t| t.session_key.as_deref() == Some(key))
        });
        let turn = s.turn_epoch;
        if let Some(task) = task {
            self.revoke(&task);
            if let Some(t) = self.tasks.get_mut(&task) {
                t.turn_epoch = turn;
            }
        }
        for r in self
            .requests
            .values_mut()
            .filter(|r| r.target.session_key == key && !terminal(&r.state))
        {
            r.transition("cancelled");
            r.error = Some(err(
                ErrorCode::TaskChanged,
                "User input or session state changed",
            ));
        }
        Ok(())
    }
    fn unresolved(&self, session: &str) -> bool {
        self.grants
            .values()
            .any(|g| g.session_key == session && g.resolution.is_none())
    }
    fn session(&mut self, input: &Value, identity: &Value) -> Result<String> {
        let installation = text(identity, "installationId")?;
        let instance = text(identity, "traeInstanceId")?;
        let native = text(input, "session_id")?;
        let (roots, cwd) =
            if wire::workspace_less_observation(text(input, "hook_event_name")?, input) {
                // No filesystem scope exists. Never resolve '.' against CodeCraft's
                // process directory or manufacture a workspace for standalone Chat.
                (Vec::new(), String::new())
            } else {
                let roots: Vec<String> = decoded(&input["workspace_roots"])?;
                let roots = files::roots(
                    roots
                        .first()
                        .ok_or_else(|| invalid("Workspace roots missing"))?,
                    &roots,
                )?;
                let cwd = files::canonical_directory(text(input, "cwd")?)?
                    .to_string_lossy()
                    .into_owned();
                if !roots.iter().any(|r| Path::new(&cwd).starts_with(r)) {
                    return Err(invalid("Working directory is outside the workspace roots"));
                }
                (roots, cwd)
            };
        let key = digest(&json!([installation, instance, native]))?;
        if roots.is_empty()
            && self
                .sessions
                .get(&key)
                .is_some_and(|s| !s.workspace_roots.is_empty())
        {
            // A disappearing workspace cannot leave earlier tool approvals live.
            self.invalidate_session(&key)?;
            return Err(err(
                ErrorCode::TaskChanged,
                "Workspace scope is no longer available",
            ));
        }
        if !self.sessions.contains_key(&key) && self.sessions.len() >= 1024 {
            return Err(err(ErrorCode::QueueFull, "Too many retained Trae sessions"));
        }
        let s = self.sessions.entry(key.clone()).or_insert_with(|| Session {
            source: "trae".into(),
            session_key: key.clone(),
            installation_id: installation.into(),
            trae_instance_id: instance.into(),
            session_id: native.into(),
            cwd: cwd.clone(),
            workspace_roots: roots.clone(),
            title: "Trae".into(),
            status: "idle".into(),
            ..Session::default()
        });
        if s.workspace_roots != roots {
            return Err(err(
                ErrorCode::TaskChanged,
                "Workspace roots changed; start a new protected task",
            ));
        }
        s.remember_output();
        s.cwd = cwd;
        if s.started_at.is_empty() {
            s.started_at = if s.updated_at.is_empty() {
                stamp(0)
            } else {
                s.updated_at.clone()
            };
        }
        s.updated_at = stamp(0);
        Ok(key)
    }
    fn current(&self, r: &Request) -> Result<()> {
        if r.target.request_version >= MAX_VERSION {
            return Err(err(ErrorCode::RequestExpired, "Request version exhausted"));
        }
        if r.target.app_epoch != self.epoch {
            return Err(err(
                ErrorCode::RequestExpired,
                "Request belongs to an earlier CodeCraft process",
            ));
        }
        let s = self
            .sessions
            .get(&r.target.session_key)
            .ok_or_else(|| err(ErrorCode::NotFound, "Session not found"))?;
        if s.turn_epoch != r.target.turn_epoch || s.task_id != r.target.task_id {
            return Err(err(ErrorCode::TaskChanged, "Request task or turn changed"));
        }
        if let Some(task) = &r.target.task_id {
            let t = self
                .tasks
                .get(task)
                .ok_or_else(|| err(ErrorCode::TaskChanged, "Task no longer exists"))?;
            if t.session_key.as_deref() != Some(&r.target.session_key)
                || t.turn_epoch != r.target.turn_epoch
            {
                return Err(err(
                    ErrorCode::TaskChanged,
                    "Task moved to another session or turn",
                ));
            }
            if matches!(
                t.state.as_str(),
                "paused" | "ended" | "armed" | "awaiting_user"
            ) || !self.live(&format!("task:{task}"))
            {
                return Err(err(ErrorCode::TaskChanged, "Protected task is inactive"));
            }
        }
        if !self.live(&format!("request:{}", r.target.request_id)) && r.state != "delivered" {
            return Err(err(ErrorCode::RequestExpired, "Request expired"));
        }
        Ok(())
    }
    fn plan_current(&self, task: &Task) -> Result<()> {
        let (_, hash) = files::plan(&task.document_path, &task.primary_root)?;
        if hash != task.content_hash
            || files::plan_identity(&task.document_path)? != task.file_identity
        {
            let mut e = err(
                ErrorCode::PlanChanged,
                "Plan file changed; submit the new version for review",
            );
            e.error.current_revision = Some(task.revision);
            return Err(e);
        }
        Ok(())
    }
    fn gate(&self, session: &str, tool: &str, args: &Value) -> Result<()> {
        let s = &self.sessions[session];
        let Some(task_id) = &s.task_id else {
            return Ok(());
        };
        let t = &self.tasks[task_id];
        if t.session_key.as_deref() != Some(session) || t.turn_epoch != s.turn_epoch {
            return Err(err(
                ErrorCode::TaskChanged,
                "Task is bound to another session or turn",
            ));
        }
        if !self.live(&format!("task:{task_id}"))
            || matches!(
                t.state.as_str(),
                "paused" | "ended" | "armed" | "awaiting_user"
            )
        {
            return Err(err(ErrorCode::TaskChanged, "Protected task is not active"));
        }
        if self.capabilities.tool_input_mappings_verified && read_arguments_known(tool, args) {
            return Ok(());
        }
        if self.capabilities.tool_input_mappings_verified
            && matches!(tool, "Write" | "Edit")
            && args.as_object().is_some_and(|o| {
                o.keys().all(|k| {
                    matches!(
                        k.as_str(),
                        "file_path"
                            | "path"
                            | "content"
                            | "old_string"
                            | "new_string"
                            | "replace_all"
                    )
                })
            })
        {
            if let Some(path) = args
                .get("file_path")
                .or_else(|| args.get("path"))
                .and_then(Value::as_str)
            {
                if Path::new(path).is_absolute()
                    && Path::new(path).canonicalize().ok()
                        == Path::new(&t.document_path).canonicalize().ok()
                    && Path::new(path)
                        .canonicalize()?
                        .starts_with(files::canonical_directory(&t.primary_root)?)
                {
                    return Ok(());
                }
            }
        }
        if !t.approved || !self.live(&format!("approval:{task_id}")) {
            return Err(err(
                ErrorCode::PlanChanged,
                "This task needs a current, delivered plan approval",
            ));
        }
        self.plan_current(t)
    }
    fn capacity(&self, session: &str) -> Result<()> {
        if self
            .requests
            .values()
            .filter(|r| !terminal(&r.state))
            .count()
            >= 128
            || self
                .requests
                .values()
                .filter(|r| r.target.session_key == session && !terminal(&r.state))
                .count()
                >= 16
        {
            return Err(err(ErrorCode::QueueFull, "Too many pending interactions"));
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    fn request(
        &mut self,
        key: &str,
        call: &str,
        tool: &str,
        args: Value,
        kind: &str,
        channel: &str,
        seconds: u64,
        connection: Option<String>,
        plan: Option<Value>,
    ) -> Result<String> {
        self.capacity(key)?;
        if self.requests.len() >= 4096 {
            return Err(err(
                ErrorCode::QueueFull,
                "Retained interaction history is full",
            ));
        }
        let s = &self.sessions[key];
        let request = id();
        let r = Request {
            target: Target {
                app_epoch: self.epoch.clone(),
                session_key: key.into(),
                task_id: s.task_id.clone(),
                turn_epoch: s.turn_epoch,
                request_id: request.clone(),
                request_version: 1,
            },
            kind: kind.into(),
            channel: channel.into(),
            tool_use_id: call.into(),
            tool_name: tool.into(),
            parameter_hash: digest(&args)?,
            arguments: args,
            state: "pending".into(),
            expires_at: stamp(seconds),
            plan,
            result: None,
            error: None,
            decision: None,
            connection,
            lease: None,
        };
        self.requests.insert(request.clone(), r);
        self.deadline(format!("request:{request}"), seconds);
        self.deadline(format!("history:{request}"), seconds + 900);
        self.sessions.get_mut(key).unwrap().status = if kind == "question" {
            "waitingForInput"
        } else {
            "waitingForApproval"
        }
        .into();
        Ok(request)
    }

    fn hook(&mut self, c: &Value) -> Result<Value> {
        let input = &c["input"];
        let event = text(input, "hook_event_name")?;
        wire::validate_hook(event, input).map_err(invalid)?;
        let key = self.session(input, &c["identity"])?;
        if event == "UserPromptSubmit" {
            self.sessions.get_mut(&key).unwrap().finish_activities();
            self.invalidate_session(&key)?;
            let prompt = text(input, "prompt")?;
            if !self.sessions[&key].workspace_roots.is_empty()
                && prompt.contains("[[CODECRAFT_TASK:")
            {
                let line = prompt.lines().next().unwrap_or("");
                let code = line
                    .strip_prefix("[[CODECRAFT_TASK:")
                    .and_then(|s| s.strip_suffix("]]"));
                let code_hash = code.map(|v| digest(&json!(v))).transpose()?;
                let task = self
                    .tasks
                    .values()
                    .find(|t| Some(&t.launch_hash) == code_hash.as_ref())
                    .map(|t| t.task_id.clone())
                    .ok_or_else(|| err(ErrorCode::TaskChanged, "Invalid launch instruction"))?;
                let t = self.tasks[&task].clone();
                if t.state != "armed"
                    || !self.live(&format!("launch:{task}"))
                    || prompt.matches("[[CODECRAFT_TASK:").count() != 1
                {
                    if let Some(old) = t.session_key {
                        if old != key {
                            self.invalidate_session(&old)?;
                        }
                    }
                    return Err(err(ErrorCode::TaskChanged,"Launch instruction was consumed or expired; resume the task for a new code"));
                }
                if t.workspace_roots != self.sessions[&key].workspace_roots {
                    return Err(err(
                        ErrorCode::TaskChanged,
                        "Launch scope does not match this session",
                    ));
                }
                if self.unresolved(&key) {
                    return Err(err(
                        ErrorCode::NativeConfirmationPending,
                        "Resolve previously delivered calls in Trae before binding a task",
                    ));
                }
                if let Some(old) = self.sessions[&key].task_id.clone() {
                    if old != task {
                        self.revoke(&old);
                        self.tasks.get_mut(&old).unwrap().state = "ended".into();
                        self.tasks.get_mut(&old).unwrap().closed_at = Some(stamp(0));
                    }
                }
                let turn = self.sessions[&key].turn_epoch.max(
                    t.turn_epoch
                        .checked_add(1)
                        .filter(|v| *v <= MAX_VERSION)
                        .ok_or_else(|| err(ErrorCode::TaskChanged, "Turn version exhausted"))?,
                );
                let s = self.sessions.get_mut(&key).unwrap();
                s.turn_epoch = turn;
                s.task_id = Some(task.clone());
                s.status = "working".into();
                let t = self.tasks.get_mut(&task).unwrap();
                t.state = "awaiting_plan".into();
                t.session_key = Some(key);
                t.turn_epoch = turn;
                t.version = t.version.saturating_add(1).min(MAX_VERSION);
                return Ok(
                    json!({"output":{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":format!("CodeCraft protected task {}. Write the plan to {}. Then call codecraft_review_plan with schemaVersion=1, planId={}, baseRevision={}. Execution requires a current plan approval. Native confirmations remain in Trae.",t.task_id,t.document_path,t.plan_id,t.revision)}}}),
                );
            }
            if let Some(task) = self.sessions[&key].task_id.clone() {
                if self.tasks[&task].state == "ended" && !self.unresolved(&key) {
                    self.sessions.get_mut(&key).unwrap().task_id = None;
                }
            }
            if let Some(task) = self.sessions[&key].task_id.clone() {
                if self.tasks[&task].session_key.as_deref() == Some(&key)
                    && self.tasks[&task].state == "awaiting_user"
                {
                    self.tasks.get_mut(&task).unwrap().state = "awaiting_plan".into();
                }
            }
            let s = self.sessions.get_mut(&key).unwrap();
            s.record_message("user", prompt);
            s.title = prompt.chars().take(200).collect();
            s.status = "working".into();
            return Ok(json!({"output":{}}));
        }
        if event == "SessionStart" {
            self.sessions.get_mut(&key).unwrap().finish_activities();
            self.invalidate_session(&key)?;
            if let Some(task) = self.sessions[&key].task_id.clone() {
                if self.tasks[&task].session_key.as_deref() == Some(&key)
                    && self.tasks[&task].state != "ended"
                {
                    self.tasks.get_mut(&task).unwrap().state = "paused".into();
                }
            }
            return Ok(json!({"output":{}}));
        }
        if event == "Stop" {
            self.invalidate_session(&key)?;
            if let Some(task) = self.sessions[&key].task_id.clone() {
                if self.tasks[&task].session_key.as_deref() == Some(&key)
                    && !matches!(self.tasks[&task].state.as_str(), "paused" | "ended")
                {
                    self.tasks.get_mut(&task).unwrap().state = "awaiting_user".into();
                }
            }
            let s = self.sessions.get_mut(&key).unwrap();
            s.status = "stopped".into();
            s.finish_activities();
            s.record_message(
                "assistant",
                input["last_assistant_message"].as_str().unwrap_or(""),
            );
            s.output = input["last_assistant_message"]
                .as_str()
                .unwrap_or("")
                .chars()
                .take(32768)
                .collect();
            return Ok(json!({"output":{}}));
        }
        if matches!(event, "PreToolUse" | "PostToolUse") {
            self.sessions.get_mut(&key).unwrap().record_activity(input);
            self.runtime
                .plan_files
                .entry(key.clone())
                .or_default()
                .observe(input);
        }
        // Trae may continue an approved native plan directly with AskUserQuestion,
        // without a PostToolUse for NotifyUser. Retire the plan before the native
        // observation branch returns early, including notification-only questions.
        if event == "PreToolUse"
            || (event == "Notification" && input["notification_type"] == "ask_user_question")
        {
            self.sessions
                .get_mut(&key)
                .unwrap()
                .native_interactions
                .retain(|item| item["kind"] != "plan");
        }
        if let Some(mut observation) = crate::native::observation(
            &key,
            self.sessions[&key].turn_epoch,
            input,
            self.runtime.plan_files.get(&key),
        )? {
            let s = self.sessions.get_mut(&key).unwrap();
            // A notification often repeats the corresponding PreToolUse. Preserve
            // its richer arguments and stable identity rather than opening two cards.
            let same_kind = s
                .native_interactions
                .iter()
                .filter(|item| item["kind"] == observation["kind"])
                .count();
            if let Some(existing) = s.native_interactions.iter_mut().find(|item| {
                item["kind"] == observation["kind"]
                    && (item["id"] == observation["id"]
                        || (observation["toolUseId"].is_null() && same_kind == 1)
                        || item["toolUseId"] == observation["toolUseId"])
            }) {
                if observation["arguments"]
                    .as_object()
                    .is_some_and(|o| o.is_empty())
                {
                    observation["arguments"] = existing["arguments"].clone();
                }
                observation["id"] = existing["id"].clone();
                observation["capturedAt"] = existing["capturedAt"].clone();
                if observation["toolUseId"].is_null() {
                    observation["toolUseId"] = existing["toolUseId"].clone();
                }
                *existing = observation;
            } else if s.native_interactions.len() < 16 {
                s.native_interactions.push(observation);
            }
            s.status = "waitingForInput".into();
            // Observation only: do not hold AskUserQuestion behind a tool-approval card.
            return Ok(json!({"output":{}}));
        }
        if event == "Notification" {
            if input["notification_type"] == "idle_prompt" {
                self.runtime.plan_files.remove(&key);
                let session = self.sessions.get_mut(&key).unwrap();
                session.native_interactions.clear();
                if session.workspace_roots.is_empty() {
                    session.status = "idle".into();
                }
            }
            return Ok(json!({"output":{}}));
        }
        if event == "PostToolUse" {
            let parameter_hash = digest(&input["tool_input"])?;
            for g in self.grants.values_mut().filter(|g| {
                g.session_key == key
                    && Some(g.tool_use_id.as_str()) == input["tool_use_id"].as_str()
                    && Some(g.tool_name.as_str()) == input["tool_name"].as_str()
                    && g.parameter_hash == parameter_hash
                    && g.resolution.is_none()
            }) {
                g.resolution = Some("observed_completed".into());
                g.closed_at = Some(stamp(0));
                g.state = "completed".into();
                g.version = g.version.saturating_add(1).min(MAX_VERSION);
            }
            let s = self.sessions.get_mut(&key).unwrap();
            s.native_notice = None;
            s.native_interactions.retain(|item| {
                item["toolUseId"].is_null() || item["toolUseId"] != input["tool_use_id"]
            });
            s.status = "working".into();
            return Ok(json!({"output":{}}));
        }
        let tool = text(input, "tool_name")?;
        self.sessions
            .get_mut(&key)
            .unwrap()
            .native_interactions
            .retain(|item| !item["toolUseId"].is_null());
        let call = text(input, "tool_use_id")?;
        let args = &input["tool_input"];
        let call_key = digest(&json!([key, call]))?;
        let fingerprint = if own_tool(tool).is_some() {
            wire::business_args(args).map_err(invalid)?
        } else {
            args.clone()
        };
        let args_hash = digest(&json!([
            tool,
            fingerprint,
            if own_tool(tool).is_none() {
                Some(&self.sessions[&key].cwd)
            } else {
                None
            }
        ]))?;
        if !self.runtime.calls.contains_key(&call_key) && self.runtime.calls.len() >= 65536 {
            return Err(err(
                ErrorCode::QueueFull,
                "Native call history is full; restart CodeCraft",
            ));
        }
        if let Some((hash, registration)) = self.runtime.calls.get(&call_key) {
            if *hash != args_hash {
                return Err(err(
                    ErrorCode::IdempotencyConflict,
                    "Native tool ID was reused with different parameters",
                ));
            }
            if let Some(r) = self.requests.get(registration) {
                self.current(r)?;
                if terminal(&r.state) {
                    return Err(err(
                        ErrorCode::RequestExpired,
                        "This tool call already finished; issue a new call",
                    ));
                }
                return Ok(json!({"requestId":registration}));
            }
            let ticket = self
                .runtime
                .tickets
                .get(registration)
                .ok_or_else(|| err(ErrorCode::TicketExpired, "Ticket expired"))?;
            if ticket.operation.is_some()
                || !self.live(&format!("ticket:{registration}"))
                || ticket.turn != self.sessions[&key].turn_epoch
            {
                return Err(err(ErrorCode::TicketExpired, "Issue a new MCP call"));
            }
            let mut updated = args.clone();
            updated["bridgeTicket"] = json!(ticket.token);
            return Ok(
                json!({"output":wire::permission("allow","CodeCraft interaction binding",Some(updated))}),
            );
        }
        if let Some(own) = own_tool(tool) {
            if self.capabilities.mcp_wait().is_none()
                || !(if own == ASK_TOOL {
                    self.capabilities.mcp_questions
                } else {
                    self.capabilities.mcp_plan_review
                })
            {
                return Err(err(
                    ErrorCode::UnsupportedVersion,
                    "MCP workflow awaits Trae runtime verification",
                ));
            }
            validate_business(own, args)?;
            if own == PLAN_TOOL && self.sessions[&key].task_id.is_none() {
                return Err(err(
                    ErrorCode::TaskChanged,
                    "Start a protected CodeCraft task before submitting a plan",
                ));
            }
            if self.runtime.tickets.len() >= 4096 {
                return Err(err(
                    ErrorCode::QueueFull,
                    "Too many retained bridge tickets",
                ));
            }
            if let Some(task) = &self.sessions[&key].task_id {
                if self.tasks[task].session_key.as_deref() != Some(&key)
                    || self.tasks[task].turn_epoch != self.sessions[&key].turn_epoch
                {
                    return Err(err(ErrorCode::TaskChanged, "Task binding changed"));
                }
                if matches!(
                    self.tasks[task].state.as_str(),
                    "paused" | "ended" | "armed" | "awaiting_user"
                ) || !self.live(&format!("task:{task}"))
                {
                    return Err(err(ErrorCode::TaskChanged, "Protected task is inactive"));
                }
            }
            let token = secret()?;
            let token_hash = digest(&json!(token))?;
            let s = &self.sessions[&key];
            self.runtime.tickets.insert(
                token_hash.clone(),
                Ticket {
                    token: token.clone(),
                    session: key,
                    task: s.task_id.clone(),
                    turn: s.turn_epoch,
                    call: call.into(),
                    tool: own.into(),
                    hash: digest(&wire::business_args(args).map_err(invalid)?)?,
                    operation: None,
                },
            );
            self.deadline(format!("ticket:{token_hash}"), 600);
            self.runtime.calls.insert(call_key, (args_hash, token_hash));
            let mut updated = args.clone();
            updated["bridgeTicket"] = json!(token);
            return Ok(
                json!({"output":wire::permission("allow","CodeCraft interaction binding",Some(updated))}),
            );
        }
        if self.requests.values().any(|r| {
            r.target.session_key == key
                && r.target.turn_epoch == self.sessions[&key].turn_epoch
                && r.kind == "plan"
                && matches!(r.state.as_str(), "user_decided" | "delivery_prepared")
                && matches!(
                    r.decision,
                    Some(Action::Plan {
                        decision: PlanDecision::Approved,
                        ..
                    })
                )
        }) {
            return Ok(json!({"waitForPlan":true}));
        }
        self.gate(&key, tool, args)?;
        let Some(wait) = self.capabilities.hook_wait() else {
            return Ok(
                json!({"output":wire::permission(if self.sessions[&key].task_id.is_some(){"deny"}else{"ask"},"CodeCraft tool approval is unavailable for this version; use native confirmation",None)}),
            );
        };
        let request = self.request(
            &key,
            call,
            tool,
            args.clone(),
            "permission",
            "hook",
            wait,
            None,
            None,
        )?;
        self.runtime
            .calls
            .insert(call_key, (args_hash, request.clone()));
        let risk = match tool {
            "RunCommand"
                if self.capabilities.tool_input_mappings_verified
                    && args.as_object().is_some_and(|o| {
                        o.keys().all(|k| matches!(k.as_str(), "command" | "cwd"))
                    }) =>
            {
                args.get("command")
                    .and_then(Value::as_str)
                    .map(crate::approval_policy::risk_for_command)
                    .unwrap_or(crate::approval_policy::ApprovalRisk::Elevated)
            }
            "Read" | "Glob" | "Grep" | "LS"
                if self.capabilities.tool_input_mappings_verified
                    && read_arguments_known(tool, args) =>
            {
                crate::approval_policy::ApprovalRisk::Low
            }
            _ => crate::approval_policy::ApprovalRisk::Elevated,
        };
        if tool != "AskUserQuestion"
            && crate::approval_policy::should_auto_approve(
                crate::approval_policy::load_settings().mode,
                risk,
            )
        {
            let r = self.requests.get_mut(&request).unwrap();
            r.decision = Some(Action::Permission {
                decision: PermissionDecision::Allow,
                message: None,
            });
            r.transition("user_decided");
        }
        Ok(json!({"requestId":request}))
    }
    fn consume(&mut self, c: &Value) -> Result<Value> {
        let ticket_hash = digest(&json!(text(c, "ticket")?))?;
        let ticket = self
            .runtime
            .tickets
            .get(&ticket_hash)
            .cloned()
            .ok_or_else(|| err(ErrorCode::TicketInvalid, "Missing or invalid bridge ticket"))?;
        let connection = text(c, "connection")?;
        let args = &c["arguments"];
        let tool = text(c, "tool")?;
        validate_business(tool, args)?;
        let s = &self.sessions[&ticket.session];
        if ticket.turn != s.turn_epoch || ticket.task != s.task_id {
            return Err(err(
                ErrorCode::TaskChanged,
                "Ticket belongs to an old task or turn",
            ));
        }
        if ticket.tool != tool
            || ticket.hash != digest(&wire::business_args(args).map_err(invalid)?)?
        {
            return Err(err(
                ErrorCode::ArgumentMismatch,
                "Tool parameters changed after Hook binding",
            ));
        }
        if let Some(op) = ticket.operation {
            let r = &self.requests[&op];
            if r.connection.as_deref() != Some(connection) {
                return Err(err(
                    ErrorCode::TicketInvalid,
                    "Ticket belongs to another MCP connection",
                ));
            }
            self.current(r)?;
            if !self.live(&format!("cache:{op}")) && terminal(&r.state) {
                return Err(err(ErrorCode::RequestExpired, "Result cache expired"));
            }
            return Ok(json!({"operationId":op,"replayed":true}));
        }
        if !self.live(&format!("ticket:{ticket_hash}")) {
            return Err(err(ErrorCode::TicketExpired, "Ticket expired before use"));
        }
        self.capacity(&ticket.session)?;
        let mut plan = None;
        if tool == PLAN_TOOL {
            let input: PlanInput = decoded(args)?;
            let task_id = ticket
                .task
                .as_ref()
                .ok_or_else(|| err(ErrorCode::TaskChanged, "No protected task"))?;
            let t = &self.tasks[task_id];
            if t.state == "paused" || t.state == "ended" || !self.live(&format!("task:{task_id}")) {
                return Err(err(ErrorCode::TaskChanged, "Task is not active"));
            }
            if self.requests.values().any(|r| {
                r.target.task_id.as_ref() == Some(task_id)
                    && r.kind == "plan"
                    && !terminal(&r.state)
            }) {
                return Err(err(
                    ErrorCode::RequestConflict,
                    "One plan review is already pending",
                ));
            }
            let (body, hash) = files::plan(&t.document_path, &t.primary_root)?;
            if input.plan_id != t.plan_id
                || input.document_path != t.document_path
                || input.base_revision != t.revision
                || files::normalize(&input.plan_markdown) != body
            {
                let mut e = err(
                    ErrorCode::PlanChanged,
                    "Plan baseline or file content changed",
                );
                e.error.current_revision = Some(t.revision);
                return Err(e);
            }
            let identity = files::plan_identity(&t.document_path)?;
            let revision = t
                .revision
                .checked_add(1)
                .filter(|v| *v <= MAX_VERSION)
                .ok_or_else(|| err(ErrorCode::PlanChanged, "Plan revision exhausted"))?;
            files::atomic(
                &self
                    .root
                    .join("tasks")
                    .join(task_id)
                    .join(format!("revision-{revision}.json")),
                &json!({"revision":revision,"contentHash":hash,"markdown":body}),
                false,
            )?;
            self.revoke(task_id);
            let t = self.tasks.get_mut(task_id).unwrap();
            t.revision = revision;
            t.content_hash = hash.clone();
            t.file_identity = identity;
            t.version = t.version.saturating_add(1).min(MAX_VERSION);
            plan = Some(
                json!({"planId":t.plan_id,"revision":revision,"contentHash":hash,"documentPath":t.document_path,"markdown":body}),
            );
        }
        let wait = self.capabilities.mcp_wait().ok_or_else(|| {
            err(
                ErrorCode::UnsupportedVersion,
                "MCP client timeout is not verified",
            )
        })?;
        let operation = self.request(
            &ticket.session,
            &ticket.call,
            tool,
            wire::business_args(args).map_err(invalid)?,
            if tool == PLAN_TOOL {
                "plan"
            } else {
                "question"
            },
            "mcp",
            wait,
            Some(connection.into()),
            plan,
        )?;
        self.runtime
            .tickets
            .get_mut(&ticket_hash)
            .unwrap()
            .operation = Some(operation.clone());
        Ok(json!({"operationId":operation,"replayed":false}))
    }
    fn decide(&mut self, c: &Value) -> Result<Value> {
        let d: DecisionRequest = decoded(&c["body"])?;
        d.validate()?;
        let r = self
            .requests
            .get(&d.target.request_id)
            .cloned()
            .ok_or_else(|| err(ErrorCode::NotFound, "Request not found"))?;
        if d.target.app_epoch != self.epoch {
            return Err(err(ErrorCode::RequestExpired, "CodeCraft restarted"));
        }
        if d.target != r.target {
            return Err(err(
                ErrorCode::RequestConflict,
                "Request version changed; refresh before submitting",
            ));
        }
        self.current(&r)?;
        if matches!(d.action, Action::Cancel { .. }) {
            if terminal(&r.state) {
                return Err(err(ErrorCode::RequestConflict, "Request already finished"));
            }
            let r = self.requests.get_mut(&d.target.request_id).unwrap();
            r.transition("cancelled");
            r.error = Some(err(ErrorCode::Cancelled, "Cancelled by user"));
            return Ok(
                json!({"schemaVersion":1,"requestId":r.target.request_id,"decisionId":d.decision_id,"accepted":true,"state":"cancelled","replayed":false}),
            );
        }
        if r.state != "pending" {
            return Err(err(
                ErrorCode::RequestConflict,
                "Another client already decided this request",
            ));
        }
        match &d.action {
            Action::Question { answers } if r.kind == "question" => {
                decoded::<QuestionInput>(&r.arguments)?.validate_answers(answers)?
            }
            Action::Permission { .. } if r.kind == "permission" => (),
            Action::Plan {
                revision,
                content_hash,
                ..
            } if r.kind == "plan" => {
                let t = &self.tasks[r.target.task_id.as_ref().unwrap()];
                self.plan_current(t)?;
                if *revision != t.revision || content_hash != &t.content_hash {
                    return Err(err(ErrorCode::PlanChanged, "Plan version changed"));
                }
            }
            _ => return Err(invalid("Decision kind does not match the request")),
        };
        let r = self.requests.get_mut(&d.target.request_id).unwrap();
        r.decision = Some(d.action);
        r.transition("user_decided");
        Ok(
            json!({"schemaVersion":1,"requestId":r.target.request_id,"decisionId":d.decision_id,"accepted":true,"state":"user_decided","replayed":false}),
        )
    }
    fn prepare(&mut self, c: &Value) -> Result<Value> {
        let op = text(c, "operation")?;
        let r = self
            .requests
            .get(op)
            .cloned()
            .ok_or_else(|| err(ErrorCode::NotFound, "Request not found"))?;
        self.current(&r)?;
        if r.channel == "mcp" && r.connection.as_deref() != c["connection"].as_str() {
            return Err(err(ErrorCode::TicketInvalid, "MCP connection mismatch"));
        }
        if r.state == "delivered" && r.channel == "mcp" {
            if !self.live(&format!("cache:{op}")) {
                return Err(err(ErrorCode::RequestExpired, "Result cache expired"));
            }
            return Ok(json!({"state":"delivered","result":r.result}));
        }
        if r.state != "user_decided" && r.state != "delivery_prepared" {
            return Err(r.error.unwrap_or_else(|| {
                err(
                    ErrorCode::RequestConflict,
                    "Request has no accepted decision",
                )
            }));
        }
        let decision = r
            .decision
            .clone()
            .ok_or_else(|| err(ErrorCode::RequestConflict, "No decision"))?;
        match &decision {
            Action::Permission { decision, .. } if *decision != PermissionDecision::Deny => {
                self.gate(&r.target.session_key, &r.tool_name, &r.arguments)?
            }
            Action::Plan { .. } => {
                let t = &self.tasks[r.target.task_id.as_ref().unwrap()];
                self.plan_current(t)?;
                if self.unresolved(&r.target.session_key) {
                    return Err(err(
                        ErrorCode::NativeConfirmationPending,
                        "Resolve previously delivered tool calls in Trae first",
                    ));
                }
            }
            _ => (),
        }
        let lease = if let Some(lease) = &r.lease {
            if !self.live(&format!("lease:{op}")) {
                return Err(err(
                    ErrorCode::DeliveryUnconfirmed,
                    "Delivery lease expired",
                ));
            }
            lease.clone()
        } else {
            id()
        };
        if r.lease.is_none() {
            self.deadline(format!("lease:{op}"), 5);
            if let Action::Permission { decision, .. } = &decision {
                if *decision != PermissionDecision::Deny {
                    if self
                        .grants
                        .values()
                        .filter(|g| g.resolution.is_none())
                        .count()
                        >= 128
                        || self
                            .grants
                            .values()
                            .filter(|g| {
                                g.resolution.is_none() && g.session_key == r.target.session_key
                            })
                            .count()
                            >= 16
                    {
                        return Err(err(ErrorCode::QueueFull, "Too many unresolved tool grants"));
                    }
                    let task = r.target.task_id.as_ref().and_then(|id| self.tasks.get(id));
                    self.grants.insert(
                        op.into(),
                        Grant {
                            grant_id: op.into(),
                            version: 1,
                            session_key: r.target.session_key.clone(),
                            task_id: r.target.task_id.clone(),
                            turn_epoch: r.target.turn_epoch,
                            revision: task.map(|t| t.revision).unwrap_or(0),
                            content_hash: task.map(|t| t.content_hash.clone()).unwrap_or_default(),
                            parameter_hash: r.parameter_hash.clone(),
                            tool_use_id: r.tool_use_id.clone(),
                            tool_name: r.tool_name.clone(),
                            state: "delivery_prepared".into(),
                            resolution: None,
                            closed_at: None,
                        },
                    );
                }
            }
        }
        let r = self.requests.get_mut(op).unwrap();
        r.lease = Some(lease.clone());
        r.transition("delivery_prepared");
        let payload = match decision {
            Action::Permission { decision, message } => {
                let d = match decision {
                    PermissionDecision::Allow => "allow",
                    PermissionDecision::Deny => "deny",
                    PermissionDecision::Ask => "ask",
                };
                return Ok(
                    json!({"state":r.state,"lease":lease,"output":wire::permission(d,message.as_deref().unwrap_or("Decided in CodeCraft"),None)}),
                );
            }
            Action::Question { answers } => json!({"kind":"question","answers":answers}),
            Action::Plan {
                decision,
                revision,
                content_hash,
                feedback,
            } => {
                json!({"kind":"plan","planId":r.plan.as_ref().unwrap()["planId"],"revision":revision,"contentHash":content_hash,"decision":decision,"feedback":feedback})
            }
            _ => return Err(err(ErrorCode::Cancelled, "Cancelled")),
        };
        let result = ToolResult {
            schema_version: 1,
            operation_id: Some(op.into()),
            request_id: Some(op.into()),
            status: "completed".into(),
            payload: Some(payload),
            error: None,
        };
        r.result = Some(result.clone());
        Ok(json!({"state":r.state,"lease":lease,"result":result}))
    }
    fn ack(&mut self, c: &Value) -> Result<Value> {
        let op = text(c, "operation")?;
        let r = self
            .requests
            .get(op)
            .cloned()
            .ok_or_else(|| err(ErrorCode::NotFound, "Request not found"))?;
        self.current(&r)?;
        if r.connection.is_some() && r.connection.as_deref() != c["connection"].as_str() {
            return Err(err(ErrorCode::TicketInvalid, "Connection mismatch"));
        }
        if r.lease.as_deref() != c["lease"].as_str()
            || !self.live(&format!("lease:{op}"))
            || r.state != "delivery_prepared"
        {
            return Err(err(
                ErrorCode::DeliveryUnconfirmed,
                "Delivery lease is invalid",
            ));
        }
        if let Some(Action::Permission { decision, .. }) = &r.decision {
            if *decision != PermissionDecision::Deny {
                self.gate(&r.target.session_key, &r.tool_name, &r.arguments)?;
            }
        }
        if let Some(Action::Plan { .. }) = &r.decision {
            self.plan_current(&self.tasks[r.target.task_id.as_ref().unwrap()])?;
        }
        if let Some(Action::Plan {
            decision: PlanDecision::Approved,
            ..
        }) = &r.decision
        {
            let task = r.target.task_id.as_ref().unwrap();
            self.plan_current(&self.tasks[task])?;
            if self.unresolved(&r.target.session_key) {
                return Err(err(
                    ErrorCode::NativeConfirmationPending,
                    "An earlier tool grant is unresolved",
                ));
            }
            let t = self.tasks.get_mut(task).unwrap();
            if matches!(t.state.as_str(), "paused" | "ended") || t.turn_epoch != r.target.turn_epoch
            {
                return Err(err(ErrorCode::TaskChanged, "Task changed before delivery"));
            }
            t.approved = true;
            t.state = "approved".into();
            t.version = t.version.saturating_add(1).min(MAX_VERSION);
            self.deadline(format!("approval:{task}"), 1800);
        }
        self.requests.get_mut(op).unwrap().transition("delivered");
        if let Some(g) = self.grants.get_mut(op) {
            g.state = "delivered".into();
            g.version = g.version.saturating_add(1).min(MAX_VERSION);
        }
        self.deadline(format!("cache:{op}"), 900);
        self.sessions.get_mut(&r.target.session_key).unwrap().status = "working".into();
        Ok(json!({"state":"delivered"}))
    }
    fn create_task(&mut self, c: &Value) -> Result<Value> {
        let req: CreateTask = decoded(&c["body"])?;
        check_version(req.schema_version)?;
        check_id(&req.control_id)?;
        check_text(&req.title, 200, true)?;
        if !self.capabilities.mcp_plan_review || self.capabilities.mcp_wait().is_none() {
            return Err(err(
                ErrorCode::UnsupportedVersion,
                "Protected tasks await Trae MCP runtime verification",
            ));
        }
        if self.tasks.len() >= 128 {
            return Err(err(ErrorCode::QueueFull, "Too many retained tasks"));
        }
        let roots = files::roots(&req.primary_root, &req.workspace_roots)?;
        let root = files::canonical_directory(&req.primary_root)?;
        let task = id();
        let plan = id();
        let directory = root.join(".trae/documents/codecraft").join(&task);
        fs::create_dir_all(&directory)?;
        if !directory.canonicalize()?.starts_with(&root) {
            return Err(invalid("Plan directory escapes the selected root"));
        }
        let path = directory.join("plan.md");
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)?;
        writeln!(file, "# {}\n", req.title.trim())?;
        file.sync_all()?;
        let code = secret()?;
        self.tasks.insert(
            task.clone(),
            Task {
                task_id: task.clone(),
                plan_id: plan,
                version: 1,
                title: req.title.trim().into(),
                primary_root: root.to_string_lossy().into(),
                workspace_roots: roots,
                document_path: path.to_string_lossy().into(),
                state: "armed".into(),
                session_key: None,
                turn_epoch: 0,
                revision: 0,
                content_hash: String::new(),
                file_identity: files::plan_identity(&path.to_string_lossy())?,
                approved: false,
                expires_at: stamp(86400),
                closed_at: None,
                launch_hash: digest(&json!(code))?,
            },
        );
        self.deadline(format!("launch:{task}"), 600);
        self.deadline(format!("task:{task}"), 86400);
        Ok(
            json!({"schemaVersion":1,"taskId":task,"taskVersion":1,"state":"armed","launchPrompt":format!("[[CODECRAFT_TASK:{code}]]\n{}",req.title.trim()),"expiresAt":stamp(600)}),
        )
    }
    fn task_action(&mut self, c: &Value) -> Result<Value> {
        let req: TaskAction = decoded(&c["body"])?;
        check_version(req.schema_version)?;
        check_id(&req.control_id)?;
        check_counter(req.expected_version)?;
        let t = self
            .tasks
            .get(&req.task_id)
            .cloned()
            .ok_or_else(|| err(ErrorCode::NotFound, "Task not found"))?;
        if t.version != req.expected_version {
            return Err(err(ErrorCode::TaskChanged, "Task version changed"));
        }
        if t.version >= MAX_VERSION {
            return Err(err(ErrorCode::TaskChanged, "Task version exhausted"));
        }
        if t.state == "ended" {
            return Err(err(
                ErrorCode::TaskChanged,
                "Ended tasks cannot be reopened",
            ));
        }
        match req.action {
            TaskActionKind::Resume => {
                if !matches!(t.state.as_str(), "paused" | "armed")
                    || !self.capabilities.mcp_plan_review
                    || self.capabilities.mcp_wait().is_none()
                {
                    return Err(err(
                        ErrorCode::TaskChanged,
                        "Only paused or unbound tasks can resume with verified capabilities",
                    ));
                }
                files::roots(&t.primary_root, &t.workspace_roots)?;
                files::plan(&t.document_path, &t.primary_root)?;
                if t.session_key.as_deref().is_some_and(|s| self.unresolved(s)) {
                    return Err(err(
                        ErrorCode::NativeConfirmationPending,
                        "Resolve prior tool calls before resuming",
                    ));
                }
            }
            TaskActionKind::RevokePlan if matches!(t.state.as_str(), "paused" | "armed") => {
                return Err(err(
                    ErrorCode::TaskChanged,
                    "Inactive tasks cannot reopen through plan revocation",
                ))
            }
            _ => (),
        }
        self.revoke(&req.task_id);
        for r in self
            .requests
            .values_mut()
            .filter(|r| r.target.task_id.as_ref() == Some(&req.task_id) && !terminal(&r.state))
        {
            r.transition("cancelled");
            r.error = Some(err(ErrorCode::TaskChanged, "Task changed"));
        }
        let mut launch = None;
        let state = match req.action {
            TaskActionKind::Pause => "paused",
            TaskActionKind::End => "ended",
            TaskActionKind::RevokePlan => "awaiting_plan",
            TaskActionKind::Resume => {
                if t.session_key.as_deref().is_some_and(|s| self.unresolved(s)) {
                    return Err(err(
                        ErrorCode::NativeConfirmationPending,
                        "Resolve prior tool calls before resuming",
                    ));
                }
                let code = secret()?;
                let task = self.tasks.get_mut(&req.task_id).unwrap();
                task.launch_hash = digest(&json!(code))?;
                task.expires_at = stamp(86400);
                launch = Some(format!("[[CODECRAFT_TASK:{code}]]\n{}", task.title));
                self.deadline(format!("launch:{}", req.task_id), 600);
                self.deadline(format!("task:{}", req.task_id), 86400);
                "armed"
            }
        };
        let t = self.tasks.get_mut(&req.task_id).unwrap();
        t.state = state.into();
        t.closed_at = if state == "ended" {
            Some(stamp(0))
        } else {
            None
        };
        t.version = t.version.saturating_add(1).min(MAX_VERSION);
        let mut reply =
            json!({"schemaVersion":1,"taskId":t.task_id,"taskVersion":t.version,"state":state});
        if let Some(prompt) = launch {
            reply["launchPrompt"] = json!(prompt);
            reply["expiresAt"] = json!(stamp(600));
        }
        Ok(reply)
    }
    pub fn apply(&mut self, c: &Value) -> Result<Value> {
        let kind = text(c, "kind")?;
        let control = c["body"]
            .get("decisionId")
            .or_else(|| c["body"].get("controlId"))
            .and_then(Value::as_str);
        let hash = digest(&c["body"])?;
        if let Some(id) = control {
            check_id(id)?;
            if let Some(entry) = self.ledger.get(id) {
                if entry.hash != hash {
                    return Err(err(
                        ErrorCode::IdempotencyConflict,
                        "Decision/control ID reused with different content",
                    ));
                }
                if entry.epoch != self.epoch {
                    return Err(err(
                        ErrorCode::RequestExpired,
                        "Control belongs to an earlier CodeCraft process",
                    ));
                }
                if let Some(result) = self.runtime.controls.get(id) {
                    let mut result = result.clone();
                    if result.get("accepted").is_some() {
                        result["replayed"] = json!(true);
                    }
                    return Ok(result);
                }
                return Err(err(ErrorCode::RequestExpired, "Control result expired"));
            }
        }
        if control.is_some() && self.ledger.len() >= 65536 {
            return Err(err(ErrorCode::QueueFull, "Control history is full"));
        }
        let outcome = match kind {
            "refresh_capabilities" => {
                let status = crate::hook::status()?;
                let enabled = c["suspend"] != true
                    && status["enabled"] == true
                    && status["filesInstalled"] == true;
                let capabilities = if enabled {
                    Capabilities::bundled(status["environment"]["productVersion"].as_str())
                } else {
                    Capabilities {
                        reason: "Trae integration is disabled".into(),
                        ..Capabilities::default()
                    }
                };
                self.refresh_capabilities(capabilities)?;
                Ok(json!({"enabled":enabled}))
            }
            "hook" => self.hook(c),
            "consume" => self.consume(c),
            "decision" => self.decide(c),
            "prepare" => self.prepare(c),
            "ack" => self.ack(c),
            "create_task" => self.create_task(c),
            "task_action" => self.task_action(c),
            "poll" => {
                let r = self
                    .requests
                    .get(text(c, "operation")?)
                    .ok_or_else(|| err(ErrorCode::NotFound, "Request not found"))?;
                self.current(r)?;
                if r.connection.is_some() && r.connection.as_deref() != c["connection"].as_str() {
                    return Err(err(ErrorCode::TicketInvalid, "Connection mismatch"));
                }
                if let Some(e) = &r.error {
                    return Err(e.clone());
                }
                Ok(json!({"state":r.state}))
            }
            "cancel" => {
                let op = text(c, "operation")?;
                let r = self
                    .requests
                    .get_mut(op)
                    .ok_or_else(|| err(ErrorCode::NotFound, "Request not found"))?;
                if r.connection.as_deref() != c["connection"].as_str() {
                    return Err(err(ErrorCode::TicketInvalid, "Connection mismatch"));
                }
                if !terminal(&r.state) {
                    r.transition("cancelled");
                    r.error = Some(err(ErrorCode::Cancelled, "MCP request cancelled"));
                }
                Ok(json!({"state":r.state}))
            }
            "resolve_grant" => {
                let req: ResolveGrant = decoded(&c["body"])?;
                check_version(req.schema_version)?;
                check_id(&req.control_id)?;
                let g = self
                    .grants
                    .get_mut(&req.grant_id)
                    .ok_or_else(|| err(ErrorCode::NotFound, "Grant not found"))?;
                if g.version != req.expected_version || g.resolution.is_some() {
                    return Err(err(ErrorCode::RequestConflict, "Grant already changed"));
                }
                g.resolution = Some(
                    match req.action {
                        GrantResolution::ConfirmedCancelledInTrae => "user_confirmed_cancelled",
                        GrantResolution::ConfirmedCompletedInTrae => "user_confirmed_completed",
                    }
                    .into(),
                );
                g.version = g.version.saturating_add(1).min(MAX_VERSION);
                g.closed_at = Some(stamp(0));
                Ok(
                    json!({"schemaVersion":1,"grantId":g.grant_id,"grantVersion":g.version,"resolution":g.resolution}),
                )
            }
            _ => Err(invalid("Unknown bridge command")),
        };
        if let (Some(control), Ok(reply)) = (control, &outcome) {
            if self.ledger.len() >= 65536 {
                return Err(err(ErrorCode::QueueFull, "Control history is full"));
            }
            self.ledger.insert(
                control.into(),
                Ledger {
                    hash,
                    epoch: self.epoch.clone(),
                },
            );
            self.runtime.controls.insert(control.into(), reply.clone());
            self.deadline(format!("control:{control}"), 900);
        }
        if kind != "poll" {
            self.version = self.version.saturating_add(1);
        }
        outcome
    }
    fn refresh_capabilities(&mut self, capabilities: Capabilities) -> Result<()> {
        // A configuration change cannot keep bindings or tickets from the old host.
        self.capabilities = capabilities;
        let keys = self.sessions.keys().cloned().collect::<Vec<_>>();
        for key in keys {
            self.invalidate_session(&key)?;
        }
        self.runtime.tickets.clear();
        for task in self.tasks.values_mut() {
            if task.state != "ended" {
                task.state = "paused".into();
                task.version = task.version.saturating_add(1).min(MAX_VERSION);
            }
        }
        Ok(())
    }
    pub fn tick(&mut self) -> bool {
        let mut changed = false;
        let task_ids = self.tasks.keys().cloned().collect::<Vec<_>>();
        for task in task_ids {
            let t = &self.tasks[&task];
            let expired = !matches!(t.state.as_str(), "paused" | "ended")
                && !self.live(&format!("task:{task}"));
            let revoke = t.approved
                && (!self.live(&format!("approval:{task}")) || self.plan_current(t).is_err());
            if expired || revoke {
                self.revoke(&task);
                if expired {
                    self.tasks.get_mut(&task).unwrap().state = "paused".into();
                }
                changed = true;
            }
        }
        let ids = self.requests.keys().cloned().collect::<Vec<_>>();
        for id in ids {
            let r = &self.requests[&id];
            if terminal(&r.state) {
                continue;
            }
            let reason = if !self.live(&format!("request:{id}")) {
                Some(err(ErrorCode::RequestExpired, "Interaction timed out"))
            } else if r.state == "delivery_prepared" && !self.live(&format!("lease:{id}")) {
                Some(err(
                    ErrorCode::DeliveryUnconfirmed,
                    "Delivery confirmation was lost",
                ))
            } else if let Some(task) = &r.target.task_id {
                if matches!(self.tasks[task].state.as_str(), "paused" | "ended") {
                    Some(err(ErrorCode::TaskChanged, "Task is not active"))
                } else if r.kind == "plan" {
                    self.plan_current(&self.tasks[task]).err()
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(e) = reason {
                let r = self.requests.get_mut(&id).unwrap();
                r.transition(if e.error.code == ErrorCode::RequestExpired {
                    "expired"
                } else {
                    "failed"
                });
                r.error = Some(e);
                changed = true;
            }
        }
        let now = chrono::Utc::now();
        let old = self.requests.len();
        self.requests.retain(|id, r| {
            !terminal(&r.state)
                || self.grants.get(id).is_some_and(|g| g.resolution.is_none())
                || chrono::DateTime::parse_from_rfc3339(&r.expires_at)
                    .ok()
                    .is_some_and(|at| now.signed_duration_since(at).num_seconds() < 86400)
        });
        changed |= old != self.requests.len();
        let deadlines = &self.runtime.deadlines;
        self.runtime.controls.retain(|id, _| {
            deadlines
                .get(&format!("control:{id}"))
                .is_some_and(|at| *at > Instant::now())
        });
        self.runtime.tickets.retain(|id, ticket| {
            ticket
                .operation
                .as_ref()
                .is_some_and(|op| self.requests.contains_key(op))
                || deadlines
                    .get(&format!("ticket:{id}"))
                    .is_some_and(|at| *at > Instant::now())
        });
        for (id, r) in &mut self.requests {
            if terminal(&r.state)
                && !deadlines
                    .get(&format!("history:{id}"))
                    .is_some_and(|at| *at > Instant::now())
                && (!r.arguments.is_null() || r.result.is_some())
            {
                r.arguments = Value::Null;
                r.result = None;
                r.plan = None;
                changed = true;
            }
        }
        self.runtime.deadlines.retain(|_, at| *at > Instant::now());
        if changed {
            self.version += 1;
        }
        changed
    }
    fn cleanup_history(&mut self) -> Result<bool> {
        if self
            .runtime
            .last_cleanup
            .is_some_and(|at| at.elapsed() < Duration::from_secs(60))
        {
            return Ok(false);
        }
        self.runtime.last_cleanup = Some(Instant::now());
        let old_counts = (self.sessions.len(), self.tasks.len(), self.grants.len());
        let old = |stamp: Option<&str>| {
            stamp
                .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
                .is_some_and(|v| chrono::Utc::now().signed_duration_since(v).num_seconds() >= 86400)
        };
        self.grants
            .retain(|_, g| g.resolution.is_none() || !old(g.closed_at.as_deref()));
        self.tasks.retain(|id, t| {
            t.state != "ended"
                || !old(t.closed_at.as_deref())
                || self
                    .sessions
                    .values()
                    .any(|s| s.task_id.as_ref() == Some(id))
                || self
                    .grants
                    .values()
                    .any(|g| g.task_id.as_ref() == Some(id) && g.resolution.is_none())
                || self
                    .requests
                    .values()
                    .any(|r| r.target.task_id.as_ref() == Some(id))
        });
        self.sessions.retain(|id, s| {
            !old(Some(&s.updated_at))
                || s.task_id
                    .as_ref()
                    .is_some_and(|id| self.tasks.contains_key(id))
                || self.requests.values().any(|r| &r.target.session_key == id)
                || self.grants.values().any(|g| &g.session_key == id)
        });
        // Only generated files under the bridge root are eligible; workspace
        // plan files and unresolved grants are never removed by TTL cleanup.
        for name in ["replies", "journal", "inbox", "processing"] {
            for entry in fs::read_dir(self.root.join(name))? {
                let entry = entry?;
                let path = entry.path();
                let metadata = entry.metadata()?;
                if !metadata.is_file()
                    || metadata.modified()?.elapsed().unwrap_or_default()
                        < Duration::from_secs(if name == "journal" { 86400 } else { 900 })
                {
                    continue;
                }
                let generated = path
                    .file_stem()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| check_id(n).is_ok());
                if (generated
                    && (name == "replies" || path.extension().is_some_and(|e| e == "tmp")))
                    || (name == "journal" && path.extension().is_some_and(|e| e == "jsonl"))
                {
                    fs::remove_file(path)?;
                }
            }
        }
        for entry in fs::read_dir(self.root.join("tasks"))? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if check_id(&name).is_ok()
                && !self.tasks.contains_key(&name)
                && entry.metadata()?.modified()?.elapsed().unwrap_or_default()
                    >= Duration::from_secs(86400)
            {
                // Reject reparse points before recursive cleanup.
                let path = entry.path();
                let final_path = path.canonicalize()?;
                if final_path == self.root.join("tasks").canonicalize()?.join(&name)
                    && fs::symlink_metadata(&path)?.file_type().is_dir()
                {
                    fs::remove_dir_all(path)?;
                }
            }
        }
        self.runtime
            .plan_files
            .retain(|key, _| self.sessions.contains_key(key));
        let changed = old_counts != (self.sessions.len(), self.tasks.len(), self.grants.len());
        if changed {
            self.version = self.version.saturating_add(1);
        }
        Ok(changed)
    }
    pub fn snapshot(&self) -> Value {
        let mut tasks =
            serde_json::to_value(self.tasks.values().collect::<Vec<_>>()).unwrap_or(json!([]));
        for t in tasks.as_array_mut().unwrap() {
            t.as_object_mut().unwrap().remove("launchHash");
        }
        let requests=self.requests.values().filter(|r| !terminal(&r.state) || !r.arguments.is_null()).map(|r|json!({"target":r.target,"kind":r.kind,"channel":r.channel,"toolUseId":r.tool_use_id,"toolName":r.tool_name,"arguments":r.arguments,"state":r.state,"expiresAt":r.expires_at,"plan":r.plan,"error":r.error})).collect::<Vec<_>>();
        let mut snapshot = json!({"source":"trae","appEpoch":self.epoch,"version":self.version,"connected":true,"capabilities":self.capabilities,"sessions":self.sessions.values().collect::<Vec<_>>(),"requests":requests,"tasks":tasks,"grants":self.grants.values().collect::<Vec<_>>()});
        redact(&mut snapshot);
        snapshot
    }
    fn recovered(mut self, root: PathBuf, caps: Capabilities) -> Self {
        self.root = root;
        self.epoch = id();
        self.capabilities = caps;
        self.runtime = Runtime::default();
        for session in self.sessions.values_mut() {
            session.native_interactions.clear();
            session.native_notice = None;
            session.finish_activities();
        }
        for t in self.tasks.values_mut() {
            t.approved = false;
            if t.state != "ended" {
                t.state = "paused".into();
            }
            t.version = t.version.saturating_add(1).min(MAX_VERSION);
        }
        for r in self.requests.values_mut() {
            if !terminal(&r.state) {
                r.transition("expired");
                r.error = Some(err(ErrorCode::RequestExpired, "CodeCraft restarted"));
            }
            r.result = None;
            r.lease = None;
        }
        self.version += 1;
        self
    }
}

static SNAPSHOT: OnceLock<Arc<Mutex<Value>>> = OnceLock::new();

pub fn snapshot() -> Value {
    SNAPSHOT.get().and_then(|s|s.lock().ok().map(|s|s.clone())).unwrap_or_else(||json!({"source":"trae","connected":false,"version":0,"sessions":[],"requests":[],"tasks":[],"grants":[],"capabilities":Capabilities::default()}))
}
fn publish(store: &Store) -> Result<()> {
    if let Some(slot) = SNAPSHOT.get() {
        *slot
            .lock()
            .map_err(|_| err(ErrorCode::StateUnavailable, "Snapshot lock poisoned"))? =
            store.snapshot();
    }
    let bindings = store
        .sessions
        .values()
        .map(|s| {
            (
                s.session_key.clone(),
                json!({"protected":s.task_id.is_some()}),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    files::atomic(
        &store.root.join("bindings.json"),
        &json!({"appEpoch":store.epoch,"sessions":bindings}),
        true,
    )
}
pub fn start(caps: Capabilities) -> Result<()> {
    let root = files::root();
    start_at(root, caps)
}
pub fn start_at(root: PathBuf, caps: Capabilities) -> Result<()> {
    files::initialize(&root)?;
    let lock = files::lock(&root.join("store.lock"))?;
    let state = recover_state(&root)?
        .map(|s| s.recovered(root.clone(), caps.clone()))
        .unwrap_or_else(|| Store::new(root.clone(), caps));
    let slot = Arc::new(Mutex::new(state.snapshot()));
    if SNAPSHOT.set(slot).is_err() {
        return Ok(());
    }
    std::thread::Builder::new()
        .name("trae-store".into())
        .spawn(move || {
            let _lock = lock;
            let result = coordinate(state);
            if let Err(error) = result {
                if let Some(snapshot) = SNAPSHOT.get() {
                    if let Ok(mut s) = snapshot.lock() {
                        s["connected"] = json!(false);
                        s["integrationError"] = json!(error);
                    }
                }
                let _ = fs::remove_file(root.join("heartbeat.json"));
            }
        })?;
    Ok(())
}
fn redact(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for key in ["bridgeTicket", "launchPrompt", "launchHash", "lease"] {
                object.remove(key);
            }
            for item in object.values_mut() {
                redact(item);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(redact),
        _ => (),
    }
}
fn redacted_activity(value: &Value) -> Value {
    let mut value = value.clone();
    redact(&mut value);
    let text = if let Some(s) = value.as_str() {
        s.to_owned()
    } else {
        value.to_string()
    };
    let mut excerpt: String = text.chars().take(8192).collect();
    if excerpt.len() < text.len() {
        excerpt.push_str("\n[内容已截断]");
    }
    json!(excerpt)
}
fn recover_state(root: &Path) -> Result<Option<Store>> {
    // This checkpoint is durable before replies become visible. Recovery never
    // restores runtime leases, tickets or approvals from an earlier app epoch.
    let checkpoint = root.join("journal/committed.json");
    let state = root.join("state.json");
    if checkpoint.exists() {
        return decoded(&files::read(&checkpoint, 256 * 1024 * 1024)?).map(Some);
    }
    if state.exists() {
        return decoded(&files::read(&state, 256 * 1024 * 1024)?).map(Some);
    }
    Ok(None)
}
fn commit(store: &Store, command: &str, reply: &Value) -> Result<()> {
    let journal = store
        .root
        .join("journal")
        .join(format!("{}.jsonl", chrono::Utc::now().format("%Y-%m-%d")));
    let mut file = OpenOptions::new().create(true).append(true).open(journal)?;
    let mut safe_reply = reply.clone();
    redact(&mut safe_reply);
    writeln!(
        file,
        "{}",
        json!({"appEpoch":store.epoch,"commandId":command,"version":store.version,"at":stamp(0),"reply":safe_reply})
    )?;
    file.sync_all()?;
    let state = serde_json::to_value(store).map_err(|e| invalid(e.to_string()))?;
    files::atomic(&store.root.join("journal/committed.json"), &state, true)?;
    files::atomic(&store.root.join("state.json"), &state, true)?;
    Ok(())
}
fn coordinate(mut store: Store) -> Result<()> {
    let mut heartbeat = Instant::now() - Duration::from_secs(3);
    let mut cache = HashMap::<String, (String, Option<Value>, Instant)>::new();
    commit(&store, "startup", &json!({}))?;
    publish(&store)?;
    let _hook_bridge = crate::transport::start(&store.root, &store.epoch)?;
    loop {
        if store.cleanup_history()? {
            commit(&store, "cleanup", &json!({}))?;
            publish(&store)?;
        }
        if store.tick() {
            commit(&store, "expiry", &json!({}))?;
            publish(&store)?;
        }
        if heartbeat.elapsed() >= Duration::from_secs(2) {
            files::atomic(
                &store.root.join("heartbeat.json"),
                &json!({"appEpoch":store.epoch,"updatedAt":chrono::Utc::now().timestamp_millis(),"healthy":true}),
                true,
            )?;
            heartbeat = Instant::now();
        }
        for directory in ["processing", "inbox"] {
            let paths = fs::read_dir(store.root.join(directory))?
                .filter_map(std::result::Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "json"))
                .take(128)
                .collect::<Vec<_>>();
            for path in paths {
                let owned = store
                    .root
                    .join("processing")
                    .join(path.file_name().unwrap());
                if directory == "inbox" {
                    fs::rename(path, &owned)?;
                }
                let message = files::read(&owned, wire::MAX_IPC);
                let message = match message {
                    Ok(v) => v,
                    Err(_) => {
                        fs::remove_file(owned)?;
                        continue;
                    }
                };
                let command_id = match message["commandId"]
                    .as_str()
                    .filter(|v| check_id(v).is_ok())
                {
                    Some(v) => v,
                    None => {
                        fs::remove_file(owned)?;
                        continue;
                    }
                };
                let hash = digest(&message["command"])?;
                let reply_path = store
                    .root
                    .join("replies")
                    .join(format!("{command_id}.json"));
                if let Some((old, reply, _)) = cache.get(command_id) {
                    if !reply_path.exists() {
                        let expired = json!({"appEpoch":store.epoch,"bodyHash":hash,"error":if *old == hash {err(ErrorCode::RequestExpired,"Command result expired")}else{err(ErrorCode::IdempotencyConflict,"Command ID reused")},"result":null});
                        files::atomic(
                            &reply_path,
                            if *old == hash {
                                reply.as_ref().unwrap_or(&expired)
                            } else {
                                &expired
                            },
                            false,
                        )?;
                    }
                } else {
                    let mut next = store.clone();
                    let result = if message["appEpoch"] != store.epoch {
                        Err(err(ErrorCode::RequestExpired, "Previous CodeCraft epoch"))
                    } else {
                        if cache.len()
                            >= if files::control_message(&message["command"]) {
                                65536
                            } else {
                                64512
                            }
                            && message["command"]["kind"] != "poll"
                        {
                            Err(err(
                                ErrorCode::QueueFull,
                                "Command history is full; restart CodeCraft",
                            ))
                        } else {
                            next.apply(&message["command"])
                        }
                    };
                    let reply = json!({"appEpoch":store.epoch,"bodyHash":hash,"result":result.as_ref().ok(),"error":result.as_ref().err()});
                    if message["command"]["kind"] != "poll" {
                        commit(&next, command_id, &reply)?;
                    }
                    store = next;
                    if message["command"]["kind"] != "poll" {
                        publish(&store)?;
                    }
                    if !reply_path.exists() {
                        files::atomic(&reply_path, &reply, false)?;
                    }
                    if message["command"]["kind"] != "poll" && cache.len() < 65536 {
                        cache.insert(command_id.into(), (hash, Some(reply), Instant::now()));
                    }
                }
                fs::remove_file(owned)?;
            }
        }
        for (id, (_, reply, at)) in &mut cache {
            if at.elapsed() >= Duration::from_secs(900) && reply.is_some() {
                *reply = None;
                let _ = fs::remove_file(store.root.join("replies").join(format!("{id}.json")));
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    #[test]
    fn configuration_refresh_expires_existing_requests_even_when_capabilities_stay_enabled() {
        let mut store = Store::new(PathBuf::new(), Capabilities::default());
        store.sessions.insert(
            "s".into(),
            Session {
                session_key: "s".into(),
                ..Session::default()
            },
        );
        let op = store
            .request(
                "s",
                "call",
                "RunCommand",
                json!({}),
                "permission",
                "hook",
                120,
                None,
                None,
            )
            .unwrap();
        store.refresh_capabilities(Capabilities::default()).unwrap();
        assert_eq!(store.requests[&op].state, "cancelled");
        assert_eq!(store.requests[&op].target.request_version, 2);
        assert_eq!(store.sessions["s"].turn_epoch, 1);
    }
    #[test]
    fn checkpoint_recovers_possible_grants_when_state_file_is_stale() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/trae-tests")
            .join(id());
        files::initialize(&root).unwrap();
        let mut store = Store::new(root.clone(), Capabilities::default());
        let original = serde_json::to_value(&store).unwrap();
        store.grants.insert(
            "grant".into(),
            Grant {
                grant_id: "grant".into(),
                version: 1,
                session_key: "session".into(),
                task_id: None,
                turn_epoch: 0,
                revision: 0,
                content_hash: String::new(),
                parameter_hash: String::new(),
                tool_use_id: "call".into(),
                tool_name: "Write".into(),
                state: "delivery_prepared".into(),
                resolution: None,
                closed_at: None,
            },
        );
        store.version += 1;
        commit(
            &store,
            "command",
            &json!({"result":{"output":{"bridgeTicket":"secret"}}}),
        )
        .unwrap();
        files::atomic(&root.join("state.json"), &original, true).unwrap();
        let recovered = recover_state(&root)
            .unwrap()
            .unwrap()
            .recovered(root.clone(), Capabilities::default());
        assert_ne!(store.epoch, recovered.epoch);
        assert!(recovered.grants["grant"].resolution.is_none());
        assert!(recovered.runtime.deadlines.is_empty());
        let log = fs::read_to_string(
            root.join("journal")
                .join(format!("{}.jsonl", chrono::Utc::now().format("%Y-%m-%d"))),
        )
        .unwrap();
        assert!(!log.contains("secret"));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn expired_lease_never_clears_possible_external_grant() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/trae-tests")
            .join(id());
        let mut store = Store::new(root, Capabilities::default());
        store.sessions.insert(
            "s".into(),
            Session {
                session_key: "s".into(),
                ..Session::default()
            },
        );
        let request = store
            .request(
                "s",
                "call",
                "RunCommand",
                json!({}),
                "permission",
                "hook",
                120,
                None,
                None,
            )
            .unwrap();
        let r = store.requests.get_mut(&request).unwrap();
        r.transition("user_decided");
        r.decision = Some(Action::Permission {
            decision: PermissionDecision::Allow,
            message: None,
        });
        let p = store.prepare(&json!({"operation":request})).unwrap();
        store.runtime.deadlines.insert(
            format!("lease:{request}"),
            Instant::now() - Duration::from_secs(1),
        );
        store.tick();
        assert_eq!(store.requests[&request].state, "failed");
        assert!(store.grants[&request].resolution.is_none());
        assert!(store
            .ack(&json!({"operation":request,"lease":p["lease"]}))
            .is_err());
        let recovered = store.recovered(PathBuf::new(), Capabilities::default());
        assert!(recovered.grants[&request].resolution.is_none());
    }
}
