use super::*;
use crate::workflow::Command as W;
use std::{fs, path::PathBuf};

struct Fixture {
    model: Model,
    root: PathBuf,
    workflow: String,
    project: String,
}
impl Fixture {
    fn new() -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("runs")
            .join(format!("independent-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let mut model = Model::new("test-install").unwrap();
        model
            .execute(
                W::SetProjectPath {
                    path: root.clone(),
                    enabled: true,
                },
                0,
            )
            .unwrap();
        let project = projects::resolve(&root).unwrap().identity.key();
        let result = model
            .execute(
                W::Independent(Command::Open {
                    connection: "connection-a".into(),
                    project_id: project.clone(),
                    title: "Independent task".into(),
                }),
                0,
            )
            .unwrap();
        let Response::Independent { value } = result else {
            panic!()
        };
        Self {
            model,
            root,
            workflow: value["workflowId"].as_str().unwrap().into(),
            project,
        }
    }
    fn execute(&mut self, c: Command, now: u64) -> Result<Value> {
        match self.model.execute(W::Independent(c), now)? {
            Response::Independent { value } => Ok(value),
            _ => panic!(),
        }
    }
    fn ask(&mut self, client: &str, rpc: i64) -> Value {
        self.execute(
            Command::Mcp {
                connection: "connection-a".into(),
                workflow_id: self.workflow.clone(),
                rpc_id: RpcId::Number(rpc),
                payload: question(client),
                wall_ms: 1000,
            },
            1,
        )
        .unwrap()
    }
    fn poll(&mut self, id: &str, now: u64) -> Result<Value> {
        self.execute(
            Command::Poll {
                connection: "connection-a".into(),
                workflow_id: Some(self.workflow.clone()),
                request_id: id.into(),
            },
            now,
        )
    }
    fn answer(&mut self, id: &str, now: u64) -> Result<Value> {
        self.execute(
            Command::Decide {
                submission: self.model.request(id).unwrap().submission(),
                decision: answer(),
                wall_ms: 2000,
            },
            now,
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn question(client: &str) -> ToolRequest {
    ToolRequest::parse(schema::ASK_USER,&serde_json::to_vec(&json!({"schemaVersion":1,"clientRequestId":client,"questions":[{"questionId":"q","prompt":"Choose","options":[{"optionId":"a","label":"Same"},{"optionId":"b","label":"Same"}],"multiSelect":false,"allowFreeText":true,"required":true,"allowSkip":true,"maxTextLength":20}]})).unwrap()).unwrap()
}
fn answer() -> Decision {
    Decision::Answered {
        answers: vec![schema::Answer {
            question_id: "q".into(),
            selected_option_ids: vec!["b".into()],
            text: Some("说明".into()),
            skipped: false,
        }],
    }
}

#[test]
fn independent_result_has_one_connection_owner_and_delivery_is_not_native_ack() {
    let mut f = Fixture::new();
    let value = f.ask("ask", 0);
    let id = value["requestId"].as_str().unwrap();
    assert_eq!(f.poll(id, 2).unwrap()["status"], "pending");
    assert_eq!(
        f.execute(
            Command::Poll {
                connection: "connection-b".into(),
                workflow_id: Some(f.workflow.clone()),
                request_id: id.into()
            },
            3
        )
        .unwrap_err(),
        "NOT_RESULT_OWNER"
    );
    f.answer(id, 4).unwrap();
    let result = f.poll(id, 5).unwrap();
    assert_eq!(
        result["decision"]["answers"][0]["selectedOptionIds"],
        json!(["b"])
    );
    assert_eq!(f.poll(id, 6).unwrap()["status"], "deliveryPending");
    f.execute(
        Command::Report {
            connection: "connection-a".into(),
            request_id: id.into(),
            lease_id: result["_deliveryLease"].as_str().unwrap().into(),
            written: true,
        },
        7,
    )
    .unwrap();
    assert_eq!(f.poll(id, 8).unwrap()["status"], "alreadyDelivered");
    let snapshot = f.model.interaction_snapshot();
    assert!(!snapshot.to_string().contains("_deliveryLease"));
    assert_eq!(snapshot["requests"][0]["origin"], "independentMcp");
    assert_eq!(snapshot["requests"][0]["nativeEvidence"], "unconfirmed");
}
#[test]
fn retry_does_not_extend_deadline_and_another_workflow_cannot_take_it() {
    let mut f = Fixture::new();
    let v = f.ask("same", 1);
    let id = v["requestId"].as_str().unwrap();
    let deadline = f.model.request(id).unwrap().deadline_ms;
    let duplicate = f
        .execute(
            Command::Mcp {
                connection: "connection-a".into(),
                workflow_id: f.workflow.clone(),
                rpc_id: RpcId::Number(2),
                payload: question("same"),
                wall_ms: 9999,
            },
            1000,
        )
        .unwrap();
    assert_eq!(duplicate["kind"], "duplicate");
    assert_eq!(f.model.request(id).unwrap().deadline_ms, deadline);
    assert_eq!(
        f.execute(
            Command::Poll {
                connection: "connection-a".into(),
                workflow_id: Some("another-workflow".into()),
                request_id: id.into()
            },
            1001
        )
        .unwrap_err(),
        "NOT_RESULT_OWNER"
    );
    assert_eq!(f.poll(id, deadline).unwrap()["status"], "terminated");
    assert_eq!(
        f.answer(id, deadline + 1).unwrap_err(),
        "REQUEST_NOT_PENDING"
    );
}
#[test]
fn opposite_submissions_conflict_and_disconnect_revokes_pending_work() {
    let mut f = Fixture::new();
    let v = f.ask("answer", 1);
    let id = v["requestId"].as_str().unwrap();
    let sub = f.model.request(id).unwrap().submission();
    f.answer(id, 2).unwrap();
    assert_eq!(
        f.execute(
            Command::Decide {
                submission: sub.clone(),
                decision: Decision::QuestionCancelled,
                wall_ms: 2001
            },
            3
        )
        .unwrap_err(),
        "DECISION_CONFLICT"
    );
    assert_eq!(
        f.execute(
            Command::Decide {
                submission: sub,
                decision: answer(),
                wall_ms: 2002
            },
            4
        )
        .unwrap()["alreadyRecorded"],
        true
    );
    f.execute(
        Command::Disconnect {
            connection: "connection-a".into(),
        },
        5,
    )
    .unwrap();
    assert_eq!(f.model.request(id).unwrap().phase, Phase::Disconnected);
    assert!(f.poll(id, 6).is_err());
}
#[test]
fn project_disable_and_directory_replacement_reject_existing_forms() {
    let mut f = Fixture::new();
    let v = f.ask("disable", 1);
    let id = v["requestId"].as_str().unwrap();
    f.execute(
        Command::DisableProject {
            project_id: f.project.clone(),
        },
        2,
    )
    .unwrap();
    assert!(f.answer(id, 3).is_err());
    let mut f = Fixture::new();
    let v = f.ask("replace", 1);
    let id = v["requestId"].as_str().unwrap();
    let old = f.root.with_extension("old");
    fs::rename(&f.root, &old).unwrap();
    fs::create_dir(&f.root).unwrap();
    assert_eq!(f.answer(id, 2).unwrap_err(), "PROJECT_SCOPE_CHANGED");
    fs::remove_dir(&f.root).unwrap();
    fs::rename(old, &f.root).unwrap();
}
#[test]
fn restart_and_unknown_delivery_never_replay_a_human_answer() {
    let mut f = Fixture::new();
    let v = f.ask("restart", 1);
    let id = v["requestId"].as_str().unwrap();
    f.answer(id, 2).unwrap();
    f.poll(id, 3).unwrap();
    let saved: Model = serde_json::from_value(serde_json::to_value(&f.model).unwrap()).unwrap();
    let recovered = Model::recover(saved, "test-install").unwrap();
    assert_eq!(
        recovered.request(id).unwrap().transport,
        Transport::OutcomeUnknown
    );
    assert_eq!(
        recovered.interaction_snapshot()["requests"][0]["actionable"],
        false
    );
    assert_eq!(
        recovered.interaction_snapshot()["sessions"][0]["updatedAt"],
        2000
    );
}
#[test]
fn hook_permissions_skip_mcp_pre_event_and_expire_without_fake_execution() {
    let mut f = Fixture::new();
    let input = json!({"conversation_id":"native-c","tool_name":"MCP:third_party","tool_input":{}});
    assert_eq!(
        f.execute(
            Command::Hook {
                connection: "hook".into(),
                event_id: "event".into(),
                event: "preToolUse".into(),
                input,
                automatic: false,
                wall_ms: 1000
            },
            1
        )
        .unwrap()["observed"],
        true
    );
    let input = json!({"conversation_id":"native-c","tool_name":"Shell","tool_input":{"command":"echo test"}});
    let v = f
        .execute(
            Command::Hook {
                connection: "hook".into(),
                event_id: "event-2".into(),
                event: "preToolUse".into(),
                input,
                automatic: false,
                wall_ms: 1000,
            },
            2,
        )
        .unwrap();
    let id = v["requestId"].as_str().unwrap();
    let r = f.model.request(id).unwrap();
    assert_eq!(r.deadline_ms, 45002);
    f.model.execute(W::Tick, 45002).unwrap();
    assert_eq!(f.model.request(id).unwrap().phase, Phase::Expired);
    assert_eq!(
        permission_response("preToolUse", &json!({"kind":"permission","action":"deny"})).unwrap()
            ["permission"],
        "deny"
    );
}
#[test]
fn public_mcp_schema_has_no_bridge_ticket_and_cannot_call_hook_permission() {
    let schemas = crate::mcp::tools();
    assert_eq!(schemas.as_array().unwrap().len(), 7);
    assert!(!schemas.to_string().contains("bridgeTicket"));
    let mut f = Fixture::new();
    let input = json!({"schemaVersion":1,"clientRequestId":"permission","eventName":"preToolUse","toolName":"Read","toolInput":{}});
    let payload = ToolRequest::parse(
        schema::HOOK_PERMISSION,
        &serde_json::to_vec(&input).unwrap(),
    )
    .unwrap();
    assert_eq!(
        f.execute(
            Command::Mcp {
                connection: "connection-a".into(),
                workflow_id: f.workflow.clone(),
                rpc_id: RpcId::Number(0),
                payload,
                wall_ms: 1000
            },
            1
        )
        .unwrap_err(),
        "UNKNOWN_TOOL"
    );
}
#[test]
fn project_rules_preserve_foreign_edits_and_back_up_owned_removal() {
    let f = Fixture::new();
    let state = f.root.join("state");
    fs::create_dir(&state).unwrap();
    crate::project_rules::configure(&state, &f.root, true).unwrap();
    assert_eq!(crate::project_rules::status(&f.root)["state"], "installed");
    let rule = f.root.join(".cursor/rules/codecraft-workflow.mdc");
    let original = fs::read(&rule).unwrap();
    fs::write(&rule, b"User content").unwrap();
    assert_eq!(
        crate::project_rules::configure(&state, &f.root, true).unwrap_err(),
        "RULE_CONFLICT"
    );
    assert_eq!(fs::read(&rule).unwrap(), b"User content");
    fs::write(&rule, original).unwrap();
    crate::project_rules::configure(&state, &f.root, false).unwrap();
    assert!(!rule.exists());
    assert!(fs::read_dir(rule.parent().unwrap()).unwrap().any(|e| e
        .unwrap()
        .path()
        .extension()
        .is_some_and(|x| x == "bak")));
}

#[test]
fn ending_a_workflow_cancels_waits_but_preserves_delivered_and_uncertain_results() {
    let mut f = Fixture::new();
    let pending = f.ask("pending", 1)["requestId"]
        .as_str()
        .unwrap()
        .to_owned();
    let delivered = f.ask("delivered", 2)["requestId"]
        .as_str()
        .unwrap()
        .to_owned();
    let uncertain = f.ask("uncertain", 3)["requestId"]
        .as_str()
        .unwrap()
        .to_owned();
    f.answer(&delivered, 2).unwrap();
    let lease = f.poll(&delivered, 3).unwrap();
    f.execute(
        Command::Report {
            connection: "connection-a".into(),
            request_id: delivered.clone(),
            lease_id: lease["_deliveryLease"].as_str().unwrap().into(),
            written: true,
        },
        4,
    )
    .unwrap();
    f.answer(&uncertain, 5).unwrap();
    f.poll(&uncertain, 6).unwrap();
    assert_eq!(
        f.execute(
            Command::End {
                connection: "connection-b".into(),
                workflow_id: f.workflow.clone()
            },
            7
        )
        .unwrap_err(),
        "WORKFLOW_NOT_OWNED"
    );
    f.execute(
        Command::End {
            connection: "connection-a".into(),
            workflow_id: f.workflow.clone(),
        },
        8,
    )
    .unwrap();
    assert_eq!(f.model.request(&pending).unwrap().phase, Phase::Cancelled);
    assert_eq!(
        f.model.request(&delivered).unwrap().transport,
        Transport::Written
    );
    assert_eq!(f.model.request(&delivered).unwrap().phase, Phase::Decided);
    assert_eq!(
        f.model.request(&uncertain).unwrap().transport,
        Transport::OutcomeUnknown
    );
    assert!(f.model.interaction_snapshot()["requests"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["actionable"] == false));
    assert_eq!(f.poll(&delivered, 9).unwrap_err(), "WORKFLOW_DISCONNECTED");
}

#[test]
fn empty_finished_workflows_release_their_sessions_and_capacity() {
    let mut f = Fixture::new();
    for _ in 0..140 {
        f.execute(
            Command::End {
                connection: "connection-a".into(),
                workflow_id: f.workflow.clone(),
            },
            0,
        )
        .unwrap();
        let opened = f
            .execute(
                Command::Open {
                    connection: "connection-a".into(),
                    project_id: f.project.clone(),
                    title: "Next task".into(),
                },
                0,
            )
            .unwrap();
        f.workflow = opened["workflowId"].as_str().unwrap().into();
    }
    let saved = serde_json::to_value(&f.model).unwrap();
    assert_eq!(saved["sessions"].as_object().unwrap().len(), 1);
    assert_eq!(
        saved["interactions"]["workflows"]
            .as_object()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn history_retention_preserves_live_requests_and_active_workflow_idempotency() {
    let mut f = Fixture::new();
    let pending = f.ask("pending", 1)["requestId"]
        .as_str()
        .unwrap()
        .to_owned();
    let delivered = f.ask("answered", 2)["requestId"]
        .as_str()
        .unwrap()
        .to_owned();
    f.answer(&delivered, 2).unwrap();
    let lease = f.poll(&delivered, 3).unwrap();
    f.execute(
        Command::Report {
            connection: "connection-a".into(),
            request_id: delivered.clone(),
            lease_id: lease["_deliveryLease"].as_str().unwrap().into(),
            written: true,
        },
        4,
    )
    .unwrap();
    for index in 0..520 {
        f.execute(
            Command::Hook {
                connection: "short-hook".into(),
                event_id: format!("event-{index}"),
                event: "preToolUse".into(),
                input: json!({"conversation_id":"native","tool_name":"Read","tool_input":{}}),
                automatic: false,
                wall_ms: 3000 + index,
            },
            5,
        )
        .unwrap();
        f.execute(
            Command::Disconnect {
                connection: "short-hook".into(),
            },
            5,
        )
        .unwrap();
    }
    assert!(f.model.request_count() <= 512);
    assert_eq!(f.model.request(&pending).unwrap().phase, Phase::Pending);
    assert_eq!(
        f.model.request(&delivered).unwrap().transport,
        Transport::Written
    );
    let duplicate = f
        .execute(
            Command::Mcp {
                connection: "connection-a".into(),
                workflow_id: f.workflow.clone(),
                rpc_id: RpcId::Number(3),
                payload: question("answered"),
                wall_ms: 4000,
            },
            6,
        )
        .unwrap();
    assert_eq!(duplicate["requestId"], delivered);
    assert_eq!(duplicate["status"], "completed");
    assert!(duplicate.get("decision").is_none());
}

#[test]
fn independent_plan_replacement_rejects_stale_forms_and_changed_request_ids() {
    let mut f = Fixture::new();
    let plan = |client: &str, base: u64, previous: Option<&str>, text: &str| {
        ToolRequest::parse(
            schema::REVIEW_PLAN,
            &serde_json::to_vec(&json!({"schemaVersion":1,"clientRequestId":client,
            "planId":"plan","baseRevision":base,"supersedesClientRequestId":previous,
            "title":"Review","overview":"Full plan","markdown":text,"todos":[],"phases":[]}))
            .unwrap(),
        )
        .unwrap()
    };
    let first = f
        .execute(
            Command::Mcp {
                connection: "connection-a".into(),
                workflow_id: f.workflow.clone(),
                rpc_id: RpcId::Number(1),
                payload: plan("p1", 0, None, "First"),
                wall_ms: 1000,
            },
            1,
        )
        .unwrap();
    let id = first["requestId"].as_str().unwrap();
    let old = f.model.request(id).unwrap().submission();
    assert_eq!(
        f.execute(
            Command::Mcp {
                connection: "connection-a".into(),
                workflow_id: f.workflow.clone(),
                rpc_id: RpcId::Number(2),
                payload: plan("p1", 0, None, "Changed"),
                wall_ms: 1001
            },
            2
        )
        .unwrap_err(),
        "REQUEST_ID_CONFLICT"
    );
    let second = f
        .execute(
            Command::Mcp {
                connection: "connection-a".into(),
                workflow_id: f.workflow.clone(),
                rpc_id: RpcId::Number(3),
                payload: plan("p2", 1, Some("p1"), "Second"),
                wall_ms: 1002,
            },
            3,
        )
        .unwrap();
    assert_eq!(f.model.request(id).unwrap().phase, Phase::Superseded);
    assert_eq!(
        f.execute(
            Command::Decide {
                submission: old,
                decision: Decision::Plan {
                    action: schema::PlanAction::Approved,
                    feedback: String::new()
                },
                wall_ms: 1003
            },
            4
        )
        .unwrap_err(),
        "REQUEST_NOT_PENDING"
    );
    let next = f
        .model
        .request(second["requestId"].as_str().unwrap())
        .unwrap();
    assert_eq!(next.plan.as_ref().unwrap().revision, 2);
    assert_eq!(next.phase, Phase::Pending);
}
