//! Observed-only, read-only display history. Never reads Cursor databases or
//! transcript paths; no ticket, connection credential or executable command is
//! part of its schema. Normalized events are trusted Rust inputs, not wire data.
use crate::{
    files, protocol,
    schema::{self, Result},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    path::{Path, PathBuf},
};

const MAX_SESSIONS: usize = 1000;
const MAX_BYTES: usize = 100 * 1024 * 1024;
const RETENTION_MS: u64 = 7 * 24 * 60 * 60 * 1000;
const INACTIVITY_MS: u64 = 10 * 60 * 1000;
const MAX_ITEMS: usize = 256;
const MAX_EVENTS: usize = 2048;
const MAX_TEXT: usize = 32 * 1024;

pub fn session_key(installation: &str, conversation: &str) -> String {
    protocol::sha256(
        &serde_json::to_vec(&("cursor", installation, conversation)).unwrap_or_default(),
    )
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Activity {
    Working,
    Stopped,
    Unknown,
    Interrupted,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    Started,
    CompletedObserved,
    FailedObserved,
    CancelledObserved,
    Unknown,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Item {
    Response {
        generation: Option<String>,
        text: String,
        truncated: bool,
    },
    Tool {
        generation: Option<String>,
        call_id: String,
        name: String,
        outcome: Outcome,
    },
    Subagent {
        generation: Option<String>,
        agent_id: String,
        parent_id: Option<String>,
        outcome: Outcome,
    },
}
#[derive(Clone, Debug, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum EventKind {
    Discovered {
        title: Option<String>,
    },
    PromptCandidate {
        generation: String,
    },
    GenerationStarted {
        generation: String,
    },
    GenerationEnded {
        generation: String,
    },
    Response {
        generation: Option<String>,
        semantic_id: Option<String>,
        text: String,
    },
    Tool {
        generation: Option<String>,
        call_id: String,
        name: String,
        outcome: Outcome,
    },
    Subagent {
        generation: Option<String>,
        agent_id: String,
        parent_id: Option<String>,
        outcome: Outcome,
    },
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Observation {
    pub event_id: String,
    pub conversation_id: String,
    pub event: EventKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Session {
    pub schema_version: u32,
    pub source: String,
    pub installation_id: String,
    pub conversation_id: String,
    pub key: String,
    pub title: String,
    pub updated_at: u64,
    pub observed_epoch: String,
    pub activity: Activity,
    pub current_generation: Option<String>,
    pub items: Vec<Item>,
    pub truncated: bool,
    retired: BTreeSet<String>,
    event_digests: BTreeMap<String, String>,
    semantic_ids: BTreeSet<String>,
    #[serde(skip)]
    last_activity_ms: u64,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub schema_version: u32,
    pub source: String,
    pub capture_scope: String,
    pub app_epoch: String,
    pub sequence: u64,
    pub sessions: Vec<SessionView>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    pub key: String,
    pub source: String,
    pub conversation_id: String,
    pub title: String,
    pub updated_at: u64,
    pub activity: Activity,
    pub items: Vec<Item>,
    pub truncated: bool,
    pub has_current_request: bool,
    pub read_only: bool,
}

pub struct History {
    root: PathBuf,
    _lock: File,
    installation_id: String,
    app_epoch: String,
    sessions: BTreeMap<String, Session>,
    sequence: u64,
    now_ms: u64,
    last_view_digest: String,
    pub quarantined: usize,
}
fn is_key(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}
fn terminal(session: &Session) -> bool {
    session.activity != Activity::Working
}
fn encoded(session: &Session) -> Result<Vec<u8>> {
    serde_json::to_vec(session).map_err(|_| "HISTORY_ENCODE_FAILED")
}
fn truncate(text: &str, limit: usize) -> (String, bool) {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_string(), end < text.len())
}
impl History {
    pub fn version(&self) -> u64 {
        self.sequence
    }
    pub fn open(root: &Path, installation: &str, epoch: &str) -> Result<Self> {
        if !schema::valid_id(installation) || uuid::Uuid::parse_str(epoch).is_err() {
            return Err("INVALID_HISTORY_IDENTITY");
        }
        let root = files::private_directory(root)?;
        let lock = files::lock(&files::child(&root, "history.lock")?)?;
        let mut history = Self {
            root,
            _lock: lock,
            installation_id: installation.into(),
            app_epoch: epoch.into(),
            sessions: BTreeMap::new(),
            sequence: 0,
            now_ms: 0,
            last_view_digest: String::new(),
            quarantined: 0,
        };
        let entries = fs::read_dir(&history.root).map_err(|_| "HISTORY_UNAVAILABLE")?;
        for (index, entry) in entries.enumerate() {
            if index >= 2048 {
                return Err("HISTORY_ENTRY_LIMIT");
            }
            let entry = entry.map_err(|_| "HISTORY_UNAVAILABLE")?;
            let name = entry.file_name().to_string_lossy().to_string();
            let Some(key) = name.strip_suffix(".json").filter(|key| is_key(key)) else {
                continue;
            };
            let path = files::child(&history.root, &name)?;
            let result = files::read_json(&path).and_then(|value| {
                serde_json::from_value::<Session>(value).map_err(|_| "INVALID_HISTORY")
            });
            let mut session = match result {
                Ok(session)
                    if session.schema_version == 1
                        && session.source == "cursor"
                        && session.installation_id == installation
                        && schema::valid_id(&session.conversation_id)
                        && session.key == key
                        && key == session_key(installation, &session.conversation_id)
                        && session.items.len() <= MAX_ITEMS
                        && session.event_digests.len() <= MAX_EVENTS =>
                {
                    session
                }
                _ => {
                    let quarantine = files::child(
                        &history.root,
                        &format!("{key}.{}.corrupt", uuid::Uuid::new_v4()),
                    )?;
                    fs::rename(path, quarantine).map_err(|_| "HISTORY_QUARANTINE_FAILED")?;
                    history.quarantined += 1;
                    continue;
                }
            };
            if session.activity == Activity::Working {
                session.activity = Activity::Interrupted;
            }
            history.sessions.insert(key.into(), session);
            let victims = Self::retention_victims(&history.sessions, &BTreeSet::new(), 0)?;
            history.remove(&victims)?;
        }
        Ok(history)
    }
    /// Pinned keys must be derived from current broker requests, never supplied
    /// by a LAN client. Display retention cannot delete active authority.
    pub fn observe(
        &mut self,
        observation: Observation,
        wall_ms: u64,
        now_ms: u64,
        pinned: &BTreeSet<String>,
    ) -> Result<bool> {
        if now_ms < self.now_ms {
            return Err("MONOTONIC_CLOCK_REGRESSED");
        }
        self.now_ms = now_ms;
        if !schema::valid_id(&observation.event_id)
            || !schema::valid_id(&observation.conversation_id)
        {
            return Err("UNASSOCIATED_HISTORY_EVENT");
        }
        let key = session_key(&self.installation_id, &observation.conversation_id);
        let event_bytes = serde_json::to_vec(&observation).map_err(|_| "INVALID_HISTORY_EVENT")?;
        if event_bytes.len() > protocol::MAX_MESSAGE_BYTES {
            return Err("HISTORY_EVENT_TOO_LARGE");
        }
        let digest = protocol::sha256(&event_bytes);
        let mut session = self.sessions.get(&key).cloned().unwrap_or_else(|| Session {
            schema_version: 1,
            source: "cursor".into(),
            installation_id: self.installation_id.clone(),
            conversation_id: observation.conversation_id.clone(),
            key: key.clone(),
            title: format!(
                "Cursor {}",
                observation
                    .conversation_id
                    .chars()
                    .take(12)
                    .collect::<String>()
            ),
            updated_at: wall_ms,
            observed_epoch: self.app_epoch.clone(),
            activity: Activity::Unknown,
            current_generation: None,
            items: Vec::new(),
            truncated: false,
            retired: BTreeSet::new(),
            event_digests: BTreeMap::new(),
            semantic_ids: BTreeSet::new(),
            last_activity_ms: now_ms,
        });
        if let Some(previous) = session.event_digests.get(&observation.event_id) {
            return if previous == &digest {
                Ok(false)
            } else {
                Err("HISTORY_EVENT_ID_CONFLICT")
            };
        }
        if session.event_digests.len() >= MAX_EVENTS {
            return Err("HISTORY_EVENT_BACKPRESSURE");
        }
        let mut actual_activity = false;
        let mut known_current_activity = false;
        let mut semantic_duplicate = false;
        match observation.event {
            EventKind::Discovered { title } => {
                // Late asynchronous sessionStart must not restart a stopped
                // generation or reorder a card already found by a tool event.
                if let Some(title) = title {
                    if session.title.starts_with("Cursor ") {
                        session.title = truncate(&title, 2048).0;
                    }
                }
            }
            EventKind::PromptCandidate { generation } => {
                if !schema::valid_id(&generation) {
                    return Err("INVALID_HISTORY_EVENT");
                }
                actual_activity = !session.retired.contains(&generation);
            }
            EventKind::GenerationStarted { generation } => {
                if !schema::valid_id(&generation) {
                    return Err("INVALID_HISTORY_EVENT");
                }
                if !session.retired.contains(&generation)
                    && (session.current_generation.as_ref() != Some(&generation)
                        || session.observed_epoch != self.app_epoch)
                {
                    if let Some(previous) = session.current_generation.replace(generation.clone()) {
                        if previous != generation {
                            session.retired.insert(previous);
                        }
                    }
                    session.activity = Activity::Working;
                    actual_activity = true;
                }
            }
            EventKind::GenerationEnded { generation } => {
                if session.current_generation.as_ref() == Some(&generation)
                    && session.activity != Activity::Stopped
                {
                    session.activity = Activity::Stopped;
                    session.retired.insert(generation);
                    actual_activity = true;
                }
            }
            EventKind::Response {
                generation,
                semantic_id,
                text,
            } => {
                let current = Self::current_event(&session, generation.as_deref())?;
                known_current_activity = current && generation.is_some();
                let semantic_key = semantic_id.as_ref().map(|id| {
                    protocol::sha256(
                        &serde_json::to_vec(&("response", &generation, id)).unwrap_or_default(),
                    )
                });
                if semantic_id.as_ref().is_some_and(|id| !schema::valid_id(id)) {
                    return Err("INVALID_HISTORY_EVENT");
                }
                semantic_duplicate = semantic_key
                    .as_ref()
                    .is_some_and(|key| session.semantic_ids.contains(key));
                if !semantic_duplicate {
                    let (text, truncated) = truncate(&text, MAX_TEXT);
                    session.items.push(Item::Response {
                        generation,
                        text,
                        truncated,
                    });
                    if let Some(key) = semantic_key {
                        session.semantic_ids.insert(key);
                    }
                    actual_activity = current;
                }
            }
            EventKind::Tool {
                generation,
                call_id,
                name,
                outcome,
            } => {
                if !schema::valid_id(&call_id) || name.len() > 2048 {
                    return Err("INVALID_HISTORY_EVENT");
                }
                let current = Self::current_event(&session, generation.as_deref())?;
                known_current_activity = current && generation.is_some();
                let existing = session.items.iter_mut().find(|item| matches!(item, Item::Tool {generation:g, call_id:id, ..} if g == &generation && id == &call_id));
                if let Some(Item::Tool { outcome: old, .. }) = existing {
                    // A delayed start cannot overwrite an observed terminal.
                    if *old == outcome || (*old != Outcome::Started && outcome == Outcome::Started)
                    {
                        semantic_duplicate = true;
                    } else {
                        *old = outcome;
                        actual_activity = current;
                    }
                } else {
                    session.items.push(Item::Tool {
                        generation,
                        call_id,
                        name,
                        outcome,
                    });
                    actual_activity = current;
                }
            }
            EventKind::Subagent {
                generation,
                agent_id,
                parent_id,
                outcome,
            } => {
                if !schema::valid_id(&agent_id)
                    || parent_id.as_ref().is_some_and(|id| !schema::valid_id(id))
                {
                    return Err("INVALID_HISTORY_EVENT");
                }
                let current = Self::current_event(&session, generation.as_deref())?;
                known_current_activity = current && generation.is_some();
                let existing = session.items.iter_mut().find(|item| matches!(item, Item::Subagent {generation:g, agent_id:id, ..} if g == &generation && id == &agent_id));
                if let Some(Item::Subagent {
                    outcome: old,
                    parent_id: parent,
                    ..
                }) = existing
                {
                    if parent != &parent_id {
                        return Err("SUBAGENT_PARENT_CONFLICT");
                    }
                    if *old == outcome || (*old != Outcome::Started && outcome == Outcome::Started)
                    {
                        semantic_duplicate = true;
                    } else {
                        *old = outcome;
                        actual_activity = current;
                    }
                } else {
                    session.items.push(Item::Subagent {
                        generation,
                        agent_id,
                        parent_id,
                        outcome,
                    });
                    actual_activity = current;
                }
            }
        }
        session.event_digests.insert(observation.event_id, digest);
        if actual_activity && !semantic_duplicate {
            session.updated_at = session.updated_at.max(wall_ms);
            session.last_activity_ms = now_ms;
            session.observed_epoch = self.app_epoch.clone();
            if known_current_activity && session.activity != Activity::Stopped {
                session.activity = Activity::Working;
            }
        }
        if session.items.len() > MAX_ITEMS {
            session.items.remove(0);
            session.truncated = true;
        }
        // A large sequence of completed answers can exceed one snapshot. Trim
        // oldest display items with an explicit flag; business payloads are in
        // the separate Store and are never truncated for human decisions.
        while encoded(&session)?.len() > protocol::MAX_MESSAGE_BYTES && !session.items.is_empty() {
            session.items.remove(0);
            session.truncated = true;
        }
        let bytes = encoded(&session)?;
        if bytes.len() > protocol::MAX_MESSAGE_BYTES {
            return Err("HISTORY_BACKPRESSURE");
        }
        let mut projected = self.sessions.clone();
        projected.insert(key.clone(), session.clone());
        let victims = Self::retention_victims(&projected, pinned, wall_ms)?;
        if victims.contains(&key) {
            return Err("HISTORY_BACKPRESSURE");
        }
        files::atomic_write(
            &files::child(&self.root, &format!("{key}.json"))?,
            &bytes,
            true,
        )?;
        self.sessions.insert(key, session);
        self.sequence += 1;
        self.remove(&victims)?;
        Ok(true)
    }
    fn current_event(session: &Session, generation: Option<&str>) -> Result<bool> {
        if generation.is_some_and(|id| !schema::valid_id(id)) {
            return Err("INVALID_HISTORY_EVENT");
        }
        Ok(generation.is_none_or(|id| {
            session.current_generation.as_deref() == Some(id) && !session.retired.contains(id)
        }))
    }
    fn retention_victims(
        sessions: &BTreeMap<String, Session>,
        pinned: &BTreeSet<String>,
        wall_ms: u64,
    ) -> Result<Vec<String>> {
        let mut total = sessions
            .values()
            .try_fold(0usize, |sum, session| -> Result<usize> {
                Ok(sum + encoded(session)?.len())
            })?;
        let mut count = sessions.len();
        let mut victims = Vec::new();
        let mut candidates = sessions
            .values()
            .filter(|session| terminal(session) && !pinned.contains(&session.key))
            .collect::<Vec<_>>();
        candidates.sort_by_key(|session| (session.updated_at, &session.key));
        for session in candidates {
            if wall_ms.saturating_sub(session.updated_at) < RETENTION_MS
                && count <= MAX_SESSIONS
                && total <= MAX_BYTES
            {
                break;
            }
            victims.push(session.key.clone());
            count -= 1;
            total -= encoded(session)?.len();
        }
        if count > MAX_SESSIONS || total > MAX_BYTES {
            return Err("HISTORY_BACKPRESSURE");
        }
        Ok(victims)
    }
    fn remove(&mut self, keys: &[String]) -> Result<()> {
        for key in keys {
            let path = files::child(&self.root, &format!("{key}.json"))?;
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err("HISTORY_CLEANUP_FAILED"),
            }
            self.sessions.remove(key);
        }
        Ok(())
    }
    pub fn tick(&mut self, wall_ms: u64, now_ms: u64, pinned: &BTreeSet<String>) -> Result<()> {
        if now_ms < self.now_ms {
            return Err("MONOTONIC_CLOCK_REGRESSED");
        }
        self.now_ms = now_ms;
        let mut inactive = Vec::new();
        for session in self.sessions.values() {
            if session.activity == Activity::Working
                && !pinned.contains(&session.key)
                && now_ms.saturating_sub(session.last_activity_ms) >= INACTIVITY_MS
            {
                let mut session = session.clone();
                session.activity = Activity::Unknown;
                inactive.push(session);
            }
        }
        for session in inactive {
            files::atomic_write(
                &files::child(&self.root, &format!("{}.json", session.key))?,
                &encoded(&session)?,
                true,
            )?;
            self.sessions.insert(session.key.clone(), session);
            self.sequence += 1;
        }
        let victims = Self::retention_victims(&self.sessions, pinned, wall_ms)?;
        if !victims.is_empty() {
            self.remove(&victims)?;
            self.sequence += 1;
        }
        Ok(())
    }
    /// Read-only projection accepts current pins and the existing global
    /// visibility threshold. It adds no Cursor-specific setting or UI entry.
    pub fn snapshot(
        &mut self,
        wall_ms: u64,
        cleanup_after_ms: u64,
        hidden: &BTreeMap<String, u64>,
        pinned: &BTreeSet<String>,
    ) -> Snapshot {
        let mut sessions = self
            .sessions
            .values()
            .filter(|session| {
                pinned.contains(&session.key)
                    || ((!terminal(session)
                        || wall_ms.saturating_sub(session.updated_at) < cleanup_after_ms)
                        && hidden
                            .get(&session.key)
                            .is_none_or(|hidden_at| session.updated_at > *hidden_at))
            })
            .map(|session| SessionView {
                key: session.key.clone(),
                source: "cursor".into(),
                conversation_id: session.conversation_id.clone(),
                title: session.title.clone(),
                updated_at: session.updated_at,
                activity: session.activity.clone(),
                items: session.items.clone(),
                truncated: session.truncated,
                has_current_request: pinned.contains(&session.key),
                // Pending forms come from the business snapshot, never these
                // possibly truncated historical items.
                read_only: true,
            })
            .collect::<Vec<_>>();
        sessions.sort_by(|a, b| {
            b.has_current_request
                .cmp(&a.has_current_request)
                .then_with(|| {
                    (b.activity == Activity::Working).cmp(&(a.activity == Activity::Working))
                })
                .then_with(|| b.updated_at.cmp(&a.updated_at))
                .then_with(|| a.key.cmp(&b.key))
        });
        let mut snapshot = Snapshot {
            schema_version: 1,
            source: "cursor".into(),
            capture_scope: "observedOnly".into(),
            app_epoch: self.app_epoch.clone(),
            sequence: 0,
            sessions,
        };
        // Pins/visibility can change without a new native event. Version the
        // resulting view too; polling unchanged data leaves activity untouched.
        let digest = protocol::sha256(&serde_json::to_vec(&snapshot).unwrap_or_default());
        if digest != self.last_view_digest {
            self.sequence += 1;
            self.last_view_digest = digest;
        }
        snapshot.sequence = self.sequence;
        snapshot
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshot_versions_change_for_live_pins_but_polling_does_not_change_activity() {
        let mut history = history();
        history
            .observe(
                event("one", EventKind::Discovered { title: None }),
                10,
                10,
                &BTreeSet::new(),
            )
            .unwrap();
        let first = snapshot(&mut history);
        let repeated = snapshot(&mut history);
        assert_eq!(first.sequence, repeated.sequence);
        let pins = BTreeSet::from([session_key("installation", "conversation")]);
        let pinned = history.snapshot(1000, u64::MAX, &BTreeMap::new(), &pins);
        assert!(pinned.sequence > repeated.sequence);
        assert!(pinned.sessions[0].has_current_request);
        assert!(pinned.sessions[0].read_only);
        assert_eq!(pinned.sessions[0].updated_at, first.sessions[0].updated_at);
    }
    #[test]
    fn unknown_generation_activity_updates_time_and_semantic_ids_cannot_collide_by_delimiters() {
        let mut history = history();
        for (index, generation, id) in [
            (0, Some("a:b"), Some("c")),
            (1, Some("a"), Some("b:c")),
            (2, None, None),
        ] {
            history
                .observe(
                    event(
                        &format!("event-{index}"),
                        EventKind::Response {
                            generation: generation.map(String::from),
                            semantic_id: id.map(String::from),
                            text: "response".into(),
                        },
                    ),
                    10 + index,
                    10 + index,
                    &BTreeSet::new(),
                )
                .unwrap();
        }
        let snapshot = snapshot(&mut history);
        assert_eq!(snapshot.sessions[0].items.len(), 3);
        assert_eq!(snapshot.sessions[0].updated_at, 12);
        assert_eq!(snapshot.sessions[0].activity, Activity::Unknown);
    }
    #[test]
    fn text_truncation_is_explicit_and_retention_cannot_evict_pinned_sessions() {
        let mut history = history();
        history
            .observe(
                event(
                    "response",
                    EventKind::Response {
                        generation: None,
                        semantic_id: None,
                        text: "问题".repeat(20000),
                    },
                ),
                10,
                10,
                &BTreeSet::new(),
            )
            .unwrap();
        let snapshot = snapshot(&mut history);
        let Item::Response {
            text, truncated, ..
        } = &snapshot.sessions[0].items[0]
        else {
            panic!()
        };
        assert!(*truncated);
        assert!(text.len() <= MAX_TEXT);
        let template = history.sessions.values().next().unwrap().clone();
        let mut sessions = BTreeMap::new();
        for index in 0..=MAX_SESSIONS {
            let mut session = template.clone();
            session.key = protocol::sha256(&index.to_le_bytes());
            session.items.clear();
            session.updated_at = index as u64;
            sessions.insert(session.key.clone(), session);
        }
        let pins = sessions.keys().cloned().collect();
        assert_eq!(
            History::retention_victims(&sessions, &pins, 100),
            Err("HISTORY_BACKPRESSURE")
        );
        assert_eq!(
            History::retention_victims(&sessions, &BTreeSet::new(), 100)
                .unwrap()
                .len(),
            1
        );
    }
    fn history() -> History {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("runs")
            .join(format!("history-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        History::open(&root, "installation", &uuid::Uuid::new_v4().to_string()).unwrap()
    }
    fn event(id: &str, kind: EventKind) -> Observation {
        Observation {
            event_id: id.into(),
            conversation_id: "conversation".into(),
            event: kind,
        }
    }
    fn snapshot(history: &mut History) -> Snapshot {
        history.snapshot(1000, u64::MAX, &BTreeMap::new(), &BTreeSet::new())
    }
    #[test]
    fn first_tool_discovers_session_late_start_merges_and_duplicate_does_not_refresh_activity() {
        let mut history = history();
        let tool = event(
            "tool",
            EventKind::Tool {
                generation: None,
                call_id: "call".into(),
                name: "Read".into(),
                outcome: Outcome::CompletedObserved,
            },
        );
        history
            .observe(tool.clone(), 10, 10, &BTreeSet::new())
            .unwrap();
        assert!(!history.observe(tool, 20, 20, &BTreeSet::new()).unwrap());
        history
            .observe(
                event("start", EventKind::Discovered { title: None }),
                30,
                30,
                &BTreeSet::new(),
            )
            .unwrap();
        let snapshot = snapshot(&mut history);
        assert_eq!(snapshot.sessions.len(), 1);
        assert_eq!(snapshot.sessions[0].updated_at, 10);
        assert_eq!(snapshot.sessions[0].items.len(), 1);
    }
    #[test]
    fn candidate_prompt_and_old_generation_end_do_not_switch_the_current_generation() {
        let mut history = history();
        let empty = BTreeSet::new();
        history
            .observe(
                event(
                    "a",
                    EventKind::GenerationStarted {
                        generation: "a".into(),
                    },
                ),
                10,
                10,
                &empty,
            )
            .unwrap();
        history
            .observe(
                event(
                    "candidate",
                    EventKind::PromptCandidate {
                        generation: "b".into(),
                    },
                ),
                20,
                20,
                &empty,
            )
            .unwrap();
        assert_eq!(
            history
                .sessions
                .values()
                .next()
                .unwrap()
                .current_generation
                .as_deref(),
            Some("a")
        );
        history
            .observe(
                event(
                    "b",
                    EventKind::GenerationStarted {
                        generation: "b".into(),
                    },
                ),
                30,
                30,
                &empty,
            )
            .unwrap();
        history
            .observe(
                event(
                    "late",
                    EventKind::GenerationEnded {
                        generation: "a".into(),
                    },
                ),
                40,
                40,
                &empty,
            )
            .unwrap();
        assert_eq!(
            snapshot(&mut history).sessions[0].activity,
            Activity::Working
        );
        assert_eq!(snapshot(&mut history).sessions[0].updated_at, 30);
    }
    #[test]
    fn identical_text_from_distinct_events_is_kept_and_verified_semantic_duplicates_are_not() {
        let mut history = history();
        for (id, semantic_id) in [
            ("a", None),
            ("b", None),
            ("c", Some("response")),
            ("d", Some("response")),
        ] {
            history
                .observe(
                    event(
                        id,
                        EventKind::Response {
                            generation: None,
                            semantic_id: semantic_id.map(String::from),
                            text: "same text".into(),
                        },
                    ),
                    10,
                    10,
                    &BTreeSet::new(),
                )
                .unwrap();
        }
        assert_eq!(snapshot(&mut history).sessions[0].items.len(), 3);
    }
    #[test]
    fn restart_is_read_only_does_not_refresh_timestamps_and_corrupt_entries_are_isolated() {
        let mut history = history();
        history
            .observe(
                event(
                    "a",
                    EventKind::GenerationStarted {
                        generation: "a".into(),
                    },
                ),
                10,
                10,
                &BTreeSet::new(),
            )
            .unwrap();
        let root = history.root.clone();
        let before = snapshot(&mut history);
        drop(history);
        let bad = root.join(format!("{}.json", protocol::sha256(b"bad")));
        fs::write(&bad, "{broken").unwrap();
        let mut history =
            History::open(&root, "installation", &uuid::Uuid::new_v4().to_string()).unwrap();
        let after = snapshot(&mut history);
        assert_eq!(history.quarantined, 1);
        assert!(!bad.exists());
        assert_eq!(after.sessions[0].key, before.sessions[0].key);
        assert_eq!(after.sessions[0].updated_at, 10);
        assert_eq!(after.sessions[0].activity, Activity::Interrupted);
        assert!(after.sessions[0].read_only);
        assert_ne!(after.app_epoch, before.app_epoch);
    }
    #[test]
    fn inactivity_is_unknown_and_live_requests_survive_hidden_and_retention_filters() {
        let mut history = history();
        let empty = BTreeSet::new();
        history
            .observe(
                event(
                    "a",
                    EventKind::GenerationStarted {
                        generation: "a".into(),
                    },
                ),
                10,
                10,
                &empty,
            )
            .unwrap();
        history.tick(600010, 600010, &empty).unwrap();
        assert_eq!(
            snapshot(&mut history).sessions[0].activity,
            Activity::Unknown
        );
        let key = session_key("installation", "conversation");
        let pinned = BTreeSet::from([key.clone()]);
        let hidden = BTreeMap::from([(key, 100)]);
        history
            .tick(RETENTION_MS + 100, RETENTION_MS + 100, &pinned)
            .unwrap();
        assert_eq!(
            history
                .snapshot(RETENTION_MS + 100, 1, &hidden, &pinned)
                .sessions
                .len(),
            1
        );
        assert!(history
            .snapshot(RETENTION_MS + 100, 1, &hidden, &empty)
            .sessions
            .is_empty());
        history
            .tick(RETENTION_MS + 101, RETENTION_MS + 101, &empty)
            .unwrap();
        assert!(history.sessions.is_empty());
    }
}
