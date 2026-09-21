use crate::protocol::{ASK_TOOL, PLAN_TOOL};
use crate::wire as protocol;
use crate::wire::MAX_NATIVE;
use compat::{Client, Command};
const POLL: std::time::Duration = std::time::Duration::from_millis(200);
mod compat {
    use serde::Serialize;
    use serde_json::Value;
    use std::path::{Path, PathBuf};
    #[derive(Clone)]
    pub struct Client {
        root: PathBuf,
    }
    impl Client {
        pub fn connect(root: &Path) -> Result<Self, String> {
            Ok(Self { root: root.into() })
        }
        pub fn call(&self, c: Command) -> Result<Value, String> {
            crate::files::Client::connect(&self.root)
                .and_then(|client| client.call(serde_json::to_value(c).unwrap()))
                .map_err(|e| serde_json::to_string(&e).unwrap())
        }
    }
    #[derive(Serialize)]
    #[serde(tag = "kind", rename_all = "snake_case")]
    pub enum Command {
        Consume {
            connection: String,
            ticket: String,
            tool: String,
            arguments: Value,
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
    }
}
use rmcp::{model::*, service::RequestContext, ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

#[derive(Clone)]
struct Delivery {
    client: Client,
    connection: String,
    operation: String,
    lease: String,
}
#[derive(Clone)]
struct TraeServer {
    client: Client,
    connection: String,
    deliveries: Arc<Mutex<HashMap<String, Delivery>>>,
    requests: Arc<Mutex<HashMap<String, (String, bool)>>>,
}

async fn command(client: &Client, value: Command) -> Result<Value, String> {
    let client = client.clone();
    tokio::task::spawn_blocking(move || client.call(value))
        .await
        .map_err(|e| e.to_string())?
}

struct CancelOnDrop {
    client: Client,
    connection: String,
    operation: String,
    armed: bool,
}
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if self.armed {
            let client = self.client.clone();
            let connection = self.connection.clone();
            let operation = self.operation.clone();
            tokio::task::spawn_blocking(move || {
                let _ = client.call(Command::Cancel {
                    connection,
                    operation,
                });
            });
        }
    }
}

fn error_result(message: String, operation: Option<&str>) -> CallToolResponse {
    let error = serde_json::from_str::<crate::protocol::ApiError>(&message).unwrap_or_else(|_| {
        crate::protocol::ApiError::new(
            if message.starts_with("CANCELLED") {
                crate::protocol::ErrorCode::Cancelled
            } else if message.starts_with("TICKET_INVALID") {
                crate::protocol::ErrorCode::TicketInvalid
            } else {
                crate::protocol::ErrorCode::StateUnavailable
            },
            message,
        )
    });
    let result = crate::protocol::ToolResult::failure(error, operation.map(str::to_string));
    let cancelled = result.status == "cancelled";
    let value = serde_json::to_value(result).unwrap();
    if cancelled {
        CallToolResult::structured(value).into()
    } else {
        CallToolResult::structured_error(value).into()
    }
}
fn definition(name: &'static str) -> Tool {
    let schemas = crate::protocol::schemas();
    let schema = &schemas[if name == ASK_TOOL {
        "QuestionInput"
    } else {
        "PlanInput"
    }];
    let mut tool = Tool::new(
        name,
        if name == ASK_TOOL {
            "Ask the user in CodeCraft. Requires a bridgeTicket injected by the Trae Hook; preserve question and option IDs."
        } else {
            "Review the bound plan file in CodeCraft. Approval applies only to this task, turn and content revision; native confirmations remain in Trae."
        },
        schema.as_object().unwrap().clone(),
    );
    tool.output_schema = Some(Arc::new(schemas["ToolResult"].as_object().unwrap().clone()));
    tool
}
impl ServerHandler for TraeServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }
    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult {
            tools: vec![definition(ASK_TOOL), definition(PLAN_TOOL)],
            ..Default::default()
        })
    }
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        if request.name != ASK_TOOL && request.name != PLAN_TOOL {
            return Err(ErrorData::invalid_params("Unknown CodeCraft tool", None));
        }
        let arguments = Value::Object(request.arguments.unwrap_or_default());
        if let Err(e) = crate::protocol::validate_business(&request.name, &arguments)
            .map_err(|e| serde_json::to_string(&e).unwrap())
        {
            return Ok(error_result(e, None));
        }
        let Some(ticket) = arguments["bridgeTicket"].as_str() else {
            return Ok(error_result(
                "TICKET_INVALID: missing Hook ticket".into(),
                None,
            ));
        };
        let consumed = match command(
            &self.client,
            Command::Consume {
                connection: self.connection.clone(),
                ticket: ticket.into(),
                tool: request.name.into_owned(),
                arguments,
            },
        )
        .await
        {
            Ok(v) => v,
            Err(e) => return Ok(error_result(e, None)),
        };
        let operation = consumed["operationId"].as_str().unwrap().to_string();
        self.requests.lock().unwrap().insert(
            serde_json::to_string(&context.id).unwrap(),
            (operation.clone(), consumed["replayed"] != true),
        );
        let mut guard = CancelOnDrop {
            client: self.client.clone(),
            connection: self.connection.clone(),
            operation: operation.clone(),
            armed: consumed["replayed"] != true,
        };
        loop {
            if context.ct.is_cancelled() {
                return Ok(error_result("CANCELLED".into(), Some(&operation)));
            }
            match command(
                &self.client,
                Command::Poll {
                    connection: self.connection.clone(),
                    operation: operation.clone(),
                },
            )
            .await
            {
                Ok(v) if v["state"] != "pending" => break,
                Ok(_) => (),
                Err(e) => return Ok(error_result(e, Some(&operation))),
            }
            tokio::select! { _=context.ct.cancelled()=>return Ok(error_result("CANCELLED".into(),Some(&operation))), _=tokio::time::sleep(POLL)=>() }
        }
        let prepared = match command(
            &self.client,
            Command::Prepare {
                connection: self.connection.clone(),
                operation: operation.clone(),
            },
        )
        .await
        {
            Ok(v) => v,
            Err(e) => return Ok(error_result(e, Some(&operation))),
        };
        if context.ct.is_cancelled() {
            return Ok(error_result("CANCELLED".into(), Some(&operation)));
        }
        if prepared["state"] != "delivered" {
            self.deliveries.lock().unwrap().insert(
                serde_json::to_string(&context.id).unwrap(),
                Delivery {
                    client: self.client.clone(),
                    connection: self.connection.clone(),
                    operation: operation.clone(),
                    lease: prepared["lease"].as_str().unwrap().into(),
                },
            );
        }
        // The transport, not the handler, acknowledges after stdout.flush().
        // Losing the response at any point before that cannot activate delivery.
        guard.armed = false;
        Ok(CallToolResult::structured(prepared["result"].clone()).into())
    }
}

/// Keep the official SDK responsible for MCP negotiation, ping, dispatch and
/// cancellation. The bounded framing shim rejects duplicate JSON keys before
/// the SDK can collapse them, and observes the actual stdout flush boundary.
pub async fn serve(root: &Path) -> Result<(), String> {
    let client = Client::connect(root)?;
    let deliveries = Arc::new(Mutex::new(HashMap::<String, Delivery>::new()));
    let requests = Arc::new(Mutex::new(HashMap::<String, (String, bool)>::new()));
    let connection = crate::protocol::id();
    let service = TraeServer {
        client: client.clone(),
        connection: connection.clone(),
        deliveries: deliveries.clone(),
        requests: requests.clone(),
    };
    let (sdk, wire) = tokio::io::duplex(64 * 1024);
    let (sdk_read, sdk_write) = tokio::io::split(sdk);
    let (wire_read, mut wire_write) = tokio::io::split(wire);
    let stdout = Arc::new(tokio::sync::Mutex::new(tokio::io::stdout()));
    let input_stdout = stdout.clone();
    let input_requests = requests.clone();
    let input =
        tokio::spawn(async move {
            let mut reader = BufReader::new(tokio::io::stdin());
            let mut seen = HashMap::<String, String>::new();
            let result = async {
        loop {
            let mut line = Vec::new();
            let count = (&mut reader)
                .take((MAX_NATIVE + 2) as u64)
                .read_until(b'\n', &mut line)
                .await
                .map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            if line.last() == Some(&b'\n') {
                line.pop();
            }
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let value = match protocol::strict_json(&line, MAX_NATIVE) {
                Ok(v) => v,
                Err(e) => {
                    let response =
                        json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":e}});
                    let mut out = input_stdout.lock().await;
                    out.write_all(format!("{response}\n").as_bytes())
                        .await
                        .map_err(|e| e.to_string())?;
                    out.flush().await.map_err(|e| e.to_string())?;
                    return Err(e);
                }
            };
            if value["method"] == "notifications/cancelled" {
                let tracked = input_requests
                    .lock()
                    .unwrap()
                    .get(&value["params"]["requestId"].to_string())
                    .cloned();
                if let Some((operation, true)) = tracked {
                    // Also covers cancellation after handler return but before
                    // transport flush/ack; a completed handler isn't delivery.
                    let _ = command(
                        &client,
                        Command::Cancel {
                            connection: connection.clone(),
                            operation,
                        },
                    )
                    .await;
                }
            }
            if value.get("method").is_some() && value.get("id").is_some() {
                let id = value["id"].to_string();
                let hash = protocol::hash(&value)?;
                if let Some(old) = seen.get(&id) {
                    if old != &hash {
                        return Err(
                            "IDEMPOTENCY_CONFLICT: JSON-RPC ID reused with different input".into(),
                        );
                    }
                    continue;
                }
                if seen.len() >= 4096 {
                    return Err("QUEUE_FULL: MCP request ID limit".into());
                }
                seen.insert(id, hash);
            }
            wire_write
                .write_all(&line)
                .await
                .map_err(|e| e.to_string())?;
            wire_write
                .write_all(b"\n")
                .await
                .map_err(|e| e.to_string())?;
        }
        Ok::<(), String>(())
        }.await;
            // Dropping a split duplex WriteHalf does not signal EOF while its read
            // half is still alive. Explicitly shut down even after framing errors.
            let shutdown = wire_write.shutdown().await.map_err(|e| e.to_string());
            result.and(shutdown)
        });

    let mut output = tokio::spawn(async move {
        let mut reader = BufReader::new(wire_read);
        loop {
            let mut line = Vec::new();
            let count = (&mut reader)
                .take((MAX_NATIVE + 2) as u64)
                .read_until(b'\n', &mut line)
                .await
                .map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            if count > MAX_NATIVE + 1 {
                return Err("PAYLOAD_TOO_LARGE: MCP result".into());
            }
            let value = protocol::strict_json(&line, MAX_NATIVE + 1)?;
            let request_key = value["id"].to_string();
            let delivery = deliveries.lock().unwrap().get(&request_key).cloned();
            let mut out = stdout.lock().await;
            out.write_all(&line).await.map_err(|e| e.to_string())?;
            out.flush().await.map_err(|e| e.to_string())?;
            drop(out);
            if let Some(d) = delivery {
                if let Err(e) = command(
                    &d.client,
                    Command::Ack {
                        connection: d.connection,
                        operation: d.operation,
                        lease: d.lease,
                    },
                )
                .await
                {
                    eprintln!("Trae delivery was flushed but not confirmed: {e}");
                }
            }
            deliveries.lock().unwrap().remove(&request_key);
            requests.lock().unwrap().remove(&request_key);
        }
        Ok::<(), String>(())
    });
    let running = service
        .serve((sdk_read, sdk_write))
        .await
        .map_err(|e| e.to_string())?;
    let cancel = running.cancellation_token();
    tokio::select! {
        finished=running.waiting()=>{
            finished.map_err(|e|e.to_string())?;
            if input.is_finished() {input.await.map_err(|e|e.to_string())??;} else {input.abort();}
            output.await.map_err(|e|e.to_string())??;
        }
        finished=&mut output=>{
            // A broken stdout must stop dispatch immediately, even if the
            // client keeps stdin open. No delivery acknowledgement was sent.
            cancel.cancel();
            if input.is_finished() {input.await.map_err(|e|e.to_string())??;} else {input.abort();}
            finished.map_err(|e|e.to_string())??;
        }
    }
    Ok(())
}
