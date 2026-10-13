//! Durable workflow model. Native ticket commands remain unavailable in the
//! product; v5 independent commands use authenticated connections and explicit
//! CodeCraft workflow identities, never inferred Cursor conversations.
use crate::{
    protocol::sha256,
    schema::{self, Body, Decision, PlanAction, ToolRequest},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

pub type Result<T> = schema::Result<T>;
const MAX_REQUESTS: usize = 1024;
const ENTRY_MS: u64 = 60_000;
const TICKET_MS: u64 = 600_000;
const HUMAN_MS: u64 = 300_000;
const CLIENT_MARGIN_MS: u64 = 10_000;
const MIN_HUMAN_MS: u64 = 30_000;
const LEASE_MS: u64 = 5_000;

fn key(parts: &[&str]) -> String {
    let mut bytes = Vec::new();
    for part in parts {
        bytes.extend_from_slice(&(part.len() as u64).to_be_bytes());
        bytes.extend_from_slice(part.as_bytes());
    }
    sha256(&bytes)
}
fn id() -> String {
    Uuid::new_v4().to_string()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scope {
    pub app_epoch: String,
    pub root_conversation_id: String,
    pub generation_id: String,
    pub config_revision: u64,
    pub projects: BTreeMap<String, u64>,
}
impl Scope {
    fn workflow_key(&self) -> String {
        key(&[
            &self.app_epoch,
            &self.root_conversation_id,
            &self.generation_id,
        ])
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeCall {
    pub conversation_id: String,
    pub tool_use_id: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallBinding {
    pub scope: Scope,
    pub call: NativeCall,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RpcId {
    Number(i64),
    Text(String),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Owner {
    pub connection_id: String,
    pub rpc_id: RpcId,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    AwaitingMcpEntry,
    Pending,
    Decided,
    Cancelled,
    Expired,
    Superseded,
    Disconnected,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Transport {
    NotStarted,
    Claimed,
    Written,
    DeliveryFailed,
    OutcomeUnknown,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NativeEvidence {
    Unconfirmed,
    BlockedObserved,
    CancelledObserved,
    FailedObserved,
    TimeoutObserved,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanRef {
    pub plan_id: String,
    pub revision: u64,
    pub content_digest: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Submission {
    pub request_id: String,
    pub app_epoch: String,
    pub request_version: u64,
    pub arguments_digest: String,
    pub plan: Option<PlanRef>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DecisionRecord {
    pub submission_version: u64,
    pub value: Decision,
}
#[derive(Clone, Debug, PartialEq)]
struct Lease {
    id: String,
    deadline_ms: u64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub request_id: String,
    pub binding: CallBinding,
    pub payload: ToolRequest,
    pub plan: Option<PlanRef>,
    pub version: u64,
    pub phase: Phase,
    pub transport: Transport,
    pub native_evidence: NativeEvidence,
    pub decision: Option<DecisionRecord>,
    pub deadline_ms: u64,
    pub reason: Option<String>,
    // No connection handles or transferable delivery credentials on disk.
    #[serde(skip)]
    pub(crate) owner: Option<Owner>,
    #[serde(skip)]
    lease: Option<Lease>,
}
impl Request {
    pub fn submission(&self) -> Submission {
        Submission {
            request_id: self.request_id.clone(),
            app_epoch: self.binding.scope.app_epoch.clone(),
            request_version: self.version,
            arguments_digest: self.payload.arguments_digest().into(),
            plan: self.plan.clone(),
        }
    }
    pub(crate) fn unfinished(&self) -> bool {
        matches!(
            self.phase,
            Phase::AwaitingMcpEntry | Phase::Pending | Phase::Decided
        ) && self.transport != Transport::Written
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Project {
    revision: u64,
    enabled: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Session {
    current: Option<String>,
    retired: BTreeSet<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlanSlot {
    scope: Scope,
    plan_id: String,
    request_id: String,
    client_request_id: String,
    revision: u64,
    temporary_wait: bool,
    human_revision_sources: BTreeSet<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Model {
    schema_version: u32,
    installation_id: String,
    app_epoch: String,
    config_revision: u64,
    pub(crate) now_ms: u64,
    gate_revision: u64,
    projects: BTreeMap<String, Project>,
    sessions: BTreeMap<String, Session>,
    pub(crate) requests: BTreeMap<String, Request>,
    idempotency: BTreeMap<String, String>,
    plans: BTreeMap<String, PlanSlot>,
    #[serde(default)]
    pub(crate) tickets: crate::tickets::Book,
    #[serde(default)]
    pub(crate) project_registry: crate::projects::Registry,
    #[serde(default)]
    pub(crate) interactions: crate::interaction::Book,
}

/// No Deserialize implementation: these include privileged lifecycle signals,
/// not a wire protocol that a model, LAN client or file can submit directly.
#[derive(Clone, Debug)]
pub enum Command {
    Independent(crate::interaction::Command),
    SetProjectPath {
        path: std::path::PathBuf,
        enabled: bool,
    },
    IssueTicket {
        candidate: Box<crate::tickets::Candidate>,
    },
    ActivateTicket {
        ticket: crate::tickets::Secret,
        binding: CallBinding,
        server: crate::tickets::ServerIdentity,
        payload: ToolRequest,
        roots: Vec<String>,
        cwd: String,
    },
    ConsumeTicket {
        ticket: crate::tickets::Secret,
        server: crate::tickets::ServerIdentity,
        payload: ToolRequest,
        owner: Owner,
        roots: Vec<String>,
        cwd: String,
    },
    TerminateTicketCall {
        binding: CallBinding,
        evidence: NativeEvidence,
    },
    SetProject {
        project_key: String,
        enabled: bool,
    },
    ConfirmGeneration {
        root: String,
        generation: String,
    },
    EndGeneration {
        root: String,
        generation: String,
    },
    ChangeConfiguration,
    Activate {
        binding: CallBinding,
        payload: ToolRequest,
        issued_at_ms: u64,
        client_deadline_ms: u64,
    },
    Enter {
        request_id: String,
        binding: CallBinding,
        owner: Owner,
        client_deadline_ms: u64,
    },
    Decide {
        submission: Submission,
        decision: Decision,
    },
    Claim {
        request_id: String,
        scope: Scope,
        owner: Owner,
    },
    ReportDelivery {
        request_id: String,
        lease_id: String,
        outcome: DeliveryOutcome,
    },
    NativeTermination {
        request_id: String,
        binding: CallBinding,
        evidence: NativeEvidence,
    },
    Tick,
}
#[derive(Clone, Debug, PartialEq)]
pub enum DeliveryOutcome {
    Written,
    KnownNotWritten,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DuplicateStatus {
    Pending,
    Completed,
    Terminated,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Response {
    Independent {
        value: serde_json::Value,
    },
    CandidateIssued {
        ticket: crate::tickets::Secret,
    },
    ProtocolDuplicate,
    Changed,
    Unchanged,
    Registered {
        submission: Submission,
    },
    Duplicate {
        request_id: String,
        status: DuplicateStatus,
        current_revision: Option<u64>,
        current_client_request_id: Option<String>,
    },
    Entered {
        submission: Submission,
    },
    EntryRejected {
        code: String,
    },
    DecisionRecorded {
        already_recorded: bool,
    },
    Claimed {
        lease_id: String,
        deadline_ms: u64,
        decision: Decision,
        plan: Option<PlanRef>,
    },
    DeliveryRecorded {
        historical: bool,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub enum Gate {
    NormalPolicy,
    ReviewPending,
    RevisionRequired,
    Unavailable,
}

impl Model {
    pub(crate) fn retire_independent_workflow(&mut self, root: &str) {
        if !self
            .requests
            .values()
            .any(|r| r.binding.scope.root_conversation_id == root)
        {
            self.sessions.remove(root);
            self.plans
                .retain(|_, p| p.scope.root_conversation_id != root);
        }
    }
    /// Only retired independent invocations can leave the bounded review log.
    /// Active workflows retain every idempotency key and all pending requests.
    pub(crate) fn retire_independent_history(&mut self) {
        if self.interactions.contexts.len() < 512 {
            return;
        }
        let mut retired = self
            .interactions
            .contexts
            .iter()
            .filter(|(id, c)| c.closed && self.requests.get(*id).is_some_and(|r| !r.unfinished()))
            .map(|(id, c)| (c.updated_at, id.clone()))
            .collect::<Vec<_>>();
        retired.sort();
        for (_, id) in retired
            .into_iter()
            .take(self.interactions.contexts.len().saturating_sub(511))
        {
            self.interactions.contexts.remove(&id);
            if let Some(request) = self.requests.remove(&id) {
                let root = &request.binding.scope.root_conversation_id;
                if !self
                    .requests
                    .values()
                    .any(|r| r.binding.scope.root_conversation_id == *root)
                {
                    self.sessions.remove(root);
                    self.plans
                        .retain(|_, p| p.scope.root_conversation_id != *root);
                }
            }
            self.idempotency.retain(|_, value| value != &id);
        }
    }
    pub fn new(installation_id: &str) -> Result<Self> {
        if !schema::valid_id(installation_id) {
            return Err("INVALID_INSTALLATION");
        }
        Ok(Self {
            schema_version: 1,
            installation_id: installation_id.into(),
            app_epoch: id(),
            config_revision: 1,
            now_ms: 0,
            gate_revision: 0,
            projects: BTreeMap::new(),
            sessions: BTreeMap::new(),
            requests: BTreeMap::new(),
            idempotency: BTreeMap::new(),
            plans: BTreeMap::new(),
            tickets: crate::tickets::Book::default(),
            project_registry: crate::projects::Registry::default(),
            interactions: crate::interaction::Book::default(),
        })
    }
    pub fn app_epoch(&self) -> &str {
        &self.app_epoch
    }
    pub fn installation_id(&self) -> &str {
        &self.installation_id
    }
    pub(crate) fn remove_project_registrations(&mut self) -> Result<()> {
        let keys = self
            .project_registry
            .entries()
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for project_key in keys {
            self.dispatch(Command::SetProject {
                project_key: project_key.clone(),
                enabled: false,
            })?;
            self.projects.remove(&project_key);
        }
        self.project_registry = crate::projects::Registry::default();
        Ok(())
    }
    pub fn gate_revision(&self) -> u64 {
        self.gate_revision
    }
    pub fn request(&self, request_id: &str) -> Option<&Request> {
        self.requests.get(request_id)
    }
    pub fn request_count(&self) -> usize {
        self.requests.len()
    }
    /// Uses project keys already resolved by the caller; this does not verify
    /// paths, junctions or whether these are the actual Cursor workspace roots.
    pub fn scope(&self, root: &str, generation: &str, project_keys: &[&str]) -> Result<Scope> {
        let mut projects = BTreeMap::new();
        for project_key in project_keys {
            let project = self
                .projects
                .get(*project_key)
                .ok_or("PROJECT_NOT_ENABLED")?;
            projects.insert((*project_key).into(), project.revision);
        }
        let scope = Scope {
            app_epoch: self.app_epoch.clone(),
            root_conversation_id: root.into(),
            generation_id: generation.into(),
            config_revision: self.config_revision,
            projects,
        };
        self.validate_scope(&scope)?;
        Ok(scope)
    }
    pub(crate) fn validate_scope(&self, scope: &Scope) -> Result<()> {
        if scope.app_epoch != self.app_epoch {
            return Err("APP_RESTARTED");
        }
        if scope.config_revision != self.config_revision {
            return Err("CONFIGURATION_CHANGED");
        }
        if !schema::valid_id(&scope.root_conversation_id)
            || !schema::valid_id(&scope.generation_id)
            || self
                .sessions
                .get(&scope.root_conversation_id)
                .and_then(|s| s.current.as_ref())
                != Some(&scope.generation_id)
        {
            return Err("GENERATION_NOT_CURRENT");
        }
        if scope.projects.is_empty() {
            return Err("PROJECT_SCOPE_UNVERIFIED");
        }
        for (key, revision) in &scope.projects {
            let project = self.projects.get(key).ok_or("PROJECT_NOT_ENABLED")?;
            if !project.enabled {
                return Err("PROJECT_NOT_ENABLED");
            }
            if project.revision != *revision {
                return Err("PROJECT_REVISION_CHANGED");
            }
        }
        Ok(())
    }
    fn plan_key(scope: &Scope, plan_id: &str) -> String {
        key(&[&scope.workflow_key(), plan_id])
    }
    pub fn gate(&self, scope: &Scope, verified_read_only: bool) -> Gate {
        if self.validate_scope(scope).is_err() {
            return Gate::Unavailable;
        }
        if self.requests.values().any(|request| {
            request.binding.scope == *scope
                && request.unfinished()
                && self.validate_ticket_projects(&request.request_id).is_err()
        }) {
            return Gate::Unavailable;
        }
        if verified_read_only {
            return Gate::NormalPolicy;
        }
        let slots = self
            .plans
            .values()
            .filter(|p| p.scope.workflow_key() == scope.workflow_key())
            .collect::<Vec<_>>();
        if slots.iter().any(|p| !p.human_revision_sources.is_empty()) {
            Gate::RevisionRequired
        } else if slots.iter().any(|p| p.temporary_wait) {
            Gate::ReviewPending
        } else {
            Gate::NormalPolicy
        }
    }
    /// All state transitions are serial. Invalid commands roll back their own
    /// changes, but deadline expiry observed before them remains committed.
    pub fn execute(&mut self, command: Command, now_ms: u64) -> Result<Response> {
        if now_ms < self.now_ms {
            return Err("MONOTONIC_CLOCK_REGRESSED");
        }
        self.now_ms = now_ms;
        self.expire();
        self.expire_tickets();
        let mut candidate = self.clone();
        let response = candidate.dispatch(command)?;
        candidate.expire_tickets();
        *self = candidate;
        Ok(response)
    }
    pub(crate) fn dispatch(&mut self, command: Command) -> Result<Response> {
        match command {
            Command::Independent(command) => self
                .independent(command)
                .map(|value| Response::Independent { value }),
            Command::SetProjectPath { path, enabled } => {
                let (project_key, revision) = self.project_registry.set(&path, enabled)?;
                let response = self.dispatch(Command::SetProject {
                    project_key: project_key.clone(),
                    enabled,
                })?;
                if self
                    .projects
                    .get(&project_key)
                    .is_none_or(|project| project.revision != revision)
                {
                    return Err("PROJECT_REVISION_CONFLICT");
                }
                Ok(response)
            }
            Command::IssueTicket { candidate } => self.issue_ticket(*candidate),
            Command::ActivateTicket {
                ticket,
                binding,
                server,
                payload,
                roots,
                cwd,
            } => self.activate_ticket(ticket, binding, server, payload, roots, cwd),
            Command::ConsumeTicket {
                ticket,
                server,
                payload,
                owner,
                roots,
                cwd,
            } => self.consume_ticket(ticket, server, payload, owner, roots, cwd),
            Command::TerminateTicketCall { binding, evidence } => {
                self.terminate_ticket_call(binding, evidence)
            }
            Command::Tick => Ok(Response::Unchanged),
            Command::SetProject {
                project_key,
                enabled,
            } => {
                if !schema::valid_id(&project_key) {
                    return Err("INVALID_PROJECT");
                }
                if self
                    .projects
                    .get(&project_key)
                    .is_some_and(|p| p.enabled == enabled)
                {
                    return Ok(Response::Unchanged);
                }
                let revision = self
                    .projects
                    .get(&project_key)
                    .map_or(1, |p| p.revision + 1);
                self.projects
                    .insert(project_key.clone(), Project { revision, enabled });
                let affected = self
                    .requests
                    .values()
                    .filter(|r| {
                        r.binding.scope.app_epoch == self.app_epoch
                            && r.binding.scope.projects.contains_key(&project_key)
                    })
                    .map(|r| r.request_id.clone())
                    .collect::<Vec<_>>();
                for request in affected {
                    self.interrupt(&request, Phase::Disconnected, "PROJECT_CHANGED");
                }
                for plan in self
                    .plans
                    .values_mut()
                    .filter(|p| p.scope.projects.contains_key(&project_key))
                {
                    plan.temporary_wait = false;
                    plan.human_revision_sources.clear();
                }
                self.gate_revision += 1;
                Ok(Response::Changed)
            }
            Command::ConfirmGeneration { root, generation } => {
                if !schema::valid_id(&root) || !schema::valid_id(&generation) {
                    return Err("INVALID_GENERATION");
                }
                if let Some(session) = self.sessions.get(&root) {
                    if session.current.as_deref() == Some(&generation) {
                        return Ok(Response::Unchanged);
                    }
                    if session.retired.contains(&generation) {
                        return Err("GENERATION_RETIRED");
                    }
                    if let Some(old) = session.current.clone() {
                        self.end_generation(&root, &old);
                    }
                }
                self.sessions
                    .entry(root)
                    .or_insert(Session {
                        current: None,
                        retired: BTreeSet::new(),
                    })
                    .current = Some(generation);
                Ok(Response::Changed)
            }
            Command::EndGeneration { root, generation } => {
                if self.sessions.get(&root).and_then(|s| s.current.as_deref()) != Some(&generation)
                {
                    return Ok(Response::Unchanged);
                }
                self.end_generation(&root, &generation);
                Ok(Response::Changed)
            }
            Command::ChangeConfiguration => {
                self.config_revision += 1;
                let ids = self.requests.keys().cloned().collect::<Vec<_>>();
                for request in ids {
                    self.interrupt(&request, Phase::Disconnected, "CONFIGURATION_CHANGED");
                }
                // Configuration changes interrupt waits, but cannot erase an
                // earlier human revision requirement in the same workflow.
                Ok(Response::Changed)
            }
            Command::Activate {
                binding,
                payload,
                issued_at_ms,
                client_deadline_ms,
            } => self.activate(binding, payload, issued_at_ms, client_deadline_ms),
            Command::Enter {
                request_id,
                binding,
                owner,
                client_deadline_ms,
            } => self.enter(&request_id, binding, owner, client_deadline_ms),
            Command::Decide {
                submission,
                decision,
            } => self.decide(submission, decision),
            Command::Claim {
                request_id,
                scope,
                owner,
            } => self.claim(&request_id, scope, owner),
            Command::ReportDelivery {
                request_id,
                lease_id,
                outcome,
            } => self.delivery(&request_id, &lease_id, outcome),
            Command::NativeTermination {
                request_id,
                binding,
                evidence,
            } => {
                if binding.scope.app_epoch != self.app_epoch {
                    return Err("APP_RESTARTED");
                }
                let request = self
                    .requests
                    .get_mut(&request_id)
                    .ok_or("REQUEST_NOT_FOUND")?;
                if request.binding != binding {
                    return Err("CALL_NOT_ASSOCIATED");
                }
                if evidence == NativeEvidence::Unconfirmed {
                    return Err("TERMINATION_UNVERIFIED");
                }
                if request.native_evidence == evidence {
                    return Ok(Response::Unchanged);
                }
                request.native_evidence = evidence.clone();
                request.version += 1;
                let (phase, reason) = match evidence {
                    NativeEvidence::BlockedObserved => (Phase::Cancelled, "NATIVE_CALL_BLOCKED"),
                    NativeEvidence::CancelledObserved => {
                        (Phase::Cancelled, "NATIVE_CALL_CANCELLED")
                    }
                    NativeEvidence::TimeoutObserved => (Phase::Expired, "NATIVE_CALL_TIMEOUT"),
                    _ => (Phase::Disconnected, "NATIVE_CALL_FAILED"),
                };
                self.interrupt(&request_id, phase, reason);
                Ok(Response::Changed)
            }
        }
    }
    pub(crate) fn activate(
        &mut self,
        binding: CallBinding,
        payload: ToolRequest,
        issued: u64,
        client_deadline: u64,
    ) -> Result<Response> {
        self.validate_scope(&binding.scope)?;
        if !schema::valid_id(&binding.call.conversation_id)
            || !schema::valid_id(&binding.call.tool_use_id)
        {
            return Err("INVALID_NATIVE_CALL");
        }
        payload.validate_saved()?;
        let idem = key(&[
            &self.installation_id,
            &binding.scope.app_epoch,
            &binding.scope.root_conversation_id,
            &binding.scope.generation_id,
            payload.tool_name(),
            payload.client_request_id(),
        ]);
        if let Some(existing_id) = self.idempotency.get(&idem) {
            let existing = self.requests.get(existing_id).ok_or("STATE_INCONSISTENT")?;
            if existing.payload.arguments_digest() != payload.arguments_digest() {
                return Err("REQUEST_ID_CONFLICT");
            }
            // The calling retry's ticket/transport is separate. No owner,
            // deadline, plan revision or human decision is returned or changed.
            let current = existing
                .plan
                .as_ref()
                .and_then(|p| self.plans.get(&Self::plan_key(&binding.scope, &p.plan_id)));
            return Ok(Response::Duplicate {
                request_id: existing_id.clone(),
                status: if existing.phase == Phase::Decided
                    && existing.transport == Transport::Written
                {
                    DuplicateStatus::Completed
                } else if existing.unfinished() {
                    DuplicateStatus::Pending
                } else {
                    DuplicateStatus::Terminated
                },
                current_revision: current.map(|p| p.revision),
                current_client_request_id: current.map(|p| p.client_request_id.clone()),
            });
        }
        if self.requests.len() >= MAX_REQUESTS {
            return Err("REQUEST_BACKPRESSURE");
        }
        if self.requests.values().any(|r| {
            r.binding.call == binding.call
                && r.binding.scope.workflow_key() == binding.scope.workflow_key()
        }) {
            return Err("NATIVE_CALL_ALREADY_BOUND");
        }
        if issued > self.now_ms {
            return Err("INVALID_ISSUED_TIME");
        }
        let deadline = issued
            .saturating_add(TICKET_MS)
            .min(self.now_ms.saturating_add(ENTRY_MS))
            .min(client_deadline.saturating_sub(CLIENT_MARGIN_MS));
        if deadline <= self.now_ms {
            return Err("INSUFFICIENT_WAIT_BUDGET");
        }
        let request_id = id();
        let mut plan_ref = None;
        if let Body::Plan(plan) = payload.body() {
            let plan_key = Self::plan_key(&binding.scope, &plan.plan_id);
            let previous = self.plans.get(&plan_key).cloned();
            let revision = match &previous {
                Some(old)
                    if old.revision == plan.base_revision
                        && plan.supersedes_client_request_id.as_deref()
                            == Some(&old.client_request_id) =>
                {
                    old.revision + 1
                }
                None if plan.base_revision == 0 && plan.supersedes_client_request_id.is_none() => 1,
                _ => return Err("REVISION_CONFLICT"),
            };
            plan_ref = Some(PlanRef {
                plan_id: plan.plan_id.clone(),
                revision,
                content_digest: payload.plan_content_digest().ok_or("INVALID_PLAN")?.into(),
            });
            // Install the replacement gate before retiring the old request.
            self.plans.insert(
                plan_key,
                PlanSlot {
                    scope: binding.scope.clone(),
                    plan_id: plan.plan_id.clone(),
                    request_id: request_id.clone(),
                    client_request_id: payload.client_request_id().into(),
                    revision,
                    temporary_wait: true,
                    human_revision_sources: previous
                        .as_ref()
                        .map_or_else(BTreeSet::new, |p| p.human_revision_sources.clone()),
                },
            );
            self.gate_revision += 1;
            if let Some(old) = previous {
                self.interrupt(&old.request_id, Phase::Superseded, "PLAN_SUPERSEDED");
            }
        }
        let request = Request {
            request_id: request_id.clone(),
            binding,
            payload,
            plan: plan_ref,
            version: 1,
            phase: Phase::AwaitingMcpEntry,
            transport: Transport::NotStarted,
            native_evidence: NativeEvidence::Unconfirmed,
            decision: None,
            deadline_ms: deadline,
            reason: None,
            owner: None,
            lease: None,
        };
        let submission = request.submission();
        self.requests.insert(request_id.clone(), request);
        self.idempotency.insert(idem, request_id);
        Ok(Response::Registered { submission })
    }
    pub(crate) fn enter(
        &mut self,
        request_id: &str,
        binding: CallBinding,
        owner: Owner,
        client_deadline: u64,
    ) -> Result<Response> {
        self.validate_scope(&binding.scope)?;
        if !schema::valid_id(&owner.connection_id)
            || matches!(&owner.rpc_id, RpcId::Text(id) if id.is_empty() || id.len() > 128)
        {
            return Err("INVALID_OWNER");
        }
        let request = self.requests.get(request_id).ok_or("REQUEST_NOT_FOUND")?;
        if request.binding != binding {
            return Err("CALL_NOT_ASSOCIATED");
        }
        if request.owner.is_some() {
            return Err("MCP_ENTRY_ALREADY_CONSUMED");
        }
        if request.phase != Phase::AwaitingMcpEntry {
            return Err("REQUEST_NOT_WAITING_FOR_ENTRY");
        }
        if self
            .requests
            .values()
            .any(|r| r.owner.as_ref() == Some(&owner))
        {
            return Err("RPC_ID_ALREADY_BOUND");
        }
        let deadline = self
            .now_ms
            .saturating_add(HUMAN_MS)
            .min(client_deadline.saturating_sub(CLIENT_MARGIN_MS));
        if deadline.saturating_sub(self.now_ms) < MIN_HUMAN_MS {
            self.interrupt(request_id, Phase::Expired, "INSUFFICIENT_WAIT_BUDGET");
            return Ok(Response::EntryRejected {
                code: "INSUFFICIENT_WAIT_BUDGET".into(),
            });
        }
        let request = self
            .requests
            .get_mut(request_id)
            .ok_or("REQUEST_NOT_FOUND")?;
        request.owner = Some(owner);
        request.phase = Phase::Pending;
        request.deadline_ms = deadline;
        request.version += 1;
        Ok(Response::Entered {
            submission: request.submission(),
        })
    }
    fn decide(&mut self, submission: Submission, decision: Decision) -> Result<Response> {
        if submission.app_epoch != self.app_epoch {
            return Err("APP_RESTARTED");
        }
        self.validate_ticket_projects(&submission.request_id)?;
        let request = self
            .requests
            .get(&submission.request_id)
            .ok_or("REQUEST_NOT_FOUND")?;
        self.validate_scope(&request.binding.scope)?;
        if request.payload.arguments_digest() != submission.arguments_digest
            || request.plan != submission.plan
        {
            return Err("STALE_CONTENT");
        }
        if let Some(record) = &request.decision {
            if submission.request_version != record.submission_version
                && submission.request_version != request.version
            {
                return Err("STALE_REQUEST");
            }
            return if record.value == decision {
                Ok(Response::DecisionRecorded {
                    already_recorded: true,
                })
            } else {
                Err("DECISION_CONFLICT")
            };
        }
        if request.phase != Phase::Pending {
            return Err("REQUEST_NOT_PENDING");
        }
        if request.version != submission.request_version {
            return Err("STALE_REQUEST");
        }
        request.payload.validate_decision(&decision)?;
        let request = self
            .requests
            .get_mut(&submission.request_id)
            .ok_or("REQUEST_NOT_FOUND")?;
        request.decision = Some(DecisionRecord {
            submission_version: submission.request_version,
            value: decision.clone(),
        });
        request.phase = Phase::Decided;
        request.version += 1;
        if matches!(
            decision,
            Decision::Plan {
                action: PlanAction::Revise | PlanAction::Rejected | PlanAction::Cancelled,
                ..
            }
        ) {
            let plan = request.plan.as_ref().ok_or("INVALID_PLAN")?;
            let slot = self
                .plans
                .get_mut(&Self::plan_key(&request.binding.scope, &plan.plan_id))
                .ok_or("STATE_INCONSISTENT")?;
            slot.human_revision_sources
                .insert(request.request_id.clone());
            self.gate_revision += 1;
        }
        Ok(Response::DecisionRecorded {
            already_recorded: false,
        })
    }
    pub(crate) fn claim(
        &mut self,
        request_id: &str,
        scope: Scope,
        owner: Owner,
    ) -> Result<Response> {
        self.validate_scope(&scope)?;
        self.validate_ticket_projects(request_id)?;
        let request = self
            .requests
            .get_mut(request_id)
            .ok_or("REQUEST_NOT_FOUND")?;
        if request.binding.scope != scope || request.owner.as_ref() != Some(&owner) {
            return Err("NOT_RESULT_OWNER");
        }
        if request.phase != Phase::Decided || request.transport != Transport::NotStarted {
            return Err("RESULT_NOT_CLAIMABLE");
        }
        let decision = request
            .decision
            .as_ref()
            .ok_or("DECISION_MISSING")?
            .value
            .clone();
        let lease = Lease {
            id: id(),
            deadline_ms: request
                .deadline_ms
                .min(self.now_ms.saturating_add(LEASE_MS)),
        };
        request.transport = Transport::Claimed;
        request.lease = Some(lease.clone());
        request.version += 1;
        Ok(Response::Claimed {
            lease_id: lease.id,
            deadline_ms: lease.deadline_ms,
            decision,
            plan: request.plan.clone(),
        })
    }
    fn delivery(
        &mut self,
        request_id: &str,
        lease_id: &str,
        outcome: DeliveryOutcome,
    ) -> Result<Response> {
        let request = self.requests.get(request_id).ok_or("REQUEST_NOT_FOUND")?;
        let lease = request
            .lease
            .as_ref()
            .filter(|l| l.id == lease_id)
            .ok_or("DELIVERY_LEASE_INVALID")?;
        let live = request.phase == Phase::Decided
            && request.transport == Transport::Claimed
            && self.now_ms < lease.deadline_ms
            && self.now_ms < request.deadline_ms
            && self.validate_scope(&request.binding.scope).is_ok();
        if request.transport == Transport::Written {
            return if outcome == DeliveryOutcome::Written {
                Ok(Response::DeliveryRecorded { historical: true })
            } else {
                Err("DELIVERY_ALREADY_WRITTEN")
            };
        }
        match outcome {
            DeliveryOutcome::Written => {
                let request = self
                    .requests
                    .get_mut(request_id)
                    .ok_or("REQUEST_NOT_FOUND")?;
                request.transport = Transport::Written;
                request.version += 1;
                if live {
                    if let Some(plan) = &request.plan {
                        if let Some(slot) = self
                            .plans
                            .get_mut(&Self::plan_key(&request.binding.scope, &plan.plan_id))
                        {
                            if slot.request_id == request_id && slot.revision == plan.revision {
                                slot.temporary_wait = false;
                                if matches!(
                                    &request.decision,
                                    Some(DecisionRecord {
                                        value: Decision::Plan {
                                            action: PlanAction::Approved,
                                            ..
                                        },
                                        ..
                                    })
                                ) {
                                    slot.human_revision_sources.clear();
                                }
                                self.gate_revision += 1;
                            }
                        }
                    }
                }
            }
            DeliveryOutcome::KnownNotWritten | DeliveryOutcome::Unknown => {
                self.interrupt(request_id, Phase::Disconnected, "DELIVERY_INTERRUPTED");
                let request = self
                    .requests
                    .get_mut(request_id)
                    .ok_or("REQUEST_NOT_FOUND")?;
                request.transport = if outcome == DeliveryOutcome::KnownNotWritten {
                    Transport::DeliveryFailed
                } else {
                    Transport::OutcomeUnknown
                };
                request.version += 1;
            }
        }
        Ok(Response::DeliveryRecorded { historical: !live })
    }
    pub(crate) fn interrupt(&mut self, request_id: &str, phase: Phase, reason: &str) {
        let Some(request) = self.requests.get_mut(request_id) else {
            return;
        };
        if !request.unfinished() {
            return;
        }
        request.phase = phase;
        request.reason = Some(reason.into());
        request.version += 1;
        if request.transport == Transport::Claimed {
            request.transport = Transport::OutcomeUnknown;
        }
        if let Some(plan) = &request.plan {
            if let Some(slot) = self
                .plans
                .get_mut(&Self::plan_key(&request.binding.scope, &plan.plan_id))
            {
                if slot.request_id == request_id
                    && slot.revision == plan.revision
                    && slot.temporary_wait
                {
                    slot.temporary_wait = false;
                    self.gate_revision += 1;
                }
            }
        }
    }
    fn expire(&mut self) {
        let expired = self
            .requests
            .values()
            .filter(|r| r.binding.scope.app_epoch == self.app_epoch && r.unfinished())
            .filter_map(|r| {
                if r.transport == Transport::Claimed
                    && r.lease
                        .as_ref()
                        .is_some_and(|l| self.now_ms >= l.deadline_ms)
                {
                    Some((
                        r.request_id.clone(),
                        Phase::Disconnected,
                        "DELIVERY_LEASE_EXPIRED",
                    ))
                } else if self.now_ms >= r.deadline_ms {
                    Some((
                        r.request_id.clone(),
                        Phase::Expired,
                        if r.phase == Phase::AwaitingMcpEntry {
                            "MCP_ENTRY_EXPIRED"
                        } else {
                            "HUMAN_WAIT_EXPIRED"
                        },
                    ))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        for (request, phase, reason) in expired {
            self.interrupt(&request, phase, reason);
        }
    }
    fn end_generation(&mut self, root: &str, generation: &str) {
        let ids = self
            .requests
            .values()
            .filter(|r| {
                r.binding.scope.app_epoch == self.app_epoch
                    && r.binding.scope.root_conversation_id == root
                    && r.binding.scope.generation_id == generation
            })
            .map(|r| r.request_id.clone())
            .collect::<Vec<_>>();
        for request in ids {
            self.interrupt(&request, Phase::Cancelled, "GENERATION_ENDED");
        }
        self.plans.retain(|_, p| {
            !(p.scope.root_conversation_id == root && p.scope.generation_id == generation)
        });
        if let Some(session) = self.sessions.get_mut(root) {
            session.retired.insert(generation.into());
            session.current = None;
        }
        self.gate_revision += 1;
    }
    /// Restored state is read-only history. No pending request, owner, lease,
    /// idempotency result or plan approval is adopted by the new appEpoch.
    pub fn recover(mut saved: Self, installation_id: &str) -> Result<Self> {
        if saved.schema_version != 1
            || saved.installation_id != installation_id
            || !schema::valid_id(installation_id)
            || saved.requests.len() > MAX_REQUESTS
            || !(1..9_007_199_254_740_991).contains(&saved.config_revision)
            || saved.projects.iter().any(|(key, project)| {
                !schema::valid_id(key) || !(1..9_007_199_254_740_991).contains(&project.revision)
            })
        {
            return Err("INVALID_SAVED_STATE");
        }
        for (id, request) in &saved.requests {
            if id != &request.request_id
                || Uuid::parse_str(id).is_err()
                || !(1..9_007_199_254_740_991).contains(&request.version)
                || (request.phase == Phase::Decided && request.decision.is_none())
                || (matches!(request.transport, Transport::Claimed | Transport::Written)
                    && request.decision.is_none())
            {
                return Err("INVALID_SAVED_STATE");
            }
            request.payload.validate_saved()?;
            match (request.payload.body(), &request.plan) {
                (Body::Question(_) | Body::Permission(_), None) => {}
                (Body::Plan(body), Some(plan))
                    if body.plan_id == plan.plan_id
                        && (1..9_007_199_254_740_991).contains(&plan.revision)
                        && request.payload.plan_content_digest()
                            == Some(plan.content_digest.as_str()) => {}
                _ => return Err("INVALID_SAVED_PLAN"),
            }
            if let Some(decision) = &request.decision {
                request.payload.validate_decision(&decision.value)?;
            }
        }
        for request in saved.requests.values_mut() {
            if request.unfinished() {
                request.phase = Phase::Disconnected;
                request.reason = Some("APP_RESTARTED".into());
                request.version += 1;
            }
            if request.transport == Transport::Claimed {
                request.transport = Transport::OutcomeUnknown;
            }
            request.owner = None;
            request.lease = None;
        }
        saved.app_epoch = id();
        saved.now_ms = 0;
        saved.gate_revision = 0;
        saved.idempotency.clear();
        saved.plans.clear();
        saved.sessions.clear();
        saved.tickets.clear();
        saved.interactions.recover();
        Ok(saved)
    }
}

#[cfg(test)]
#[path = "cursor_workflow_tests.rs"]
mod tests;
