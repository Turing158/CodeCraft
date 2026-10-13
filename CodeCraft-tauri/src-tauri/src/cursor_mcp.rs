//! Public, short-call MCP workflow API. No native ticket injection or implicit
//! conversation matching. Only the authenticated stdio adapter calls `call`.
use crate::{
    interaction,
    schema::{self, Result, ToolRequest},
    store::Store,
    workflow::{self, RpcId},
};
use serde::Deserialize;
use serde_json::{json, Value};

fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
fn id() -> Value {
    json!({"type":"string","minLength":1,"maxLength":128,"pattern":"^[A-Za-z0-9_.:-]+$"})
}
fn string(max: usize) -> Value {
    json!({"type":"string","maxLength":max})
}
fn tool(name: &str, description: &str, schema: Value) -> Value {
    json!({"name":name,"description":description,"inputSchema":schema})
}
pub fn tools() -> Value {
    let option = object(
        json!({"optionId":id(),"label":string(2048)}),
        &["optionId", "label"],
    );
    let question = object(
        json!({"questionId":id(),"prompt":string(8192),"options":{"type":"array","maxItems":64,"items":option},
        "multiSelect":{"type":"boolean"},"allowFreeText":{"type":"boolean"},"required":{"type":"boolean"},"allowSkip":{"type":"boolean"},
        "maxTextLength":{"type":"integer","minimum":0,"maximum":4096}}),
        &[
            "questionId",
            "prompt",
            "options",
            "multiSelect",
            "allowFreeText",
            "required",
            "allowSkip",
            "maxTextLength",
        ],
    );
    let phase = object(
        json!({"id":id(),"title":string(2048),"description":string(8192)}),
        &["id", "title", "description"],
    );
    let todo = object(
        json!({"id":id(),"description":string(8192),"status":{"enum":["pending","inProgress","completed","cancelled"]},
        "phaseId":{"type":["string","null"],"maxLength":128}}),
        &["id", "description", "status"],
    );
    json!([
        tool("codecraft_list_projects","List projects explicitly enabled in CodeCraft desktop Settings > Hook. These are review targets, not a claim about the active Cursor workspace.",object(json!({}),&[])),
        tool("codecraft_start_workflow","Start a separate CodeCraft review workflow for one enabled project. Call once per independent task; keep the returned workflowId. It is bound to this MCP connection.",object(json!({"projectId":id(),"title":string(2048)}),&["projectId","title"])),
        tool(schema::ASK_USER,"Ask a human through CodeCraft desktop/LAN. Returns pending. Poll codecraft_get_result about once a second; never invent an answer or proceed while pending. New clientRequestId means new human review.",object(json!({"workflowId":id(),"schemaVersion":{"const":1},"clientRequestId":id(),"questions":{"type":"array","minItems":1,"maxItems":32,"items":question}}),&["workflowId","schemaVersion","clientRequestId","questions"])),
        tool(schema::REVIEW_PLAN,"Request human plan review in CodeCraft. Supply the complete plan. First baseRevision is 0; revisions must name supersedesClientRequestId. Poll codecraft_get_result. This does not enforce native Cursor execution restrictions.",object(json!({"workflowId":id(),"schemaVersion":{"const":1},"clientRequestId":id(),"planId":id(),"baseRevision":{"type":"integer","minimum":0},"supersedesClientRequestId":{"type":["string","null"],"maxLength":128},"title":string(2048),"overview":string(16384),"markdown":string(131072),"todos":{"type":"array","maxItems":256,"items":todo},"phases":{"type":"array","maxItems":64,"items":phase}}),&["workflowId","schemaVersion","clientRequestId","planId","baseRevision","title","overview","markdown","todos","phases"])),
        tool("codecraft_get_result","Poll this workflow's request for a human result. Pending is not approval. A decision is delivered at most once; after interrupted/unknown delivery use a NEW clientRequestId to request a fresh human decision.",object(json!({"workflowId":id(),"requestId":id()}),&["workflowId","requestId"])),
        tool("codecraft_cancel_request","Cancel this workflow request. This does not cancel native Cursor execution or retract an already delivered result.",object(json!({"workflowId":id(),"requestId":id()}),&["workflowId","requestId"])),
        tool("codecraft_end_workflow","Close a finished or abandoned independent workflow and cancel its unfinished requests. Does not retract delivered results or stop native Cursor tools.",object(json!({"workflowId":id()}),&["workflowId"]))
    ])
}
fn execute(store: &mut Store, command: interaction::Command) -> Result<Value> {
    match store.execute(workflow::Command::Independent(command))? {
        workflow::Response::Independent { value } => Ok(value),
        _ => Err("RESPONSE_INVALID"),
    }
}
pub(crate) fn call(
    store: &mut Store,
    connection: &str,
    rpc_id: RpcId,
    name: &str,
    mut arguments: Value,
    wall_ms: u64,
) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Open {
        project_id: String,
        title: String,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Target {
        workflow_id: String,
        request_id: String,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct End {
        workflow_id: String,
    }
    match name {
        "codecraft_end_workflow" => {
            let args: End = serde_json::from_value(arguments).map_err(|_| "INVALID_ARGUMENTS")?;
            if !schema::valid_id(&args.workflow_id) {
                return Err("INVALID_ARGUMENTS");
            }
            execute(
                store,
                interaction::Command::End {
                    connection: connection.into(),
                    workflow_id: args.workflow_id,
                },
            )
        }
        "codecraft_list_projects" => {
            if arguments != json!({}) {
                return Err("INVALID_ARGUMENTS");
            }
            let projects = store
                .model()
                .independent_projects()
                .as_array()
                .ok_or("INVALID_PROJECTS")?
                .iter()
                .filter(|p| p["enabled"] == true)
                .cloned()
                .collect::<Vec<_>>();
            Ok(json!({"projects":projects}))
        }
        "codecraft_start_workflow" => {
            let args: Open = serde_json::from_value(arguments).map_err(|_| "INVALID_ARGUMENTS")?;
            execute(
                store,
                interaction::Command::Open {
                    connection: connection.into(),
                    project_id: args.project_id,
                    title: args.title,
                },
            )
        }
        schema::ASK_USER | schema::REVIEW_PLAN => {
            let args = arguments.as_object_mut().ok_or("INVALID_ARGUMENTS")?;
            let workflow_id = args
                .remove("workflowId")
                .and_then(|v| v.as_str().map(str::to_owned))
                .filter(|s| schema::valid_id(s))
                .ok_or("WORKFLOW_REQUIRED")?;
            if args.contains_key("bridgeTicket") {
                return Err("UNEXPECTED_TICKET");
            }
            let payload = ToolRequest::parse(
                name,
                &serde_json::to_vec(&arguments).map_err(|_| "INVALID_ARGUMENTS")?,
            )?;
            execute(
                store,
                interaction::Command::Mcp {
                    connection: connection.into(),
                    workflow_id,
                    rpc_id,
                    payload,
                    wall_ms,
                },
            )
        }
        "codecraft_get_result" | "codecraft_cancel_request" => {
            let args: Target =
                serde_json::from_value(arguments).map_err(|_| "INVALID_ARGUMENTS")?;
            if !schema::valid_id(&args.workflow_id) || !schema::valid_id(&args.request_id) {
                return Err("INVALID_ARGUMENTS");
            }
            execute(
                store,
                if name == "codecraft_get_result" {
                    interaction::Command::Poll {
                        connection: connection.into(),
                        workflow_id: Some(args.workflow_id),
                        request_id: args.request_id,
                    }
                } else {
                    interaction::Command::Cancel {
                        connection: connection.into(),
                        workflow_id: args.workflow_id,
                        request_id: args.request_id,
                    }
                },
            )
        }
        _ => Err("UNKNOWN_TOOL"),
    }
}
