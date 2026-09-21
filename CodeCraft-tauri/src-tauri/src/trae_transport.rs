//! Authenticated loopback transport for sandboxed Hooks. Only the desktop-side
//! workers write the existing durable IPC queue; clients need read access only.
use crate::{files, protocol::*, wire};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

const ENDPOINT_FILE: &str = "hook-endpoint.json";
const IO_TIMEOUT: Duration = Duration::from_secs(5);

fn receive_exact(stream: &mut TcpStream, mut buffer: &mut [u8], deadline: Instant) -> Result<()> {
    while !buffer.is_empty() {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| {
                ApiError::new(ErrorCode::BridgeUnavailable, "Hook transport timed out")
            })?;
        stream.set_read_timeout(Some(remaining))?;
        let count = stream.read(buffer)?;
        if count == 0 {
            return Err(invalid("Incomplete Hook transport frame"));
        }
        buffer = &mut buffer[count..];
    }
    Ok(())
}
fn receive(stream: &mut TcpStream) -> Result<Value> {
    let deadline = Instant::now() + IO_TIMEOUT;
    let mut header = [0; 4];
    receive_exact(stream, &mut header, deadline)?;
    let size = u32::from_be_bytes(header) as usize;
    if size == 0 || size > wire::MAX_IPC {
        return Err(ApiError::new(
            ErrorCode::PayloadTooLarge,
            "Hook transport frame exceeds limit",
        ));
    }
    let mut bytes = vec![0; size];
    receive_exact(stream, &mut bytes, deadline)?;
    wire::strict_json(&bytes, wire::MAX_IPC).map_err(invalid)
}
fn send(stream: &mut TcpStream, message: &Value) -> Result<()> {
    let bytes = serde_json::to_vec(message).map_err(|e| invalid(e.to_string()))?;
    if bytes.len() > wire::MAX_IPC {
        return Err(ApiError::new(
            ErrorCode::PayloadTooLarge,
            "Hook transport frame exceeds limit",
        ));
    }
    let mut frame = (bytes.len() as u32).to_be_bytes().to_vec();
    frame.extend_from_slice(&bytes);
    let mut remaining = frame.as_slice();
    let deadline = Instant::now() + IO_TIMEOUT;
    while !remaining.is_empty() {
        let timeout = deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| {
                ApiError::new(ErrorCode::BridgeUnavailable, "Hook transport timed out")
            })?;
        stream.set_write_timeout(Some(timeout))?;
        let count = stream.write(remaining)?;
        if count == 0 {
            return Err(invalid("Hook transport closed while writing"));
        }
        remaining = &remaining[count..];
    }
    Ok(())
}
fn token_matches(expected: &str, actual: &str) -> bool {
    expected.len() == actual.len()
        && expected
            .bytes()
            .zip(actual.bytes())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}
fn authorize(message: &Value, epoch: &str, token: &str) -> Result<()> {
    if !message["token"]
        .as_str()
        .is_some_and(|v| token_matches(token, v))
    {
        return Err(ApiError::new(
            ErrorCode::Forbidden,
            "Invalid Hook transport credential",
        ));
    }
    if message["schemaVersion"] != 1 {
        return Err(invalid("Invalid Hook transport version"));
    }
    if message["appEpoch"] != epoch {
        return Err(ApiError::new(
            ErrorCode::RequestExpired,
            "CodeCraft restarted",
        ));
    }
    check_id(
        message["commandId"]
            .as_str()
            .ok_or_else(|| invalid("Missing command ID"))?,
    )?;
    let command = message["command"]
        .as_object()
        .ok_or_else(|| invalid("Missing Hook command"))?;
    let fields: &[&str] = match command.get("kind").and_then(Value::as_str) {
        Some("hook") => &["kind", "input", "identity"],
        Some("poll" | "prepare") => &["kind", "operation"],
        Some("ack") => &["kind", "operation", "lease"],
        _ => {
            return Err(ApiError::new(
                ErrorCode::Forbidden,
                "Hook transport cannot submit approvals or control tasks",
            ))
        }
    };
    if command.keys().any(|key| !fields.contains(&key.as_str())) {
        return Err(ApiError::new(
            ErrorCode::Forbidden,
            "Unexpected Hook transport command field",
        ));
    }
    Ok(())
}

/// The only credentials exposed to a Hook authorize event delivery and collection
/// of an existing decision. They never authorize creating a user decision.
pub struct Client {
    root: PathBuf,
    port: u16,
    token: String,
    pub epoch: String,
}
impl Client {
    pub fn connect(root: &Path) -> Result<Self> {
        let epoch = files::Client::connect(root)?.epoch;
        let endpoint = files::read(&root.join(ENDPOINT_FILE), 4096).map_err(|_| {
            ApiError::new(
                ErrorCode::BridgeUnavailable,
                "Restart CodeCraft to start the Trae Hook bridge",
            )
        })?;
        if endpoint["schemaVersion"] != 1 || endpoint["appEpoch"] != epoch {
            return Err(ApiError::new(
                ErrorCode::RequestExpired,
                "Hook bridge belongs to a previous CodeCraft process",
            ));
        }
        let port = endpoint["port"]
            .as_u64()
            .filter(|v| *v > 0 && *v <= u16::MAX as u64)
            .ok_or_else(|| invalid("Invalid loopback port"))? as u16;
        let token = endpoint["token"]
            .as_str()
            .filter(|v| v.len() == 64 && v.bytes().all(|c| c.is_ascii_hexdigit()))
            .ok_or_else(|| invalid("Invalid Hook transport credential"))?
            .to_owned();
        Ok(Self {
            root: root.into(),
            port,
            token,
            epoch,
        })
    }
    pub fn call(&self, command: Value) -> Result<Value> {
        self.call_id(command, &id())
    }
    pub fn call_id(&self, command: Value, command_id: &str) -> Result<Value> {
        if files::Client::connect(&self.root)?.epoch != self.epoch {
            return Err(ApiError::new(
                ErrorCode::RequestExpired,
                "CodeCraft restarted",
            ));
        }
        let message = json!({"schemaVersion":1,"appEpoch":self.epoch,"token":self.token,"commandId":command_id,"command":command});
        authorize(&message, &self.epoch, &self.token)?;
        let hash = wire::hash(&command).map_err(invalid)?;
        let mut stream = TcpStream::connect_timeout(
            &SocketAddr::from((Ipv4Addr::LOCALHOST, self.port)),
            Duration::from_secs(1),
        )?;
        send(&mut stream, &message)?;
        let reply = receive(&mut stream)?;
        if reply["appEpoch"] != self.epoch
            || reply["commandId"] != command_id
            || reply["bodyHash"] != hash
        {
            return Err(invalid("Hook transport reply identity mismatch"));
        }
        if !reply["error"].is_null() {
            return Err(serde_json::from_value(reply["error"].clone())
                .map_err(|e| invalid(e.to_string()))?);
        }
        Ok(reply["result"].clone())
    }
}

pub struct Server {
    stop: Arc<AtomicBool>,
    workers: Vec<JoinHandle<()>>,
    root: PathBuf,
    epoch: String,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
        let path = self.root.join(ENDPOINT_FILE);
        if files::read(&path, 4096).is_ok_and(|v| v["appEpoch"] == self.epoch) {
            let _ = std::fs::remove_file(path);
        }
    }
}
pub fn start(root: &Path, epoch: &str) -> Result<Server> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let mut server = Server {
        stop: Arc::new(AtomicBool::new(false)),
        workers: Vec::new(),
        root: root.into(),
        epoch: epoch.into(),
    };
    // Two bounded workers handle one framed command at a time, never one thread
    // per connection. Long-lived human approval waits stay in the Hook process.
    for index in 0..2 {
        let listener = listener.try_clone()?;
        let stop = server.stop.clone();
        let root = root.to_path_buf();
        let epoch = epoch.to_owned();
        let token = token.clone();
        server.workers.push(std::thread::Builder::new().name(format!("trae-hook-bridge-{index}")).spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, address)) if address.ip().is_loopback() => {
                        if stream.set_nonblocking(false).is_err() { continue; }
                        let Ok(message) = receive(&mut stream) else { continue; };
                        let result = authorize(&message, &epoch, &token).and_then(|()| {
                            files::Client::connect(&root)?.call_id(message["command"].clone(), message["commandId"].as_str().unwrap())
                        });
                        let reply = json!({"appEpoch":epoch,"commandId":message["commandId"],"bodyHash":wire::hash(&message["command"]).ok(),"result":result.as_ref().ok(),"error":result.as_ref().err()});
                        let _ = send(&mut stream, &reply);
                    }
                    Ok(_) => (),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(20)),
                    Err(_) => break,
                }
            }
        })?);
    }
    files::atomic(
        &root.join(ENDPOINT_FILE),
        &json!({"schemaVersion":1,"appEpoch":epoch,"port":port,"token":token}),
        true,
    )?;
    Ok(server)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bridge_credentials_cannot_create_decisions_or_bypass_epochs() {
        let mut message = json!({"schemaVersion":1,"appEpoch":"epoch","token":"secret","commandId":id(),"command":{"kind":"hook","input":{},"identity":{}}});
        assert!(authorize(&message, "epoch", "secret").is_ok());
        assert!(authorize(&message, "epoch", "wrong").is_err());
        assert!(authorize(&message, "new-epoch", "secret").is_err());
        for kind in [
            "decision",
            "create_task",
            "task_action",
            "refresh_capabilities",
            "resolve_grant",
            "consume",
        ] {
            message["command"] = json!({"kind":kind});
            assert_eq!(
                authorize(&message, "epoch", "secret")
                    .unwrap_err()
                    .error
                    .code,
                ErrorCode::Forbidden
            );
        }
        message["command"] = json!({"kind":"prepare","operation":id(),"connection":"mcp"});
        assert_eq!(
            authorize(&message, "epoch", "secret")
                .unwrap_err()
                .error
                .code,
            ErrorCode::Forbidden
        );
    }
}
