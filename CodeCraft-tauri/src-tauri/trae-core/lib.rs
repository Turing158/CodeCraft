#[allow(dead_code)]
#[path = "../src/approval_policy.rs"]
mod approval_policy;

#[path = "../src/trae_files.rs"]
pub mod files;
#[path = "../src/trae_hook.rs"]
pub mod hook;
#[path = "../src/trae_mcp.rs"]
pub mod mcp;
#[path = "../src/trae_native.rs"]
pub mod native;
#[path = "../src/trae_protocol.rs"]
pub mod protocol;
#[path = "../src/trae.rs"]
pub mod store;
#[cfg(test)]
mod tests;
#[path = "../src/trae_transport.rs"]
pub mod transport;
#[path = "../src/trae_wire.rs"]
pub mod wire;

pub fn run_mcp() -> std::result::Result<(), String> {
    Err("Trae questions and Plan/Spec are read-only native interactions. Remove the legacy CodeCraft MCP server and use the Trae Hook integration.".into())
}

pub fn dispatch(route: &str, body: serde_json::Value) -> protocol::Result<serde_json::Value> {
    if !matches!(route, "permission" | "cancel" | "grants/resolve") {
        return Err(protocol::ApiError::new(protocol::ErrorCode::Forbidden,
            "Trae questions and plans must be handled in Trae; task/MCP workflow controls are retired"));
    }
    let kind = match route {
        "permission" | "question" | "plan" | "cancel" => {
            let d: protocol::DecisionRequest = serde_json::from_value(body.clone())
                .map_err(|e| protocol::invalid(e.to_string()))?;
            d.validate()?;
            let actual = match d.action {
                protocol::Action::Permission { .. } => "permission",
                protocol::Action::Question { .. } => "question",
                protocol::Action::Plan { .. } => "plan",
                protocol::Action::Cancel { .. } => "cancel",
            };
            if actual != route {
                return Err(protocol::invalid("Action does not match this endpoint"));
            }
            "decision"
        }
        "tasks" => "create_task",
        "tasks/action" => "task_action",
        "grants/resolve" => "resolve_grant",
        _ => return Err(protocol::invalid("Unknown Trae endpoint")),
    };
    files::Client::connect(&files::root())?.call(serde_json::json!({"kind":kind,"body":body}))
}
