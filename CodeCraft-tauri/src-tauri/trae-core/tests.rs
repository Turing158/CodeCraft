use super::{files, protocol::*, store::Store};
use serde_json::{json, Value};
use std::{fs, path::PathBuf};

struct Fixture {
    root: PathBuf,
    workspace: PathBuf,
    store: Store,
}
impl Fixture {
    fn new() -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/trae-tests")
            .join(id());
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        files::initialize(&root).unwrap();
        let store = Store::new(root.clone(), verified());
        Self {
            root,
            workspace,
            store,
        }
    }
    fn event(&mut self, session: &str, event: &str, extra: Value) -> Result<Value> {
        let mut input = json!({"hook_event_name":event,"session_id":session,"cwd":self.workspace,"workspace_roots":[self.workspace]});
        input
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        self.store.apply(&json!({"kind":"hook","identity":{"installationId":"synthetic-installation","traeInstanceId":"synthetic-instance"},"input":input}))
    }
    fn tool(&mut self, session: &str, call: &str, name: &str, args: Value) -> Result<Value> {
        self.event(
            session,
            "PreToolUse",
            json!({"tool_use_id":call,"tool_name":name,"llm_tool_name":name,"tool_input":args}),
        )
    }
    fn consume(&mut self, session: &str, call: &str, name: &str, args: Value) -> String {
        let native = format!("mcp__codecraft__{name}");
        let reply = self.tool(session, call, &native, args).unwrap();
        let args = reply["output"]["hookSpecificOutput"]["updatedInput"].clone();
        let result = self.store.apply(&json!({"kind":"consume","connection":"connection","tool":name,"ticket":args["bridgeTicket"],"arguments":args})).unwrap();
        result["operationId"].as_str().unwrap().into()
    }
    fn decision(&mut self, op: &str, action: Value) -> Result<Value> {
        self.store.apply(&json!({"kind":"decision","body":{"schemaVersion":1,"decisionId":id(),"target":self.store.requests[op].target,"action":action}}))
    }
    fn prepare(&mut self, op: &str) -> Result<Value> {
        self.store
            .apply(&json!({"kind":"prepare","connection":"connection","operation":op}))
    }
    fn ack(&mut self, op: &str, lease: &Value) -> Result<Value> {
        self.store
            .apply(&json!({"kind":"ack","connection":"connection","operation":op,"lease":lease}))
    }
    fn task(&mut self) -> String {
        let result = self.store.apply(&json!({"kind":"create_task","body":{"schemaVersion":1,"controlId":id(),"title":"Synthetic task","primaryRoot":self.workspace,"workspaceRoots":[self.workspace]}})).unwrap();
        self.event(
            "one",
            "UserPromptSubmit",
            json!({"prompt":result["launchPrompt"]}),
        )
        .unwrap();
        result["taskId"].as_str().unwrap().into()
    }
    fn plan(&mut self, task: &str) -> String {
        let task = &self.store.tasks[task];
        let path = task.document_path.clone();
        fs::write(&path, "# Plan\r\n\r\nImplement safely.\r\n").unwrap();
        self.consume("one","plan-1",PLAN_TOOL,json!({"schemaVersion":1,"planId":task.plan_id,"baseRevision":task.revision,"documentPath":path,"planMarkdown":"# Plan\n\nImplement safely.\n"}))
    }
    fn approve(&mut self, op: &str) {
        let plan = self.store.requests[op].plan.clone().unwrap();
        self.decision(op,json!({"kind":"plan","decision":"approved","revision":plan["revision"],"contentHash":plan["contentHash"],"feedback":null})).unwrap();
    }
    fn native_file_change(&mut self, session: &str, call: &str, path: &str, body: &str) {
        let file = self.workspace.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        let args = json!({"file_path":file,"content":body});
        self.tool(session, call, "Write", args.clone()).unwrap();
        fs::write(&file, body).unwrap();
        self.event(session, "PostToolUse", json!({"tool_use_id":call,"tool_name":"Write","llm_tool_name":"Write","tool_input":args,"tool_response":{"changes":[{"file_path":file,"file_action":"added","new_content":body}]}})).unwrap();
    }
    fn native_plan_notice(&mut self, session: &str) -> Value {
        self.event(session, "Notification", json!({"notification_type":"document_review","message":"Tool 'NotifyUser' requires user confirmation","tool_use_id":"review-call"})).unwrap();
        self.store
            .sessions
            .values()
            .find(|s| s.session_id == session)
            .unwrap()
            .native_interactions[0]
            .clone()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let expected = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/trae-tests");
        assert!(self.root.starts_with(expected));
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn verified() -> Capabilities {
    Capabilities {
        tool_input_mappings_verified: true,
        tool_approval: true,
        mcp_questions: true,
        mcp_plan_review: true,
        native_observation: true,
        hook_timeout_seconds: Some(HOOK_TIMEOUT_SECONDS),
        verified_hook_timeout_seconds: Some(150),
        verified_mcp_timeout_seconds: Some(270),
        verified_version: Some("synthetic-test-only".into()),
        reason: String::new(),
    }
}
fn question() -> Value {
    json!({"schemaVersion":1,"questions":[{"questionId":"q","prompt":"Which one?","kind":"single","options":[{"optionId":"first","label":"Same label","description":null},{"optionId":"second","label":"Same label","description":null}],"required":true,"allowText":false,"minSelections":1,"maxSelections":1}]})
}
fn answer() -> Value {
    json!({"kind":"question","answers":[{"questionId":"q","status":"answered","selectedOptionIds":["second"],"text":null}]})
}

#[test]
fn session_details_retain_turns_and_survive_serialization() {
    let mut f = Fixture::new();
    for prompt in ["First question", "Second question"] {
        f.event("one", "UserPromptSubmit", json!({"prompt":prompt}))
            .unwrap();
        f.event(
            "one",
            "Stop",
            json!({"last_assistant_message":"Same final reply"}),
        )
        .unwrap();
    }
    f.event(
        "one",
        "Stop",
        json!({"last_assistant_message":"Same final reply"}),
    )
    .unwrap();
    f.event("two", "UserPromptSubmit", json!({"prompt":"Other session"}))
        .unwrap();
    let persisted = serde_json::to_value(&f.store).unwrap();
    f.store = serde_json::from_value(persisted).unwrap();
    let session = f
        .store
        .sessions
        .values()
        .find(|s| s.session_id == "one")
        .unwrap();
    assert_eq!(
        session
            .messages
            .iter()
            .map(|m| (m.role.as_str(), m.text.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("user", "First question"),
            ("assistant", "Same final reply"),
            ("user", "Second question"),
            ("assistant", "Same final reply"),
        ]
    );
    assert!(!session.started_at.is_empty());
    assert!(session.messages.iter().all(|m| !m.at.is_empty()));
    assert_eq!(
        f.store
            .sessions
            .values()
            .find(|s| s.session_id == "two")
            .unwrap()
            .messages
            .len(),
        1
    );
}

#[test]
fn session_details_deduplicate_truncated_replies_within_a_turn() {
    let mut f = Fixture::new();
    let reply = "🦀".repeat(40000);
    for prompt in ["First question", "Next question"] {
        f.event("one", "UserPromptSubmit", json!({"prompt":prompt}))
            .unwrap();
        for _ in 0..2 {
            f.event("one", "Stop", json!({"last_assistant_message":reply}))
                .unwrap();
        }
    }
    let session = f.store.sessions.values().next().unwrap();
    assert!(session.history_truncated);
    assert_eq!(session.messages.len(), 4);
    assert_eq!(session.messages[1].text.chars().count(), 32768);
    assert_eq!(session.messages[3].text, session.messages[1].text);
}

#[test]
fn session_details_pair_tools_and_do_not_claim_empty_results_succeeded() {
    let mut f = Fixture::new();
    let args = json!({"file_path":"example.txt","bridgeTicket":"hidden"});
    f.tool("one", "call", "Read", args.clone()).unwrap();
    assert_eq!(
        f.store.sessions.values().next().unwrap().activities[0]["status"],
        "running"
    );
    for _ in 0..2 {
        f.event("one", "PostToolUse", json!({"tool_use_id":"call","tool_name":"Read","llm_tool_name":"Read","tool_input":args,"tool_response":{}})).unwrap();
    }
    let session = f.store.sessions.values().next().unwrap();
    assert_eq!(session.activities.len(), 1);
    assert_eq!(session.activities[0]["status"], "unknown");
    assert!(!session.activities[0].to_string().contains("hidden"));
    f.event("one", "UserPromptSubmit", json!({"prompt":"next"}))
        .unwrap();
    f.tool("one", "call-2", "Read", args.clone()).unwrap();
    f.event("one", "PostToolUse", json!({"tool_use_id":"call-2","tool_name":"Read","llm_tool_name":"Read","tool_input":args,"tool_response":{"is_error":true,"message":"failed"}})).unwrap();
    let session = f.store.sessions.values().next().unwrap();
    assert_eq!(session.activities.len(), 2);
    assert_ne!(session.activities[0]["id"], session.activities[1]["id"]);
    assert_eq!(session.activities[1]["status"], "failed");
}

#[test]
fn session_details_upgrade_old_output_and_bound_unicode_history() {
    let mut f = Fixture::new();
    f.event("one", "SessionStart", json!({})).unwrap();
    let session = f.store.sessions.values_mut().next().unwrap();
    session.output = "Legacy reply".into();
    let mut old = serde_json::to_value(&*session).unwrap();
    for field in ["messages", "startedAt", "historyTruncated"] {
        old.as_object_mut().unwrap().remove(field);
    }
    *session = serde_json::from_value(old).unwrap();
    f.event("one", "UserPromptSubmit", json!({"prompt":"New prompt"}))
        .unwrap();
    assert_eq!(
        f.store.sessions.values().next().unwrap().messages[0].text,
        "Legacy reply"
    );
    for _ in 0..20 {
        f.event(
            "one",
            "UserPromptSubmit",
            json!({"prompt":"🦀".repeat(40000)}),
        )
        .unwrap();
    }
    let session = f.store.sessions.values().next().unwrap();
    assert!(session.history_truncated);
    assert!(session.messages.iter().map(|m| m.text.len()).sum::<usize>() <= 512 * 1024);
    assert_eq!(session.messages.last().unwrap().text.chars().count(), 32768);
}

#[test]
fn questions_preserve_ids_and_first_decision_wins() {
    let mut f = Fixture::new();
    let op = f.consume("one", "q-1", ASK_TOOL, question());
    let command = json!({"kind":"decision","body":{"schemaVersion":1,"decisionId":id(),"target":f.store.requests[&op].target,"action":answer()}});
    f.store.apply(&command).unwrap();
    assert_eq!(f.store.requests[&op].target.request_version, 2);
    assert_eq!(f.store.apply(&command).unwrap()["replayed"], true);
    assert_eq!(
        f.decision(&op, answer()).unwrap_err().error.code,
        ErrorCode::RequestConflict
    );
    let prepared = f.prepare(&op).unwrap();
    assert_eq!(f.store.requests[&op].target.request_version, 3);
    let mut stale_cancel = command.clone();
    stale_cancel["body"]["decisionId"] = json!(id());
    stale_cancel["body"]["action"] = json!({"kind":"cancel","reason":null});
    assert_eq!(
        f.store.apply(&stale_cancel).unwrap_err().error.code,
        ErrorCode::RequestConflict
    );
    assert_eq!(
        prepared["result"]["payload"]["answers"][0]["selectedOptionIds"][0],
        "second"
    );
    f.ack(&op, &prepared["lease"]).unwrap();
    assert_eq!(f.store.requests[&op].target.request_version, 4);
    assert_eq!(f.prepare(&op).unwrap()["state"], "delivered");
    f.event("one", "UserPromptSubmit", json!({"prompt":"continue"}))
        .unwrap();
    assert_eq!(
        f.prepare(&op).unwrap_err().error.code,
        ErrorCode::TaskChanged
    );
}
#[test]
fn tickets_bind_parameters_session_connection_and_one_operation() {
    let mut f = Fixture::new();
    let args = f
        .tool(
            "one",
            "call",
            "mcp__codecraft__codecraft_ask_user",
            question(),
        )
        .unwrap()["output"]["hookSpecificOutput"]["updatedInput"]
        .clone();
    let c = json!({"kind":"consume","connection":"a","tool":ASK_TOOL,"ticket":args["bridgeTicket"],"arguments":args});
    let first = f.store.apply(&c).unwrap();
    assert_eq!(
        f.store.apply(&c).unwrap()["operationId"],
        first["operationId"]
    );
    let mut wrong = c.clone();
    wrong["connection"] = json!("b");
    assert_eq!(
        f.store.apply(&wrong).unwrap_err().error.code,
        ErrorCode::TicketInvalid
    );
    wrong = c.clone();
    wrong["arguments"]["questions"][0]["prompt"] = json!("Changed");
    assert_eq!(
        f.store.apply(&wrong).unwrap_err().error.code,
        ErrorCode::ArgumentMismatch
    );
    let second = f.consume("two", "call", ASK_TOOL, question());
    assert_ne!(second, first["operationId"].as_str().unwrap());
    assert_eq!(f.store.requests.len(), 2);
    assert!(!f
        .store
        .snapshot()
        .to_string()
        .contains(args["bridgeTicket"].as_str().unwrap()));
}
#[test]
fn external_bridge_ticket_parameter_is_part_of_native_fingerprint() {
    let mut f = Fixture::new();
    f.tool("one", "call", "external", json!({"bridgeTicket":"first"}))
        .unwrap();
    assert_eq!(
        f.tool("one", "call", "external", json!({"bridgeTicket":"changed"}))
            .unwrap_err()
            .error
            .code,
        ErrorCode::IdempotencyConflict
    );
}
#[test]
fn approval_activates_only_after_flush_ack_and_blocks_after_file_change() {
    let mut f = Fixture::new();
    let task = f.task();
    assert!(f
        .tool(
            "one",
            "early",
            "RunCommand",
            json!({"command":"echo early"})
        )
        .is_err());
    let op = f.plan(&task);
    f.approve(&op);
    assert!(!f.store.tasks[&task].approved);
    assert_eq!(
        f.tool("one", "next", "RunCommand", json!({"command":"echo next"}))
            .unwrap()["waitForPlan"],
        true
    );
    let prepared = f.prepare(&op).unwrap();
    assert!(!f.store.tasks[&task].approved);
    f.ack(&op, &prepared["lease"]).unwrap();
    assert!(f.store.tasks[&task].approved);
    f.tool("one", "after", "RunCommand", json!({"command":"echo next"}))
        .unwrap();
    fs::write(&f.store.tasks[&task].document_path, "changed").unwrap();
    f.store.tick();
    assert!(!f.store.tasks[&task].approved);
    assert!(f
        .tool(
            "one",
            "changed",
            "RunCommand",
            json!({"command":"echo next"})
        )
        .is_err());
}
#[test]
fn cancelled_or_changed_plan_cannot_activate_from_late_ack() {
    for cancel in [true, false] {
        let mut f = Fixture::new();
        let task = f.task();
        let op = f.plan(&task);
        f.approve(&op);
        let prepared = f.prepare(&op).unwrap();
        if cancel {
            f.decision(&op, json!({"kind":"cancel","reason":null}))
                .unwrap();
        } else {
            fs::write(&f.store.tasks[&task].document_path, "changed").unwrap();
        }
        assert!(f.ack(&op, &prepared["lease"]).is_err());
        assert!(!f.store.tasks[&task].approved);
    }
}
#[test]
fn pause_and_end_invalidate_delivery_and_cannot_be_reopened_by_revoke() {
    let mut f = Fixture::new();
    let task = f.task();
    let op = f.plan(&task);
    f.approve(&op);
    let prepared = f.prepare(&op).unwrap();
    for action in ["pause", "end"] {
        f.store.apply(&json!({"kind":"task_action","body":{"schemaVersion":1,"controlId":id(),"taskId":task,"expectedVersion":f.store.tasks[&task].version,"action":action}})).unwrap();
        assert!(f.ack(&op, &prepared["lease"]).is_err());
        assert!(f.store.apply(&json!({"kind":"task_action","body":{"schemaVersion":1,"controlId":id(),"taskId":task,"expectedVersion":f.store.tasks[&task].version,"action":"revoke_plan"}})).is_err());
    }
    f.event("one", "SessionStart", json!({})).unwrap();
    assert_eq!(f.store.tasks[&task].state, "ended");
}

#[test]
fn completion_evidence_matches_the_full_call_and_does_not_overwrite_a_resolution() {
    let mut f = Fixture::new();
    let args = json!({"command":"echo synthetic","bridgeTicket":"external-business-field"});
    let result = f
        .tool("one", "external", "RunCommand", args.clone())
        .unwrap();
    let op = result["requestId"].as_str().unwrap();
    if f.store.requests[op].state == "pending" {
        f.decision(
            op,
            json!({"kind":"permission","decision":"allow","message":null}),
        )
        .unwrap();
    }
    let delivery = f.prepare(op).unwrap();
    f.ack(op, &delivery["lease"]).unwrap();
    let mut event = json!({"tool_use_id":"external","tool_name":"RunCommand","llm_tool_name":"RunCommand","tool_input":args,"tool_response":"ok"});
    event["tool_input"]["bridgeTicket"] = json!("modified");
    f.event("one", "PostToolUse", event.clone()).unwrap();
    assert!(f.store.grants[op].resolution.is_none());
    event["tool_input"] = args;
    event["tool_name"] = json!("Write");
    f.event("one", "PostToolUse", event.clone()).unwrap();
    assert!(f.store.grants[op].resolution.is_none());
    event["tool_name"] = json!("RunCommand");
    f.event("one", "PostToolUse", event.clone()).unwrap();
    assert_eq!(
        f.store.grants[op].resolution.as_deref(),
        Some("observed_completed")
    );
    let version = f.store.grants[op].version;
    f.event("one", "PostToolUse", event).unwrap();
    assert_eq!(f.store.grants[op].version, version);
}
#[test]
fn schemas_require_null_fields_reject_null_ticket_and_match_runtime() {
    let mut q = question();
    assert!(validate_business(ASK_TOOL, &q).is_ok());
    q["bridgeTicket"] = Value::Null;
    assert!(validate_business(ASK_TOOL, &q).is_err());
    let mut q = question();
    q["questions"][0]["options"][0]
        .as_object_mut()
        .unwrap()
        .remove("description");
    assert!(validate_business(ASK_TOOL, &q).is_err());
    let mut q = question();
    q["sessionId"] = json!("forged");
    assert!(validate_business(ASK_TOOL, &q).is_err());
    let schema = schemas();
    assert_eq!(
        schema,
        serde_json::from_str::<Value>(include_str!("../../protocol/trae/codecraft-v1/schema.json"))
            .unwrap()
    );
    assert_eq!(
        schema["QuestionInput"]["properties"]["schemaVersion"]["const"],
        1
    );
    assert!(schema["ToolResult"]["$defs"]["Answer"].is_object());
}
#[test]
fn text_and_skip_constraints_count_unicode_scalars() {
    let input:QuestionInput=serde_json::from_value(json!({"schemaVersion":1,"questions":[{"questionId":"text","prompt":"Text","kind":"text","options":[],"required":true,"allowText":true,"minSelections":0,"maxSelections":0},{"questionId":"skip","prompt":"Optional","kind":"text","options":[],"required":false,"allowText":true,"minSelections":0,"maxSelections":0}]})).unwrap();
    input.validate().unwrap();
    let answers = vec![
        Answer {
            question_id: "text".into(),
            status: AnswerStatus::Answered,
            selected_option_ids: vec![],
            text: Some("🦀".repeat(4096)),
        },
        Answer {
            question_id: "skip".into(),
            status: AnswerStatus::Skipped,
            selected_option_ids: vec![],
            text: None,
        },
    ];
    input.validate_answers(&answers).unwrap();
    let mut bad = answers;
    bad[0].text = Some(" ".into());
    assert!(input.validate_answers(&bad).is_err());
    bad[0].status = AnswerStatus::Skipped;
    assert!(input.validate_answers(&bad).is_err());
}
#[test]
fn unknown_versions_do_not_enable_any_execution_capability() {
    for version in [None, Some("99.0")] {
        let c = Capabilities::bundled(version);
        assert!(!c.tool_approval && !c.mcp_questions && !c.mcp_plan_review);
        assert!(c.hook_wait().is_none());
    }
    let mut f = Fixture::new();
    f.store.capabilities = Capabilities::default();
    assert!(f
        .tool("one", "q", "mcp__codecraft__codecraft_ask_user", question())
        .is_err());
    assert!(f.store.requests.is_empty());
}

#[test]
fn bundled_tool_approval_does_not_depend_on_retired_mcp_verification() {
    let caps = Capabilities::bundled(Some("3.3.102"));
    assert!(caps.tool_approval);
    assert_eq!(caps.hook_wait(), Some(120));
    assert!(!caps.mcp_questions && !caps.mcp_plan_review);
    assert!(!caps.tool_input_mappings_verified);
    assert!(caps.verified_version.is_none());
    assert!(caps.verified_hook_timeout_seconds.is_none());
    let mut f = Fixture::new();
    f.store.capabilities = caps;
    for (i, decision) in ["allow", "deny", "ask"].iter().enumerate() {
        let args = json!({"file_path":f.workspace.join(format!("approval-{i}.txt")),"content":"APPROVAL_TEST"});
        let registered = f
            .tool(
                "approval-session",
                &format!("call-{i}"),
                "Write",
                args.clone(),
            )
            .unwrap();
        let op = registered["requestId"]
            .as_str()
            .expect("must create a desktop approval request");
        assert!(registered.get("output").is_none());
        let r = &f.store.requests[op];
        assert_eq!(r.kind, "permission");
        assert_eq!(r.channel, "hook");
        assert_eq!(r.arguments, args);
        assert_eq!(r.state, "pending");
        assert!(f.prepare(op).is_err());
        f.decision(
            op,
            json!({"kind":"permission","decision":decision,"message":null}),
        )
        .unwrap();
        let prepared = f.prepare(op).unwrap();
        assert_eq!(
            prepared["output"]["hookSpecificOutput"]["permissionDecision"],
            *decision
        );
        f.ack(op, &prepared["lease"]).unwrap();
        assert_eq!(f.store.requests[op].state, "delivered");
    }
}

#[test]
fn native_questions_are_observed_without_blocking_or_creating_an_approval() {
    let mut f = Fixture::new();
    f.store.capabilities = Capabilities::default();
    let args =
        json!({"questions":[{"question":"Which mode?","options":[{"label":"A"},{"label":"B"}]}]});
    assert_eq!(
        f.tool("one", "question-native", "AskUserQuestion", args.clone())
            .unwrap(),
        json!({"output":{}})
    );
    assert!(f.store.requests.is_empty());
    let first = f
        .store
        .sessions
        .values()
        .next()
        .unwrap()
        .native_interactions[0]
        .clone();
    f.event("one", "Notification", json!({"notification_type":"ask_user_question","tool_use_id":"question-native","message":"Waiting"})).unwrap();
    let current = &f
        .store
        .sessions
        .values()
        .next()
        .unwrap()
        .native_interactions;
    assert_eq!(current.len(), 1);
    assert_eq!(current[0]["id"], first["id"]);
    assert_eq!(current[0]["capturedAt"], first["capturedAt"]);
    assert_eq!(current[0]["arguments"], args);
    f.event("one", "PostToolUse", json!({"tool_use_id":"unrelated","tool_name":"Read","llm_tool_name":"Read","tool_input":{}})).unwrap();
    assert_eq!(
        f.store
            .sessions
            .values()
            .next()
            .unwrap()
            .native_interactions
            .len(),
        1
    );
    f.event("one", "PostToolUse", json!({"tool_use_id":"question-native","tool_name":"AskUserQuestion","llm_tool_name":"AskUserQuestion","tool_input":args})).unwrap();
    assert!(f
        .store
        .sessions
        .values()
        .next()
        .unwrap()
        .native_interactions
        .is_empty());
}

#[test]
fn native_plan_closes_when_trae_continues_with_a_native_question() {
    for event in ["PreToolUse", "Notification"] {
        let mut f = Fixture::new();
        f.native_plan_notice("one");
        let other_plan = f.native_plan_notice("two");
        let args = json!({"questions":[{"question":"Continue?","options":[{"label":"Yes"}]}]});
        f.event(
            "one",
            event,
            json!({
                "tool_use_id":"next-question", "tool_name":"AskUserQuestion",
                "llm_tool_name":"AskUserQuestion", "tool_input":args,
                "notification_type":"ask_user_question", "message":"Waiting for an answer"
            }),
        )
        .unwrap();
        let session = f
            .store
            .sessions
            .values()
            .find(|s| s.session_id == "one")
            .unwrap();
        assert_eq!(
            session.native_interactions.len(),
            1,
            "stale plan after {event}"
        );
        assert_eq!(session.native_interactions[0]["kind"], "question");
        assert_eq!(session.native_interactions[0]["toolUseId"], "next-question");
        assert_eq!(session.status, "waitingForInput");
        assert_eq!(
            f.store
                .sessions
                .values()
                .find(|s| s.session_id == "two")
                .unwrap()
                .native_interactions,
            vec![other_plan]
        );
        assert!(f.store.requests.is_empty());
        f.event(
            "one",
            "PostToolUse",
            json!({
                "tool_use_id":"next-question", "tool_name":"AskUserQuestion",
                "llm_tool_name":"AskUserQuestion", "tool_input":args
            }),
        )
        .unwrap();
        assert!(f
            .store
            .sessions
            .values()
            .find(|s| s.session_id == "one")
            .unwrap()
            .native_interactions
            .is_empty());
    }
}

#[test]
fn native_plan_reads_only_an_explicit_document_inside_trae_directories() {
    let mut f = Fixture::new();
    let folder = f.workspace.join(".trae/documents");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("plan.md"), "# Native plan").unwrap();
    fs::write(f.workspace.join("private.md"), "must not be read").unwrap();
    f.event("one", "Notification", json!({"notification_type":"document_review","message":"Review plan","document_path":".trae/documents/plan.md"})).unwrap();
    assert_eq!(
        f.store
            .sessions
            .values()
            .next()
            .unwrap()
            .native_interactions[0]["plan"],
        "# Native plan"
    );
    f.event("one", "UserPromptSubmit", json!({"prompt":"continue"}))
        .unwrap();
    for path in [
        "private.md",
        ".trae/documents/../../private.md",
        "missing.md",
    ] {
        f.event(
            "one",
            "Notification",
            json!({"notification_type":"document_review","document_path":path}),
        )
        .unwrap();
        assert!(f
            .store
            .sessions
            .values()
            .next()
            .unwrap()
            .native_interactions[0]["plan"]
            .is_null());
    }
    f.event("one", "Stop", json!({})).unwrap();
    assert!(f
        .store
        .sessions
        .values()
        .next()
        .unwrap()
        .native_interactions
        .is_empty());
}

#[test]
fn native_plan_associates_the_captured_trae_write_and_review_sequence() {
    let mut f = Fixture::new();
    f.store.capabilities = Capabilities::bundled(Some("3.3.102"));
    let mut fixture: Value = serde_json::from_str(include_str!(
        "../../protocol/trae/3.3.102/fixtures/plan-document-association.json"
    ))
    .unwrap();
    fn relocate(value: &mut Value, workspace: &std::path::Path) {
        match value {
            Value::String(s) => {
                *s = s
                    .replace(
                        "D:\\fixture-workspace\\.trae\\documents\\acceptance-plan-approval.md",
                        &workspace
                            .join(".trae/documents/acceptance-plan-approval.md")
                            .to_string_lossy(),
                    )
                    .replace("D:\\fixture-workspace", &workspace.to_string_lossy());
            }
            Value::Array(items) => items.iter_mut().for_each(|v| relocate(v, workspace)),
            Value::Object(items) => items.values_mut().for_each(|v| relocate(v, workspace)),
            _ => (),
        }
    }
    relocate(&mut fixture, &f.workspace);
    for input in fixture["events"].as_array().unwrap() {
        if input["hook_event_name"] == "PostToolUse" {
            let path = PathBuf::from(input["tool_input"]["file_path"].as_str().unwrap());
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, input["tool_input"]["content"].as_str().unwrap()).unwrap();
        }
        f.event(
            "fixture-plan-session",
            input["hook_event_name"].as_str().unwrap(),
            input.clone(),
        )
        .unwrap();
    }
    let session = f.store.sessions.values().next().unwrap();
    let review = &session.native_interactions[0];
    assert_eq!(review["planSource"], "session_file");
    assert_eq!(
        review["plan"],
        fixture["events"][0]["tool_input"]["content"]
    );
    assert_eq!(review["documentToolUseId"], "fixture-write-plan");
    assert_eq!(review["toolUseId"], "fixture-review-plan");
    assert!(review["documentPath"]
        .as_str()
        .unwrap()
        .ends_with("acceptance-plan-approval.md"));
    assert_eq!(review["readOnly"], true);
    // A native document review remains observation-only.
    assert_eq!(f.store.requests.len(), 1);
}

#[test]
fn native_plan_candidates_are_isolated_by_session_turn_and_restart() {
    for boundary in [
        "other-session",
        "UserPromptSubmit",
        "SessionStart",
        "Stop",
        "idle_prompt",
        "restart",
    ] {
        let mut f = Fixture::new();
        f.native_file_change("one", "write-plan", ".trae/documents/plan.md", "# Plan A");
        match boundary {
            "other-session" => (),
            "restart" => {
                f.store = serde_json::from_value(serde_json::to_value(&f.store).unwrap()).unwrap()
            }
            "idle_prompt" => {
                f.event(
                    "one",
                    "Notification",
                    json!({"notification_type":"idle_prompt"}),
                )
                .unwrap();
            }
            event => {
                f.event("one", event, json!({"prompt":"next"})).unwrap();
            }
        }
        let review = f.native_plan_notice(if boundary == "other-session" {
            "two"
        } else {
            "one"
        });
        assert!(review["plan"].is_null(), "candidate survived {boundary}");
        assert!(review["documentPath"].is_null());
    }
}

#[test]
fn native_plan_uses_latest_confirmed_edit_but_never_guesses_between_documents() {
    let mut f = Fixture::new();
    f.native_file_change("one", "first", ".trae/documents/plan.md", "# First");
    let file = f.workspace.join(".trae/documents/plan.md");
    let args = json!({"file_path":file,"old_string":"First","new_string":"Edited"});
    f.tool("one", "edit", "Edit", args.clone()).unwrap();
    fs::write(&file, "# Edited").unwrap();
    f.event("one", "PostToolUse", json!({"tool_use_id":"edit","tool_name":"Edit","llm_tool_name":"Edit","tool_input":args,"tool_response":{"changes":[{"file_path":file,"file_action":"modified","new_content":"# Edited"}]}})).unwrap();
    let first = f.native_plan_notice("one");
    assert_eq!(first["plan"], "# Edited");
    assert_eq!(first["documentToolUseId"], "edit");
    assert_eq!(f.native_plan_notice("one")["id"], first["id"]);
    f.native_file_change("one", "second", ".trae/specs/another.md", "# Another");
    let ambiguous = f.native_plan_notice("one");
    assert_eq!(ambiguous["planSource"], "ambiguous");
    assert!(ambiguous["plan"].is_null());
    f.event("one", "Notification", json!({"notification_type":"document_review","tool_use_id":"review-call","document_path":".trae/specs/another.md"})).unwrap();
    assert_eq!(
        f.store
            .sessions
            .values()
            .next()
            .unwrap()
            .native_interactions[0]["plan"],
        "# Another"
    );
}

#[test]
fn native_plan_excludes_unconfirmed_mismatched_and_late_results() {
    for case in [
        "empty",
        "null",
        "error",
        "changed-args",
        "changed-result-path",
        "changed-body",
        "late",
        "no-pre",
    ] {
        let mut f = Fixture::new();
        let file = f.workspace.join(".trae/documents/plan.md");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "# Existing").unwrap();
        let mut args = json!({"file_path":file,"content":"# Existing"});
        if case != "no-pre" {
            f.tool("one", "write", "Write", args.clone()).unwrap();
        }
        let mut response = json!({"changes":[{"file_path":file,"file_action":"added","new_content":"# Existing"}]});
        match case {
            "empty" => response = json!({}),
            "null" => response = Value::Null,
            "error" => response["is_error"] = json!(true),
            "changed-args" => args["content"] = json!("# Other"),
            "changed-result-path" => {
                response["changes"][0]["file_path"] = json!(f.workspace.join("other.md"))
            }
            "changed-body" => response["changes"][0]["new_content"] = json!("# Other"),
            "late" => {
                f.event("one", "UserPromptSubmit", json!({"prompt":"next"}))
                    .unwrap();
            }
            _ => (),
        }
        f.event("one", "PostToolUse", json!({"tool_use_id":"write","tool_name":"Write","llm_tool_name":"Write","tool_input":args,"tool_response":response})).unwrap();
        assert!(
            f.native_plan_notice("one")["plan"].is_null(),
            "unexpected association: {case}"
        );
    }
}

#[test]
fn native_plan_rechecks_content_scope_and_size_before_display() {
    for case in [
        "outside",
        "too-large",
        "overwritten",
        "missing",
        "explicit-missing",
    ] {
        let mut f = Fixture::new();
        let name = if case == "outside" {
            "private.md"
        } else {
            ".trae/documents/plan.md"
        };
        let body = if case == "too-large" {
            "x".repeat(PLAN_LIMIT + 1)
        } else {
            "# Plan".into()
        };
        f.native_file_change("one", "write", name, &body);
        if case == "overwritten" {
            fs::write(f.workspace.join(name), "# Another session's plan").unwrap();
        }
        if case == "missing" {
            fs::remove_file(f.workspace.join(name)).unwrap();
        }
        let review = if case == "explicit-missing" {
            f.event("one", "Notification", json!({"notification_type":"document_review","document_path":".trae/documents/missing.md"})).unwrap();
            f.store
                .sessions
                .values()
                .next()
                .unwrap()
                .native_interactions[0]
                .clone()
        } else {
            f.native_plan_notice("one")
        };
        assert!(review["plan"].is_null(), "unexpected association: {case}");
    }
}

#[test]
fn product_dispatch_rejects_retired_question_plan_and_task_controls() {
    for route in ["question", "plan", "tasks", "tasks/action"] {
        assert_eq!(
            crate::dispatch(route, json!({})).unwrap_err().error.code,
            ErrorCode::Forbidden
        );
    }
    assert!(crate::run_mcp().is_err());
}

#[test]
fn workspace_less_chat_records_readonly_activity_without_a_filesystem_scope() {
    let mut f = Fixture::new();
    let chat = json!({"agent_id":"chat","agent_type":"chat","cwd":".","workspace_roots":[]});
    let mut event = |kind, extra: Value| {
        let mut input = chat.clone();
        input
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        f.event("chat", kind, input)
    };
    assert_eq!(
        event("SessionStart", json!({})).unwrap()["output"],
        json!({})
    );
    // Task-like text is still ordinary text in a chat without a project.
    event(
        "UserPromptSubmit",
        json!({"prompt":"[[CODECRAFT_TASK:plain-text]]"}),
    )
    .unwrap();
    event(
        "Notification",
        json!({"notification_type":"ask_user_question","message":"Choose an option"}),
    )
    .unwrap();
    event("Notification", json!({"notification_type":"document_review","document_path":".trae/documents/plan.md","message":"Review in Trae"})).unwrap();
    let session = f.store.sessions.values().next().unwrap();
    assert!(session.cwd.is_empty());
    assert!(session.workspace_roots.is_empty());
    assert_eq!(session.title, "[[CODECRAFT_TASK:plain-text]]");
    assert_eq!(session.native_interactions.len(), 2);
    assert!(session
        .native_interactions
        .iter()
        .all(|n| n["readOnly"] == true && n["plan"].is_null()));
    assert!(f.store.requests.is_empty());
    assert!(f.store.tasks.is_empty());
    let mut idle = chat.clone();
    idle["notification_type"] = json!("idle_prompt");
    f.event("chat", "Notification", idle).unwrap();
    let session = f.store.sessions.values().next().unwrap();
    assert!(session.native_interactions.is_empty());
    assert_eq!(session.status, "idle");
    f.event("chat", "Stop", chat).unwrap();
    assert_eq!(f.store.sessions.values().next().unwrap().status, "stopped");
}

#[test]
fn loss_of_workspace_cancels_earlier_tool_approvals() {
    let mut f = Fixture::new();
    let reply = f
        .tool(
            "one",
            "pending-command",
            "RunCommand",
            json!({"command":"echo test"}),
        )
        .unwrap();
    let op = reply["requestId"].as_str().unwrap();
    // Both manual and automatic policy decisions must become invalid before delivery.
    assert!(matches!(
        f.store.requests[op].state.as_str(),
        "pending" | "user_decided"
    ));
    let error = f
        .event(
            "one",
            "UserPromptSubmit",
            json!({"agent_type":"chat","cwd":".","workspace_roots":[],"prompt":"continue"}),
        )
        .unwrap_err();
    assert_eq!(error.error.code, ErrorCode::TaskChanged);
    assert_eq!(f.store.requests[op].state, "cancelled");
    assert!(f
        .decision(
            op,
            json!({"kind":"permission","decision":"allow","message":null})
        )
        .is_err());
}

#[test]
fn solo_without_workspace_creates_a_session_and_clears_waiting_on_idle() {
    let mut f = Fixture::new();
    let mut input = json!({"agent_type":"solo_agent","agent_id":"solo_agent","cwd":".","workspace_roots":[],"prompt":"Solo session regression"});
    assert_eq!(
        f.event("solo", "UserPromptSubmit", input.clone()).unwrap()["output"],
        json!({})
    );
    let session = f.store.sessions.values().next().unwrap();
    assert_eq!(session.title, "Solo session regression");
    assert!(session.workspace_roots.is_empty());
    assert_eq!(session.status, "working");
    input["notification_type"] = json!("idle_prompt");
    f.event("solo", "Notification", input).unwrap();
    assert_eq!(f.store.sessions.values().next().unwrap().status, "idle");
}

#[test]
fn resume_to_another_session_keeps_the_old_session_blocked() {
    let mut f = Fixture::new();
    let task = f.task();
    let action = |f: &Fixture, kind: &str| json!({"kind":"task_action","body":{"schemaVersion":1,"controlId":id(),"taskId":task,"expectedVersion":f.store.tasks[&task].version,"action":kind}});
    f.store.apply(&action(&f, "pause")).unwrap();
    let resume = f.store.apply(&action(&f, "resume")).unwrap();
    f.event(
        "two",
        "UserPromptSubmit",
        json!({"prompt":resume["launchPrompt"]}),
    )
    .unwrap();
    assert!(f
        .tool(
            "one",
            "old-read",
            "Read",
            json!({"file_path":f.workspace.join("file")})
        )
        .is_err());
    assert!(f
        .tool(
            "one",
            "old-question",
            "mcp__codecraft__codecraft_ask_user",
            question()
        )
        .is_err());
    assert!(f
        .tool(
            "two",
            "new-read",
            "Read",
            json!({"file_path":f.workspace.join("file")})
        )
        .is_ok());
    let turn = f.store.tasks[&task].turn_epoch;
    for (event, extra) in [
        ("Stop", json!({"last_assistant_message":"old session done"})),
        ("SessionStart", json!({})),
        (
            "UserPromptSubmit",
            json!({"prompt":"old session continues"}),
        ),
    ] {
        f.event("one", event, extra).unwrap();
        assert_eq!(f.store.tasks[&task].turn_epoch, turn);
        assert_eq!(f.store.tasks[&task].state, "awaiting_plan");
    }
}
#[test]
fn stop_and_revocation_cancel_old_calls_before_new_approvals() {
    let mut f = Fixture::new();
    let task = f.task();
    let plan = f.plan(&task);
    f.approve(&plan);
    let delivery = f.prepare(&plan).unwrap();
    f.ack(&plan, &delivery["lease"]).unwrap();
    let pending = f
        .tool(
            "one",
            "command",
            "RunCommand",
            json!({"command":"echo test"}),
        )
        .unwrap();
    let op = pending["requestId"].as_str().unwrap();
    fs::write(&f.store.tasks[&task].document_path, "modified").unwrap();
    f.store.tick();
    assert_eq!(f.store.requests[op].state, "cancelled");
    f.event("one", "Stop", json!({"last_assistant_message":"done"}))
        .unwrap();
    assert!(f
        .tool(
            "one",
            "read-after-stop",
            "Read",
            json!({"file_path":f.workspace.join("file")})
        )
        .is_err());
    f.event("one", "UserPromptSubmit", json!({"prompt":"continue"}))
        .unwrap();
    assert!(f
        .tool(
            "one",
            "read-new-turn",
            "Read",
            json!({"file_path":f.workspace.join("file")})
        )
        .is_ok());
}
