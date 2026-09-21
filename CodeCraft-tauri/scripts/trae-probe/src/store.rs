//! A deliberately small P0 coordinator. Only this process owns mutable state;
//! Hook/MCP processes submit immutable commands. Nothing here authorizes a
//! production CodeCraft task or restores permission from a previous run.
use crate::protocol::{business_args, hash, read_bounded, strict_json, MAX_IPC};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

pub const POLL: Duration = Duration::from_millis(200);
const TICKET_TTL: Duration = Duration::from_secs(600);
const LEASE_TTL: Duration = Duration::from_secs(5);
const CACHE_TTL: Duration = Duration::from_secs(900);

pub fn id() -> String {
    Uuid::new_v4().to_string()
}
pub fn wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn secret() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Identity {
    pub host: String,
    pub session: String,
    pub root: String,
    pub call: String,
}
impl Identity {
    fn session_key(&self) -> Result<String, String> {
        hash(&json!([self.host, self.session, self.root]))
    }
    fn call_key(&self) -> Result<String, String> {
        hash(&serde_json::to_value(self).map_err(|e| e.to_string())?)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Observe {
        identity: Identity,
        event: String,
        prompt: Option<String>,
    },
    Arm,
    Register {
        identity: Identity,
        tool: String,
        arguments: Value,
    },
    Consume {
        connection: String,
        ticket: String,
        tool: String,
        arguments: Value,
    },
    Release {
        operation: String,
    },
    Poll {
        connection: String,
        operation: String,
    },
    Prepare {
        connection: String,
        operation: String,
    },
    Ack {
        connection: String,
        operation: String,
        lease: String,
    },
    Cancel {
        connection: String,
        operation: String,
    },
    Inspect,
}
impl Command {
    fn control(&self) -> bool {
        matches!(
            self,
            Self::Cancel { .. } | Self::Ack { .. } | Self::Observe { .. }
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Envelope {
    pub message_id: String,
    pub command_id: String,
    pub app_epoch: String,
    pub command: Command,
}

#[derive(Clone)]
struct Ticket {
    token: String,
    identity: Identity,
    tool: String,
    args_hash: String,
    turn: u32,
    created: Instant,
    operation: Option<String>,
}
#[derive(Clone)]
struct Operation {
    session: String,
    turn: u32,
    connection: String,
    args: Value,
    state: String,
    created: Instant,
    lease: Option<(String, Instant)>,
    delivery: bool,
}
#[derive(Clone, Default)]
struct Session {
    turn: u32,
    protected: bool,
    delivered: bool,
}
#[derive(Clone)]
struct Armed {
    code: String,
    created: Instant,
    plan_id: String,
    path: String,
    bound: Option<String>,
}

#[derive(Clone)]
pub struct State {
    epoch: String,
    sessions: HashMap<String, Session>,
    tickets: HashMap<String, Ticket>,
    calls: HashMap<String, String>,
    operations: HashMap<String, Operation>,
    armed: Option<Armed>,
    root: PathBuf,
}
impl State {
    fn new(root: &Path) -> Self {
        Self {
            epoch: id(),
            sessions: HashMap::new(),
            tickets: HashMap::new(),
            calls: HashMap::new(),
            operations: HashMap::new(),
            armed: None,
            root: root.into(),
        }
    }
    fn invalidate(&mut self, session: &str) -> Result<(), String> {
        let s = self.sessions.entry(session.into()).or_default();
        s.turn = s
            .turn
            .checked_add(1)
            .filter(|v| *v <= i32::MAX as u32)
            .ok_or("TASK_CHANGED: turn exhausted")?;
        s.delivered = false;
        for op in self
            .operations
            .values_mut()
            .filter(|o| o.session == session && o.state != "delivered")
        {
            op.state = "cancelled".into();
        }
        Ok(())
    }
    fn expire(&mut self, now: Instant) -> bool {
        let mut changed = false;
        for op in self.operations.values_mut() {
            if matches!(
                op.state.as_str(),
                "delivered" | "cancelled" | "expired" | "failed"
            ) {
                continue;
            }
            if now.duration_since(op.created) >= Duration::from_secs(240) {
                op.state = "expired".into();
                changed = true;
            } else if op.state == "delivery_prepared"
                && op
                    .lease
                    .as_ref()
                    .is_some_and(|l| now.duration_since(l.1) >= LEASE_TTL)
            {
                op.state = "failed".into();
                changed = true;
            }
        }
        changed
    }
    fn apply(&mut self, command: &Command, now: Instant) -> Result<Value, String> {
        match command {
            Command::Observe {
                identity,
                event,
                prompt,
            } => {
                let key = identity.session_key()?;
                // No Prompt ID is verified. Every prompt conservatively advances
                // the turn, including a duplicate of the consumed launch code.
                self.sessions.entry(key.clone()).or_default();
                if matches!(event.as_str(), "UserPromptSubmit" | "Stop" | "SessionStart") {
                    self.invalidate(&key)?;
                }
                if event == "UserPromptSubmit" {
                    let prompt = prompt.as_deref().unwrap_or("");
                    if prompt.contains("[[CODECRAFT_TASK:") {
                        let Some(armed) = self.armed.clone() else {
                            return Err("TASK_CHANGED: no armed task".into());
                        };
                        let line = format!("[[CODECRAFT_TASK:{}]]", armed.code);
                        if prompt.lines().next() != Some(&line)
                            || prompt.matches("[[CODECRAFT_TASK:").count() != 1
                            || armed.bound.is_some()
                            || now.duration_since(armed.created) >= TICKET_TTL
                        {
                            if let Some(previous) = armed.bound.as_deref() {
                                if previous != key {
                                    self.invalidate(previous)?;
                                }
                            }
                            return Err(
                                "TASK_CHANGED: invalid, consumed or expired launch code".into()
                            );
                        }
                        self.armed.as_mut().unwrap().bound = Some(key.clone());
                        self.sessions.get_mut(&key).unwrap().protected = true;
                        return Ok(
                            json!({"additionalContext":format!("CodeCraft P0 test only. planId={}, documentPath={}, baseRevision=0. No production execution permission is granted.", armed.plan_id, armed.path), "bound":true}),
                        );
                    }
                }
                Ok(json!({"observed":true, "protected":self.sessions[&key].protected}))
            }
            Command::Arm => {
                if self.armed.as_ref().is_some_and(|a| a.bound.is_some()) {
                    return Err("TASK_CHANGED: start a new probe run for another task".into());
                }
                let task = id();
                let plan = id();
                let directory = self
                    .root
                    .join("project/.trae/documents/codecraft")
                    .join(&task);
                fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
                let canonical = directory.canonicalize().map_err(|e| e.to_string())?;
                let project = self
                    .root
                    .join("project")
                    .canonicalize()
                    .map_err(|e| e.to_string())?;
                if !canonical.starts_with(project) {
                    return Err("INVALID_ARGUMENT: plan path escapes project".into());
                }
                let path = directory.join("plan.md");
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .map_err(|e| e.to_string())?;
                file.write_all(b"# CodeCraft P0 test plan\n")
                    .and_then(|_| file.sync_all())
                    .map_err(|e| e.to_string())?;
                let code = secret()?;
                self.armed = Some(Armed {
                    code: code.clone(),
                    created: now,
                    plan_id: plan,
                    path: path.to_string_lossy().into(),
                    bound: None,
                });
                Ok(
                    json!({"taskId":task,"launchPrompt":format!("[[CODECRAFT_TASK:{code}]]\nRun the CodeCraft P0 binding probe only.")}),
                )
            }
            Command::Register {
                identity,
                tool,
                arguments,
            } => {
                crate::protocol::validate_probe_args(arguments)?;
                if tool != crate::protocol::ECHO_TOOL && tool != crate::protocol::DELIVERY_TOOL {
                    return Err("INVALID_ARGUMENT: unregistered tool".into());
                }
                let args_hash = hash(&business_args(arguments)?)?;
                let call = identity.call_key()?;
                let session_key = identity.session_key()?;
                let session = self.sessions.entry(session_key).or_default();
                if let Some(token_hash) = self.calls.get(&call) {
                    let t = &self.tickets[token_hash];
                    if t.args_hash != args_hash || t.tool != *tool {
                        return Err("IDEMPOTENCY_CONFLICT".into());
                    }
                    if t.turn != session.turn
                        || now.duration_since(t.created) >= TICKET_TTL
                        || t.operation.is_some()
                    {
                        return Err("TICKET_EXPIRED: issue a new native call".into());
                    }
                    return Ok(json!({"bridgeTicket":t.token}));
                }
                if self.tickets.len() >= 1024 {
                    return Err("QUEUE_FULL: start a fresh probe run".into());
                }
                let token = secret()?;
                let token_hash = hash(&json!(token))?;
                self.tickets.insert(
                    token_hash.clone(),
                    Ticket {
                        token: token.clone(),
                        identity: identity.clone(),
                        tool: tool.clone(),
                        args_hash,
                        turn: session.turn,
                        created: now,
                        operation: None,
                    },
                );
                self.calls.insert(call, token_hash);
                Ok(json!({"bridgeTicket":token}))
            }
            Command::Consume {
                connection,
                ticket,
                tool,
                arguments,
            } => {
                let token_hash = hash(&json!(ticket))?;
                let t = self
                    .tickets
                    .get(&token_hash)
                    .ok_or("TICKET_INVALID")?
                    .clone();
                let session = t.identity.session_key()?;
                if self.sessions[&session].turn != t.turn {
                    return Err("TASK_CHANGED".into());
                }
                if t.tool != *tool || t.args_hash != hash(&business_args(arguments)?)? {
                    return Err("ARGUMENT_MISMATCH".into());
                }
                if let Some(operation) = &t.operation {
                    let op = &self.operations[operation];
                    if op.connection != *connection {
                        return Err("TICKET_INVALID: different MCP connection".into());
                    }
                    if now.duration_since(op.created) >= CACHE_TTL {
                        return Err("REQUEST_EXPIRED".into());
                    }
                    return Ok(json!({"operationId":operation,"replayed":true,"state":op.state}));
                }
                if now.duration_since(t.created) >= TICKET_TTL {
                    return Err("TICKET_EXPIRED".into());
                }
                let waiting = |o: &&Operation| {
                    !matches!(
                        o.state.as_str(),
                        "delivered" | "cancelled" | "expired" | "failed"
                    )
                };
                if self.operations.values().filter(waiting).count() >= 128
                    || self
                        .operations
                        .values()
                        .filter(|o| o.session == session)
                        .filter(waiting)
                        .count()
                        >= 16
                    || self.operations.len() >= 1024
                {
                    return Err("QUEUE_FULL".into());
                }
                let operation = id();
                if tool == crate::protocol::DELIVERY_TOOL {
                    self.sessions.get_mut(&session).unwrap().delivered = false;
                }
                self.operations.insert(
                    operation.clone(),
                    Operation {
                        session,
                        turn: t.turn,
                        connection: connection.clone(),
                        args: business_args(arguments)?,
                        state: if tool == crate::protocol::ECHO_TOOL {
                            "user_decided"
                        } else {
                            "pending"
                        }
                        .into(),
                        created: now,
                        lease: None,
                        delivery: tool == crate::protocol::DELIVERY_TOOL,
                    },
                );
                self.tickets.get_mut(&token_hash).unwrap().operation = Some(operation.clone());
                Ok(json!({"operationId":operation,"replayed":false}))
            }
            Command::Release { operation } => {
                let op = self.operations.get_mut(operation).ok_or("NOT_FOUND")?;
                if op.state != "pending" {
                    return Err("REQUEST_CONFLICT".into());
                }
                if now.duration_since(op.created) >= Duration::from_secs(240) {
                    return Err("REQUEST_EXPIRED".into());
                }
                op.state = "user_decided".into();
                Ok(json!({"state":op.state}))
            }
            Command::Poll {
                connection,
                operation,
            }
            | Command::Prepare {
                connection,
                operation,
            }
            | Command::Ack {
                connection,
                operation,
                ..
            }
            | Command::Cancel {
                connection,
                operation,
            } => {
                let op = self.operations.get_mut(operation).ok_or("NOT_FOUND")?;
                if op.connection != *connection {
                    return Err("FORBIDDEN".into());
                }
                if matches!(command, Command::Cancel { .. }) {
                    if op.state != "delivered" {
                        op.state = "cancelled".into();
                    }
                    return Ok(json!({"state":op.state}));
                }
                if self.sessions[&op.session].turn != op.turn {
                    return Err("TASK_CHANGED".into());
                }
                // Ticket TTL no longer applies after consumption. The operation
                // uses its original deadline, never the time of a retry.
                if op.state != "delivered"
                    && now.duration_since(op.created) >= Duration::from_secs(240)
                {
                    op.state = "expired".into();
                }
                if op.state == "cancelled" {
                    return Err("CANCELLED".into());
                }
                if op.state == "expired" {
                    return Err("REQUEST_EXPIRED".into());
                }
                if op.state == "failed" {
                    return Err("DELIVERY_UNCONFIRMED".into());
                }
                if matches!(command, Command::Prepare { .. }) {
                    if op.state == "user_decided" {
                        op.lease = Some((id(), now));
                        op.state = "delivery_prepared".into();
                    } else if op.state != "delivery_prepared" && op.state != "delivered" {
                        return Err("REQUEST_CONFLICT".into());
                    }
                    if op.state == "delivery_prepared"
                        && now.duration_since(op.lease.as_ref().unwrap().1) >= LEASE_TTL
                    {
                        return Err("DELIVERY_UNCONFIRMED".into());
                    }
                    return Ok(
                        json!({"operationId":operation,"lease":op.lease.as_ref().map(|l| &l.0),"payload":op.args["payload"],"state":op.state}),
                    );
                }
                if let Command::Ack { lease, .. } = command {
                    let Some((expected, started)) = &op.lease else {
                        return Err("DELIVERY_UNCONFIRMED".into());
                    };
                    if expected != lease
                        || (op.state != "delivered" && now.duration_since(*started) >= LEASE_TTL)
                    {
                        return Err("DELIVERY_UNCONFIRMED".into());
                    }
                    if op.state != "delivery_prepared" && op.state != "delivered" {
                        return Err("REQUEST_CONFLICT".into());
                    }
                    op.state = "delivered".into();
                    if op.delivery {
                        self.sessions.get_mut(&op.session).unwrap().delivered = true;
                    }
                }
                Ok(json!({"state":op.state}))
            }
            Command::Inspect => Ok(
                json!({"appEpoch":self.epoch,"sessions":self.sessions.iter().map(|(k,s)| json!({"key":k,"turn":s.turn,"protected":s.protected,"probeDeliveryConfirmed":s.delivered})).collect::<Vec<_>>(),"operations":self.operations.iter().map(|(k,o)| json!({"operationId":k,"session":o.session,"state":o.state})).collect::<Vec<_>>()}),
            ),
        }
    }
}

pub fn atomic_new(path: &Path, value: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_IPC {
        return Err("PAYLOAD_TOO_LARGE".into());
    }
    let temp = path.with_extension(format!("{}.tmp", id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        drop(file);
        // hard_link atomically publishes without replacing an existing immutable
        // reply on either Windows or Unix; the temp and destination share a volume.
        fs::hard_link(&temp, path).map_err(|e| e.to_string())?;
        Ok(())
    })();
    let _ = fs::remove_file(temp);
    result
}

fn read_json(path: &Path) -> Result<Value, String> {
    strict_json(
        &read_bounded(File::open(path).map_err(|e| e.to_string())?, MAX_IPC)?,
        MAX_IPC,
    )
}

pub fn assert_run(root: &Path) -> Result<(), String> {
    let manifest = read_json(&root.join("probe-run.json"))?;
    if manifest["kind"] != "codecraft-trae-p0" {
        return Err("INVALID_ARGUMENT: not a probe run".into());
    }
    // All mutable paths must remain in this disposable run; no junction escape.
    let canonical = root.canonicalize().map_err(|e| e.to_string())?;
    for dir in [
        "inbox",
        "processing",
        "replies",
        "journal",
        "events",
        "project",
    ] {
        let target = root.join(dir).canonicalize().map_err(|e| e.to_string())?;
        if !target.starts_with(&canonical) {
            return Err("INVALID_ARGUMENT: probe directory escapes run".into());
        }
    }
    Ok(())
}

#[derive(Clone)]
pub struct Client {
    pub root: PathBuf,
    pub epoch: String,
}
impl Client {
    pub fn connect(root: &Path) -> Result<Self, String> {
        assert_run(root)?;
        let status = read_json(&root.join("heartbeat.json"))?;
        let age = wall_ms()
            .checked_sub(status["updatedAtMs"].as_u64().ok_or("BRIDGE_UNAVAILABLE")?)
            .ok_or("BRIDGE_UNAVAILABLE")?;
        if age > 10_000 {
            return Err("BRIDGE_UNAVAILABLE: stale heartbeat".into());
        }
        Ok(Self {
            root: root.into(),
            epoch: status["appEpoch"]
                .as_str()
                .ok_or("BRIDGE_UNAVAILABLE")?
                .into(),
        })
    }
    pub fn call(&self, command: Command) -> Result<Value, String> {
        self.call_with_id(command, &id())
    }
    pub fn call_with_id(&self, command: Command, command_id: &str) -> Result<Value, String> {
        Uuid::parse_str(command_id).map_err(|_| "INVALID_ARGUMENT: command UUID")?;
        let current = Self::connect(&self.root)?;
        if current.epoch != self.epoch {
            return Err("REQUEST_EXPIRED: old app epoch".into());
        }
        let control = command.control();
        let envelope = Envelope {
            message_id: id(),
            command_id: command_id.into(),
            app_epoch: self.epoch.clone(),
            command,
        };
        let value = serde_json::to_value(&envelope).map_err(|e| e.to_string())?;
        let bytes = serde_json::to_vec(&value).map_err(|e| e.to_string())?.len();
        if bytes > MAX_IPC {
            return Err("PAYLOAD_TOO_LARGE".into());
        }
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.root.join("queue.lock"))
            .map_err(|e| e.to_string())?;
        let started = Instant::now();
        while lock.try_lock_exclusive().is_err() {
            if started.elapsed() >= Duration::from_secs(3) {
                return Err("QUEUE_FULL: queue lock timeout".into());
            }
            thread::sleep(Duration::from_millis(10));
        }
        let mut count = 0usize;
        let mut size = bytes as u64;
        for folder in ["inbox", "processing"] {
            for file in fs::read_dir(self.root.join(folder)).map_err(|e| e.to_string())? {
                let file = file.map_err(|e| e.to_string())?;
                count += 1;
                size += file.metadata().map_err(|e| e.to_string())?.len();
            }
        }
        if count >= if control { 1024 } else { 896 }
            || size
                > if control {
                    64 * 1024 * 1024
                } else {
                    56 * 1024 * 1024
                }
        {
            return Err("QUEUE_FULL".into());
        }
        atomic_new(
            &self
                .root
                .join("inbox")
                .join(format!("{}.json", envelope.message_id)),
            &value,
        )?;
        FileExt::unlock(&lock).map_err(|e| e.to_string())?;
        let path = self.root.join("replies").join(format!("{command_id}.json"));
        let started = Instant::now();
        loop {
            if path.exists() {
                let reply = read_json(&path)?;
                if reply["appEpoch"] != self.epoch
                    || reply["bodyHash"]
                        != hash(
                            &serde_json::to_value(&envelope.command).map_err(|e| e.to_string())?,
                        )?
                {
                    return Err("IDEMPOTENCY_CONFLICT".into());
                }
                if let Some(error) = reply["error"].as_str() {
                    return Err(error.into());
                }
                return Ok(reply["result"].clone());
            }
            if started.elapsed() >= Duration::from_secs(3) {
                return Err("BRIDGE_UNAVAILABLE: control timeout".into());
            }
            thread::sleep(POLL);
        }
    }
}

pub fn coordinate(root: &Path) -> Result<(), String> {
    assert_run(root)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("store.lock"))
        .map_err(|e| e.to_string())?;
    lock.try_lock_exclusive()
        .map_err(|_| "STATE_UNAVAILABLE: coordinator already running")?;
    let mut state = State::new(root);
    let mut journal = OpenOptions::new()
        .append(true)
        .create_new(true)
        .open(root.join("journal").join(format!("{}.jsonl", state.epoch)))
        .map_err(|e| e.to_string())?;
    let mut commands = HashMap::<String, (String, Option<Value>, Instant)>::new();
    let mut heartbeat = Instant::now() - Duration::from_secs(3);
    loop {
        let mut expiring = state.clone();
        if expiring.expire(Instant::now()) {
            writeln!(
                journal,
                "{}",
                json!({"kind":"deadline_expiry","atMs":wall_ms()})
            )
            .and_then(|_| journal.sync_all())
            .map_err(|e| format!("STATE_UNAVAILABLE: {e}"))?;
            state = expiring;
        }
        if heartbeat.elapsed() >= Duration::from_secs(2) {
            let path = root.join("heartbeat.json");
            let temporary = root.join(format!("heartbeat-{}.tmp", id()));
            atomic_new(
                &temporary,
                &json!({"appEpoch":state.epoch,"updatedAtMs":wall_ms(),"healthy":true}),
            )?;
            // heartbeat isn't an authorization channel; readers encountering the
            // Windows replacement gap fail closed and can reconnect.
            if path.exists() {
                fs::remove_file(&path).map_err(|e| e.to_string())?;
            }
            fs::rename(temporary, path).map_err(|e| e.to_string())?;
            heartbeat = Instant::now();
        }
        for folder in ["processing", "inbox"] {
            let mut paths = fs::read_dir(root.join(folder))
                .map_err(|e| e.to_string())?
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "json"))
                .collect::<Vec<_>>();
            paths.sort();
            for path in paths {
                let processing = root
                    .join("processing")
                    .join(path.file_name().ok_or("INVALID_ARGUMENT")?);
                if folder == "inbox" {
                    fs::rename(&path, &processing).map_err(|e| e.to_string())?;
                }
                let parsed = read_json(&processing)
                    .and_then(|v| serde_json::from_value::<Envelope>(v).map_err(|e| e.to_string()));
                let envelope = match parsed {
                    Ok(e) => e,
                    Err(_) => {
                        fs::remove_file(processing).map_err(|e| e.to_string())?;
                        continue;
                    }
                };
                if Uuid::parse_str(&envelope.command_id).is_err()
                    || Uuid::parse_str(&envelope.message_id).is_err()
                {
                    fs::remove_file(processing).map_err(|e| e.to_string())?;
                    continue;
                }
                let body_hash =
                    hash(&serde_json::to_value(&envelope.command).map_err(|e| e.to_string())?)?;
                let now = Instant::now();
                let reply_path = root
                    .join("replies")
                    .join(format!("{}.json", envelope.command_id));
                if let Some((old_hash, reply, _)) = commands.get(&envelope.command_id) {
                    // A conflicting immutable reply is never overwritten. The
                    // client compares bodyHash and receives IDEMPOTENCY_CONFLICT.
                    if old_hash == &body_hash && !reply_path.exists() {
                        let expired = json!({"appEpoch":state.epoch,"bodyHash":body_hash,"result":null,"error":"REQUEST_EXPIRED"});
                        atomic_new(&reply_path, reply.as_ref().unwrap_or(&expired))?;
                    }
                } else {
                    let mut next = state.clone();
                    let outcome = if envelope.app_epoch != state.epoch {
                        Err("REQUEST_EXPIRED: old app epoch".into())
                    } else if commands.len() >= 8192 {
                        Err("QUEUE_FULL: restart probe coordinator".into())
                    } else {
                        next.apply(&envelope.command, now)
                    };
                    // Invalidating events still commit their conservative state
                    // changes even when a consumed launch marker is rejected.
                    let reply = json!({"appEpoch":state.epoch,"bodyHash":body_hash,"result":outcome.as_ref().ok(),"error":outcome.as_ref().err()});
                    if journal.metadata().map_err(|e| e.to_string())?.len() >= 64 * 1024 * 1024 {
                        return Err("STATE_UNAVAILABLE: P0 journal full".into());
                    }
                    let record = json!({"commandId":envelope.command_id,"bodyHash":body_hash,"replyHash":hash(&reply)?,"accepted":outcome.is_ok(),"atMs":wall_ms()});
                    writeln!(journal, "{record}")
                        .and_then(|_| journal.sync_all())
                        .map_err(|e| format!("STATE_UNAVAILABLE: {e}"))?;
                    state = next;
                    if commands.len() < 8192 {
                        commands.insert(
                            envelope.command_id.clone(),
                            (body_hash, Some(reply.clone()), now),
                        );
                    }
                    if !reply_path.exists() {
                        atomic_new(&reply_path, &reply)?;
                    }
                }
                fs::remove_file(processing).map_err(|e| e.to_string())?;
            }
        }
        // P0 is a bounded, disposable experiment. No successful response is
        // reconstructed from wall-clock time or a previous epoch's journal.
        for (command, (_, reply, at)) in &mut commands {
            if at.elapsed() >= CACHE_TTL && reply.is_some() {
                let _ = fs::remove_file(root.join("replies").join(format!("{command}.json")));
                // Retain the tombstone. Expiration must not turn an old command
                // into a new logical operation, even when its cache is gone.
                *reply = None;
            }
        }
        thread::sleep(POLL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn native(call: &str) -> Identity {
        Identity {
            host: "host:created".into(),
            session: "session-a".into(),
            root: "root".into(),
            call: call.into(),
        }
    }
    fn register(s: &mut State, now: Instant, identity: Identity, tool: &str) -> String {
        s.apply(
            &Command::Register {
                identity,
                tool: tool.into(),
                arguments: json!({"payload":{"nested":["中文",null,true]}}),
            },
            now,
        )
        .unwrap()["bridgeTicket"]
            .as_str()
            .unwrap()
            .into()
    }
    fn consume(
        s: &mut State,
        now: Instant,
        ticket: &str,
        connection: &str,
        tool: &str,
    ) -> Result<Value, String> {
        s.apply(
            &Command::Consume {
                connection: connection.into(),
                ticket: ticket.into(),
                tool: tool.into(),
                arguments: json!({"payload":{"nested":["中文",null,true]}}),
            },
            now,
        )
    }
    #[test]
    fn ticket_retries_remain_in_one_operation_and_connection() {
        let mut s = State::new(Path::new("."));
        let now = Instant::now();
        let t = register(&mut s, now, native("call-a"), crate::protocol::ECHO_TOOL);
        assert_eq!(
            t,
            register(&mut s, now, native("call-a"), crate::protocol::ECHO_TOOL)
        );
        let first = consume(&mut s, now, &t, "connection-a", crate::protocol::ECHO_TOOL).unwrap();
        let replay = consume(&mut s, now, &t, "connection-a", crate::protocol::ECHO_TOOL).unwrap();
        assert_eq!(first["operationId"], replay["operationId"]);
        assert!(consume(&mut s, now, &t, "connection-b", crate::protocol::ECHO_TOOL).is_err());
        assert!(consume(
            &mut s,
            now,
            &t,
            "connection-a",
            crate::protocol::DELIVERY_TOOL
        )
        .is_err());
        assert_eq!(s.operations.len(), 1);
    }
    #[test]
    fn changed_arguments_and_old_turns_cannot_rebind_native_calls() {
        let mut s = State::new(Path::new("."));
        let now = Instant::now();
        let identity = native("call-a");
        let ticket = register(&mut s, now, identity.clone(), crate::protocol::ECHO_TOOL);
        assert!(s
            .apply(
                &Command::Register {
                    identity: identity.clone(),
                    tool: crate::protocol::ECHO_TOOL.into(),
                    arguments: json!({"payload":false})
                },
                now
            )
            .unwrap_err()
            .starts_with("IDEMPOTENCY_CONFLICT"));
        s.apply(
            &Command::Observe {
                identity,
                event: "UserPromptSubmit".into(),
                prompt: Some("continue".into()),
            },
            now,
        )
        .unwrap();
        assert_eq!(
            consume(&mut s, now, &ticket, "c", crate::protocol::ECHO_TOOL).unwrap_err(),
            "TASK_CHANGED"
        );
    }
    #[test]
    fn ticket_expiry_does_not_extend_and_consumed_ticket_uses_operation_deadline() {
        let mut s = State::new(Path::new("."));
        let now = Instant::now();
        let ticket = register(&mut s, now, native("a"), crate::protocol::ECHO_TOOL);
        assert_eq!(
            consume(
                &mut s,
                now + TICKET_TTL,
                &ticket,
                "c",
                crate::protocol::ECHO_TOOL
            )
            .unwrap_err(),
            "TICKET_EXPIRED"
        );
        let op = consume(
            &mut s,
            now + Duration::from_secs(599),
            &ticket,
            "c",
            crate::protocol::ECHO_TOOL,
        )
        .unwrap()["operationId"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(s
            .apply(
                &Command::Poll {
                    connection: "c".into(),
                    operation: op
                },
                now + Duration::from_secs(601)
            )
            .is_ok());
    }
    #[test]
    fn delivery_requires_current_lease_and_cancellation_wins() {
        for cancel in [false, true] {
            let mut s = State::new(Path::new("."));
            let now = Instant::now();
            let t = register(&mut s, now, native("a"), crate::protocol::DELIVERY_TOOL);
            let op = consume(&mut s, now, &t, "c", crate::protocol::DELIVERY_TOOL).unwrap()
                ["operationId"]
                .as_str()
                .unwrap()
                .to_string();
            s.apply(
                &Command::Release {
                    operation: op.clone(),
                },
                now,
            )
            .unwrap();
            assert!(!s.sessions.values().next().unwrap().delivered);
            let lease = s
                .apply(
                    &Command::Prepare {
                        connection: "c".into(),
                        operation: op.clone(),
                    },
                    now,
                )
                .unwrap()["lease"]
                .as_str()
                .unwrap()
                .to_string();
            if cancel {
                s.apply(
                    &Command::Cancel {
                        connection: "c".into(),
                        operation: op.clone(),
                    },
                    now,
                )
                .unwrap();
            }
            let result = s.apply(
                &Command::Ack {
                    connection: "c".into(),
                    operation: op,
                    lease,
                },
                now + if cancel { Duration::ZERO } else { LEASE_TTL },
            );
            assert!(result.is_err());
            assert!(!s.sessions.values().next().unwrap().delivered);
        }
    }
    #[test]
    fn sessions_are_isolated_and_stop_invalidates_only_its_session() {
        let mut s = State::new(Path::new("."));
        let now = Instant::now();
        let a = native("call");
        let mut b = a.clone();
        b.session = "session-b".into();
        let ta = register(&mut s, now, a.clone(), crate::protocol::ECHO_TOOL);
        let tb = register(&mut s, now, b, crate::protocol::ECHO_TOOL);
        assert_ne!(ta, tb);
        s.apply(
            &Command::Observe {
                identity: a,
                event: "Stop".into(),
                prompt: None,
            },
            now,
        )
        .unwrap();
        assert!(consume(&mut s, now, &ta, "c", crate::protocol::ECHO_TOOL).is_err());
        assert!(consume(&mut s, now, &tb, "c", crate::protocol::ECHO_TOOL).is_ok());
    }
    #[test]
    fn replayed_launch_code_invalidates_its_original_session() {
        let mut s = State::new(Path::new("."));
        let now = Instant::now();
        let a = native("a");
        let key = a.session_key().unwrap();
        s.sessions.insert(
            key.clone(),
            Session {
                turn: 1,
                protected: true,
                delivered: true,
            },
        );
        s.armed = Some(Armed {
            code: "code".into(),
            created: now,
            plan_id: id(),
            path: "plan.md".into(),
            bound: Some(key.clone()),
        });
        let mut b = a;
        b.session = "session-b".into();
        assert!(s
            .apply(
                &Command::Observe {
                    identity: b,
                    event: "UserPromptSubmit".into(),
                    prompt: Some("[[CODECRAFT_TASK:code]]".into())
                },
                now
            )
            .is_err());
        assert!(!s.sessions[&key].delivered);
        assert_eq!(s.sessions[&key].turn, 2);
    }
    #[test]
    fn abandoned_operations_expire_without_a_polling_client() {
        let mut s = State::new(Path::new("."));
        let now = Instant::now();
        let t = register(&mut s, now, native("a"), crate::protocol::DELIVERY_TOOL);
        let op = consume(&mut s, now, &t, "c", crate::protocol::DELIVERY_TOOL).unwrap()
            ["operationId"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(s.expire(now + Duration::from_secs(240)));
        assert_eq!(s.operations[&op].state, "expired");
    }
}
