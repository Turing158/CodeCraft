//! Candidate -> active -> consumed ticket transactions, persisted inside the
//! workflow model. No Cursor event parser is implied. Commands are trusted
//! post-verification inputs; production adapters must pass P0 before using them.
use crate::{
    projects::{Registry, Workspace},
    protocol,
    schema::{self, Result, ToolRequest},
    workflow::{self, CallBinding, Command, Model, Owner, Response},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);
impl Secret {
    pub fn from_input(value: &str) -> Result<Self> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err("INVALID_TICKET");
        }
        Ok(Self(value.into()))
    }
    /// Only use to inject the self-owned bridgeTicket property into a verified
    /// candidate's tool arguments. Never put it in logs, LAN or tool results.
    pub fn for_injection(&self) -> &str {
        &self.0
    }
    fn hash(&self) -> String {
        protocol::sha256(self.0.as_bytes())
    }
}
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret([redacted])")
    }
}
impl Serialize for Secret {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str("[redacted]")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServerIdentity {
    pub registration_id: String,
    pub command_digest: String,
    pub configuration_digest: String,
}
impl ServerIdentity {
    fn valid(&self) -> bool {
        schema::valid_id(&self.registration_id)
            && [&self.command_digest, &self.configuration_digest]
                .iter()
                .all(|value| {
                    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
    }
}
#[derive(Clone, Debug)]
pub struct Candidate {
    pub binding: CallBinding,
    pub candidate_event_id: String,
    pub server: ServerIdentity,
    pub roots: Vec<String>,
    pub cwd: String,
    pub payload: ToolRequest,
    pub client_deadline_ms: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Phase {
    Candidate,
    Active,
    Consumed,
    Expired,
    Revoked,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Ticket {
    binding: CallBinding,
    candidate_event_id: String,
    server: ServerIdentity,
    workspace: Workspace,
    roots: Vec<String>,
    cwd: String,
    tool_name: String,
    arguments_digest: String,
    phase: Phase,
    issued_ms: u64,
    deadline_ms: u64,
    client_deadline_ms: u64,
    request_id: Option<String>,
    status_only: bool,
    owner: Option<Owner>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Book(BTreeMap<String, Ticket>);
impl Book {
    pub(crate) fn clear(&mut self) {
        self.0.clear();
    }
}

impl Model {
    pub(crate) fn issue_ticket(&mut self, candidate: Candidate) -> Result<Response> {
        self.validate_scope(&candidate.binding.scope)?;
        candidate.payload.validate_saved()?;
        if !candidate.server.valid()
            || !schema::valid_id(&candidate.candidate_event_id)
            || !schema::valid_id(&candidate.binding.call.conversation_id)
            || !schema::valid_id(&candidate.binding.call.tool_use_id)
        {
            return Err("CANDIDATE_UNVERIFIED");
        }
        if self.tickets.0.len() >= 1024 {
            return Err("TICKET_BACKPRESSURE");
        }
        if self.tickets.0.values().any(|ticket| {
            ticket.binding == candidate.binding
                || (ticket.binding.scope == candidate.binding.scope
                    && ticket.candidate_event_id == candidate.candidate_event_id)
        }) {
            return Err("CANDIDATE_ALREADY_ISSUED");
        }
        let workspace = self
            .project_registry
            .capture(&candidate.roots, &candidate.cwd)?;
        if workspace.revisions() != &candidate.binding.scope.projects {
            return Err("PROJECT_SCOPE_CHANGED");
        }
        let deadline = self
            .now_ms
            .saturating_add(600_000)
            .min(candidate.client_deadline_ms.saturating_sub(10_000));
        if deadline <= self.now_ms {
            return Err("INSUFFICIENT_WAIT_BUDGET");
        }
        let secret = Secret(format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        ));
        self.tickets.0.insert(
            secret.hash(),
            Ticket {
                binding: candidate.binding,
                candidate_event_id: candidate.candidate_event_id,
                server: candidate.server,
                workspace,
                roots: candidate.roots,
                cwd: candidate.cwd,
                tool_name: candidate.payload.tool_name().into(),
                arguments_digest: candidate.payload.arguments_digest().into(),
                phase: Phase::Candidate,
                issued_ms: self.now_ms,
                deadline_ms: deadline,
                client_deadline_ms: candidate.client_deadline_ms,
                request_id: None,
                status_only: false,
                owner: None,
            },
        );
        Ok(Response::CandidateIssued { ticket: secret })
    }
    fn checked_ticket(
        &self,
        secret: &Secret,
        server: &ServerIdentity,
        payload: &ToolRequest,
        roots: &[String],
        cwd: &str,
    ) -> Result<Ticket> {
        let ticket = self.tickets.0.get(&secret.hash()).ok_or("TICKET_UNKNOWN")?;
        if ticket.binding.scope.app_epoch != self.app_epoch() {
            return Err("APP_RESTARTED");
        }
        self.validate_scope(&ticket.binding.scope)?;
        if &ticket.server != server {
            return Err("TICKET_SERVER_MISMATCH");
        }
        payload.validate_saved()?;
        if ticket.tool_name != payload.tool_name()
            || ticket.arguments_digest != payload.arguments_digest()
        {
            return Err("TICKET_ARGUMENTS_CHANGED");
        }
        self.project_registry
            .revalidate(&ticket.workspace, roots, cwd)?;
        match ticket.phase {
            Phase::Expired => return Err("TICKET_EXPIRED"),
            Phase::Revoked => return Err("TICKET_REVOKED"),
            _ => {}
        }
        Ok(ticket.clone())
    }
    pub(crate) fn activate_ticket(
        &mut self,
        secret: Secret,
        binding: CallBinding,
        server: ServerIdentity,
        payload: ToolRequest,
        roots: Vec<String>,
        cwd: String,
    ) -> Result<Response> {
        let ticket = self.checked_ticket(&secret, &server, &payload, &roots, &cwd)?;
        if ticket.binding != binding {
            return Err("TICKET_CALL_MISMATCH");
        }
        if ticket.phase != Phase::Candidate {
            return Err("TICKET_ALREADY_ACTIVATED");
        }
        let response = self.activate(
            binding,
            payload,
            ticket.issued_ms,
            ticket.client_deadline_ms,
        )?;
        let (request_id, status_only) = match &response {
            Response::Registered { submission } => (submission.request_id.clone(), false),
            Response::Duplicate { request_id, .. } => (request_id.clone(), true),
            _ => return Err("TICKET_STATE_INCONSISTENT"),
        };
        let current = self
            .tickets
            .0
            .get_mut(&secret.hash())
            .ok_or("TICKET_UNKNOWN")?;
        current.phase = Phase::Active;
        current.request_id = Some(request_id);
        current.status_only = status_only;
        current.deadline_ms = current.deadline_ms.min(self.now_ms.saturating_add(60_000));
        current.roots = roots;
        current.cwd = cwd;
        Ok(response)
    }
    pub(crate) fn consume_ticket(
        &mut self,
        secret: Secret,
        server: ServerIdentity,
        payload: ToolRequest,
        owner: Owner,
        roots: Vec<String>,
        cwd: String,
    ) -> Result<Response> {
        if self.tickets.0.get(&secret.hash()).is_some_and(|ticket| {
            ticket.owner.as_ref() == Some(&owner)
                && (ticket.tool_name != payload.tool_name()
                    || ticket.arguments_digest != payload.arguments_digest())
        }) {
            return Err("RPC_ID_CONFLICT");
        }
        let ticket = self.checked_ticket(&secret, &server, &payload, &roots, &cwd)?;
        if !schema::valid_id(&owner.connection_id)
            || matches!(&owner.rpc_id, workflow::RpcId::Text(text) if text.is_empty() || text.len() > 128)
        {
            return Err("INVALID_MCP_OWNER");
        }
        if let Some((hash, existing)) = self
            .tickets
            .0
            .iter()
            .find(|(_, value)| value.owner.as_ref() == Some(&owner))
        {
            if hash == &secret.hash() && existing.arguments_digest == payload.arguments_digest() {
                return Ok(Response::ProtocolDuplicate);
            }
            return Err("RPC_ID_CONFLICT");
        }
        if ticket.phase == Phase::Consumed {
            return Err("TICKET_USED");
        }
        if ticket.phase != Phase::Active {
            return Err("TICKET_NOT_ACTIVE");
        }
        let response = if ticket.status_only {
            // Refresh status metadata; never expose the original answer, owner
            // or lease to this business retry's distinct transport context.
            self.activate(
                ticket.binding,
                payload,
                ticket.issued_ms,
                ticket.client_deadline_ms,
            )?
        } else {
            self.enter(
                ticket
                    .request_id
                    .as_deref()
                    .ok_or("TICKET_STATE_INCONSISTENT")?,
                ticket.binding,
                owner.clone(),
                ticket.client_deadline_ms,
            )?
        };
        let current = self
            .tickets
            .0
            .get_mut(&secret.hash())
            .ok_or("TICKET_UNKNOWN")?;
        current.phase = Phase::Consumed;
        current.owner = Some(owner);
        current.roots = roots;
        current.cwd = cwd;
        Ok(response)
    }
    pub(crate) fn terminate_ticket_call(
        &mut self,
        binding: CallBinding,
        evidence: workflow::NativeEvidence,
    ) -> Result<Response> {
        if binding.scope.app_epoch != self.app_epoch() {
            return Err("APP_RESTARTED");
        }
        if evidence == workflow::NativeEvidence::Unconfirmed {
            return Err("TERMINATION_UNVERIFIED");
        }
        let Some(hash) = self
            .tickets
            .0
            .iter()
            .find(|(_, ticket)| ticket.binding == binding)
            .map(|(hash, _)| hash.clone())
        else {
            return Err("CALL_NOT_ASSOCIATED");
        };
        let ticket = self.tickets.0.get(&hash).ok_or("TICKET_UNKNOWN")?.clone();
        if let Some(request_id) = ticket.request_id.filter(|_| !ticket.status_only) {
            self.dispatch(Command::NativeTermination {
                request_id,
                binding,
                evidence,
            })?;
        }
        self.tickets.0.get_mut(&hash).ok_or("TICKET_UNKNOWN")?.phase = Phase::Revoked;
        Ok(Response::Changed)
    }
    pub(crate) fn expire_tickets(&mut self) {
        let mut updates = Vec::new();
        for (hash, ticket) in &self.tickets.0 {
            if !matches!(ticket.phase, Phase::Candidate | Phase::Active) {
                continue;
            }
            if self.now_ms >= ticket.deadline_ms {
                updates.push((hash.clone(), Phase::Expired));
            } else if self.validate_scope(&ticket.binding.scope).is_err()
                || (!ticket.status_only
                    && ticket
                        .request_id
                        .as_deref()
                        .and_then(|id| self.request(id))
                        .is_some_and(|r| r.phase != workflow::Phase::AwaitingMcpEntry))
            {
                updates.push((hash.clone(), Phase::Revoked));
            }
        }
        for (hash, phase) in updates {
            if let Some(ticket) = self.tickets.0.get_mut(&hash) {
                ticket.phase = phase;
            }
        }
    }
    pub(crate) fn validate_ticket_projects(&self, request_id: &str) -> Result<()> {
        self.validate_independent_project(request_id)?;
        if let Some(ticket) =
            self.tickets.0.values().find(|ticket| {
                !ticket.status_only && ticket.request_id.as_deref() == Some(request_id)
            })
        {
            self.project_registry
                .revalidate(&ticket.workspace, &ticket.roots, &ticket.cwd)?;
        }
        Ok(())
    }
    pub fn project_registry(&self) -> &Registry {
        &self.project_registry
    }
}

#[cfg(all(test, windows))]
#[path = "cursor_tickets_tests.rs"]
mod tests;
