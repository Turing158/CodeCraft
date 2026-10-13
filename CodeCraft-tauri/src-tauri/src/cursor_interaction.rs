//! v5 owned interactions. A workflow ID identifies a CodeCraft review context;
//! it never asserts a native Cursor conversation or grants tool execution.
use crate::{
    history, projects, protocol,
    schema::{self, Body, Decision, PermissionAction, ToolRequest},
    workflow::{self, CallBinding, Model, NativeCall, Owner, Phase, Response, RpcId, Transport},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

type Result<T> = schema::Result<T>;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Workflow {
    id: String,
    connection_id: String,
    app_epoch: String,
    project_id: String,
    project_revision: u64,
    title: String,
    active: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Context {
    pub session_key: String,
    pub conversation_id: String,
    pub title: String,
    pub workflow_id: Option<String>,
    pub connection_id: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub deadline_at: u64,
    #[serde(default)]
    pub closed: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Book {
    workflows: BTreeMap<String, Workflow>,
    pub(crate) contexts: BTreeMap<String, Context>,
}
impl Book {
    pub(crate) fn recover(&mut self) {
        for w in self.workflows.values_mut() {
            w.active = false;
        }
        for c in self.contexts.values_mut() {
            c.closed = true;
        }
    }
}

/// Constructed only by the broker after authenticating a transport connection.
#[derive(Clone, Debug)]
pub enum Command {
    End {
        connection: String,
        workflow_id: String,
    },
    DisableProject {
        project_id: String,
    },
    Open {
        connection: String,
        project_id: String,
        title: String,
    },
    Mcp {
        connection: String,
        workflow_id: String,
        rpc_id: RpcId,
        payload: ToolRequest,
        wall_ms: u64,
    },
    Hook {
        connection: String,
        event_id: String,
        event: String,
        input: Value,
        automatic: bool,
        wall_ms: u64,
    },
    Poll {
        connection: String,
        workflow_id: Option<String>,
        request_id: String,
    },
    Cancel {
        connection: String,
        workflow_id: String,
        request_id: String,
    },
    Report {
        connection: String,
        request_id: String,
        lease_id: String,
        written: bool,
    },
    Disconnect {
        connection: String,
    },
    Decide {
        submission: workflow::Submission,
        decision: Decision,
        wall_ms: u64,
    },
}

impl Model {
    fn independent_project(&self, id: &str, revision: Option<u64>) -> Result<&projects::Project> {
        let project = self
            .project_registry
            .project(id)
            .ok_or("PROJECT_NOT_ENABLED")?;
        if !project.enabled || revision.is_some_and(|r| r != project.revision) {
            return Err("PROJECT_NOT_ENABLED");
        }
        if projects::resolve(&project.directory.final_path)?.identity != project.directory.identity
        {
            return Err("PROJECT_SCOPE_CHANGED");
        }
        Ok(project)
    }
    pub(crate) fn validate_independent_project(&self, request_id: &str) -> Result<()> {
        if let Some(wid) = self
            .interactions
            .contexts
            .get(request_id)
            .and_then(|c| c.workflow_id.as_ref())
        {
            let w = self
                .interactions
                .workflows
                .get(wid)
                .ok_or("WORKFLOW_NOT_FOUND")?;
            if !w.active || w.app_epoch != self.app_epoch() {
                return Err("WORKFLOW_DISCONNECTED");
            }
            self.independent_project(&w.project_id, Some(w.project_revision))?;
        }
        Ok(())
    }
    fn owned_context(
        &self,
        request: &str,
        connection: &str,
        workflow: Option<&str>,
    ) -> Result<&Context> {
        let c = self
            .interactions
            .contexts
            .get(request)
            .ok_or("REQUEST_NOT_FOUND")?;
        if c.connection_id != connection || c.workflow_id.as_deref() != workflow {
            return Err("NOT_RESULT_OWNER");
        }
        Ok(c)
    }
    pub fn independent_projects(&self) -> Value {
        json!(self
            .project_registry
            .entries()
            .map(|(id, p)| json!({"id":id,"path":p.directory.final_path,
            "name":p.directory.final_path.file_name().and_then(|s|s.to_str()).unwrap_or("Project"),
            "enabled":p.enabled,"revision":p.revision}))
            .collect::<Vec<_>>())
    }
    pub fn interaction_pins(&self) -> BTreeSet<String> {
        self.interactions
            .contexts
            .iter()
            .filter_map(|(id, c)| {
                self.requests
                    .get(id)
                    .filter(|r| r.binding.scope.app_epoch == self.app_epoch() && r.unfinished())
                    .map(|_| c.session_key.clone())
            })
            .collect()
    }
    pub fn interaction_snapshot(&self) -> Value {
        let mut sessions: BTreeMap<String, Value> = BTreeMap::new();
        let mut requests = Vec::new();
        for (id, c) in &self.interactions.contexts {
            let Some(r) = self.requests.get(id) else {
                continue;
            };
            let live = r.binding.scope.app_epoch == self.app_epoch()
                && r.phase == Phase::Pending
                && self.validate_independent_project(id).is_ok();
            requests.push(json!({"requestId":id,"sessionKey":c.session_key,"workflowId":c.workflow_id,
                "origin":if c.workflow_id.is_some(){"independentMcp"}else{"hook"},
                "title":c.title,"createdAt":c.created_at,"updatedAt":c.updated_at,
                "submission":r.submission(),"body":r.payload.body(),"phase":r.phase,"transport":r.transport,
                "decision":r.decision,"reason":r.reason,"actionable":live,"expiresAt":c.deadline_at,
                "nativeEvidence":r.native_evidence}));
            let s = sessions.entry(c.session_key.clone()).or_insert_with(|| json!({"key":c.session_key,"source":"cursor",
                "conversationId":c.conversation_id,"title":c.title,"updatedAt":c.updated_at,
                "activity":"stopped","items":[],"truncated":false,"hasCurrentRequest":false,"readOnly":true,
                "interactionOrigin":if c.workflow_id.is_some(){"independentMcp"}else{"hook"}}));
            if c.updated_at > s["updatedAt"].as_u64().unwrap_or(0) {
                s["updatedAt"] = c.updated_at.into();
            }
            if live {
                s["hasCurrentRequest"] = true.into();
                s["activity"] = "working".into();
            } else if r.phase == Phase::Disconnected && s["activity"] != "working" {
                s["activity"] = "interrupted".into();
            }
        }
        json!({"sessions":sessions.into_values().collect::<Vec<_>>(),"requests":requests})
    }
    pub(crate) fn touch_interaction(&mut self, request_id: &str, wall_ms: u64) {
        if let Some(c) = self.interactions.contexts.get_mut(request_id) {
            c.updated_at = c.updated_at.max(wall_ms);
        }
    }
    pub(crate) fn independent(&mut self, command: Command) -> Result<Value> {
        self.retire_independent_history();
        match command {
            Command::End {
                connection,
                workflow_id,
            } => {
                let epoch = self.app_epoch().to_owned();
                let w = self
                    .interactions
                    .workflows
                    .get_mut(&workflow_id)
                    .ok_or("WORKFLOW_NOT_FOUND")?;
                if w.connection_id != connection || w.app_epoch != epoch {
                    return Err("WORKFLOW_NOT_OWNED");
                }
                w.active = false;
                let ids = self
                    .interactions
                    .contexts
                    .iter_mut()
                    .filter(|(_, c)| c.workflow_id.as_deref() == Some(&workflow_id))
                    .map(|(id, c)| {
                        c.closed = true;
                        id.clone()
                    })
                    .collect::<Vec<_>>();
                for id in ids {
                    self.interrupt(&id, Phase::Cancelled, "WORKFLOW_ENDED");
                }
                Ok(json!({"closed":true}))
            }
            Command::DisableProject { project_id } => {
                self.project_registry.disable(&project_id)?;
                self.dispatch(workflow::Command::SetProject {
                    project_key: project_id,
                    enabled: false,
                })?;
                Ok(json!({"enabled":false}))
            }
            Command::Decide {
                submission,
                decision,
                wall_ms,
            } => {
                if !self
                    .interactions
                    .contexts
                    .contains_key(&submission.request_id)
                {
                    return Err("REQUEST_NOT_FOUND");
                }
                let id = submission.request_id.clone();
                let result = self.dispatch(workflow::Command::Decide {
                    submission,
                    decision,
                })?;
                if !matches!(
                    result,
                    Response::DecisionRecorded {
                        already_recorded: true
                    }
                ) {
                    self.touch_interaction(&id, wall_ms);
                }
                serde_json::to_value(result).map_err(|_| "RESPONSE_INVALID")
            }
            Command::Open {
                connection,
                project_id,
                title,
            } => {
                if !schema::valid_id(&connection) || title.trim().is_empty() || title.len() > 2048 {
                    return Err("INVALID_WORKFLOW");
                }
                let retired = self
                    .interactions
                    .workflows
                    .iter()
                    .filter(|(id, w)| {
                        !w.active
                            && !self
                                .interactions
                                .contexts
                                .values()
                                .any(|c| c.workflow_id.as_ref() == Some(id))
                    })
                    .map(|(id, _)| id.clone())
                    .collect::<Vec<_>>();
                for id in retired {
                    self.interactions.workflows.remove(&id);
                    self.retire_independent_workflow(&id);
                }
                if self
                    .interactions
                    .workflows
                    .values()
                    .filter(|w| w.active)
                    .count()
                    >= 128
                {
                    return Err("WORKFLOW_BACKPRESSURE");
                }
                let revision = self.independent_project(&project_id, None)?.revision;
                let id = format!("workflow-{}", uuid::Uuid::new_v4());
                self.dispatch(workflow::Command::ConfirmGeneration {
                    root: id.clone(),
                    generation: "independent".into(),
                })?;
                self.interactions.workflows.insert(
                    id.clone(),
                    Workflow {
                        id: id.clone(),
                        connection_id: connection,
                        app_epoch: self.app_epoch().into(),
                        project_id,
                        project_revision: revision,
                        title,
                        active: true,
                    },
                );
                Ok(
                    json!({"workflowId":id,"scope":"independent","instructions":"Keep this workflowId for this review. Use codecraft_get_result until a human decision is returned. This does not control native Cursor tools."}),
                )
            }
            Command::Mcp {
                connection,
                workflow_id,
                rpc_id,
                payload,
                wall_ms,
            } => {
                if !matches!(payload.body(), Body::Question(_) | Body::Plan(_)) {
                    return Err("UNKNOWN_TOOL");
                }
                let w = self
                    .interactions
                    .workflows
                    .get(&workflow_id)
                    .cloned()
                    .ok_or("WORKFLOW_NOT_FOUND")?;
                if !w.active || w.connection_id != connection || w.app_epoch != self.app_epoch() {
                    return Err("WORKFLOW_NOT_OWNED");
                }
                self.independent_project(&w.project_id, Some(w.project_revision))?;
                let scope = self.scope(&workflow_id, "independent", &[&w.project_id])?;
                let binding = CallBinding {
                    scope,
                    call: NativeCall {
                        conversation_id: workflow_id.clone(),
                        tool_use_id: uuid::Uuid::new_v4().to_string(),
                    },
                };
                let result =
                    self.activate(binding.clone(), payload, self.now_ms, self.now_ms + 310_000)?;
                let Response::Registered { submission } = result else {
                    return serde_json::to_value(result).map_err(|_| "RESPONSE_INVALID");
                };
                let id = submission.request_id;
                self.enter(
                    &id,
                    binding,
                    Owner {
                        connection_id: connection.clone(),
                        rpc_id,
                    },
                    self.now_ms + 310_000,
                )?;
                self.interactions.contexts.insert(
                    id.clone(),
                    Context {
                        session_key: history::session_key(self.installation_id(), &workflow_id),
                        conversation_id: workflow_id.clone(),
                        title: format!("{} · CodeCraft", w.title),
                        workflow_id: Some(workflow_id),
                        connection_id: connection,
                        created_at: wall_ms,
                        updated_at: wall_ms,
                        deadline_at: wall_ms + 300_000,
                        closed: false,
                    },
                );
                Ok(
                    json!({"requestId":id,"status":"pending","pollAfterMs":1000,"expiresInMs":300000}),
                )
            }
            Command::Hook {
                connection,
                event_id,
                event,
                input,
                automatic,
                wall_ms,
            } => {
                if !schema::valid_id(&event_id) || !schema::valid_id(&connection) {
                    return Err("INVALID_HOOK");
                }
                let conversation = input["conversation_id"]
                    .as_str()
                    .filter(|s| schema::valid_id(s))
                    .ok_or("CONVERSATION_REQUIRED")?;
                let name = input["tool_name"].as_str().ok_or("TOOL_NAME_REQUIRED")?;
                if event == "preToolUse" && name.starts_with("MCP:") {
                    return Ok(json!({"observed":true}));
                }
                let tool_input = protocol::tool_input(&input).map_err(|_| "INVALID_TOOL_INPUT")?;
                let display_name = if event == "beforeMCPExecution" {
                    let server = input["mcp_server_name"]
                        .as_str()
                        .filter(|s| !s.is_empty() && s.len() <= 512)
                        .ok_or("SERVER_REQUIRED")?;
                    format!("{server} · {name}")
                } else {
                    name.to_owned()
                };
                let payload = ToolRequest::parse(schema::HOOK_PERMISSION,&serde_json::to_vec(&json!({"schemaVersion":1,
                    "clientRequestId":event_id,"eventName":event,"toolName":display_name,"toolInput":tool_input})).map_err(|_|"INVALID_HOOK")?)?;
                self.dispatch(workflow::Command::SetProject {
                    project_key: "cursor-hook-policy".into(),
                    enabled: true,
                })?;
                // This internal scope belongs to one Hook invocation, not a native generation.
                let root = format!("hook-{event_id}");
                self.dispatch(workflow::Command::ConfirmGeneration {
                    root: root.clone(),
                    generation: "hook".into(),
                })?;
                let scope = self.scope(&root, "hook", &["cursor-hook-policy"])?;
                let binding = CallBinding {
                    scope,
                    call: NativeCall {
                        conversation_id: root,
                        tool_use_id: event_id.clone(),
                    },
                };
                let result =
                    self.activate(binding.clone(), payload, self.now_ms, self.now_ms + 55_000)?;
                let Response::Registered { submission } = result else {
                    return Err("HOOK_ALREADY_REGISTERED");
                };
                let id = submission.request_id;
                let entered = self.enter(
                    &id,
                    binding,
                    Owner {
                        connection_id: connection.clone(),
                        rpc_id: RpcId::Text(event_id),
                    },
                    self.now_ms + 55_000,
                )?;
                self.interactions.contexts.insert(
                    id.clone(),
                    Context {
                        session_key: history::session_key(self.installation_id(), conversation),
                        conversation_id: conversation.into(),
                        title: display_name,
                        workflow_id: None,
                        connection_id: connection,
                        created_at: wall_ms,
                        updated_at: wall_ms,
                        deadline_at: wall_ms + 45_000,
                        closed: false,
                    },
                );
                if automatic {
                    let Response::Entered { submission } = entered else {
                        return Err("HOOK_ENTRY_FAILED");
                    };
                    self.dispatch(workflow::Command::Decide {
                        submission,
                        decision: Decision::Permission {
                            action: PermissionAction::AllowOnce,
                        },
                    })?;
                }
                Ok(json!({"requestId":id,"status":"pending"}))
            }
            Command::Poll {
                connection,
                workflow_id,
                request_id,
            } => {
                self.owned_context(&request_id, &connection, workflow_id.as_deref())?;
                self.validate_independent_project(&request_id)?;
                let r = self.requests.get(&request_id).ok_or("REQUEST_NOT_FOUND")?;
                if r.binding.scope.app_epoch != self.app_epoch() {
                    return Err("APP_RESTARTED");
                }
                if r.phase == Phase::Decided && r.transport == Transport::NotStarted {
                    let scope = r.binding.scope.clone();
                    let owner = r.owner.clone().ok_or("NOT_RESULT_OWNER")?;
                    let result = self.claim(&request_id, scope, owner)?;
                    let Response::Claimed {
                        lease_id,
                        decision,
                        plan,
                        ..
                    } = result
                    else {
                        return Err("RESULT_NOT_CLAIMABLE");
                    };
                    return Ok(
                        json!({"status":"decided","requestId":request_id,"decision":decision,"plan":plan,"_deliveryLease":lease_id}),
                    );
                }
                Ok(
                    json!({"requestId":request_id,"status":if r.phase == Phase::Pending {"pending"} else if r.transport == Transport::Written {"alreadyDelivered"} else if r.transport == Transport::Claimed {"deliveryPending"} else {"terminated"},
                    "phase":r.phase,"transport":r.transport,"reason":r.reason,"pollAfterMs":1000}),
                )
            }
            Command::Cancel {
                connection,
                workflow_id,
                request_id,
            } => {
                self.owned_context(&request_id, &connection, Some(&workflow_id))?;
                self.interrupt(&request_id, Phase::Cancelled, "MCP_REQUEST_CANCELLED");
                let r = self.requests.get(&request_id).ok_or("REQUEST_NOT_FOUND")?;
                Ok(
                    json!({"requestId":request_id,"status":if r.transport==Transport::Written {"alreadyDelivered"}else if r.transport==Transport::OutcomeUnknown {"deliveryUnknown"}else{"cancelled"},"phase":r.phase,"transport":r.transport}),
                )
            }
            Command::Report {
                connection,
                request_id,
                lease_id,
                written,
            } => {
                let c = self
                    .interactions
                    .contexts
                    .get(&request_id)
                    .ok_or("REQUEST_NOT_FOUND")?;
                if c.connection_id != connection {
                    return Err("NOT_RESULT_OWNER");
                }
                let result = self.dispatch(workflow::Command::ReportDelivery {
                    request_id,
                    lease_id,
                    outcome: if written {
                        workflow::DeliveryOutcome::Written
                    } else {
                        workflow::DeliveryOutcome::Unknown
                    },
                })?;
                serde_json::to_value(result).map_err(|_| "RESPONSE_INVALID")
            }
            Command::Disconnect { connection } => {
                let ids = self
                    .interactions
                    .contexts
                    .iter()
                    .filter(|(_, c)| c.connection_id == connection)
                    .map(|(id, _)| id.clone())
                    .collect::<Vec<_>>();
                for id in ids {
                    self.interrupt(&id, Phase::Disconnected, "CONNECTION_CLOSED");
                    if let Some(c) = self.interactions.contexts.get_mut(&id) {
                        c.closed = true;
                    }
                }
                for w in self
                    .interactions
                    .workflows
                    .values_mut()
                    .filter(|w| w.connection_id == connection)
                {
                    w.active = false;
                }
                Ok(json!({"closed":true}))
            }
        }
    }
}

pub fn permission_response(event: &str, decision: &Value) -> Result<Value> {
    let allow = match decision["action"].as_str() {
        Some("allowOnce") => true,
        Some("deny") => false,
        _ => return Err("INVALID_DECISION"),
    };
    if decision["kind"] != "permission" {
        return Err("INVALID_DECISION");
    }
    match event {
        "preToolUse" => Ok(
            json!({"permission":if allow {"allow"}else{"deny"},"user_message":if allow {"CodeCraft allowed this call"}else{"Denied in CodeCraft"}}),
        ),
        "beforeMCPExecution" => Ok(
            json!({"permission":if allow {"allow"}else{"deny"},"user_message":if allow {"CodeCraft allowed this call"}else{"Denied in CodeCraft"}}),
        ),
        _ => Err("INVALID_EVENT"),
    }
}

#[cfg(all(test, windows))]
#[path = "cursor_interaction_tests.rs"]
mod tests;
