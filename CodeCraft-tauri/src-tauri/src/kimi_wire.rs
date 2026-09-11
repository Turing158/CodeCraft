//! Read-only observer for Kimi Code's internal `wire.jsonl` stream.
//!
//! Kimi hooks expose tool lifecycle events, but interactive requests are
//! written to this file instead.  The observer never writes to Kimi files and
//! only retains unresolved request records for the desktop review surface.

use std::{
    collections::{HashMap, HashSet},
    env,
    fs::{self, File},
    io::{BufRead, BufReader, ErrorKind, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use serde_json::{Map, Value};

const MAX_INITIAL_SCAN_BYTES: u64 = 4 * 1024 * 1024;
const MAX_READ_BYTES_PER_FILE: u64 = 2 * 1024 * 1024;
const MAX_WIRE_FILES: usize = 256;
const MAX_ACTIVE_INTERACTIONS: usize = 128;

#[derive(Clone, Debug)]
pub(crate) struct KimiWireInteraction {
    pub(crate) wire_path: PathBuf,
    pub(crate) native_session_id: String,
    pub(crate) agent_id: Option<String>,
    pub(crate) native_interaction_id: String,
    pub(crate) tool_call_id: Option<String>,
    pub(crate) kind: String,
    pub(crate) request: Value,
    pub(crate) captured_at: u64,
}

#[derive(Default)]
struct FileCursor {
    offset: u64,
    initialized: bool,
}

#[derive(Default)]
pub(crate) struct KimiWireStore {
    files: HashMap<PathBuf, FileCursor>,
    pending: HashMap<String, KimiWireInteraction>,
    resolved: HashSet<String>,
}

impl KimiWireStore {
    pub(crate) fn clear(&mut self) {
        self.files.clear();
        self.pending.clear();
        self.resolved.clear();
    }

    pub(crate) fn refresh(&mut self) -> Result<Vec<KimiWireInteraction>, String> {
        self.refresh_at(&sessions_root())
    }

    fn refresh_at(&mut self, root: &Path) -> Result<Vec<KimiWireInteraction>, String> {
        if !root.exists() {
            self.pending.clear();
            self.files.clear();
            return Ok(Vec::new());
        }

        let mut paths = Vec::new();
        collect_wire_files(&root, 0, &mut paths);
        paths.sort();
        paths.truncate(MAX_WIRE_FILES);
        let known: HashSet<PathBuf> = paths.iter().cloned().collect();
        self.files.retain(|path, _| known.contains(path));
        self.pending
            .retain(|_, interaction| known.contains(&interaction.wire_path));

        for path in paths {
            self.read_file(&path)?;
        }

        if self.pending.len() > MAX_ACTIVE_INTERACTIONS {
            let mut entries: Vec<_> = self
                .pending
                .values()
                .map(|item| (item.captured_at, item.native_interaction_id.clone()))
                .collect();
            entries.sort_by_key(|entry| entry.0);
            for (_, id) in entries
                .into_iter()
                .take(self.pending.len() - MAX_ACTIVE_INTERACTIONS)
            {
                self.pending.remove(&id);
            }
        }

        let mut active: Vec<_> = self.pending.values().cloned().collect();
        active.sort_by_key(|item| item.captured_at);
        Ok(active)
    }

    fn read_file(&mut self, path: &Path) -> Result<(), String> {
        let metadata = match fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.to_string()),
        };
        let length = metadata.len();
        let (mut offset, initialized) = self
            .files
            .get(path)
            .map(|cursor| (cursor.offset, cursor.initialized))
            .unwrap_or_default();
        let first_read = !initialized;
        if offset > length {
            offset = 0;
            self.pending
                .retain(|_, interaction| interaction.wire_path != path);
        }
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.to_string()),
        };
        let mut reader = BufReader::new(file);
        if first_read && length > MAX_INITIAL_SCAN_BYTES {
            offset = length - MAX_INITIAL_SCAN_BYTES;
            reader
                .seek(SeekFrom::Start(offset))
                .map_err(|error| error.to_string())?;
            // Discard a possible partial JSON record at the scan boundary.
            let mut discarded = Vec::new();
            let _ = reader.read_until(b'\n', &mut discarded);
            offset = reader
                .stream_position()
                .map_err(|error| error.to_string())?;
        } else {
            reader
                .seek(SeekFrom::Start(offset))
                .map_err(|error| error.to_string())?;
        }

        let scan_limit = offset.saturating_add(MAX_READ_BYTES_PER_FILE);
        loop {
            let start = reader
                .stream_position()
                .map_err(|error| error.to_string())?;
            if start >= scan_limit {
                break;
            }
            let mut line = Vec::new();
            let read = reader
                .read_until(b'\n', &mut line)
                .map_err(|error| error.to_string())?;
            if read == 0 {
                offset = start;
                break;
            }
            if !line.ends_with(b"\n") {
                // Kimi may be in the middle of an append.  Re-read this line
                // on the next refresh instead of parsing incomplete JSON.
                offset = start;
                break;
            }
            offset = reader
                .stream_position()
                .map_err(|error| error.to_string())?;
            if let Ok(value) = serde_json::from_slice::<Value>(&line) {
                self.apply_line(path, &value);
            }
        }
        self.files.insert(
            path.to_path_buf(),
            FileCursor {
                offset,
                initialized: true,
            },
        );
        Ok(())
    }

    fn apply_line(&mut self, path: &Path, value: &Value) {
        let Some(kind) = value.get("type").and_then(Value::as_str) else {
            return;
        };
        let Some((session_id, agent_id)) = path_identity(path) else {
            return;
        };
        let native_id = value
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty());
        let Some(native_id) = native_id else {
            return;
        };
        let key = format!("{session_id}:{agent_id}:{native_id}");
        match kind {
            "interaction.request" => {
                if self.resolved.contains(&key) {
                    return;
                }
                let Some(request) = value.get("request") else {
                    return;
                };
                let interaction = KimiWireInteraction {
                    wire_path: path.to_path_buf(),
                    native_session_id: request
                        .get("sessionId")
                        .or_else(|| request.get("session_id"))
                        .and_then(Value::as_str)
                        .filter(|id| !id.is_empty())
                        .unwrap_or(&session_id)
                        .to_string(),
                    agent_id: Some(agent_id),
                    native_interaction_id: native_id.to_string(),
                    tool_call_id: value
                        .get("toolCallId")
                        .or_else(|| request.get("toolCallId"))
                        .or_else(|| request.get("tool_call_id"))
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    kind: value
                        .get("kind")
                        .and_then(Value::as_str)
                        .unwrap_or("approval")
                        .to_string(),
                    request: sanitize_value(request, 0),
                    captured_at: value
                        .get("time")
                        .and_then(Value::as_u64)
                        .unwrap_or_default(),
                };
                self.resolved.remove(&key);
                self.pending.insert(key, interaction);
            }
            "interaction.resolved" => {
                self.pending.remove(&key);
                self.resolved.insert(key);
                if self.resolved.len() > 4096 {
                    let keep: HashSet<_> = self.resolved.drain().take(2048).collect();
                    self.resolved = keep;
                }
            }
            _ => {}
        }
    }
}

fn sessions_root() -> PathBuf {
    env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
        .map(|path| path.join(".kimi-code").join("sessions"))
        .unwrap_or_else(|| PathBuf::from(".kimi-code/sessions"))
}

fn collect_wire_files(path: &Path, depth: usize, output: &mut Vec<PathBuf>) {
    if depth > 6 || output.len() >= MAX_WIRE_FILES {
        return;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let child = entry.path();
        if child.file_name().and_then(|name| name.to_str()) == Some("wire.jsonl") {
            output.push(child);
        } else if child.is_dir() {
            collect_wire_files(&child, depth + 1, output);
        }
        if output.len() >= MAX_WIRE_FILES {
            return;
        }
    }
}

fn path_identity(path: &Path) -> Option<(String, String)> {
    let agent = path.parent()?.file_name()?.to_str()?.to_string();
    let agents = path.parent()?.parent()?.file_name()?.to_str()?;
    if agents != "agents" {
        return None;
    }
    let session = path.parent()?.parent()?.parent()?.file_name()?.to_str()?;
    if !session.starts_with("session_") {
        return None;
    }
    Some((session.to_string(), agent))
}

fn sanitize_value(value: &Value, depth: usize) -> Value {
    if depth > 8 {
        return Value::Null;
    }
    match value {
        Value::String(text) => {
            Value::String(crate::kimi_hook::redact_text(&cap_text(text, 512 * 1024)))
        }
        Value::Array(values) => Value::Array(
            values
                .iter()
                .take(32)
                .map(|value| sanitize_value(value, depth + 1))
                .collect(),
        ),
        Value::Object(values) => {
            let mut result = Map::new();
            for (key, value) in values.iter().take(64) {
                let lower = key.to_ascii_lowercase();
                if [
                    "token",
                    "secret",
                    "password",
                    "authorization",
                    "cookie",
                    "credential",
                    "env",
                ]
                .iter()
                .any(|needle| lower.contains(needle))
                {
                    continue;
                }
                result.insert(key.clone(), sanitize_value(value, depth + 1));
            }
            Value::Object(result)
        }
        _ => value.clone(),
    }
}

fn cap_text(text: &str, limit: usize) -> String {
    let mut chars = text.chars();
    let mut result: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        result.push('\u{2026}');
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parses_request_and_resolved_records() {
        let dir = env::temp_dir().join(format!("codecraft-kimi-wire-{}", std::process::id()));
        let wire = dir
            .join("session_test")
            .join("agents")
            .join("main")
            .join("wire.jsonl");
        fs::create_dir_all(wire.parent().unwrap()).unwrap();
        let mut file = File::create(&wire).unwrap();
        writeln!(file, "{}", serde_json::json!({
            "type":"interaction.request", "id":"question_1", "kind":"question",
            "toolCallId":"call_1", "request":{"questions":[{"question":"Which file?"}]}, "time":12
        })).unwrap();
        let (session, agent) = path_identity(&wire).unwrap();
        assert_eq!(session, "session_test");
        assert_eq!(agent, "main");
        let mut store = KimiWireStore::default();
        let value: Value = serde_json::from_str(&fs::read_to_string(&wire).unwrap()).unwrap();
        store.apply_line(&wire, &value);
        assert_eq!(store.pending.len(), 1);
        let resolved = serde_json::json!({"type":"interaction.resolved","id":"question_1"});
        store.apply_line(&wire, &resolved);
        assert!(store.pending.is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn refresh_reads_incrementally_without_repeating_records() {
        let dir = env::temp_dir().join(format!(
            "codecraft-kimi-wire-refresh-{}",
            std::process::id()
        ));
        let wire = dir
            .join("session_test")
            .join("agents")
            .join("main")
            .join("wire.jsonl");
        fs::create_dir_all(wire.parent().unwrap()).unwrap();
        let mut file = File::create(&wire).unwrap();
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "type":"interaction.request", "id":"approval_1", "kind":"approval",
                "toolCallId":"call_1", "request":{"toolName":"Bash","action":"echo test"}, "time":12
            })
        )
        .unwrap();
        let mut store = KimiWireStore::default();
        assert_eq!(store.refresh_at(&dir).unwrap().len(), 1);
        assert_eq!(store.refresh_at(&dir).unwrap().len(), 1);
        writeln!(file, "{}", serde_json::json!({
            "type":"interaction.resolved", "id":"approval_1", "response":{"decision":"approved"}, "time":13
        })).unwrap();
        file.flush().unwrap();
        assert!(store.refresh_at(&dir).unwrap().is_empty());
        let _ = fs::remove_dir_all(dir);
    }
}
