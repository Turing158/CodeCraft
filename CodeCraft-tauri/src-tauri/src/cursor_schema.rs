//! CodeCraft-owned MCP business schemas. These do not describe or enable any
//! Cursor native API. Raw input is validated before tickets are discarded.
use crate::protocol::{arguments_digest, plan_content_digest, strict_json};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

pub type Result<T> = std::result::Result<T, &'static str>;
pub const ASK_USER: &str = "codecraft_ask_user";
pub const REVIEW_PLAN: &str = "codecraft_review_plan";
pub const HOOK_PERMISSION: &str = "codecraft_hook_permission";

pub fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
}
fn text(value: &str, max: usize, required: bool) -> bool {
    value.len() <= max && (!required || !value.trim().is_empty())
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QuestionOption {
    pub option_id: String,
    pub label: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Question {
    pub question_id: String,
    pub prompt: String,
    pub options: Vec<QuestionOption>,
    pub multi_select: bool,
    pub allow_free_text: bool,
    pub required: bool,
    pub allow_skip: bool,
    pub max_text_length: usize,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AskRequest {
    pub schema_version: u32,
    pub client_request_id: String,
    pub questions: Vec<Question>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
    Cancelled,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanTodo {
    pub id: String,
    pub description: String,
    pub status: TodoStatus,
    pub phase_id: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanPhase {
    pub id: String,
    pub title: String,
    pub description: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanRequest {
    pub schema_version: u32,
    pub client_request_id: String,
    pub plan_id: String,
    pub base_revision: u64,
    pub supersedes_client_request_id: Option<String>,
    pub title: String,
    pub overview: String,
    pub markdown: String,
    pub todos: Vec<PlanTodo>,
    pub phases: Vec<PlanPhase>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "kind", content = "content", rename_all = "camelCase")]
pub enum Body {
    Question(AskRequest),
    Plan(PlanRequest),
    Permission(PermissionRequest),
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PermissionRequest {
    pub schema_version: u32,
    pub client_request_id: String,
    pub event_name: String,
    pub tool_name: String,
    pub tool_input: Value,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum PermissionAction {
    AllowOnce,
    Deny,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Answer {
    pub question_id: String,
    pub selected_option_ids: Vec<String>,
    pub text: Option<String>,
    pub skipped: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum PlanAction {
    Approved,
    Revise,
    Rejected,
    Cancelled,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Decision {
    Permission {
        action: PermissionAction,
    },
    Answered {
        answers: Vec<Answer>,
    },
    QuestionCancelled,
    Plan {
        action: PlanAction,
        feedback: String,
    },
}

/// Only `parse` constructs a business request for admission. It preserves all
/// original business fields for the digest and never stores bridgeTicket.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolRequest {
    tool_name: String,
    business: Value,
    arguments_digest: String,
    plan_content_digest: Option<String>,
    body: Box<Body>,
}
impl ToolRequest {
    pub fn parse(tool: &str, bytes: &[u8]) -> Result<Self> {
        let mut business = strict_json(bytes).map_err(|_| "INVALID_JSON")?;
        let object = business.as_object_mut().ok_or("INVALID_ARGUMENTS")?;
        if let Some(ticket) = object.remove("bridgeTicket") {
            if !ticket
                .as_str()
                .is_some_and(|t| !t.is_empty() && t.len() <= 512)
            {
                return Err("INVALID_TICKET_FIELD");
            }
        }
        let body = match tool {
            HOOK_PERMISSION => {
                let input: PermissionRequest =
                    serde_json::from_value(business.clone()).map_err(|_| "INVALID_PERMISSION")?;
                if input.schema_version != 1
                    || !valid_id(&input.client_request_id)
                    || !matches!(
                        input.event_name.as_str(),
                        "preToolUse" | "beforeMCPExecution"
                    )
                    || !text(&input.tool_name, 2048, true)
                    || serde_json::to_vec(&input.tool_input)
                        .map_err(|_| "INVALID_PERMISSION")?
                        .len()
                        > 131072
                {
                    return Err("INVALID_PERMISSION");
                }
                Body::Permission(input)
            }
            ASK_USER => {
                let input: AskRequest =
                    serde_json::from_value(business.clone()).map_err(|_| "INVALID_QUESTION")?;
                if input.schema_version != 1 || !valid_id(&input.client_request_id) {
                    return Err("INVALID_REQUEST");
                }
                if input.questions.is_empty() || input.questions.len() > 32 {
                    return Err("INVALID_QUESTION");
                }
                let mut ids = BTreeSet::new();
                for question in &input.questions {
                    if !valid_id(&question.question_id)
                        || !ids.insert(&question.question_id)
                        || !text(&question.prompt, 8192, true)
                        || question.options.len() > 64
                        || question.max_text_length > 4096
                        || (question.allow_free_text && question.max_text_length == 0)
                        || (!question.allow_free_text && question.options.is_empty())
                    {
                        return Err("INVALID_QUESTION");
                    }
                    let mut options = BTreeSet::new();
                    for option in &question.options {
                        if !valid_id(&option.option_id)
                            || !options.insert(&option.option_id)
                            || !text(&option.label, 2048, true)
                        {
                            return Err("INVALID_OPTION");
                        }
                    }
                }
                Body::Question(input)
            }
            REVIEW_PLAN => {
                let input: PlanRequest =
                    serde_json::from_value(business.clone()).map_err(|_| "INVALID_PLAN")?;
                if input.schema_version != 1
                    || !valid_id(&input.client_request_id)
                    || !valid_id(&input.plan_id)
                    || input
                        .supersedes_client_request_id
                        .as_ref()
                        .is_some_and(|id| !valid_id(id))
                    || input.base_revision > 9_007_199_254_740_990
                {
                    return Err("INVALID_REQUEST");
                }
                if !text(&input.title, 2048, true)
                    || !text(&input.overview, 16384, false)
                    || !text(&input.markdown, 131072, true)
                    || input.todos.len() > 256
                    || input.phases.len() > 64
                {
                    return Err("INVALID_PLAN");
                }
                let mut phases = BTreeSet::new();
                for phase in &input.phases {
                    if !valid_id(&phase.id)
                        || !phases.insert(&phase.id)
                        || !text(&phase.title, 2048, true)
                        || !text(&phase.description, 8192, false)
                    {
                        return Err("INVALID_PHASE");
                    }
                }
                let mut todos = BTreeSet::new();
                for todo in &input.todos {
                    if !valid_id(&todo.id)
                        || !todos.insert(&todo.id)
                        || !text(&todo.description, 8192, true)
                        || todo
                            .phase_id
                            .as_ref()
                            .is_some_and(|id| !phases.contains(id))
                    {
                        return Err("INVALID_TODO");
                    }
                }
                Body::Plan(input)
            }
            _ => return Err("UNKNOWN_TOOL"),
        };
        Ok(Self {
            tool_name: tool.into(),
            arguments_digest: arguments_digest(tool, &business).map_err(|_| "INVALID_DIGEST")?,
            plan_content_digest: if matches!(body, Body::Plan(_)) {
                Some(plan_content_digest(&business).map_err(|_| "INVALID_DIGEST")?)
            } else {
                None
            },
            business,
            body: Box::new(body),
        })
    }
    pub fn tool_name(&self) -> &str {
        &self.tool_name
    }
    pub fn client_request_id(&self) -> &str {
        match self.body.as_ref() {
            Body::Permission(p) => &p.client_request_id,
            Body::Question(q) => &q.client_request_id,
            Body::Plan(p) => &p.client_request_id,
        }
    }
    pub fn body(&self) -> &Body {
        &self.body
    }
    pub fn arguments_digest(&self) -> &str {
        &self.arguments_digest
    }
    pub fn plan_content_digest(&self) -> Option<&str> {
        self.plan_content_digest.as_deref()
    }
    /// Disk snapshots are not a route around input validation.
    pub fn validate_saved(&self) -> Result<()> {
        let bytes = serde_json::to_vec(&self.business).map_err(|_| "INVALID_REQUEST")?;
        let parsed = Self::parse(&self.tool_name, &bytes)?;
        if &parsed != self {
            return Err("INVALID_SAVED_REQUEST");
        }
        Ok(())
    }
    pub fn validate_decision(&self, decision: &Decision) -> Result<()> {
        match (self.body.as_ref(), decision) {
            (Body::Permission(_), Decision::Permission { .. }) => Ok(()),
            (Body::Question(_), Decision::QuestionCancelled) => Ok(()),
            (Body::Question(request), Decision::Answered { answers }) => {
                if answers.len() != request.questions.len() {
                    return Err("INVALID_ANSWERS");
                }
                let mut seen = BTreeSet::new();
                for answer in answers {
                    let question = request
                        .questions
                        .iter()
                        .find(|q| q.question_id == answer.question_id)
                        .ok_or("UNKNOWN_QUESTION")?;
                    if !seen.insert(&answer.question_id) {
                        return Err("DUPLICATE_ANSWER");
                    }
                    let selected = answer.selected_option_ids.iter().collect::<BTreeSet<_>>();
                    if selected.len() != answer.selected_option_ids.len()
                        || (!question.multi_select && selected.len() > 1)
                        || selected
                            .iter()
                            .any(|id| !question.options.iter().any(|o| &o.option_id == *id))
                    {
                        return Err("INVALID_SELECTION");
                    }
                    let free = answer.text.as_deref().unwrap_or("");
                    if (!question.allow_free_text && answer.text.is_some())
                        || free.chars().count() > question.max_text_length
                        || free.len() > 16384
                    {
                        return Err("INVALID_ANSWER_TEXT");
                    }
                    if answer.skipped {
                        if !question.allow_skip || !selected.is_empty() || answer.text.is_some() {
                            return Err("SKIP_NOT_ALLOWED");
                        }
                    } else if question.required && selected.is_empty() && free.trim().is_empty() {
                        return Err("ANSWER_REQUIRED");
                    }
                }
                Ok(())
            }
            (Body::Plan(_), Decision::Plan { feedback, .. }) if text(feedback, 16384, false) => {
                Ok(())
            }
            _ => Err("INVALID_DECISION"),
        }
    }
}
