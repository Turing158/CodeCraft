//! Small, bounded P0 stdio server. It only exposes a harmless digest tool;
//! it is not the production questions/plans MCP server.
use crate::{
    capture::{self, ECHO_TOOL},
    protocol,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::{BufRead, Write},
    path::Path,
};

pub fn tool_schema() -> Value {
    json!({"type":"object","additionalProperties":false,
        "properties":{
            "schemaVersion":{"type":"integer","const":1},
            "clientRequestId":{"type":"string","minLength":1,"maxLength":128},
            "payload":{"type":"object"},
            "bridgeTicket":{"type":"string","minLength":1,"maxLength":256}
        },"required":["schemaVersion","clientRequestId","payload"]})
}

pub fn validate_arguments(arguments: &Value) -> Result<(), &'static str> {
    let map = arguments.as_object().ok_or("INVALID_ARGUMENTS")?;
    if map.keys().any(|key| {
        ![
            "schemaVersion",
            "clientRequestId",
            "payload",
            "bridgeTicket",
        ]
        .contains(&key.as_str())
    }) {
        return Err("UNKNOWN_ARGUMENT");
    }
    if arguments["schemaVersion"] != 1 {
        return Err("UNSUPPORTED_SCHEMA");
    }
    if !arguments["clientRequestId"]
        .as_str()
        .is_some_and(|s| !s.is_empty() && s.chars().count() <= 128)
        || !arguments["payload"].is_object()
    {
        return Err("INVALID_ARGUMENTS");
    }
    if map.get("bridgeTicket").is_some_and(|v| {
        !v.as_str()
            .is_some_and(|s| !s.is_empty() && s.chars().count() <= 256)
    }) {
        return Err("INVALID_TICKET_FIELD");
    }
    Ok(())
}

fn rpc_error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

#[derive(Default)]
pub struct Server {
    initialized: bool,
    initialize_requested: bool,
    seen: HashMap<String, String>,
}

impl Server {
    pub fn handle(&mut self, input: &Value, server: &str) -> Option<Value> {
        let id = input.get("id").cloned();
        let method = input["method"].as_str();
        let params_valid = input.get("params").is_none_or(Value::is_object);
        if !input.is_object() || input["jsonrpc"] != "2.0" || method.is_none() || !params_valid {
            return Some(rpc_error(Value::Null, -32600, "Invalid Request"));
        }
        if let Some(ref id) = id {
            if !(id.is_string() || id.is_i64() || id.is_u64()) {
                return Some(rpc_error(Value::Null, -32600, "Invalid request ID"));
            }
            let key = id.to_string();
            let digest = protocol::digest(input).ok()?;
            if let Some(previous) = self.seen.get(&key) {
                // Do not replay even a previously written response to a duplicate frame.
                return if previous == &digest {
                    None
                } else {
                    Some(rpc_error(id.clone(), -32600, "RPC_ID_CONFLICT"))
                };
            }
            if self.seen.len() >= 4096 {
                return Some(rpc_error(id.clone(), -32000, "CONNECTION_LIMIT"));
            }
            self.seen.insert(key, digest);
        } else {
            if method == Some("notifications/initialized") && self.initialize_requested {
                self.initialized = true;
            }
            // Cancellation is non-blocking: this probe has no waiting requests.
            return None;
        }
        let id = id.unwrap();
        let result = match method.unwrap() {
            "initialize" => {
                if self.initialize_requested {
                    return Some(rpc_error(id, -32600, "Already initialized"));
                }
                if !input["params"]["protocolVersion"].is_string()
                    || !input["params"]["capabilities"].is_object()
                    || !input["params"]["clientInfo"].is_object()
                {
                    return Some(rpc_error(id, -32602, "Invalid initialize parameters"));
                }
                self.initialize_requested = true;
                let requested = input["params"]["protocolVersion"].as_str().unwrap();
                let protocol = if ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"]
                    .contains(&requested)
                {
                    requested
                } else {
                    "2025-11-25"
                };
                json!({"protocolVersion":protocol,"capabilities":{"tools":{}},"serverInfo":{"name":"codecraft-cursor-probe","version":env!("CARGO_PKG_VERSION")}})
            }
            "ping" => json!({}),
            _ if !self.initialized => return Some(rpc_error(id, -32002, "NOT_INITIALIZED")),
            "tools/list" => {
                json!({"tools":[{"name":ECHO_TOOL,"description":"P0 only: hash a harmless payload. Never runs commands or approves a workflow.","inputSchema":tool_schema(),"annotations":{"readOnlyHint":true,"destructiveHint":false,"openWorldHint":false}}]})
            }
            "tools/call" => {
                if input["params"]["name"] != ECHO_TOOL {
                    return Some(rpc_error(id, -32602, "Unknown probe tool"));
                }
                let arguments = &input["params"]["arguments"];
                if let Err(error) = validate_arguments(arguments) {
                    return Some(rpc_error(id, -32602, error));
                }
                let result = json!({"probeOnly":true,"server":server,"argumentsDigest":protocol::arguments_digest(ECHO_TOOL,arguments).ok()?,"ticketPresent":arguments.get("bridgeTicket").is_some(),"workflowEnabled":false});
                json!({"content":[{"type":"text","text":result.to_string()}],"isError":false})
            }
            _ => return Some(rpc_error(id, -32601, "Method not found")),
        };
        Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
    }
}

pub fn serve(
    root: &Path,
    server_name: &str,
    mut input: impl BufRead,
    mut output: impl Write,
) -> Result<(), String> {
    if !["owned", "collision"].contains(&server_name) {
        return Err("UNKNOWN_PROBE_SERVER".into());
    }
    capture::manifest(root)?;
    let mut server = Server::default();
    loop {
        let frame = match protocol::read_frame(&mut input) {
            Ok(Some(frame)) => frame,
            Ok(None) => return Ok(()),
            Err(error) => {
                write_response(&mut output, &rpc_error(Value::Null, -32700, &error))?;
                // Oversize/truncated streams are not resynchronized by guessing.
                return Err(error);
            }
        };
        let method = frame["method"].as_str().unwrap_or("invalid");
        let known_method = if [
            "initialize",
            "notifications/initialized",
            "notifications/cancelled",
            "ping",
            "tools/list",
            "tools/call",
        ]
        .contains(&method)
        {
            method
        } else {
            "unknown"
        };
        capture::record(
            root,
            "mcpStdin",
            known_method,
            &frame,
            json!({"server":server_name}),
        )?;
        if let Some(response) = server.handle(&frame, server_name) {
            write_response(&mut output, &response)?;
            // A successful local flush is deliberately not a Cursor ACK.
            capture::record(
                root,
                "mcpStdout",
                "responseWritten",
                &json!({"id":response["id"]}),
                json!({"server":server_name,"transport":"written","nativeAcceptance":"unconfirmed"}),
            )?;
        }
    }
}

fn write_response(output: &mut impl Write, value: &Value) -> Result<(), String> {
    serde_json::to_writer(&mut *output, value).map_err(|_| "STDOUT_FAILED")?;
    output
        .write_all(b"\n")
        .and_then(|_| output.flush())
        .map_err(|_| "STDOUT_FAILED".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn initialized() -> Server {
        let mut server = Server::default();
        let response = server.handle(&json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"fixture","version":"1"}}}),"owned").unwrap();
        assert_eq!(response["id"], 0);
        assert!(response.get("result").is_some());
        assert!(server
            .handle(
                &json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
                "owned"
            )
            .is_none());
        server
    }
    #[test]
    fn rpc_zero_duplicate_frames_and_conflicting_ids_are_distinct() {
        let mut server = initialized();
        let call = json!({"jsonrpc":"2.0","id":1,"method":"tools/list"});
        assert!(server.handle(&call, "owned").is_some());
        assert!(server.handle(&call, "owned").is_none());
        assert_eq!(
            server
                .handle(&json!({"jsonrpc":"2.0","id":1,"method":"ping"}), "owned")
                .unwrap()["error"]["message"],
            "RPC_ID_CONFLICT"
        );
    }
    #[test]
    fn schema_accepts_ticketless_business_input_but_never_approves() {
        let arguments = json!({"schemaVersion":1,"clientRequestId":"a","payload":{}});
        assert!(validate_arguments(&arguments).is_ok());
        let response = initialized().handle(&json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":ECHO_TOOL,"arguments":arguments}}),"owned").unwrap();
        let result: Value =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(result["workflowEnabled"], false);
        assert!(!tool_schema()["required"]
            .as_array()
            .unwrap()
            .contains(&json!("bridgeTicket")));
    }
    #[test]
    fn rejects_extra_arguments_and_oversized_ticket() {
        let mut args = json!({"schemaVersion":1,"clientRequestId":"a","payload":{},"extra":true});
        assert!(validate_arguments(&args).is_err());
        args.as_object_mut().unwrap().remove("extra");
        args["bridgeTicket"] = json!("x".repeat(257));
        assert!(validate_arguments(&args).is_err());
    }
}
