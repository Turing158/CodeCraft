use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use uuid::Uuid;

pub const ASK_TOOL: &str = "codecraft_ask_user";
pub const PLAN_TOOL: &str = "codecraft_review_plan";
pub const PLAN_LIMIT: usize = 128 * 1024;
pub const MAX_VERSION: u32 = i32::MAX as u32;
pub const HOOK_TIMEOUT_SECONDS: u64 = 150;
pub fn id() -> String {
    Uuid::new_v4().to_string()
}
pub fn stamp(seconds: u64) -> String {
    (chrono::Utc::now() + chrono::Duration::seconds(seconds as i64))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
pub fn nullable<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> std::result::Result<Option<T>, D::Error> {
    Option::deserialize(d)
}
fn optional_ticket<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<String>, D::Error> {
    String::deserialize(d).map(Some)
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    InvalidArgument,
    PayloadTooLarge,
    TicketInvalid,
    TicketExpired,
    ArgumentMismatch,
    IdempotencyConflict,
    BridgeUnavailable,
    StateUnavailable,
    QueueFull,
    RequestExpired,
    Cancelled,
    TaskChanged,
    PlanChanged,
    DeliveryUnconfirmed,
    UnsupportedVersion,
    Forbidden,
    NotFound,
    RequestConflict,
    NativeConfirmationPending,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ErrorDetail {
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
    #[serde(deserialize_with = "nullable")]
    #[schemars(required, schema_with = "nullable_number")]
    pub current_revision: Option<u32>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApiError {
    pub schema_version: u8,
    pub error: ErrorDetail,
}
impl ApiError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            schema_version: 1,
            error: ErrorDetail {
                code,
                message: message.into(),
                retryable: false,
                current_revision: None,
            },
        }
    }
    pub fn status(&self) -> u16 {
        match self.error.code {
            ErrorCode::InvalidArgument => 400,
            ErrorCode::Forbidden => 403,
            ErrorCode::NotFound => 404,
            ErrorCode::RequestExpired | ErrorCode::TicketExpired => 410,
            ErrorCode::PayloadTooLarge => 413,
            ErrorCode::QueueFull => 429,
            ErrorCode::BridgeUnavailable
            | ErrorCode::StateUnavailable
            | ErrorCode::UnsupportedVersion => 503,
            _ => 409,
        }
    }
}
impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.error.message)
    }
}
impl std::error::Error for ApiError {}
impl From<std::io::Error> for ApiError {
    fn from(e: std::io::Error) -> Self {
        Self::new(ErrorCode::StateUnavailable, e.to_string())
    }
}
pub type Result<T> = std::result::Result<T, ApiError>;
pub fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(ErrorCode::InvalidArgument, message)
}
pub fn check_version(v: u8) -> Result<()> {
    if v == 1 {
        Ok(())
    } else {
        Err(ApiError::new(
            ErrorCode::UnsupportedVersion,
            "Expected schemaVersion 1",
        ))
    }
}
pub fn check_id(v: &str) -> Result<()> {
    Uuid::parse_str(v)
        .map(|_| ())
        .map_err(|_| invalid("Invalid UUID"))
}
pub fn check_counter(v: u32) -> Result<()> {
    if v <= MAX_VERSION {
        Ok(())
    } else {
        Err(invalid("Version exceeds 2^31-1"))
    }
}
pub fn check_text(v: &str, max: usize, required: bool) -> Result<()> {
    if v.chars().count() > max || (required && v.trim().is_empty()) {
        Err(invalid("Text is empty or too long"))
    } else {
        Ok(())
    }
}
pub fn check_hash(v: &str) -> Result<()> {
    if v.len() == 64
        && v.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        Ok(())
    } else {
        Err(invalid("Invalid SHA-256"))
    }
}
fn short_id(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 64
        && v.bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QuestionOption {
    pub option_id: String,
    #[schemars(length(min = 1, max = 256))]
    pub label: String,
    #[serde(deserialize_with = "nullable")]
    #[schemars(required, schema_with = "nullable_string")]
    pub description: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum QuestionKind {
    Single,
    Multiple,
    Text,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Question {
    pub question_id: String,
    #[schemars(length(min = 1, max = 4096))]
    pub prompt: String,
    pub kind: QuestionKind,
    #[schemars(length(max = 12))]
    pub options: Vec<QuestionOption>,
    pub required: bool,
    pub allow_text: bool,
    pub min_selections: u32,
    pub max_selections: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QuestionInput {
    pub schema_version: u8,
    #[schemars(length(min = 1, max = 8))]
    pub questions: Vec<Question>,
    #[serde(
        default,
        deserialize_with = "optional_ticket",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(with = "String")]
    pub bridge_ticket: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AnswerStatus {
    Answered,
    Skipped,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Answer {
    pub question_id: String,
    pub status: AnswerStatus,
    pub selected_option_ids: Vec<String>,
    #[serde(deserialize_with = "nullable")]
    #[schemars(required, schema_with = "nullable_string")]
    pub text: Option<String>,
}
impl QuestionInput {
    pub fn validate(&self) -> Result<()> {
        check_version(self.schema_version)?;
        if self.questions.is_empty() || self.questions.len() > 8 {
            return Err(invalid("Expected 1–8 questions"));
        }
        let mut ids = HashSet::new();
        for q in &self.questions {
            if !short_id(&q.question_id) || !ids.insert(&q.question_id) {
                return Err(invalid("Duplicate or invalid question ID"));
            }
            check_text(&q.prompt, 4096, true)?;
            let mut options = HashSet::new();
            if q.options.len() > 12 {
                return Err(invalid("Too many options"));
            }
            for o in &q.options {
                if !short_id(&o.option_id) || !options.insert(&o.option_id) {
                    return Err(invalid("Duplicate or invalid option ID"));
                }
                check_text(&o.label, 256, true)?;
                if let Some(v) = &o.description {
                    check_text(v, 1024, false)?;
                }
            }
            let valid = match q.kind {
                QuestionKind::Text => {
                    q.options.is_empty()
                        && q.allow_text
                        && q.min_selections == 0
                        && q.max_selections == 0
                }
                QuestionKind::Single => {
                    !q.options.is_empty() && q.max_selections == 1 && q.min_selections <= 1
                }
                QuestionKind::Multiple => {
                    !q.options.is_empty()
                        && q.min_selections <= q.max_selections
                        && q.max_selections <= q.options.len() as u32
                }
            };
            if !valid {
                return Err(invalid("Question selection constraints are inconsistent"));
            }
        }
        Ok(())
    }
    pub fn validate_answers(&self, answers: &[Answer]) -> Result<()> {
        if answers.len() != self.questions.len() {
            return Err(invalid("Answer every question exactly once"));
        }
        let mut seen = HashSet::new();
        for a in answers {
            let q = self
                .questions
                .iter()
                .find(|q| q.question_id == a.question_id)
                .ok_or_else(|| invalid("Unknown question ID"))?;
            if !seen.insert(&a.question_id) {
                return Err(invalid("Duplicate answer"));
            }
            if let Some(t) = &a.text {
                check_text(t, 4096, false)?;
            }
            if a.status == AnswerStatus::Skipped {
                if q.required || !a.selected_option_ids.is_empty() || a.text.is_some() {
                    return Err(invalid("Invalid skipped answer"));
                }
                continue;
            }
            let mut selected = HashSet::new();
            for id in &a.selected_option_ids {
                if !selected.insert(id) || !q.options.iter().any(|o| &o.option_id == id) {
                    return Err(invalid("Invalid option ID"));
                }
            }
            let count = a.selected_option_ids.len() as u32;
            if count < q.min_selections
                || count > q.max_selections
                || (!q.allow_text && a.text.is_some())
                || (count == 0 && a.text.as_ref().is_none_or(|t| t.trim().is_empty()))
            {
                return Err(invalid("Answer does not satisfy question constraints"));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanInput {
    pub schema_version: u8,
    pub plan_id: String,
    pub base_revision: u32,
    pub document_path: String,
    pub plan_markdown: String,
    #[serde(
        default,
        deserialize_with = "optional_ticket",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(with = "String")]
    pub bridge_ticket: Option<String>,
}
impl PlanInput {
    pub fn validate(&self) -> Result<()> {
        check_version(self.schema_version)?;
        check_id(&self.plan_id)?;
        check_counter(self.base_revision)?;
        if self.plan_markdown.len() > PLAN_LIMIT {
            return Err(ApiError::new(
                ErrorCode::PayloadTooLarge,
                "Plan exceeds 128 KiB",
            ));
        }
        check_text(&self.plan_markdown, PLAN_LIMIT, true)
    }
}
pub fn validate_business(tool: &str, args: &Value) -> Result<()> {
    match tool {
        ASK_TOOL => serde_json::from_value::<QuestionInput>(args.clone())
            .map_err(|e| invalid(e.to_string()))?
            .validate(),
        PLAN_TOOL => serde_json::from_value::<PlanInput>(args.clone())
            .map_err(|e| invalid(e.to_string()))?
            .validate(),
        _ => Err(invalid("Unknown CodeCraft MCP tool")),
    }
}
pub fn own_tool(name: &str) -> Option<&'static str> {
    match name {
        "mcp__codecraft__codecraft_ask_user" => Some(ASK_TOOL),
        "mcp__codecraft__codecraft_review_plan" => Some(PLAN_TOOL),
        _ => None,
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Target {
    pub app_epoch: String,
    pub session_key: String,
    #[serde(deserialize_with = "nullable")]
    #[schemars(required, schema_with = "nullable_string")]
    pub task_id: Option<String>,
    pub turn_epoch: u32,
    pub request_id: String,
    pub request_version: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PermissionDecision {
    Allow,
    Deny,
    Ask,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PlanDecision {
    Approved,
    ChangesRequested,
    Rejected,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Permission {
        decision: PermissionDecision,
        #[serde(deserialize_with = "nullable")]
        #[schemars(required, schema_with = "nullable_string")]
        message: Option<String>,
    },
    Question {
        answers: Vec<Answer>,
    },
    Plan {
        decision: PlanDecision,
        revision: u32,
        #[serde(rename = "contentHash")]
        content_hash: String,
        #[serde(deserialize_with = "nullable")]
        #[schemars(required, schema_with = "nullable_string")]
        feedback: Option<String>,
    },
    Cancel {
        #[serde(deserialize_with = "nullable")]
        #[schemars(required, schema_with = "nullable_string")]
        reason: Option<String>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DecisionRequest {
    pub schema_version: u8,
    pub decision_id: String,
    pub target: Target,
    pub action: Action,
}
impl DecisionRequest {
    pub fn validate(&self) -> Result<()> {
        check_version(self.schema_version)?;
        check_id(&self.decision_id)?;
        check_id(&self.target.app_epoch)?;
        check_id(&self.target.request_id)?;
        if let Some(id) = &self.target.task_id {
            check_id(id)?;
        }
        check_counter(self.target.turn_epoch)?;
        check_counter(self.target.request_version)?;
        match &self.action {
            Action::Permission { message, .. } | Action::Cancel { reason: message } => {
                if let Some(v) = message {
                    check_text(v, 8192, false)?;
                }
            }
            Action::Plan {
                decision,
                feedback,
                content_hash,
                revision,
            } => {
                check_hash(content_hash)?;
                check_counter(*revision)?;
                if let Some(v) = feedback {
                    check_text(v, 8192, false)?;
                }
                if *decision == PlanDecision::ChangesRequested
                    && feedback.as_ref().is_none_or(|s| s.trim().is_empty())
                {
                    return Err(invalid("Changes requested requires feedback"));
                }
            }
            _ => (),
        };
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateTask {
    pub schema_version: u8,
    pub control_id: String,
    pub title: String,
    pub primary_root: String,
    pub workspace_roots: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskActionKind {
    Pause,
    Resume,
    End,
    RevokePlan,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskAction {
    pub schema_version: u8,
    pub control_id: String,
    pub task_id: String,
    pub expected_version: u32,
    pub action: TaskActionKind,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GrantResolution {
    ConfirmedCancelledInTrae,
    ConfirmedCompletedInTrae,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolveGrant {
    pub schema_version: u8,
    pub control_id: String,
    pub grant_id: String,
    pub expected_version: u32,
    pub action: GrantResolution,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolResult {
    pub schema_version: u8,
    #[serde(deserialize_with = "nullable")]
    #[schemars(required, schema_with = "nullable_string")]
    pub operation_id: Option<String>,
    #[serde(deserialize_with = "nullable")]
    #[schemars(required, schema_with = "nullable_string")]
    pub request_id: Option<String>,
    pub status: String,
    #[serde(deserialize_with = "nullable")]
    #[schemars(required, schema_with = "nullable_value")]
    pub payload: Option<Value>,
    #[serde(deserialize_with = "nullable")]
    #[schemars(required, schema_with = "nullable_error")]
    pub error: Option<ErrorDetail>,
}
impl ToolResult {
    pub fn failure(error: ApiError, id: Option<String>) -> Self {
        Self {
            schema_version: 1,
            operation_id: id.clone(),
            request_id: id,
            status: match error.error.code {
                ErrorCode::Cancelled => "cancelled",
                ErrorCode::RequestExpired | ErrorCode::TicketExpired => "expired",
                _ => "failed",
            }
            .into(),
            payload: None,
            error: Some(error.error),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    #[serde(default)]
    pub tool_input_mappings_verified: bool,
    pub tool_approval: bool,
    pub mcp_questions: bool,
    pub mcp_plan_review: bool,
    pub native_observation: bool,
    #[serde(default)]
    pub hook_timeout_seconds: Option<u64>,
    pub verified_hook_timeout_seconds: Option<u64>,
    pub verified_mcp_timeout_seconds: Option<u64>,
    pub verified_version: Option<String>,
    pub reason: String,
}
impl Capabilities {
    pub fn bundled(version: Option<&str>) -> Self {
        let evidence: Value = serde_json::from_str(include_str!(
            "../../protocol/trae/3.3.102/capabilities.json"
        ))
        .unwrap_or(Value::Null);
        if version.is_none() || version != evidence["productVersion"].as_str() {
            return Self {
                reason: "This Trae version does not support CodeCraft tool approval yet".into(),
                ..Self::default()
            };
        }
        // Feature availability is independent of the retired MCP/task suite's
        // overall verification status. Keep measured evidence separate from the
        // timeout installed in hooks.json; do not label configuration as verified.
        let tool_approval = evidence["toolApproval"] == true;
        Self {
            tool_input_mappings_verified: evidence["toolInputMappingsVerified"] == true,
            tool_approval,
            mcp_questions: false,
            mcp_plan_review: false,
            native_observation: evidence["nativeInteractionObservation"] == true,
            hook_timeout_seconds: tool_approval.then_some(HOOK_TIMEOUT_SECONDS),
            verified_hook_timeout_seconds: evidence["verifiedHookTimeoutSeconds"].as_u64(),
            verified_mcp_timeout_seconds: evidence["verifiedMcpTimeoutSeconds"].as_u64(),
            verified_version: (evidence["status"] == "verified")
                .then(|| version.unwrap().to_string()),
            reason: String::new(),
        }
    }
    pub fn hook_wait(&self) -> Option<u64> {
        self.hook_timeout_seconds
            .or(self.verified_hook_timeout_seconds)
            .and_then(|v| v.checked_sub(30))
            .map(|v| v.min(120))
            .filter(|v| *v >= 30 && self.tool_approval)
    }
    pub fn mcp_wait(&self) -> Option<u64> {
        self.verified_mcp_timeout_seconds
            .and_then(|v| v.checked_sub(30))
            .map(|v| v.min(240))
            .filter(|v| *v >= 30)
    }
}
pub fn schemas() -> Value {
    let mut schemas = json!({"QuestionInput":schemars::schema_for!(QuestionInput),"PlanInput":schemars::schema_for!(PlanInput),"ToolResult":schemars::schema_for!(ToolResult),"DecisionRequest":schemars::schema_for!(DecisionRequest),"CreateTask":schemars::schema_for!(CreateTask),"TaskAction":schemars::schema_for!(TaskAction),"ResolveGrant":schemars::schema_for!(ResolveGrant),"ApiError":schemars::schema_for!(ApiError)});
    fn constrain(v: &mut Value) {
        if let Some(properties) = v.get_mut("properties").and_then(Value::as_object_mut) {
            for (name, schema) in properties {
                match name.as_str() {
                    "schemaVersion" => schema["const"] = json!(1),
                    "appEpoch" | "requestId" | "operationId" | "taskId" | "planId"
                    | "decisionId" | "controlId" | "grantId" => schema["format"] = json!("uuid"),
                    "sessionKey" | "contentHash" => schema["pattern"] = json!("^[a-f0-9]{64}$"),
                    "questionId" | "optionId" => {
                        schema["pattern"] = json!("^[A-Za-z0-9_-]{1,64}$");
                    }
                    "bridgeTicket" => {
                        schema["type"] = json!("string");
                        schema["pattern"] = json!("^[A-Za-z0-9_-]{43}$");
                    }
                    "requestVersion" | "revision" | "expectedVersion" | "baseRevision"
                    | "turnEpoch" => {
                        schema["minimum"] = json!(0);
                        schema["maximum"] = json!(MAX_VERSION);
                    }
                    "minSelections" | "maxSelections" => {
                        schema["minimum"] = json!(0);
                        schema["maximum"] = json!(12);
                    }
                    "feedback" | "message" | "reason" => schema["maxLength"] = json!(8192),
                    "text" => schema["maxLength"] = json!(4096),
                    "description" => schema["maxLength"] = json!(1024),
                    "title" => {
                        schema["minLength"] = json!(1);
                        schema["maxLength"] = json!(200);
                    }
                    "workspaceRoots" => {
                        schema["minItems"] = json!(1);
                        schema["maxItems"] = json!(16);
                        schema["uniqueItems"] = json!(true);
                    }
                    "selectedOptionIds" => {
                        schema["maxItems"] = json!(12);
                        schema["uniqueItems"] = json!(true);
                        schema["items"]["pattern"] = json!("^[A-Za-z0-9_-]{1,64}$");
                    }
                    "answers" => {
                        schema["minItems"] = json!(1);
                        schema["maxItems"] = json!(8);
                    }
                    "planMarkdown" => {
                        schema["minLength"] = json!(1);
                        schema["maxLength"] = json!(PLAN_LIMIT);
                    }
                    _ => (),
                }
            }
        }
        match v {
            Value::Object(o) => o.values_mut().for_each(constrain),
            Value::Array(a) => a.iter_mut().for_each(constrain),
            _ => (),
        }
    }
    constrain(&mut schemas);
    let question = json!({"type":"object","additionalProperties":false,"required":["kind","answers"],"properties":{"kind":{"const":"question"},"answers":{"type":"array","minItems":1,"maxItems":8,"items":{"$ref":"#/$defs/Answer"}}}});
    let plan = json!({"type":"object","additionalProperties":false,"required":["kind","planId","revision","contentHash","decision","feedback"],"properties":{"kind":{"const":"plan"},"planId":{"type":"string","format":"uuid"},"revision":{"type":"integer","minimum":0,"maximum":MAX_VERSION},"contentHash":{"type":"string","pattern":"^[a-f0-9]{64}$"},"decision":{"enum":["approved","changes_requested","rejected"]},"feedback":{"type":["string","null"],"maxLength":8192}}});
    schemas["ToolResult"]["$defs"]["Answer"] =
        schemas["DecisionRequest"]["$defs"]["Answer"].clone();
    schemas["ToolResult"]["$defs"]["AnswerStatus"] =
        schemas["DecisionRequest"]["$defs"]["AnswerStatus"].clone();
    schemas["ToolResult"]["properties"]["payload"] =
        json!({"anyOf":[question,plan,{"type":"null"}]});
    schemas["ToolResult"]["properties"]["status"] =
        json!({"enum":["completed","cancelled","expired","failed"]});
    schemas["ToolResult"]["allOf"] = json!([{"if":{"properties":{"status":{"const":"completed"}}},"then":{"properties":{"operationId":{"type":"string"},"requestId":{"type":"string"},"payload":{"type":"object"},"error":{"type":"null"}}},"else":{"properties":{"payload":{"type":"null"},"error":{"type":"object"}}}}]);
    schemas
}

fn nullable_string(g: &mut schemars::SchemaGenerator) -> schemars::Schema {
    <Option<String> as JsonSchema>::json_schema(g)
}
fn nullable_number(g: &mut schemars::SchemaGenerator) -> schemars::Schema {
    <Option<u32> as JsonSchema>::json_schema(g)
}
fn nullable_value(g: &mut schemars::SchemaGenerator) -> schemars::Schema {
    <Option<Value> as JsonSchema>::json_schema(g)
}
fn nullable_error(g: &mut schemars::SchemaGenerator) -> schemars::Schema {
    <Option<ErrorDetail> as JsonSchema>::json_schema(g)
}
