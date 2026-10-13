//! One production broker, shared by desktop and authenticated LAN commands.
//! v5 interactions use explicit authenticated connections; native API and ticket
//! injection are never enabled by this adapter.
use crate::{
    files,
    history::{self, EventKind, Observation, Outcome},
    interaction, process, product, protocol,
    schema::{self, Result},
    store::Store,
    transport::{AdapterInput, Client, Heartbeat, Hub, Liveness, Permit, Role},
    workflow,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiError {
    pub schema_version: u32,
    pub error: ErrorBody,
}
#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
}
impl ApiError {
    pub fn new(code: &str) -> Self {
        let message = match code {
            "CURSOR_NOT_FOUND" | "CURSOR_EXECUTABLE_REQUIRED" | "CURSOR_PACKAGE_UNAVAILABLE" => {
                "未找到 Cursor，请安装或启动 Cursor 后重试。"
            }
            "CURSOR_WORKSPACES_UNAVAILABLE" => {
                "无法读取 Cursor 当前项目，请在 Cursor 中打开项目后重新安装 Hook。"
            }
            "RULE_CONFLICT" | "RULE_RECOVERY_CONFLICT" | "CURSOR_PROJECT_RULES_CHANGED" => {
                "Cursor 项目规则存在冲突，请检查 codecraft-workflow.mdc 后重新安装 Hook。"
            }
            "MCP_ENTRY_CONFLICT"
            | "HOOK_ENTRY_CONFLICT"
            | "INSTALL_FILE_CONFLICT"
            | "INSTALL_RECOVERY_CONFLICT" => {
                "Cursor 配置存在冲突，请核对当前配置和同目录的 .bak 备份后重试。"
            }
            "CAPABILITY_NOT_SUPPORTED" => product::REASON,
            "BROKER_OFFLINE" | "BROKER_UNAVAILABLE" => {
                "Cursor 采集服务未连接，请重新打开 CodeCraft。"
            }
            "FORBIDDEN" => "当前连接没有审批写入权限。",
            _ => code,
        };
        Self {
            schema_version: 1,
            error: ErrorBody {
                code: code.into(),
                message: message.into(),
            },
        }
    }
    pub fn status(&self) -> u16 {
        match self.error.code.as_str() {
            "FORBIDDEN" => 403,
            "CAPABILITY_NOT_SUPPORTED" => 409,
            "BROKER_OFFLINE" => 503,
            "INVALID_JSON" | "INVALID_SUBMISSION" | "INVALID_ROUTE" => 400,
            _ => 409,
        }
    }
}
impl From<&str> for ApiError {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Hello {
    schema_version: u32,
    app_epoch: String,
    nonce: String,
    process: process::Identity,
    #[serde(default = "hook_role")]
    role: Role,
}
fn hook_role() -> Role {
    Role::Hook
}

struct Connection {
    process: process::Identity,
    since: Instant,
    role: Role,
    rpc_ids: BTreeMap<String, String>,
}
struct Broker {
    root: PathBuf,
    home: PathBuf,
    config: product::Config,
    store: Store,
    history: history::History,
    hub: Hub,
    connections: BTreeMap<String, Connection>,
    started: Instant,
    beat: Instant,
    config_check: Instant,
    status: Value,
    snapshot: Value,
    version: u64,
    last_digest: String,
    last_observation: Option<u64>,
    published_history_version: u64,
    published_store_version: u64,
}
impl Drop for Broker {
    fn drop(&mut self) {
        // Revocation must also wake idle stdio helpers if reconfiguration fails
        // before a replacement broker is published.
        let path = self.root.join("broker.json");
        if files::read_json(&path).is_ok_and(|v| v["appEpoch"] == self.store.model().app_epoch()) {
            let _ = fs::remove_file(path);
        }
    }
}
struct Service {
    broker: Option<Broker>,
    error: Option<String>,
    last_snapshot: Value,
}
static SERVICE: OnceLock<Mutex<Service>> = OnceLock::new();
static STARTED: OnceLock<()> = OnceLock::new();
type Policy = fn(&str, &Value) -> bool;
static POLICY: OnceLock<Policy> = OnceLock::new();
pub fn set_policy(policy: Policy) {
    let _ = POLICY.set(policy);
}
fn wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn capabilities(enabled: bool) -> Value {
    json!({"hookOnly":enabled,"nativeShared":false,"askUser":enabled,"reviewPlan":enabled,"reason":product::REASON,
        "blockers":["NATIVE_SHARED_NOT_SUPPORTED"],"interactionMode":"independent"})
}
fn empty(error: Option<&str>) -> Value {
    json!({"schemaVersion":1,"source":"cursor","appEpoch":"","version":0,"captureScope":"observedOnly",
        "interactionProtocol":1,"connected":false,"integrationError":error,"capabilities":capabilities(false),"sessions":[],"requests":[],"hook":null})
}

impl Broker {
    fn open(root: &Path, home: &Path) -> Result<Self> {
        product::initialize(root)?;
        product::recover(root)?;
        let config = product::load(root)?;
        let store = Store::open(&root.join("workflow"), &config.installation_id)?;
        let epoch = store.model().app_epoch().to_owned();
        let history =
            history::History::open(&root.join("history"), &config.installation_id, &epoch)?;
        let hub = Hub::open(
            &root.join("channel"),
            &config.installation_id,
            &epoch,
            config.revision.max(1),
        )?;
        files::private_directory(&root.join("bootstrap"))?;
        let identity = process::identify(std::process::id())?;
        let status = product::status_at(root, home)?;
        let mut broker = Self {
            root: root.to_path_buf(),
            home: home.to_path_buf(),
            config,
            store,
            history,
            hub,
            connections: BTreeMap::new(),
            started: Instant::now(),
            beat: Instant::now(),
            config_check: Instant::now(),
            status,
            snapshot: empty(None),
            version: 0,
            last_digest: String::new(),
            last_observation: None,
            published_history_version: u64::MAX,
            published_store_version: u64::MAX,
        };
        broker.publish()?;
        files::atomic_json(
            &root.join("broker.json"),
            &json!({"schemaVersion":1,"appEpoch":epoch,"process":identity}),
            true,
        )?;
        Ok(broker)
    }
    fn now(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }
    fn handshake(&mut self) -> Result<()> {
        let root = self.root.join("bootstrap");
        let mut count = 0;
        for entry in fs::read_dir(&root)
            .map_err(|_| "BOOTSTRAP_UNAVAILABLE")?
            .take(129)
        {
            let entry = entry.map_err(|_| "BOOTSTRAP_UNAVAILABLE")?;
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.ends_with(".hello.json") && !name.ends_with(".permit.json") {
                continue;
            }
            count += 1;
            if count > 128 {
                return Err("BOOTSTRAP_BACKPRESSURE");
            }
            let path = files::child(&root, &name)?;
            let age = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok());
            if age.is_some_and(|a| a > Duration::from_secs(10)) {
                fs::remove_file(path).map_err(|_| "BOOTSTRAP_CLEANUP_FAILED")?;
                continue;
            }
            let Some(nonce) = name.strip_suffix(".hello.json") else {
                continue;
            };
            let result = (|| {
                let hello: Hello = serde_json::from_value(files::read_json(&path)?)
                    .map_err(|_| "INVALID_HANDSHAKE")?;
                if hello.schema_version != 1
                    || hello.nonce != nonce
                    || uuid::Uuid::parse_str(nonce).is_err()
                    || hello.app_epoch != self.store.model().app_epoch()
                    || !self.config.enabled
                    || self.status["filesInstalled"] != true
                    || hello.role == Role::Native
                {
                    return Err("INVALID_HANDSHAKE");
                }
                let actual = process::identify(hello.process.pid)?;
                let exe = self
                    .config
                    .executable
                    .as_ref()
                    .ok_or("INVALID_HANDSHAKE")?
                    .canonicalize()
                    .map_err(|_| "EXECUTABLE_UNAVAILABLE")?;
                let cursor = self
                    .config
                    .cursor_executable
                    .as_ref()
                    .ok_or("CURSOR_NOT_FOUND")?;
                if actual != hello.process
                    || actual.executable != exe
                    || !process::has_cursor_ancestor(&actual, cursor)
                    || !process::helper_role(&actual, hello.role)
                {
                    return Err("PROCESS_IDENTITY_MISMATCH");
                }
                let permit = self.hub.authorize(hello.role)?;
                let id = permit.connection_id().to_string();
                self.connections.insert(
                    id.clone(),
                    Connection {
                        process: actual,
                        since: Instant::now(),
                        role: hello.role,
                        rpc_ids: BTreeMap::new(),
                    },
                );
                if let Err(error) =
                    permit.write_bootstrap(&root.join(format!("{nonce}.permit.json")), nonce)
                {
                    let _ = self.hub.close(&id);
                    self.connections.remove(&id);
                    return Err(error);
                }
                Ok(())
            })();
            fs::remove_file(path).map_err(|_| "BOOTSTRAP_CLEANUP_FAILED")?;
            // Invalid/untrusted files do not stop healthy observations; no raw
            // input, PID claims or credentials enter diagnostic snapshots.
            if let Err(error) = result {
                if matches!(error, "CHANNEL_UNAVAILABLE" | "CHANNEL_PERSIST_FAILED") {
                    return Err(error);
                }
            }
        }
        Ok(())
    }
    fn tick(&mut self) -> Result<()> {
        if self.config_check.elapsed() >= Duration::from_secs(5) {
            self.status = product::status_at(&self.root, &self.home)?;
            self.config_check = Instant::now();
        }
        if self.beat.elapsed() >= Duration::from_secs(1) {
            self.hub.heartbeat()?;
            if self.store.model().request_count() > 0 {
                self.store.execute(workflow::Command::Tick)?;
            }
            self.history.tick(
                wall_ms(),
                self.now(),
                &self.store.model().interaction_pins(),
            )?;
            let dead = self
                .connections
                .iter()
                .filter(|(_, c)| {
                    (c.role == Role::Hook && c.since.elapsed() > Duration::from_secs(65))
                        || process::identify(c.process.pid).as_ref() != Ok(&c.process)
                })
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            for id in dead {
                self.store.execute(workflow::Command::Independent(
                    interaction::Command::Disconnect {
                        connection: id.clone(),
                    },
                ))?;
                self.hub.close(&id)?;
                self.connections.remove(&id);
            }
            self.beat = Instant::now();
        }
        self.handshake()?;
        for _ in 0..16 {
            let delivery = match self.hub.take() {
                Ok(Some(d)) => d,
                Ok(None) => break,
                Err(
                    "CONNECTION_UNKNOWN"
                    | "MESSAGE_EXPIRED"
                    | "INVALID_HANDSHAKE"
                    | "APP_RESTARTED"
                    | "CONNECTION_IDENTITY_MISMATCH"
                    | "MESSAGE_SEQUENCE_REPLAY"
                    | "MESSAGE_ALREADY_PROCESSED",
                ) => continue,
                Err(e) => return Err(e),
            };
            let connection = delivery.connection_id().to_owned();
            let result = match &delivery.input {
                AdapterInput::Hook(value) if value.get("op").is_some() && self.config.enabled => {
                    self.channel_operation(&connection, value, Role::Hook)
                }
                AdapterInput::Hook(value) if self.config.enabled => {
                    normalize(value).and_then(|events| {
                        let now = self.now();
                        for event in events {
                            self.history.observe(
                                event,
                                wall_ms(),
                                now,
                                &self.store.model().interaction_pins(),
                            )?;
                        }
                        self.last_observation = Some(wall_ms());
                        let event = value["event"].as_str().unwrap_or("");
                        if matches!(event, "preToolUse" | "beforeMCPExecution") {
                            let input = &value["input"];
                            let name = input["tool_name"].as_str().unwrap_or("");
                            let automatic = POLICY
                                .get()
                                .is_some_and(|policy| policy(name, &input["tool_input"]));
                            self.execute_independent(interaction::Command::Hook {
                                connection: connection.clone(),
                                event_id: value["eventId"].as_str().unwrap_or("").into(),
                                event: event.into(),
                                input: input.clone(),
                                automatic,
                                wall_ms: wall_ms(),
                            })
                        } else {
                            Ok(json!({"observed":true}))
                        }
                    })
                }
                AdapterInput::Mcp(value) if self.config.enabled => {
                    self.channel_operation(&connection, value, Role::Mcp)
                }
                _ => Err("CAPABILITY_NOT_SUPPORTED"),
            };
            let reply = result.unwrap_or_else(|e| json!({"error":{"code":e}}));
            self.hub.reply(delivery, &reply)?;
        }
        self.publish()
    }
    fn publish(&mut self) -> Result<()> {
        if self.history.version() == self.published_history_version
            && self.store.revision() == self.published_store_version
            && self.snapshot["hook"] == self.status
            && self.snapshot["lastObservationAt"] == json!(self.last_observation)
        {
            return Ok(());
        }
        let history =
            self.history
                .snapshot(wall_ms(), u64::MAX, &BTreeMap::new(), &BTreeSet::new());
        self.published_history_version = self.history.version();
        self.published_store_version = self.store.revision();
        let interactions = self.store.model().interaction_snapshot();
        let mut sessions = serde_json::to_value(history.sessions)
            .map_err(|_| "SNAPSHOT_FAILED")?
            .as_array()
            .cloned()
            .unwrap_or_default();
        for session in interactions["sessions"].as_array().into_iter().flatten() {
            if let Some(existing) = sessions.iter_mut().find(|s| s["key"] == session["key"]) {
                existing["hasCurrentRequest"] = session["hasCurrentRequest"].clone();
                if session["hasCurrentRequest"] == true {
                    existing["activity"] = "working".into();
                }
                if session["updatedAt"].as_u64() > existing["updatedAt"].as_u64() {
                    existing["updatedAt"] = session["updatedAt"].clone();
                }
            } else {
                sessions.push(session.clone());
            }
        }
        sessions.sort_by_key(|s| {
            (
                s["hasCurrentRequest"] != true,
                std::cmp::Reverse(s["updatedAt"].as_u64().unwrap_or(0)),
                s["key"].as_str().unwrap_or("").to_owned(),
            )
        });
        sessions.truncate(1000);
        let enabled = self.config.enabled && self.status["filesInstalled"] == true;
        if !enabled {
            sessions.clear();
        }
        let mut next = json!({"schemaVersion":1,"source":"cursor","captureScope":"observedOnly",
            "appEpoch":self.store.model().app_epoch(),"version":0,"connected":self.config.enabled && self.status["filesInstalled"] == true,
            "interactionProtocol":1,"integrationError":self.status["error"],"capabilities":capabilities(enabled),"sessions":sessions,
            "requests":if enabled {interactions["requests"].clone()} else {json!([])},"lastObservationAt":self.last_observation,"hook":self.status});
        let digest = protocol::digest(&next).map_err(|_| "SNAPSHOT_FAILED")?;
        if digest != self.last_digest {
            self.version += 1;
            self.last_digest = digest;
        }
        next["version"] = self.version.into();
        self.snapshot = next;
        Ok(())
    }
    fn execute_independent(&mut self, command: interaction::Command) -> Result<Value> {
        match self
            .store
            .execute(workflow::Command::Independent(command))?
        {
            workflow::Response::Independent { value } => Ok(value),
            _ => Err("RESPONSE_INVALID"),
        }
    }
    fn channel_operation(&mut self, connection: &str, input: &Value, role: Role) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(
            tag = "op",
            rename_all = "camelCase",
            rename_all_fields = "camelCase",
            deny_unknown_fields
        )]
        enum Operation {
            Poll {
                request_id: String,
            },
            Report {
                request_id: String,
                lease_id: String,
                written: bool,
            },
            Call {
                rpc_id: workflow::RpcId,
                name: String,
                arguments: Value,
            },
        }
        let op: Operation =
            serde_json::from_value(input.clone()).map_err(|_| "INVALID_OPERATION")?;
        match op {
            Operation::Poll { request_id } if role == Role::Hook => {
                self.execute_independent(interaction::Command::Poll {
                    connection: connection.into(),
                    workflow_id: None,
                    request_id,
                })
            }
            Operation::Report {
                request_id,
                lease_id,
                written,
            } => self.execute_independent(interaction::Command::Report {
                connection: connection.into(),
                request_id,
                lease_id,
                written,
            }),
            Operation::Call {
                rpc_id,
                name,
                arguments,
            } if role == Role::Mcp => {
                let key = serde_json::to_string(&rpc_id).map_err(|_| "INVALID_RPC_ID")?;
                let digest = protocol::digest(&json!([&name, &arguments]))
                    .map_err(|_| "INVALID_ARGUMENTS")?;
                let c = self
                    .connections
                    .get_mut(connection)
                    .ok_or("CONNECTION_UNKNOWN")?;
                if let Some(previous) = c.rpc_ids.get(&key) {
                    return Err(if previous == &digest {
                        "RPC_ALREADY_PROCESSED"
                    } else {
                        "RPC_ID_CONFLICT"
                    });
                }
                // Polling is safe to repeat: the model's delivery lease never
                // returns a decision twice. Do not exhaust a long-lived
                // connection's mutation replay budget with read calls.
                if !matches!(
                    name.as_str(),
                    "codecraft_get_result" | "codecraft_list_projects"
                ) {
                    if c.rpc_ids.len() >= 1024 {
                        return Err("CONNECTION_BACKPRESSURE");
                    }
                    c.rpc_ids.insert(key, digest);
                }
                crate::mcp::call(
                    &mut self.store,
                    connection,
                    rpc_id,
                    &name,
                    arguments,
                    wall_ms(),
                )
            }
            _ => Err("INVALID_OPERATION"),
        }
    }
}

/// Normalization explicitly projects display fields. Hook-provided commands,
/// tickets, transcript paths, approvals and arbitrary extra keys are discarded.
pub fn normalize(value: &Value) -> Result<Vec<Observation>> {
    let event = value["event"]
        .as_str()
        .filter(|e| protocol::EVENTS.contains(e))
        .ok_or("UNKNOWN_EVENT")?;
    let raw = &value["input"];
    if raw["hook_event_name"].as_str().is_some_and(|e| e != event) {
        return Err("EVENT_NAME_MISMATCH");
    }
    let id = |name: &str| {
        raw[name]
            .as_str()
            .filter(|s| schema::valid_id(s))
            .map(str::to_owned)
    };
    let conversation = id("conversation_id").ok_or("UNASSOCIATED_HISTORY_EVENT")?;
    let generation = id("generation_id");
    let text = |name: &str| raw[name].as_str().unwrap_or("").to_owned();
    let kind = match event {
        "sessionStart" | "beforeMCPExecution" => EventKind::Discovered { title: None },
        "beforeSubmitPrompt" => EventKind::PromptCandidate {
            generation: generation.clone().ok_or("GENERATION_REQUIRED")?,
        },
        "stop" | "sessionEnd" => EventKind::GenerationEnded {
            generation: generation.clone().ok_or("GENERATION_REQUIRED")?,
        },
        "afterAgentResponse" => EventKind::Response {
            generation: generation.clone(),
            semantic_id: id("response_id"),
            text: text("text"),
        },
        "preToolUse" | "postToolUse" | "postToolUseFailure" => EventKind::Tool {
            generation: generation.clone(),
            call_id: id("tool_use_id").ok_or("TOOL_ID_REQUIRED")?,
            name: text("tool_name"),
            outcome: match event {
                "preToolUse" => Outcome::Started,
                "postToolUse" => Outcome::CompletedObserved,
                _ if raw["is_interrupt"] == true => Outcome::CancelledObserved,
                _ => Outcome::FailedObserved,
            },
        },
        "subagentStart" | "subagentStop" => EventKind::Subagent {
            generation: generation.clone(),
            agent_id: id("subagent_id").ok_or("SUBAGENT_ID_REQUIRED")?,
            parent_id: id("parent_conversation_id"),
            outcome: if event == "subagentStart" {
                Outcome::Started
            } else {
                match raw["status"].as_str() {
                    Some("completed") => Outcome::CompletedObserved,
                    Some("error") => Outcome::FailedObserved,
                    Some("aborted") => Outcome::CancelledObserved,
                    _ => Outcome::Unknown,
                }
            },
        },
        _ => return Err("UNKNOWN_EVENT"),
    };
    // This ID is generated by our authenticated helper, outside Cursor's raw
    // input. Equal response text is not proof of duplicate delivery.
    let event_id = value["eventId"]
        .as_str()
        .filter(|id| schema::valid_id(id))
        .ok_or("INVALID_HISTORY_EVENT")?
        .to_owned();
    let mut result = Vec::new();
    if matches!(event, "postToolUse" | "afterAgentResponse") {
        if let Some(generation) = generation {
            result.push(Observation {
                event_id: format!("start:{event_id}"),
                conversation_id: conversation.clone(),
                event: EventKind::GenerationStarted { generation },
            });
        }
    }
    result.push(Observation {
        event_id,
        conversation_id: conversation,
        event: kind,
    });
    Ok(result)
}

pub fn start() -> Result<()> {
    let service = SERVICE.get_or_init(|| {
        Mutex::new(Service {
            broker: None,
            error: None,
            last_snapshot: empty(None),
        })
    });
    let result = {
        let mut state = service.lock().map_err(|_| "BROKER_UNAVAILABLE")?;
        if state.broker.is_some() {
            return Ok(());
        }
        let root = product::root()?;
        let home = product::cursor_home()?;
        let ready = std::env::current_exe()
            .map_err(|_| "EXECUTABLE_UNAVAILABLE")
            .and_then(|exe| product::repair_moved_executable(&root, &home, &exe))
            .and_then(|()| Broker::open(&root, &home));
        match ready {
            Ok(broker) => {
                state.broker = Some(broker);
                state.error = None;
                Ok(())
            }
            Err(error) => {
                state.error = Some(error.into());
                Err(error)
            }
        }
    };
    STARTED.get_or_init(|| {
        thread::spawn(|| loop {
            thread::sleep(Duration::from_millis(100));
            if let Some(service) = SERVICE.get() {
                if let Ok(mut state) = service.lock() {
                    if let Some(broker) = state.broker.as_mut() {
                        if let Err(error) = broker.tick() {
                            let mut snapshot = broker.snapshot.clone();
                            snapshot["connected"] = false.into();
                            snapshot["integrationError"] = error.into();
                            snapshot["version"] = (broker.version + 1).into();
                            state.last_snapshot = snapshot;
                            state.broker = None;
                            state.error = Some(error.into());
                        }
                    }
                }
            }
        });
    });
    result
}
pub fn snapshot() -> Value {
    SERVICE
        .get()
        .and_then(|s| s.lock().ok())
        .map(|s| {
            s.broker
                .as_ref()
                .map(|b| b.snapshot.clone())
                .unwrap_or_else(|| {
                    let mut snapshot = s.last_snapshot.clone();
                    snapshot["connected"] = false.into();
                    snapshot["integrationError"] = json!(s.error);
                    snapshot
                })
        })
        .unwrap_or_else(|| empty(Some("BROKER_OFFLINE")))
}
pub fn configure(enabled: bool, cursor: Option<PathBuf>) -> std::result::Result<Value, ApiError> {
    let service = SERVICE.get_or_init(|| {
        Mutex::new(Service {
            broker: None,
            error: None,
            last_snapshot: empty(None),
        })
    });
    let mut state = service
        .lock()
        .map_err(|_| ApiError::new("BROKER_UNAVAILABLE"))?;
    let projects = if enabled {
        std::env::var_os("APPDATA")
            .map(|path| PathBuf::from(path).join("Cursor/User/globalStorage/storage.json"))
            .map(|path| crate::project_rules::installation_projects(&path))
            .transpose()?
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    // Drain by revoking the whole epoch before updating files. Short-lived Hook
    // clients time out legally; no Cursor/shared process is killed or replayed.
    if let Some(broker) = state.broker.take() {
        state.last_snapshot = broker.snapshot.clone();
        drop(broker);
    }
    let root = product::root()?;
    let home = product::cursor_home()?;
    let exe = std::env::current_exe().map_err(|_| ApiError::new("EXECUTABLE_UNAVAILABLE"))?;
    let mut result =
        product::configure_with_projects_at(&root, &home, &exe, enabled, cursor, projects);
    if !enabled && result.is_ok() {
        state.last_snapshot = empty(None);
    }
    match Broker::open(&root, &home) {
        Ok(broker) => {
            state.broker = Some(broker);
            state.error = None;
        }
        Err(error) => {
            state.error = Some(error.into());
            if enabled && result.is_ok() {
                result = Err(error);
            }
        }
    }
    result.map_err(ApiError::from)
}

pub fn dispatch(route: &str, body: Value) -> std::result::Result<Value, ApiError> {
    if !matches!(route, "permission" | "question" | "plan" | "cancel") {
        return Err(ApiError::new("INVALID_ROUTE"));
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Submission {
        schema_version: u32,
        submission: workflow::Submission,
        decision: schema::Decision,
    }
    // There is intentionally no route to deserialize a trusted Command.
    let input: Submission =
        serde_json::from_value(body).map_err(|_| ApiError::new("INVALID_SUBMISSION"))?;
    if input.schema_version != 1 || !schema::valid_id(&input.submission.request_id) {
        return Err(ApiError::new("INVALID_SUBMISSION"));
    }
    let valid_route = matches!(
        (route, &input.decision),
        ("permission", schema::Decision::Permission { .. })
            | ("question", schema::Decision::Answered { .. })
            | ("question" | "cancel", schema::Decision::QuestionCancelled)
            | ("plan", schema::Decision::Plan { .. })
            | (
                "cancel",
                schema::Decision::Plan {
                    action: schema::PlanAction::Cancelled,
                    ..
                }
            )
    );
    if !valid_route {
        return Err(ApiError::new("INVALID_SUBMISSION"));
    }
    let mut state = SERVICE
        .get()
        .ok_or("BROKER_OFFLINE")?
        .lock()
        .map_err(|_| "BROKER_UNAVAILABLE")?;
    let broker = state.broker.as_mut().ok_or("BROKER_OFFLINE")?;
    if !broker.config.enabled || broker.status["filesInstalled"] != true {
        return Err(ApiError::new("INTEGRATION_DISABLED"));
    }
    let result = broker.execute_independent(interaction::Command::Decide {
        submission: input.submission,
        decision: input.decision,
        wall_ms: wall_ms(),
    })?;
    broker.publish()?;
    Ok(result)
}

fn connect_at(root: &Path, role: Role) -> Result<Client> {
    let broker = files::read_json(&root.join("broker.json"))?;
    let epoch = broker["appEpoch"].as_str().ok_or("BROKER_OFFLINE")?;
    let identity: process::Identity =
        serde_json::from_value(broker["process"].clone()).map_err(|_| "BROKER_OFFLINE")?;
    let current = process::identify(std::process::id())?;
    if process::identify(identity.pid)? != identity || identity.executable != current.executable {
        return Err("BROKER_IDENTITY_MISMATCH");
    }
    let nonce = uuid::Uuid::new_v4().to_string();
    let hello = root.join("bootstrap").join(format!("{nonce}.hello.json"));
    let reply = root.join("bootstrap").join(format!("{nonce}.permit.json"));
    files::atomic_json(
        &hello,
        &serde_json::to_value(Hello {
            schema_version: 1,
            app_epoch: epoch.into(),
            nonce: nonce.clone(),
            process: current,
            role,
        })
        .map_err(|_| "INVALID_HANDSHAKE")?,
        false,
    )?;
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(2) {
        if reply.exists() {
            let permit =
                Permit::read_bootstrap(&root.join("channel"), &reply, &nonce, epoch, role)?;
            return Ok(Client::new(permit));
        }
        thread::sleep(Duration::from_millis(25));
    }
    let _ = fs::remove_file(hello);
    Err("BROKER_TIMEOUT")
}
fn channel_request(client: &mut Client, role: Role, value: Value) -> Result<Value> {
    let id = client.submit(match role {
        Role::Hook => AdapterInput::Hook(value),
        Role::Mcp => AdapterInput::Mcp(value),
        _ => return Err("INVALID_ROLE"),
    })?;
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(3) {
        if let Some(value) = client.read_reply(&id)? {
            return Ok(value);
        }
        thread::sleep(Duration::from_millis(25));
    }
    Err("BROKER_TIMEOUT")
}
type HookDelivery = Option<(Client, String, String)>;
fn hook_at(root: &Path, event: &str, input: Value) -> Result<(Value, HookDelivery)> {
    let mut client = connect_at(root, Role::Hook)?;
    let result = channel_request(
        &mut client,
        Role::Hook,
        json!({"eventId":uuid::Uuid::new_v4().to_string(),"event":event,"input":input}),
    )?;
    let passive = protocol::passive_response(event).map_err(|_| "INVALID_EVENT")?;
    let Some(request) = result["requestId"].as_str().map(str::to_owned) else {
        return Ok((passive, None));
    };
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(50) {
        let result = channel_request(
            &mut client,
            Role::Hook,
            json!({"op":"poll","requestId":request}),
        )?;
        if result["status"] == "decided" {
            let lease = result["_deliveryLease"]
                .as_str()
                .ok_or("DELIVERY_LEASE_INVALID")?
                .to_owned();
            let response = interaction::permission_response(event, &result["decision"])?;
            return Ok((response, Some((client, request, lease))));
        }
        if result["status"] != "pending" {
            return Ok((passive, None));
        }
        thread::sleep(Duration::from_millis(750));
    }
    Ok((passive, None))
}
#[cfg(test)]
fn observe_at(root: &Path, event: &str, input: Value) -> Result<()> {
    hook_at(root, event, input).map(|_| ())
}
pub fn capture() -> i32 {
    let args = std::env::args().collect::<Vec<_>>();
    let event = args
        .windows(2)
        .find(|pair| pair[0] == "--event")
        .map(|p| p[1].as_str())
        .unwrap_or("");
    let Ok(mut response) = protocol::passive_response(event) else {
        return 1;
    };
    let mut delivery = None;
    if process::limit_cpu().is_ok() {
        if let Ok(bytes) = protocol::read_bytes(io::stdin().lock()) {
            if let Ok(input) = protocol::hook_json(&bytes) {
                if let Ok(root) = product::root() {
                    if let Ok((value, receipt)) = hook_at(&root, event, input) {
                        response = value;
                        delivery = receipt;
                    }
                }
            }
        }
    }
    // D3-B: a legal event-specific response even when CodeCraft is offline.
    let written = {
        let mut output = io::stdout().lock();
        writeln!(output, "{response}")
            .and_then(|()| output.flush())
            .is_ok()
    };
    if let Some((mut client, request, lease)) = delivery {
        let _ = channel_request(
            &mut client,
            Role::Hook,
            json!({"op":"report","requestId":request,"leaseId":lease,"written":written}),
        );
    }
    if written {
        0
    } else {
        1
    }
}

pub fn mcp_response(frame: Value) -> Option<Value> {
    if frame["jsonrpc"] != "2.0" || !frame["method"].is_string() {
        return Some(
            json!({"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"Invalid request"}}),
        );
    }
    let id = frame.get("id")?.clone();
    if !(id.is_string() || id.as_i64().is_some()) {
        return Some(
            json!({"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"Invalid request"}}),
        );
    }
    let result = match frame["method"].as_str() {
        Some("initialize") => {
            json!({"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"codecraft-cursor","version":env!("CARGO_PKG_VERSION")},"instructions":product::REASON})
        }
        Some("ping") => json!({}),
        Some("tools/list") => json!({"tools":[]}),
        Some("tools/call") => {
            let code = if frame["params"]["arguments"]["bridgeTicket"]
                .as_str()
                .is_none()
            {
                "TICKET_REQUIRED"
            } else {
                "CAPABILITY_NOT_SUPPORTED"
            };
            json!({"isError":true,"content":[{"type":"text","text":code}]})
        }
        _ => {
            return Some(
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Method not found"}}),
            )
        }
    };
    Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
}
pub fn run_mcp() -> Result<()> {
    process::limit_cpu()?;
    let root = product::root()?;
    run_mcp_at(&root)
}
fn run_mcp_at(root: &Path) -> Result<()> {
    if !product::load(root)?.enabled {
        return Err("INTEGRATION_DISABLED");
    }
    let mut client = connect_at(root, Role::Mcp)?;
    let descriptor = files::read_json(&root.join("broker.json"))?;
    let epoch = descriptor["appEpoch"]
        .as_str()
        .ok_or("BROKER_OFFLINE")?
        .to_owned();
    let identity: process::Identity =
        serde_json::from_value(descriptor["process"].clone()).map_err(|_| "BROKER_OFFLINE")?;
    if process::identify(identity.pid)? != identity
        || identity.executable != process::identify(std::process::id())?.executable
    {
        return Err("BROKER_IDENTITY_MISMATCH");
    }
    let heartbeat = || -> Result<Heartbeat> {
        serde_json::from_value(files::read_json(&root.join("channel/heartbeat.json"))?)
            .map_err(|_| "INVALID_HEARTBEAT")
    };
    let first = heartbeat()?;
    if first.app_epoch != epoch {
        return Err("BROKER_OFFLINE");
    }
    let mut liveness = Liveness::after_handshake(first, 0)?;
    let started = Instant::now();
    let (send, receive) = std::sync::mpsc::sync_channel(4);
    thread::spawn(move || {
        let mut input = io::stdin().lock();
        loop {
            let frame = protocol::read_frame(&mut input);
            let finished = !matches!(frame, Ok(Some(_)));
            if send.send(frame).is_err() || finished {
                break;
            }
        }
    });
    let mut output = io::stdout().lock();
    let mut initialized = false;
    let mut seen = BTreeMap::new();
    loop {
        // Exit even while stdin is idle when the broker restarts or drains for
        // an upgrade. The installer never needs to kill a shared Cursor host.
        if process::identify(identity.pid).as_ref() != Ok(&identity)
            || files::read_json(&root.join("broker.json"))?["appEpoch"] != epoch
        {
            return Err("BROKER_OFFLINE");
        }
        liveness.observe(&heartbeat()?, started.elapsed().as_millis() as u64)?;
        let frame = match receive.recv_timeout(Duration::from_secs(1)) {
            Ok(Ok(Some(frame))) => frame,
            Ok(Ok(None)) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
            Ok(Err(_)) => return Err("MCP_INVALID_FRAME"),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
        };
        let Some(id) = frame.get("id").cloned() else {
            continue;
        };
        if frame["jsonrpc"] != "2.0" {
            return Err("MCP_INVALID_FRAME");
        }
        let rpc: workflow::RpcId =
            serde_json::from_value(id.clone()).map_err(|_| "MCP_INVALID_ID")?;
        if matches!(&rpc,workflow::RpcId::Text(s) if s.is_empty()||s.len()>128) {
            return Err("MCP_INVALID_ID");
        }
        let key = serde_json::to_string(&id).map_err(|_| "MCP_INVALID_ID")?;
        let digest = protocol::digest(&frame).map_err(|_| "MCP_INVALID_FRAME")?;
        if let Some(previous) = seen.get(&key) {
            if previous != &digest {
                return Err("MCP_RPC_ID_CONFLICT");
            }
            continue;
        }
        let method = frame["method"].as_str().unwrap_or("");
        let read_call = matches!(method, "ping" | "tools/list")
            || (method == "tools/call"
                && matches!(
                    frame["params"]["name"].as_str(),
                    Some("codecraft_get_result" | "codecraft_list_projects")
                ));
        if !read_call {
            if seen.len() >= 4096 {
                return Err("MCP_CONNECTION_LIMIT");
            }
            seen.insert(key, digest);
        }
        let mut receipt = None;
        let result = match method {
            "initialize" if !initialized => {
                initialized = true;
                let requested = frame["params"]["protocolVersion"].as_str().unwrap_or("");
                let version = if ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"]
                    .contains(&requested)
                {
                    requested
                } else {
                    "2024-11-05"
                };
                Ok(
                    json!({"protocolVersion":version,"capabilities":{"tools":{}},"serverInfo":{"name":"codecraft-cursor","version":env!("CARGO_PKG_VERSION")},"instructions":product::REASON}),
                )
            }
            "ping" => Ok(json!({})),
            _ if !initialized => Err("MCP_NOT_INITIALIZED"),
            "tools/list" => Ok(json!({"tools":crate::mcp::tools()})),
            "tools/call" => {
                let mut value = channel_request(
                    &mut client,
                    Role::Mcp,
                    json!({"op":"call","rpcId":rpc,"name":frame["params"]["name"],"arguments":frame["params"].get("arguments").cloned().unwrap_or(json!({}))}),
                )?;
                if let Some(lease) = value
                    .as_object_mut()
                    .and_then(|v| v.remove("_deliveryLease"))
                {
                    receipt = Some((value["requestId"].clone(), lease));
                }
                Ok(
                    json!({"isError":value.get("error").is_some(),"content":[{"type":"text","text":value.to_string()}],"structuredContent":value}),
                )
            }
            _ => Err("METHOD_NOT_FOUND"),
        };
        let response = match result {
            Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
            Err(code) => json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":code}}),
        };
        let written = writeln!(output, "{response}")
            .and_then(|()| output.flush())
            .is_ok();
        if let Some((request, lease)) = receipt {
            let _ = channel_request(
                &mut client,
                Role::Mcp,
                json!({"op":"report","requestId":request,"leaseId":lease,"written":written}),
            );
        }
        if !written {
            return Err("MCP_WRITE_FAILED");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn observation_never_imports_ticket_path_or_decision_authority() {
        let events = normalize(&json!({"eventId":"test-event","event":"postToolUse","input":{"conversation_id":"c","generation_id":"g","tool_use_id":"t","tool_name":"Read","bridgeTicket":"secret","transcript_path":"private","command":"Decide"}})).unwrap();
        let encoded = serde_json::to_string(&events).unwrap();
        assert!(
            !encoded.contains("secret")
                && !encoded.contains("private")
                && !encoded.contains("Decide")
        );
        assert_eq!(events.len(), 2);
        assert!(normalize(
            &json!({"eventId":"test-event","event":"postToolUse","input":{"conversation_id":"c"}})
        )
        .is_err());
    }
    #[test]
    fn identical_response_text_does_not_replace_distinct_observation_ids() {
        let raw = json!({"conversation_id":"c","generation_id":"g","text":"Same answer"});
        let a = normalize(&json!({"eventId":"first","event":"afterAgentResponse","input":raw}))
            .unwrap();
        let b = normalize(&json!({"eventId":"second","event":"afterAgentResponse","input":raw}))
            .unwrap();
        assert_ne!(a.last().unwrap().event_id, b.last().unwrap().event_id);
        assert!(normalize(&json!({"event":"afterAgentResponse","input":raw})).is_err());
    }
    #[test]
    fn blocked_prompt_does_not_confirm_generation_and_mismatched_event_is_rejected() {
        let events = normalize(&json!({"eventId":"test-event","event":"beforeSubmitPrompt","input":{"conversation_id":"c","generation_id":"g"}})).unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].event, EventKind::PromptCandidate { .. }));
        assert!(
            normalize(&json!({"event":"stop","input":{"hook_event_name":"preToolUse"}})).is_err()
        );
    }
    #[test]
    fn interruption_and_subagent_stop_do_not_imply_success() {
        for (status, expected) in [
            ("completed", Outcome::CompletedObserved),
            ("error", Outcome::FailedObserved),
            ("aborted", Outcome::CancelledObserved),
            ("future-status", Outcome::Unknown),
        ] {
            let events = normalize(&json!({"eventId":"stop-event","event":"subagentStop",
                "input":{"conversation_id":"c","generation_id":"g","subagent_id":"s","status":status}})).unwrap();
            assert!(
                matches!(&events[0].event, EventKind::Subagent {outcome, ..} if *outcome == expected)
            );
        }
        let interrupted = normalize(&json!({"eventId":"cancel-event","event":"postToolUseFailure",
            "input":{"conversation_id":"c","generation_id":"g","tool_use_id":"t","tool_name":"Shell","is_interrupt":true}})).unwrap();
        assert!(matches!(
            &interrupted[0].event,
            EventKind::Tool {
                outcome: Outcome::CancelledObserved,
                ..
            }
        ));
        // Some documented stop payloads omit a stable subagent ID. Never attach
        // such an event by task text, type or the most recently active child.
        assert_eq!(
            normalize(&json!({"eventId":"unbound-stop","event":"subagentStop",
            "input":{"conversation_id":"c","subagent_type":"explore","status":"completed"}}))
            .unwrap_err(),
            "SUBAGENT_ID_REQUIRED"
        );
    }
    #[test]
    fn unsupported_mcp_has_no_tools_and_never_returns_an_approval() {
        assert_eq!(
            mcp_response(json!({"jsonrpc":"2.0","id":0,"method":"tools/list"})).unwrap()["result"]
                ["tools"],
            json!([])
        );
        let response = mcp_response(json!({"jsonrpc":"2.0","id":0,"method":"tools/call","params":{"name":"codecraft_review_plan","arguments":{}}})).unwrap();
        assert_eq!(response["id"], 0);
        assert_eq!(response["result"]["isError"], true);
        assert_eq!(response["result"]["content"][0]["text"], "TICKET_REQUIRED");
        assert!(
            mcp_response(json!({"jsonrpc":"2.0","method":"notifications/cancelled"})).is_none()
        );
        assert_eq!(
            dispatch("install", json!({})).unwrap_err().error.code,
            "INVALID_ROUTE"
        );
        assert_eq!(
            dispatch("question", json!({"kind":"SetProject"}))
                .unwrap_err()
                .error
                .code,
            "INVALID_SUBMISSION"
        );
    }

    // These helpers only run under this explicit test environment. The copied
    // test binary is a synthetic Cursor ancestor, never Cursor runtime evidence.
    #[cfg(windows)]
    #[test]
    fn subprocess_observation_helper() {
        let Some(root) = std::env::var_os("CODECRAFT_CURSOR_TEST_ROOT") else {
            return;
        };
        observe_at(&PathBuf::from(root), "afterAgentResponse", json!({"hook_event_name":"afterAgentResponse",
            "conversation_id":"synthetic-conversation","generation_id":"synthetic-generation","text":"Synthetic observed response"})).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn subprocess_cursor_ancestor() {
        use std::os::windows::process::CommandExt;
        let Some(exe) = std::env::var_os("CODECRAFT_CURSOR_TEST_HELPER") else {
            return;
        };
        let status = std::process::Command::new(exe)
            .args([
                "--exact",
                &std::env::var("CODECRAFT_CURSOR_TEST_METHOD")
                    .unwrap_or_else(|_| "runtime::tests::subprocess_observation_helper".into()),
                "--test-threads=1",
                "--nocapture",
            ])
            .creation_flags(0x08000000)
            .status()
            .unwrap();
        assert!(status.success());
    }
    #[cfg(windows)]
    #[test]
    fn subprocess_live_mcp_helper() {
        let Some(root) = std::env::var_os("CODECRAFT_CURSOR_TEST_ROOT") else {
            return;
        };
        let _ = run_mcp_at(&PathBuf::from(root));
    }
    #[cfg(windows)]
    #[test]
    fn idle_mcp_exits_after_broker_revocation_without_waiting_for_stdin() {
        use std::os::windows::process::CommandExt;
        let (root, home, exe, cursor) = product::tests::fixture();
        fs::copy(&exe, &cursor).unwrap();
        product::configure_at(&root, &home, &exe, true, Some(cursor.clone())).unwrap();
        let mut broker = Broker::open(&root, &home).unwrap();
        let mut child = std::process::Command::new(cursor)
            .args([
                "--exact",
                "runtime::tests::subprocess_cursor_ancestor",
                "--test-threads=1",
            ])
            .env("CODECRAFT_CURSOR_TEST_ROOT", &root)
            .env("CODECRAFT_CURSOR_TEST_HELPER", &exe)
            .env(
                "CODECRAFT_CURSOR_TEST_METHOD",
                "runtime::tests::subprocess_live_mcp_helper",
            )
            .creation_flags(0x08000000)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let ready = Instant::now();
        while broker.connections.is_empty() && ready.elapsed() < Duration::from_secs(3) {
            broker.tick().unwrap();
            thread::sleep(Duration::from_millis(25));
        }
        assert_eq!(
            broker.connections.len(),
            1,
            "MCP must finish authenticated bootstrap before revocation is tested"
        );
        assert!(child.try_wait().unwrap().is_none());
        drop(broker);
        let started = Instant::now();
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            if started.elapsed() > Duration::from_secs(4) {
                let _ = child.kill();
                panic!("Idle synthetic MCP did not exit after revocation");
            }
            thread::sleep(Duration::from_millis(25));
        }
    }
    #[cfg(windows)]
    #[test]
    fn synthetic_process_handshake_to_history_and_restart_are_end_to_end() {
        use std::os::windows::process::CommandExt;
        let (root, home, exe, cursor) = product::tests::fixture();
        fs::copy(&exe, &cursor).unwrap();
        product::configure_at(&root, &home, &exe, true, Some(cursor.clone())).unwrap();
        let mut broker = Broker::open(&root, &home).unwrap();
        let epoch = broker.store.model().app_epoch().to_owned();
        let mut child = std::process::Command::new(cursor)
            .args([
                "--exact",
                "runtime::tests::subprocess_cursor_ancestor",
                "--test-threads=1",
            ])
            .env("CODECRAFT_CURSOR_TEST_ROOT", &root)
            .env("CODECRAFT_CURSOR_TEST_HELPER", &exe)
            .creation_flags(0x08000000)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now();
        let status = loop {
            broker.tick().unwrap();
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if deadline.elapsed() > Duration::from_secs(10) {
                let _ = child.kill();
                panic!("Synthetic IPC child timed out");
            }
            thread::sleep(Duration::from_millis(25));
        };
        assert!(status.success());
        assert_eq!(
            broker.snapshot["sessions"][0]["items"][0]["text"],
            "Synthetic observed response"
        );
        assert_eq!(broker.snapshot["requests"], json!([]));
        assert_eq!(broker.snapshot["capabilities"]["nativeShared"], false);
        assert_eq!(
            broker.snapshot["capabilities"]["interactionMode"],
            "independent"
        );
        drop(broker);
        let recovered = Broker::open(&root, &home).unwrap();
        assert_ne!(recovered.store.model().app_epoch(), epoch);
        assert_eq!(recovered.snapshot["sessions"][0]["activity"], "interrupted");
        assert_eq!(recovered.snapshot["sessions"][0]["readOnly"], true);
    }

    #[cfg(windows)]
    #[test]
    fn synthetic_stdio_mcp_reaches_production_broker_and_delivers_once() {
        use std::{
            io::{BufRead, BufReader},
            os::windows::process::CommandExt,
            process::Stdio,
        };
        let (root, home, exe, cursor) = product::tests::fixture();
        fs::copy(&exe, &cursor).unwrap();
        product::configure_at(&root, &home, &exe, true, Some(cursor.clone())).unwrap();
        let mut broker = Broker::open(&root, &home).unwrap();
        let project = root.parent().unwrap().to_path_buf();
        broker
            .store
            .execute(workflow::Command::SetProjectPath {
                path: project.clone(),
                enabled: true,
            })
            .unwrap();
        let project_id = crate::projects::resolve(&project).unwrap().identity.key();
        let mut child = std::process::Command::new(cursor)
            .args([
                "--exact",
                "runtime::tests::subprocess_cursor_ancestor",
                "--test-threads=1",
                "--nocapture",
            ])
            .env("CODECRAFT_CURSOR_TEST_ROOT", &root)
            .env("CODECRAFT_CURSOR_TEST_HELPER", &exe)
            .env(
                "CODECRAFT_CURSOR_TEST_METHOD",
                "runtime::tests::subprocess_live_mcp_helper",
            )
            .creation_flags(0x08000000)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(output)
                .lines()
                .map_while(std::result::Result::ok)
            {
                if let Some(start) = line.find("{\"jsonrpc\"").or_else(|| line.find("{\"id\"")) {
                    if let Ok(value) = serde_json::from_str::<Value>(&line[start..]) {
                        let _ = send.send(value);
                    }
                }
            }
        });
        let mut exchange = |broker: &mut Broker, id: u64, method: &str, params: Value| {
            writeln!(
                input,
                "{}",
                json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
            )
            .unwrap();
            input.flush().unwrap();
            let started = Instant::now();
            loop {
                broker.tick().unwrap();
                if let Ok(value) = receive.try_recv() {
                    assert_eq!(value["id"], id);
                    return value;
                }
                if started.elapsed() > Duration::from_secs(8) {
                    let _ = child.kill();
                    panic!("Synthetic MCP response timed out");
                }
                thread::sleep(Duration::from_millis(20));
            }
        };
        assert_eq!(
            exchange(
                &mut broker,
                0,
                "initialize",
                json!({"protocolVersion":"2025-11-25"})
            )["result"]["protocolVersion"],
            "2025-11-25"
        );
        assert_eq!(
            exchange(&mut broker, 1, "tools/list", json!({}))["result"]["tools"]
                .as_array()
                .unwrap()
                .len(),
            7
        );
        let opened = exchange(
            &mut broker,
            2,
            "tools/call",
            json!({"name":"codecraft_start_workflow","arguments":{"projectId":project_id,"title":"Synthetic MCP workflow"}}),
        );
        let wid = opened["result"]["structuredContent"]["workflowId"]
            .as_str()
            .unwrap()
            .to_owned();
        let created = exchange(
            &mut broker,
            3,
            "tools/call",
            json!({"name":"codecraft_ask_user","arguments":{"workflowId":wid,"schemaVersion":1,"clientRequestId":"stdio-request",
            "questions":[{"questionId":"q","prompt":"Synthetic approval","options":[],"multiSelect":false,"allowFreeText":true,"required":true,"allowSkip":false,"maxTextLength":30}]}}),
        );
        let request = created["result"]["structuredContent"]["requestId"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(broker.snapshot["requests"][0]["actionable"], true);
        broker
            .execute_independent(interaction::Command::Decide {
                submission: broker.store.model().request(&request).unwrap().submission(),
                decision: schema::Decision::Answered {
                    answers: vec![schema::Answer {
                        question_id: "q".into(),
                        selected_option_ids: vec![],
                        text: Some("verified".into()),
                        skipped: false,
                    }],
                },
                wall_ms: wall_ms(),
            })
            .unwrap();
        let received = exchange(
            &mut broker,
            4,
            "tools/call",
            json!({"name":"codecraft_get_result","arguments":{"workflowId":wid,"requestId":request}}),
        );
        assert_eq!(
            received["result"]["structuredContent"]["decision"]["answers"][0]["text"],
            "verified"
        );
        assert!(!received.to_string().contains("_deliveryLease"));
        let duplicate = exchange(
            &mut broker,
            5,
            "tools/call",
            json!({"name":"codecraft_get_result","arguments":{"workflowId":wid,"requestId":request}}),
        );
        assert_eq!(
            duplicate["result"]["structuredContent"]["status"],
            "alreadyDelivered"
        );
        drop(input);
        let started = Instant::now();
        while child.try_wait().unwrap().is_none() {
            broker.tick().unwrap();
            if started.elapsed() > Duration::from_secs(4) {
                let _ = child.kill();
                panic!("Synthetic MCP did not exit");
            }
            thread::sleep(Duration::from_millis(25));
        }
    }
}
