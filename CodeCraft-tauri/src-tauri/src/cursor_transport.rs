//! Bounded, epoch-scoped file transport. This is transport only: payloads are
//! untrusted adapter input and cannot deserialize a privileged workflow command.
//! Connection permits are issued by trusted Rust code after adapter validation.
use crate::{
    files, protocol,
    schema::{self, Result},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs::{self, File},
    path::{Path, PathBuf},
    time::Instant,
};
use uuid::Uuid;

const MAX_CONNECTIONS: usize = 32;
const MAX_QUEUE: usize = 128;
const MAX_RECEIPTS: usize = 1024;
const FAST_COMMAND_MS: u64 = 3_000;
const HEARTBEAT_STALE_MS: u64 = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Role {
    Hook,
    Mcp,
    Native,
}

/// Deliberately no Debug or Serialize, preventing accidental credential logs.
pub struct Permit {
    root: PathBuf,
    installation_id: String,
    app_epoch: String,
    config_revision: u64,
    connection_id: String,
    credential: String,
    role: Role,
    broker_ms: u64,
    received: Instant,
    heartbeat: Heartbeat,
}
impl Permit {
    pub fn connection_id(&self) -> &str {
        &self.connection_id
    }

    /// Only the authenticated local bootstrap writes this secret, into its
    /// current-user directory. It is never part of a snapshot or tool result.
    pub(crate) fn write_bootstrap(&self, path: &Path, nonce: &str) -> Result<()> {
        files::atomic_json(
            path,
            &serde_json::json!({
                "nonce":nonce,"installationId":self.installation_id,"appEpoch":self.app_epoch,
                "configRevision":self.config_revision,"connectionId":self.connection_id,
                "credential":self.credential,"role":self.role,"heartbeat":self.heartbeat
            }),
            false,
        )
    }

    pub(crate) fn read_bootstrap(
        root: &Path,
        path: &Path,
        nonce: &str,
        epoch: &str,
        role: Role,
    ) -> Result<Self> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Bootstrap {
            nonce: String,
            installation_id: String,
            app_epoch: String,
            config_revision: u64,
            connection_id: String,
            credential: String,
            role: Role,
            heartbeat: Heartbeat,
        }
        let wire: Bootstrap =
            serde_json::from_value(files::read_json(path)?).map_err(|_| "INVALID_HANDSHAKE")?;
        if wire.nonce != nonce
            || wire.app_epoch != epoch
            || wire.role != role
            || !canonical_uuid(&wire.connection_id)
            || !canonical_uuid(&wire.app_epoch)
            || wire.credential.len() != 64
            || !wire.credential.bytes().all(|c| c.is_ascii_hexdigit())
            || wire.heartbeat.installation_id != wire.installation_id
            || wire.heartbeat.app_epoch != wire.app_epoch
            || wire.heartbeat.config_revision != wire.config_revision
        {
            return Err("INVALID_HANDSHAKE");
        }
        Liveness::after_handshake(wire.heartbeat.clone(), 0)?;
        fs::remove_file(path).map_err(|_| "CHANNEL_CLEANUP_FAILED")?;
        Ok(Self {
            root: files::child(root, &wire.app_epoch)?,
            installation_id: wire.installation_id,
            app_epoch: wire.app_epoch,
            config_revision: wire.config_revision,
            connection_id: wire.connection_id,
            credential: wire.credential,
            role,
            broker_ms: wire.heartbeat.broker_ms,
            received: Instant::now(),
            heartbeat: wire.heartbeat,
        })
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Envelope {
    schema_version: u32,
    installation_id: String,
    app_epoch: String,
    config_revision: u64,
    connection_id: String,
    message_id: String,
    sequence: u64,
    credential: String,
    role: Role,
    deadline_ms: u64,
    payload: AdapterInput,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(
    tag = "source",
    content = "data",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum AdapterInput {
    Hook(Value),
    Mcp(Value),
    Native(Value),
}
impl AdapterInput {
    fn role(&self) -> Role {
        match self {
            Self::Hook(_) => Role::Hook,
            Self::Mcp(_) => Role::Mcp,
            Self::Native(_) => Role::Native,
        }
    }
}

/// A single delivery has one trusted owner. It cannot be deserialized from a
/// file, cloned, or manufactured by an adapter. Receiving it confers no native
/// or workflow authorization: the next layer must normalize/verify its input.
pub struct Delivery {
    message_id: String,
    connection_id: String,
    app_epoch: String,
    digest: String,
    deadline_ms: u64,
    pub input: AdapterInput,
}
impl Delivery {
    pub fn connection_id(&self) -> &str {
        &self.connection_id
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum ReceiptState {
    Claimed,
    ReplyClaimed,
    ReplyWritten,
    OutcomeUnknown,
    Expired,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Receipt {
    connection_id: String,
    digest: String,
    state: ReceiptState,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Connection {
    role: Role,
    credential_hash: String,
    last_sequence: u64,
    closed: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct State {
    schema_version: u32,
    installation_id: String,
    app_epoch: String,
    config_revision: u64,
    connections: BTreeMap<String, Connection>,
    receipts: BTreeMap<String, Receipt>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Heartbeat {
    pub schema_version: u32,
    pub installation_id: String,
    pub app_epoch: String,
    pub config_revision: u64,
    pub sequence: u64,
    pub broker_ms: u64,
}

/// A client begins this tracker only after its live handshake, using local
/// monotonic elapsed time. Rereading an unchanged heartbeat never renews it.
pub struct Liveness {
    heartbeat: Heartbeat,
    observed_ms: u64,
    handshake_local_ms: u64,
    handshake_broker_ms: u64,
}
impl Liveness {
    pub fn after_handshake(heartbeat: Heartbeat, local_ms: u64) -> Result<Self> {
        if heartbeat.schema_version != 1
            || !canonical_uuid(&heartbeat.app_epoch)
            || heartbeat.sequence == 0
        {
            return Err("INVALID_HEARTBEAT");
        }
        Ok(Self {
            handshake_local_ms: local_ms,
            handshake_broker_ms: heartbeat.broker_ms,
            heartbeat,
            observed_ms: local_ms,
        })
    }
    pub fn observe(&mut self, next: &Heartbeat, local_ms: u64) -> Result<()> {
        if local_ms < self.observed_ms {
            return Err("CLOCK_REGRESSED");
        }
        if next.app_epoch != self.heartbeat.app_epoch {
            return Err("APP_RESTARTED");
        }
        if next.schema_version != 1
            || next.installation_id != self.heartbeat.installation_id
            || next.config_revision != self.heartbeat.config_revision
        {
            return Err("CONFIGURATION_CHANGED");
        }
        if next.sequence < self.heartbeat.sequence || next.broker_ms < self.heartbeat.broker_ms {
            return Err("STALE_HEARTBEAT");
        }
        let expected = self
            .handshake_broker_ms
            .saturating_add(local_ms.saturating_sub(self.handshake_local_ms));
        if expected.saturating_sub(next.broker_ms) >= HEARTBEAT_STALE_MS {
            return Err("BROKER_OFFLINE");
        }
        if next.sequence > self.heartbeat.sequence {
            self.heartbeat = next.clone();
            self.observed_ms = local_ms;
        }
        if local_ms.saturating_sub(self.observed_ms) >= HEARTBEAT_STALE_MS {
            return Err("BROKER_OFFLINE");
        }
        Ok(())
    }
}

pub struct Hub {
    root: PathBuf,
    epoch_root: PathBuf,
    _lock: File,
    state: State,
    started: Instant,
    heartbeat_sequence: u64,
    unavailable: bool,
}

fn canonical_uuid(value: &str) -> bool {
    Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value)
}
fn queue_size(root: &Path) -> Result<usize> {
    let mut count = 0;
    for name in ["inbox", "processing", "replies"] {
        let directory = files::child(root, name)?;
        for entry in fs::read_dir(directory).map_err(|_| "CHANNEL_UNAVAILABLE")? {
            entry.map_err(|_| "CHANNEL_UNAVAILABLE")?;
            count += 1;
            if count >= MAX_QUEUE {
                return Err("CHANNEL_BACKPRESSURE");
            }
        }
    }
    Ok(count)
}

impl Hub {
    /// Epoch must be the already durably published workflow broker epoch.
    /// Never resumes a previous epoch's queue, connection or claimed reply.
    pub fn open(
        root: &Path,
        installation_id: &str,
        app_epoch: &str,
        config_revision: u64,
    ) -> Result<Self> {
        if !schema::valid_id(installation_id) || !canonical_uuid(app_epoch) || config_revision == 0
        {
            return Err("INVALID_HANDSHAKE");
        }
        let root = files::private_directory(root)?;
        let lock = files::lock(&files::child(&root, "transport.lock")?)?;
        let epoch_root = files::child(&root, app_epoch)?;
        if epoch_root.exists() {
            return Err("EPOCH_ALREADY_USED");
        }
        fs::create_dir(&epoch_root).map_err(|_| "CHANNEL_CREATE_FAILED")?;
        for name in ["inbox", "processing", "replies"] {
            fs::create_dir(files::child(&epoch_root, name)?)
                .map_err(|_| "CHANNEL_CREATE_FAILED")?;
        }
        let state = State {
            schema_version: 1,
            installation_id: installation_id.into(),
            app_epoch: app_epoch.into(),
            config_revision,
            connections: BTreeMap::new(),
            receipts: BTreeMap::new(),
        };
        let mut hub = Self {
            root,
            epoch_root,
            _lock: lock,
            state,
            started: Instant::now(),
            heartbeat_sequence: 0,
            unavailable: false,
        };
        hub.persist()?;
        hub.heartbeat()?;
        hub.prune_retired_epochs()?;
        Ok(hub)
    }
    fn prune_retired_epochs(&self) -> Result<()> {
        let mut count = 0;
        for entry in fs::read_dir(&self.root).map_err(|_| "CHANNEL_UNAVAILABLE")? {
            let entry = entry.map_err(|_| "CHANNEL_UNAVAILABLE")?;
            let name = entry.file_name().to_string_lossy().to_string();
            if !canonical_uuid(&name) || name == self.state.app_epoch {
                continue;
            }
            count += 1;
            if count > 32 {
                return Err("RETIRED_CHANNEL_BACKPRESSURE");
            }
            let epoch = files::child(&self.root, &name)?;
            // Only the known transport layout is eligible, with no recursive
            // traversal through links or removal of unknown files.
            let mut paths = Vec::new();
            for directory in ["inbox", "processing", "replies"] {
                let path = files::child(&epoch, directory)?;
                if !path.exists() {
                    continue;
                }
                for child in fs::read_dir(&path)
                    .map_err(|_| "CHANNEL_UNAVAILABLE")?
                    .take(MAX_QUEUE + 1)
                {
                    let child = child.map_err(|_| "CHANNEL_UNAVAILABLE")?;
                    let file = child.file_name().to_string_lossy().to_string();
                    let Some(id) = file
                        .strip_suffix(".json")
                        .or_else(|| file.strip_suffix(".tmp"))
                    else {
                        return Err("RETIRED_CHANNEL_CONFLICT");
                    };
                    if !canonical_uuid(id) || paths.len() >= MAX_QUEUE {
                        return Err("RETIRED_CHANNEL_CONFLICT");
                    }
                    let child = files::child(&path, &file)?;
                    if !child.is_file() {
                        return Err("RETIRED_CHANNEL_CONFLICT");
                    }
                    paths.push(child);
                }
            }
            let state = files::child(&epoch, "transport.json")?;
            if state.exists() {
                paths.push(state);
            }
            for path in paths {
                fs::remove_file(path).map_err(|_| "CHANNEL_CLEANUP_FAILED")?;
            }
            for directory in ["inbox", "processing", "replies"] {
                let path = files::child(&epoch, directory)?;
                if path.exists() {
                    let _ = fs::remove_dir(path);
                }
            }
            // An already-running stale writer may finish an atomic temporary
            // publication. Leave a nonempty directory for the next cleanup;
            // new-epoch heartbeat checks already prevent its admission.
            let _ = fs::remove_dir(epoch);
        }
        Ok(())
    }
    fn now_ms(&self) -> u64 {
        self.started.elapsed().as_millis().min(u64::MAX as u128) as u64
    }
    fn available(&self) -> Result<()> {
        if self.unavailable {
            Err("CHANNEL_UNAVAILABLE")
        } else {
            Ok(())
        }
    }
    fn persist(&mut self) -> Result<()> {
        let result = files::child(&self.epoch_root, "transport.json").and_then(|path| {
            files::atomic_json(
                &path,
                &serde_json::to_value(&self.state).map_err(|_| "CHANNEL_ENCODE_FAILED")?,
                true,
            )
        });
        if result.is_err() {
            self.unavailable = true;
        }
        result
    }
    pub fn heartbeat(&mut self) -> Result<Heartbeat> {
        self.available()?;
        self.heartbeat_sequence = self
            .heartbeat_sequence
            .checked_add(1)
            .ok_or("CHANNEL_COUNTER_OVERFLOW")?;
        let heartbeat = Heartbeat {
            schema_version: 1,
            installation_id: self.state.installation_id.clone(),
            app_epoch: self.state.app_epoch.clone(),
            config_revision: self.state.config_revision,
            sequence: self.heartbeat_sequence,
            broker_ms: self.now_ms(),
        };
        let result = files::child(&self.root, "heartbeat.json").and_then(|path| {
            files::atomic_json(
                &path,
                &serde_json::to_value(&heartbeat).map_err(|_| "CHANNEL_ENCODE_FAILED")?,
                true,
            )
        });
        if result.is_err() {
            self.unavailable = true;
        }
        result?;
        Ok(heartbeat)
    }
    /// Trusted process registration only. Role cannot be chosen by an inbox
    /// message. Actual executable/config/capability validation belongs to the
    /// adapter launcher; a permit is not evidence that Cursor supports a tool.
    pub fn authorize(&mut self, role: Role) -> Result<Permit> {
        self.available()?;
        if self.state.connections.len() >= MAX_CONNECTIONS {
            return Err("CONNECTION_LIMIT");
        }
        let connection_id = Uuid::new_v4().to_string();
        let credential = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        self.state.connections.insert(
            connection_id.clone(),
            Connection {
                role,
                credential_hash: protocol::sha256(credential.as_bytes()),
                last_sequence: 0,
                closed: false,
            },
        );
        self.persist()?;
        let heartbeat = self.heartbeat()?;
        Ok(Permit {
            root: self.epoch_root.clone(),
            installation_id: self.state.installation_id.clone(),
            app_epoch: self.state.app_epoch.clone(),
            config_revision: self.state.config_revision,
            connection_id,
            credential,
            role,
            broker_ms: self.now_ms(),
            received: Instant::now(),
            heartbeat,
        })
    }
    pub fn close(&mut self, connection_id: &str) -> Result<()> {
        self.available()?;
        let messages = self
            .state
            .receipts
            .iter()
            .filter(|(_, receipt)| receipt.connection_id == connection_id)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        self.state
            .connections
            .remove(connection_id)
            .ok_or("CONNECTION_UNKNOWN")?;
        // A removed random connection ID can never be authorized again. Its
        // replay receipts can be discarded without allowing an old frame.
        self.state
            .receipts
            .retain(|_, receipt| receipt.connection_id != connection_id);
        self.persist()?;
        for id in messages {
            for directory in ["processing", "replies"] {
                let path = files::child(&self.epoch_root, &format!("{directory}/{id}.json"))?;
                match fs::remove_file(path) {
                    Ok(()) => (),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                    Err(_) => return Err("CHANNEL_CLEANUP_FAILED"),
                }
            }
        }
        Ok(())
    }
    pub fn take(&mut self) -> Result<Option<Delivery>> {
        self.available()?;
        if self.state.receipts.len() >= 128 {
            let acknowledged = self
                .state
                .receipts
                .iter()
                .filter(|(id, r)| {
                    r.state == ReceiptState::ReplyWritten
                        && !self.epoch_root.join(format!("replies/{id}.json")).exists()
                })
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            if !acknowledged.is_empty() {
                for id in &acknowledged {
                    self.state.receipts.remove(id);
                }
                // The committed connection sequence remains a replay barrier.
                self.persist()?;
                for id in acknowledged {
                    let path = files::child(&self.epoch_root, &format!("processing/{id}.json"))?;
                    match fs::remove_file(path) {
                        Ok(()) => (),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                        Err(_) => return Err("CHANNEL_CLEANUP_FAILED"),
                    }
                }
            }
        }
        let inbox = files::child(&self.epoch_root, "inbox")?;
        let mut names = Vec::new();
        for entry in fs::read_dir(&inbox)
            .map_err(|_| "CHANNEL_UNAVAILABLE")?
            .take(MAX_QUEUE + 1)
        {
            let entry = entry.map_err(|_| "CHANNEL_UNAVAILABLE")?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| "INVALID_CHANNEL_FILE")?;
            // Atomic writers publish a sibling UUID.tmp before linking the
            // final UUID.json. It is not a malformed protocol message.
            if name.strip_suffix(".tmp").is_some_and(canonical_uuid) {
                continue;
            }
            let id = name.strip_suffix(".json").ok_or("INVALID_CHANNEL_FILE")?;
            if !canonical_uuid(id) {
                return Err("INVALID_CHANNEL_FILE");
            }
            names.push(name);
        }
        if names.len() > MAX_QUEUE {
            return Err("CHANNEL_BACKPRESSURE");
        }
        let mut candidates = Vec::new();
        for name in names {
            let path = files::child(&inbox, &name)?;
            let parsed = files::read_json(&path).and_then(|value| {
                serde_json::from_value::<Envelope>(value).map_err(|_| "INVALID_CHANNEL_MESSAGE")
            });
            let envelope = match parsed.and_then(|envelope| {
                if name != format!("{}.json", envelope.message_id)
                    || !canonical_uuid(&envelope.message_id)
                {
                    return Err("INVALID_CHANNEL_MESSAGE");
                }
                self.verify(&envelope)?;
                Ok(envelope)
            }) {
                Ok(envelope) => envelope,
                Err(error) => {
                    fs::remove_file(&path).map_err(|_| "CHANNEL_CLEANUP_FAILED")?;
                    return Err(error);
                }
            };
            candidates.push((name, envelope));
        }
        // UUID filenames have no arrival order. Drain each connection's frames
        // in its sender sequence so a random filename cannot skip valid frames.
        candidates.sort_by(|a, b| {
            (&a.1.connection_id, a.1.sequence).cmp(&(&b.1.connection_id, b.1.sequence))
        });
        let Some((name, envelope)) = candidates.into_iter().next() else {
            return Ok(None);
        };
        let path = files::child(&inbox, &name)?;
        let digest =
            protocol::sha256(&serde_jcs::to_vec(&envelope).map_err(|_| "INVALID_CHANNEL_MESSAGE")?);
        if let Some(receipt) = self.state.receipts.get(&envelope.message_id) {
            let same = receipt.digest == digest && receipt.connection_id == envelope.connection_id;
            fs::remove_file(path).map_err(|_| "CHANNEL_UNAVAILABLE")?;
            return Err(if same {
                "MESSAGE_ALREADY_PROCESSED"
            } else {
                "MESSAGE_ID_CONFLICT"
            });
        }
        if self.state.receipts.len() >= MAX_RECEIPTS {
            return Err("CHANNEL_BACKPRESSURE");
        }
        let connection = self
            .state
            .connections
            .get_mut(&envelope.connection_id)
            .ok_or("CONNECTION_UNKNOWN")?;
        if envelope.sequence <= connection.last_sequence {
            fs::remove_file(&path).map_err(|_| "CHANNEL_CLEANUP_FAILED")?;
            return Err("MESSAGE_SEQUENCE_REPLAY");
        }
        connection.last_sequence = envelope.sequence;
        self.state.receipts.insert(
            envelope.message_id.clone(),
            Receipt {
                connection_id: envelope.connection_id.clone(),
                digest: digest.clone(),
                state: ReceiptState::Claimed,
            },
        );
        // Durable claim precedes returning the untrusted input to a handler.
        // Failure after this point requires a new epoch, never redispatch.
        self.persist()?;
        let processing = files::child(&self.epoch_root, &format!("processing/{name}"))?;
        if processing.exists() || fs::rename(&path, processing).is_err() {
            self.unavailable = true;
            return Err("CHANNEL_CLAIM_FAILED");
        }
        Ok(Some(Delivery {
            message_id: envelope.message_id,
            connection_id: envelope.connection_id,
            app_epoch: envelope.app_epoch,
            digest,
            deadline_ms: envelope.deadline_ms,
            input: envelope.payload,
        }))
    }
    fn verify(&self, envelope: &Envelope) -> Result<()> {
        if envelope.schema_version != 1 || envelope.installation_id != self.state.installation_id {
            return Err("INVALID_HANDSHAKE");
        }
        if envelope.app_epoch != self.state.app_epoch {
            return Err("APP_RESTARTED");
        }
        if envelope.config_revision != self.state.config_revision {
            return Err("CONFIGURATION_CHANGED");
        }
        let connection = self
            .state
            .connections
            .get(&envelope.connection_id)
            .ok_or("CONNECTION_UNKNOWN")?;
        if connection.closed {
            return Err("CONNECTION_CLOSED");
        }
        if connection.role != envelope.role
            || envelope.payload.role() != envelope.role
            || protocol::sha256(envelope.credential.as_bytes()) != connection.credential_hash
        {
            return Err("CONNECTION_IDENTITY_MISMATCH");
        }
        let now = self.now_ms();
        if envelope.deadline_ms <= now || envelope.deadline_ms > now.saturating_add(FAST_COMMAND_MS)
        {
            return Err("MESSAGE_EXPIRED");
        }
        Ok(())
    }
    /// No persisted result payload, reply replay, or retry after an uncertain
    /// publish. Business result ownership/leases are an additional broker check.
    pub fn reply(&mut self, delivery: Delivery, result: &Value) -> Result<()> {
        self.available()?;
        if delivery.app_epoch != self.state.app_epoch {
            return Err("APP_RESTARTED");
        }
        let expired = self.now_ms() >= delivery.deadline_ms;
        let connection = self
            .state
            .connections
            .get(&delivery.connection_id)
            .ok_or("CONNECTION_UNKNOWN")?;
        if connection.closed {
            return Err("CONNECTION_CLOSED");
        }
        let receipt = self
            .state
            .receipts
            .get_mut(&delivery.message_id)
            .ok_or("MESSAGE_UNKNOWN")?;
        if receipt.connection_id != delivery.connection_id
            || receipt.digest != delivery.digest
            || receipt.state != ReceiptState::Claimed
        {
            return Err("REPLY_ALREADY_CLAIMED");
        }
        if expired {
            receipt.state = ReceiptState::Expired;
            self.persist()?;
            return Err("MESSAGE_EXPIRED");
        }
        receipt.state = ReceiptState::ReplyClaimed;
        self.persist()?;
        let path = files::child(
            &self.epoch_root,
            &format!("replies/{}.json", delivery.message_id),
        )?;
        let envelope = serde_json::json!({"schemaVersion":1,"installationId":self.state.installation_id,"appEpoch":self.state.app_epoch,"configRevision":self.state.config_revision,"connectionId":delivery.connection_id,"messageId":delivery.message_id,"result":result});
        let outcome = files::atomic_json(&path, &envelope, false);
        self.state
            .receipts
            .get_mut(&delivery.message_id)
            .ok_or("MESSAGE_UNKNOWN")?
            .state = if outcome.is_ok() {
            ReceiptState::ReplyWritten
        } else {
            ReceiptState::OutcomeUnknown
        };
        self.persist()?;
        outcome?;
        fs::remove_file(files::child(
            &self.epoch_root,
            &format!("processing/{}.json", delivery.message_id),
        )?)
        .map_err(|_| "CHANNEL_CLEANUP_FAILED")?;
        Ok(())
    }
}

pub struct Client {
    permit: Permit,
    sequence: u64,
    pending: BTreeMap<String, u64>,
    liveness: Liveness,
}
impl Client {
    pub fn new(permit: Permit) -> Self {
        let liveness = Liveness {
            heartbeat: permit.heartbeat.clone(),
            observed_ms: 0,
            handshake_local_ms: 0,
            handshake_broker_ms: permit.broker_ms,
        };
        Self {
            permit,
            sequence: 0,
            pending: BTreeMap::new(),
            liveness,
        }
    }
    fn check_live(&mut self) -> Result<()> {
        let root = self.permit.root.parent().ok_or("INVALID_HANDSHAKE")?;
        let heartbeat: Heartbeat =
            serde_json::from_value(files::read_json(&files::child(root, "heartbeat.json")?)?)
                .map_err(|_| "INVALID_HEARTBEAT")?;
        self.liveness.observe(
            &heartbeat,
            self.permit
                .received
                .elapsed()
                .as_millis()
                .min(u64::MAX as u128) as u64,
        )
    }
    pub fn submit(&mut self, payload: AdapterInput) -> Result<String> {
        self.check_live()?;
        if payload.role() != self.permit.role {
            return Err("CONNECTION_ROLE_MISMATCH");
        }
        if self.pending.len() >= 8 {
            return Err("CONNECTION_BACKPRESSURE");
        }
        let _queue_lock = files::lock(&files::child(&self.permit.root, "queue.lock")?)?;
        queue_size(&self.permit.root)?;
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or("CHANNEL_COUNTER_OVERFLOW")?;
        let message_id = Uuid::new_v4().to_string();
        let deadline_ms = self
            .permit
            .broker_ms
            .saturating_add(
                self.permit
                    .received
                    .elapsed()
                    .as_millis()
                    .min(u64::MAX as u128) as u64,
            )
            .saturating_add(FAST_COMMAND_MS);
        let envelope = Envelope {
            schema_version: 1,
            installation_id: self.permit.installation_id.clone(),
            app_epoch: self.permit.app_epoch.clone(),
            config_revision: self.permit.config_revision,
            connection_id: self.permit.connection_id.clone(),
            message_id: message_id.clone(),
            sequence: self.sequence,
            credential: self.permit.credential.clone(),
            role: self.permit.role,
            deadline_ms,
            payload,
        };
        files::atomic_json(
            &files::child(&self.permit.root, &format!("inbox/{message_id}.json"))?,
            &serde_json::to_value(envelope).map_err(|_| "CHANNEL_ENCODE_FAILED")?,
            false,
        )?;
        self.pending.insert(message_id.clone(), deadline_ms);
        Ok(message_id)
    }
    /// In-memory ownership and deletion before returning prevent a second
    /// stdout/native send from rereading the same result. If deletion is
    /// uncertain, no result is returned and the owner still cannot retry it.
    pub fn read_reply(&mut self, message_id: &str) -> Result<Option<Value>> {
        self.check_live()?;
        if !canonical_uuid(message_id) || !self.pending.contains_key(message_id) {
            return Err("REPLY_NOT_OWNED");
        }
        let now = self.permit.broker_ms.saturating_add(
            self.permit
                .received
                .elapsed()
                .as_millis()
                .min(u64::MAX as u128) as u64,
        );
        if self
            .pending
            .get(message_id)
            .is_some_and(|deadline| now >= *deadline)
        {
            self.pending.remove(message_id);
            return Err("MESSAGE_EXPIRED");
        }
        let path = files::child(&self.permit.root, &format!("replies/{message_id}.json"))?;
        if !path.exists() {
            return Ok(None);
        }
        let reply = files::read_json(&path)?;
        if reply["schemaVersion"] != 1
            || reply["appEpoch"] != self.permit.app_epoch
            || reply["installationId"] != self.permit.installation_id
            || reply["configRevision"] != self.permit.config_revision
            || reply["connectionId"] != self.permit.connection_id
            || reply["messageId"] != message_id
            || reply
                .as_object()
                .is_none_or(|object| object.len() != 7 || !object.contains_key("result"))
        {
            return Err("INVALID_CHANNEL_REPLY");
        }
        self.pending.remove(message_id);
        fs::remove_file(path).map_err(|_| "CHANNEL_CLEANUP_FAILED")?;
        Ok(Some(reply["result"].clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn acknowledged_poll_receipts_are_bounded_without_allowing_sequence_replay() {
        let mut hub = hub();
        let mut client = Client::new(hub.authorize(Role::Mcp).unwrap());
        let first = client
            .submit(AdapterInput::Mcp(json!({"op":"poll"})))
            .unwrap();
        let mut replay: Envelope =
            serde_json::from_slice(&fs::read(path(&client, &first)).unwrap()).unwrap();
        let delivery = hub.take().unwrap().unwrap();
        hub.reply(delivery, &json!({"status":"pending"})).unwrap();
        client.read_reply(&first).unwrap().unwrap();
        for _ in 0..130 {
            let id = client
                .submit(AdapterInput::Mcp(json!({"op":"poll"})))
                .unwrap();
            let delivery = hub.take().unwrap().unwrap();
            hub.reply(delivery, &json!({"status":"pending"})).unwrap();
            client.read_reply(&id).unwrap().unwrap();
        }
        assert!(hub.state.receipts.len() < 128);
        assert!(!hub.state.receipts.contains_key(&first));
        // Exercise the retained sequence barrier, even if slow disks/builds
        // take longer than the original frame's three-second deadline.
        replay.deadline_ms = hub.now_ms().saturating_add(FAST_COMMAND_MS);
        fs::write(path(&client, &first), serde_json::to_vec(&replay).unwrap()).unwrap();
        assert!(matches!(hub.take(), Err("MESSAGE_SEQUENCE_REPLAY")));
    }
    #[test]
    fn queued_frames_follow_sender_order_and_atomic_temporary_files_are_ignored() {
        let mut hub = hub();
        let mut client = Client::new(hub.authorize(Role::Mcp).unwrap());
        let temporary = client
            .permit
            .root
            .join("inbox")
            .join(format!("{}.tmp", Uuid::new_v4()));
        fs::write(&temporary, "partial").unwrap();
        for index in 0..8 {
            client
                .submit(AdapterInput::Mcp(json!({"id":index})))
                .unwrap();
        }
        for index in 0..8 {
            let delivery = hub.take().unwrap().unwrap();
            let AdapterInput::Mcp(value) = &delivery.input else {
                panic!()
            };
            assert_eq!(value["id"], index);
            hub.reply(delivery, &json!({"ok":true})).unwrap();
        }
        assert!(hub.take().unwrap().is_none());
    }
    #[test]
    fn claim_persistence_failure_never_dispatches_and_requires_a_new_epoch() {
        let mut hub = hub();
        let mut client = Client::new(hub.authorize(Role::Hook).unwrap());
        client.submit(AdapterInput::Hook(json!({}))).unwrap();
        let state = hub.epoch_root.join("transport.json");
        fs::remove_file(&state).unwrap();
        fs::create_dir(state).unwrap();
        assert!(hub.take().is_err());
        assert!(matches!(hub.take(), Err("CHANNEL_UNAVAILABLE")));
        assert!(hub.heartbeat().is_err());
    }
    #[test]
    fn an_unread_but_old_heartbeat_cannot_revive_a_dead_broker() {
        let mut hub = hub();
        let initial = hub.heartbeat().unwrap();
        let mut tracker = Liveness::after_handshake(initial.clone(), 0).unwrap();
        let mut stale = initial;
        stale.sequence += 1;
        stale.broker_ms += 2000;
        assert_eq!(tracker.observe(&stale, 12000), Err("BROKER_OFFLINE"));
    }
    fn root() -> PathBuf {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("runs")
            .join(format!("transport-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        root
    }
    fn hub() -> Hub {
        Hub::open(&root(), "installation", &Uuid::new_v4().to_string(), 1).unwrap()
    }
    fn path(client: &Client, id: &str) -> PathBuf {
        client.permit.root.join("inbox").join(format!("{id}.json"))
    }
    #[test]
    fn role_bound_messages_deliver_once_and_a_duplicate_never_replays_results() {
        let mut hub = hub();
        let mut client = Client::new(hub.authorize(Role::Mcp).unwrap());
        let id = client
            .submit(AdapterInput::Mcp(json!({"id":0,"method":"tools/call"})))
            .unwrap();
        let original = fs::read(path(&client, &id)).unwrap();
        let delivery = hub.take().unwrap().unwrap();
        hub.reply(delivery, &json!({"code":"TICKET_REQUIRED"}))
            .unwrap();
        assert_eq!(
            client.read_reply(&id).unwrap(),
            Some(json!({"code":"TICKET_REQUIRED"}))
        );
        assert_eq!(client.read_reply(&id), Err("REPLY_NOT_OWNED"));
        files::atomic_write(&path(&client, &id), &original, false).unwrap();
        assert!(matches!(hub.take(), Err("MESSAGE_ALREADY_PROCESSED")));
        assert!(hub.take().unwrap().is_none());
    }
    #[test]
    fn role_spoofing_epoch_changes_and_privileged_command_shapes_do_not_dispatch() {
        let mut hub = hub();
        let mut client = Client::new(hub.authorize(Role::Hook).unwrap());
        assert_eq!(
            client.submit(AdapterInput::Native(json!({}))),
            Err("CONNECTION_ROLE_MISMATCH")
        );
        let id = client
            .submit(AdapterInput::Hook(
                json!({"hook_event_name":"sessionStart"}),
            ))
            .unwrap();
        let mut value = files::read_json(&path(&client, &id)).unwrap();
        value["role"] = json!("native");
        files::atomic_json(&path(&client, &id), &value, true).unwrap();
        assert!(matches!(hub.take(), Err("CONNECTION_IDENTITY_MISMATCH")));
        value["role"] = json!("hook");
        value["appEpoch"] = json!(Uuid::new_v4().to_string());
        files::atomic_json(&path(&client, &id), &value, false).unwrap();
        assert!(matches!(hub.take(), Err("APP_RESTARTED")));
        value["appEpoch"] = json!(hub.state.app_epoch);
        value["payload"] = json!({"SetProject":{"enabled":true}});
        files::atomic_json(&path(&client, &id), &value, false).unwrap();
        assert!(matches!(hub.take(), Err("INVALID_CHANNEL_MESSAGE")));
    }
    #[test]
    fn crash_after_claim_cannot_reopen_epoch_or_dispatch_old_requests_in_new_epoch() {
        let mut hub = hub();
        let root = hub.root.clone();
        let epoch = hub.state.app_epoch.clone();
        let mut client = Client::new(hub.authorize(Role::Mcp).unwrap());
        let id = client.submit(AdapterInput::Mcp(json!({"id":0}))).unwrap();
        let delivery = hub.take().unwrap().unwrap();
        drop(hub);
        assert!(matches!(
            Hub::open(&root, "installation", &epoch, 1),
            Err("EPOCH_ALREADY_USED")
        ));
        let mut next = Hub::open(&root, "installation", &Uuid::new_v4().to_string(), 1).unwrap();
        assert!(next.take().unwrap().is_none());
        assert_eq!(
            next.reply(delivery, &json!({"approved":true})),
            Err("APP_RESTARTED")
        );
        assert_eq!(client.read_reply(&id), Err("APP_RESTARTED"));
    }
    #[test]
    fn reply_publish_failure_is_durable_unknown_and_closed_connections_cannot_write() {
        let mut hub = hub();
        let mut client = Client::new(hub.authorize(Role::Mcp).unwrap());
        let id = client.submit(AdapterInput::Mcp(json!({}))).unwrap();
        let delivery = hub.take().unwrap().unwrap();
        fs::create_dir(
            client
                .permit
                .root
                .join("replies")
                .join(format!("{id}.json")),
        )
        .unwrap();
        assert!(hub.reply(delivery, &json!({"approved":true})).is_err());
        assert_eq!(hub.state.receipts[&id].state, ReceiptState::OutcomeUnknown);
        let id = client.submit(AdapterInput::Mcp(json!({}))).unwrap();
        let delivery = hub.take().unwrap().unwrap();
        // The obstructing directory prevents cleanup, but revocation must
        // already be durable even when cleanup reports that conflict.
        assert_eq!(
            hub.close(client.permit.connection_id()),
            Err("CHANNEL_CLEANUP_FAILED")
        );
        assert_eq!(hub.reply(delivery, &json!({})), Err("CONNECTION_UNKNOWN"));
        assert_eq!(client.read_reply(&id).unwrap(), None);
    }
    #[test]
    fn short_lived_connections_release_slots_and_reply_files_without_replay() {
        let mut hub = hub();
        for _ in 0..40 {
            let mut client = Client::new(hub.authorize(Role::Hook).unwrap());
            let id = client.submit(AdapterInput::Hook(json!({}))).unwrap();
            let delivery = hub.take().unwrap().unwrap();
            hub.reply(delivery, &json!({"observed":true})).unwrap();
            hub.close(client.permit.connection_id()).unwrap();
            assert!(!client
                .permit
                .root
                .join(format!("replies/{id}.json"))
                .exists());
            assert!(hub.state.connections.is_empty());
            assert!(hub.state.receipts.is_empty());
        }
    }
    #[test]
    fn restarting_discards_retired_transport_credentials_and_queues() {
        let old = hub();
        let retired = old.epoch_root.clone();
        let root = old.root.clone();
        let installation = old.state.installation_id.clone();
        drop(old);
        let next = Hub::open(&root, &installation, &Uuid::new_v4().to_string(), 1).unwrap();
        assert!(!retired.exists());
        assert!(next.epoch_root.join("transport.json").is_file());
    }
    #[test]
    fn queue_is_bounded_and_unchanged_heartbeat_does_not_renew_liveness() {
        let mut hub = hub();
        let mut client = Client::new(hub.authorize(Role::Hook).unwrap());
        for _ in 0..8 {
            client.submit(AdapterInput::Hook(json!({}))).unwrap();
        }
        assert_eq!(
            client.submit(AdapterInput::Hook(json!({}))),
            Err("CONNECTION_BACKPRESSURE")
        );
        let heartbeat = hub.heartbeat().unwrap();
        let mut tracker = Liveness::after_handshake(heartbeat.clone(), 0).unwrap();
        tracker.observe(&heartbeat, 9999).unwrap();
        assert_eq!(tracker.observe(&heartbeat, 10000), Err("BROKER_OFFLINE"));
        let mut next = hub.heartbeat().unwrap();
        next.broker_ms += 10001;
        tracker.observe(&next, 10001).unwrap();
        let mut old = next.clone();
        old.app_epoch = Uuid::new_v4().to_string();
        assert_eq!(tracker.observe(&old, 10002), Err("APP_RESTARTED"));
    }
}
