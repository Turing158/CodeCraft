use crate::protocol::{self, sha256};
use fs2::FileExt;
use serde_json::{json, Value};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

pub const ECHO_TOOL: &str = "codecraft_probe_echo";
pub const MAX_RECORDS: usize = 4096;
const MAX_CAPTURE_BYTES: u64 = 64 * 1024 * 1024;

pub fn initialize(root: &Path) -> Result<(), String> {
    // create_dir, not create_dir_all: a run is never reused, repaired or reset.
    fs::create_dir(root).map_err(|_| "RUN_ALREADY_EXISTS_OR_PARENT_MISSING")?;
    let root = root.canonicalize().map_err(|_| "RUN_PATH_FAILED")?;
    let project = root.join("project");
    fs::create_dir(&project).map_err(|_| "PROJECT_CREATE_FAILED")?;
    fs::create_dir(project.join(".cursor")).map_err(|_| "PROJECT_CREATE_FAILED")?;
    fs::create_dir(root.join("events")).map_err(|_| "EVENTS_CREATE_FAILED")?;
    let executable = std::env::current_exe().map_err(|_| "EXECUTABLE_MISSING")?;
    // Each run owns an immutable binary copy, so compiling the next probe does
    // not break an open Cursor project or misattribute captures to a new build.
    fs::create_dir(root.join("bin")).map_err(|_| "BINARY_DIRECTORY_FAILED")?;
    let run_executable = root
        .join("bin")
        .join(executable.file_name().ok_or("EXECUTABLE_MISSING")?);
    fs::copy(&executable, &run_executable).map_err(|_| "BINARY_COPY_FAILED")?;
    let manifest = json!({
        "schemaVersion":1,"runId":Uuid::new_v4().to_string(),"aliasSalt":Uuid::new_v4().to_string(),
        "project":project,"probeVersion":env!("CARGO_PKG_VERSION"),
        "probeSha256":sha256(&fs::read(&executable).map_err(|_| "EXECUTABLE_UNREADABLE")?),
        "captureOnly":true,"capabilitiesEnabled":false
    });
    write_new(&root.join("run.json"), &manifest)?;
    let mut hooks = serde_json::Map::new();
    let exe = command_path(&run_executable)?;
    let run = command_path(&root)?;
    for event in protocol::EVENTS {
        hooks.insert(
            (*event).into(),
            json!([{
                "command":format!("{exe} hook --root {run} --event {event}"),
                "timeout":10,"failClosed":false
            }]),
        );
    }
    write_new(
        &project.join(".cursor/hooks.json"),
        &json!({"version":1,"hooks":hooks}),
    )?;
    let servers = json!({"mcpServers":{
        "codecraft_cursor_probe":{"command":run_executable,"args":["mcp","--root",root,"--server","owned"]},
        "codecraft_cursor_collision":{"command":run_executable,"args":["mcp","--root",root,"--server","collision"]}
    }});
    write_new(&project.join(".cursor/mcp.json"), &servers)?;
    fs::write(project.join("README.md"), "# CodeCraft Cursor P0 isolated project\n\nThis project contains only protocol probes. Both MCP servers expose codecraft_probe_echo for the server identity collision experiment. The tools never run shell commands or read workspace files. No CodeCraft workflow or native approval bridge is enabled.\n").map_err(|_| "PROJECT_WRITE_FAILED")?;
    fs::write(
        project.join("sample.txt"),
        "CodeCraft Cursor P0 harmless read fixture.\n",
    )
    .map_err(|_| "PROJECT_WRITE_FAILED")?;
    Ok(())
}

fn command_path(path: &Path) -> Result<String, String> {
    let value = path.to_str().ok_or("NON_UNICODE_PATH")?;
    // Windows shell expansion could make even a quoted command unsafe.
    if value.contains(['"', '%', '!', '$', '`', '\r', '\n']) {
        return Err("UNSAFE_COMMAND_PATH".into());
    }
    Ok(format!("\"{value}\""))
}

fn write_new(path: &Path, value: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|_| "ENCODE_FAILED")?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|_| "CREATE_FAILED")?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "WRITE_FAILED".into())
}

pub fn manifest(root: &Path) -> Result<Value, String> {
    let file = File::open(root.join("run.json")).map_err(|_| "RUN_MISSING")?;
    let value = protocol::read_input(file)?;
    if value["schemaVersion"] != 1
        || value["captureOnly"] != true
        || value["aliasSalt"].as_str().is_none()
    {
        return Err("INVALID_RUN".into());
    }
    let executable = std::env::current_exe().map_err(|_| "EXECUTABLE_MISSING")?;
    let fingerprint = sha256(&fs::read(executable).map_err(|_| "EXECUTABLE_UNREADABLE")?);
    if value["probeSha256"] != fingerprint {
        return Err("PROBE_BUILD_CHANGED_CREATE_NEW_RUN".into());
    }
    Ok(value)
}

fn native_path(value: &str) -> Option<PathBuf> {
    // URI.fsPath and URI.path differ on Windows. The latter slash-drive form
    // was observed in real Hook workspace_roots. Do not decode arbitrary URLs.
    let bytes = value.as_bytes();
    let value = if cfg!(windows)
        && bytes.len() >= 4
        && bytes[0] == b'/'
        && bytes[1].is_ascii_alphabetic()
        && bytes[2] == b':'
        && bytes[3] == b'/'
    {
        &value[1..]
    } else {
        value
    };
    Path::new(value).canonicalize().ok()
}

pub fn in_scope(manifest: &Value, input: &Value) -> bool {
    let Some(expected) = manifest["project"].as_str().and_then(native_path) else {
        return false;
    };
    let Some(roots) = input["workspace_roots"].as_array() else {
        return false;
    };
    if roots.len() != 1 || roots[0].as_str().and_then(native_path).as_ref() != Some(&expected) {
        return false;
    }
    // If cwd is present, it must stay inside the single isolated project.
    input.get("cwd").is_none_or(|cwd| {
        cwd.as_str()
            .and_then(native_path)
            .is_some_and(|p| p.starts_with(&expected))
    })
}

pub fn alias(salt: &str, value: &str) -> String {
    // No raw ID/ticket or reversible ticket mapping is written to event logs.
    format!("alias:{}", sha256(format!("{salt}\0{value}").as_bytes()))
}

const KNOWN_FIELDS: &[&str] = &[
    "hook_event_name",
    "conversation_id",
    "generation_id",
    "session_id",
    "tool_use_id",
    "tool_call_id",
    "tool_name",
    "tool_input",
    "tool_output",
    "mcp_server_name",
    "command",
    "args",
    "url",
    "mcp_server_url",
    "workspace_roots",
    "cwd",
    "model",
    "model_id",
    "model_params",
    "agent_message",
    "user_message",
    "bridgeTicket",
    "schemaVersion",
    "clientRequestId",
    "payload",
    "message",
    "text",
    "nested",
    "values",
    "prompt",
    "attachments",
    "cursor_version",
    "reason",
    "duration",
    "duration_ms",
    "is_interrupt",
    "failure_type",
    "error_message",
    "subagent_id",
    "parent_conversation_id",
    "subagent_type",
    "status",
    "response",
    "is_background_agent",
    "composer_mode",
    "jsonrpc",
    "id",
    "method",
    "params",
    "name",
    "arguments",
    "protocolVersion",
    "capabilities",
    "clientInfo",
];

pub fn redact(salt: &str, value: &Value) -> Value {
    match value {
        Value::String(text) => {
            json!({"type":"string","length":text.chars().count(),"alias":alias(salt,text)})
        }
        Value::Array(values) => Value::Array(values.iter().map(|v| redact(salt, v)).collect()),
        Value::Object(values) => Value::Object(
            values
                .iter()
                .map(|(k, v)| {
                    let key = if KNOWN_FIELDS.contains(&k.as_str()) {
                        k.clone()
                    } else {
                        alias(salt, k)
                    };
                    (key, redact(salt, v))
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

fn safe_tool(value: &Value) -> Option<&str> {
    value.as_str().filter(|name| {
        matches!(
            *name,
            "Shell"
                | "Read"
                | "Write"
                | "Edit"
                | "Grep"
                | "Glob"
                | "Task"
                | "Delete"
                | "MCP:codecraft_probe_echo"
                | "codecraft_probe_echo"
        )
    })
}

pub fn record(
    root: &Path,
    origin: &str,
    event: &str,
    input: &Value,
    extra: Value,
) -> Result<(), String> {
    let manifest = manifest(root)?;
    let salt = manifest["aliasSalt"].as_str().ok_or("INVALID_RUN")?;
    let received_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "CLOCK_FAILED")?
        .as_millis();
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("capture.lock"))
        .map_err(|_| "LOCK_FAILED")?;
    let started = Instant::now();
    while lock.try_lock_exclusive().is_err() {
        if started.elapsed() >= Duration::from_secs(3) {
            return Err("CAPTURE_BUSY".into());
        }
        thread::sleep(Duration::from_millis(10));
    }
    let entries = fs::read_dir(root.join("events"))
        .map_err(|_| "EVENTS_UNREADABLE")?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "EVENTS_UNREADABLE")?;
    let total = entries
        .iter()
        .try_fold(0u64, |sum, e| {
            e.metadata().map(|m| sum.saturating_add(m.len()))
        })
        .map_err(|_| "EVENTS_UNREADABLE")?;
    let mut sequence = 0u64;
    for entry in &entries {
        if let Some(index) = entry
            .path()
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.parse::<u64>().ok())
        {
            sequence = sequence.max(index);
        }
    }
    if entries.len() >= MAX_RECORDS {
        return Err("CAPTURE_BACKPRESSURE".into());
    }
    let mut details = json!({
        "schemaVersion":1,"evidenceKind":"probeCapture","cursorClientExecuted":null,"probeInjected":false,
        "runId":manifest["runId"],"probeSha256":manifest["probeSha256"],
        "sequence":sequence+1,"receivedAtUnixMs":received_at,"eventId":Uuid::new_v4().to_string(),
        "origin":origin,"event":event,"input":redact(salt,input),"details":extra
    });
    if let Some(name) = safe_tool(&input["tool_name"]) {
        details["toolName"] = json!(name);
    }
    if let Some(failure) = input["failure_type"]
        .as_str()
        .filter(|s| ["permission_denied", "timeout", "error"].contains(s))
    {
        details["failureType"] = json!(failure);
    }
    if let Some(server) = input["mcp_server_name"]
        .as_str()
        .filter(|s| ["codecraft_cursor_probe", "codecraft_cursor_collision"].contains(s))
    {
        details["probeServerName"] = json!(server);
    }
    if let Ok(arguments) = protocol::tool_input(input) {
        details["parsedToolInput"] = redact(salt, &arguments);
        if let Some(ticket) = arguments.get("bridgeTicket").and_then(Value::as_str) {
            details["ticketAlias"] = json!(alias(salt, ticket));
        }
    }
    let bytes = serde_json::to_vec_pretty(&details).map_err(|_| "ENCODE_FAILED")?;
    if bytes.len() > protocol::MAX_MESSAGE_BYTES || total + bytes.len() as u64 > MAX_CAPTURE_BYTES {
        return Err("CAPTURE_BACKPRESSURE".into());
    }
    // Readers only enumerate .json, so a crash before the immutable publication
    // leaves an ignored temporary file rather than a truncated event fixture.
    let temporary = root.join("events").join(format!("{}.tmp", Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|_| "CREATE_FAILED")?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "WRITE_FAILED")?;
    drop(file);
    let published = root
        .join("events")
        .join(format!("{:06}.json", sequence + 1));
    fs::hard_link(&temporary, &published).map_err(|_| "PUBLISH_FAILED")?;
    fs::remove_file(temporary).map_err(|_| "TEMP_CLEANUP_FAILED")?;
    Ok(())
}

pub fn capture_hook(root: &Path, event: &str, input: &Value, utf8_bom: bool) -> Result<(), String> {
    let manifest = manifest(root)?;
    if !in_scope(&manifest, input) {
        return Err("PROJECT_SCOPE_UNVERIFIED".into());
    }
    if input["hook_event_name"] != event {
        return Err("EVENT_MISMATCH".into());
    }
    let identity = if event == "preToolUse"
        && input["tool_name"]
            .as_str()
            .is_some_and(|n| n.starts_with("MCP:"))
    {
        "UNQUALIFIED_MCP_IDENTITY"
    } else {
        "OBSERVE_ONLY"
    };
    record(
        root,
        "hookStdin",
        event,
        input,
        json!({"identityAssessment":identity,"inputModified":false,"permissionWaitCreated":false,"utf8BomRemoved":utf8_bom}),
    )
}

pub fn event_paths(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut paths = fs::read_dir(root.join("events"))
        .map_err(|_| "EVENTS_UNREADABLE")?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aliases_correlate_without_revealing_ticket_or_path() {
        let raw = json!({"bridgeTicket":"secret-ticket","cwd":"private-path","unexpected-secret-key":"secret-ticket"});
        let result = redact("salt", &raw).to_string();
        for secret in ["secret-ticket", "private-path", "unexpected-secret-key"] {
            assert!(!result.contains(secret));
        }
        assert_eq!(alias("salt", "same"), alias("salt", "same"));
        assert_ne!(alias("salt", "same"), alias("other-run", "same"));
    }
}
