use super::*;
use crate::schema::{Answer, ASK_USER, REVIEW_PLAN};
use serde_json::{json, Value};

fn setup() -> (Model, Scope) {
    let mut model = Model::new("installation").unwrap();
    model
        .execute(
            Command::SetProject {
                project_key: "project".into(),
                enabled: true,
            },
            0,
        )
        .unwrap();
    model
        .execute(
            Command::ConfirmGeneration {
                root: "conversation".into(),
                generation: "generation".into(),
            },
            0,
        )
        .unwrap();
    let scope = model
        .scope("conversation", "generation", &["project"])
        .unwrap();
    (model, scope)
}
fn plan(client: &str, plan: &str, base: u64, previous: Option<&str>) -> ToolRequest {
    parse(
        REVIEW_PLAN,
        json!({"schemaVersion":1,"clientRequestId":client,"planId":plan,"baseRevision":base,
        "supersedesClientRequestId":previous,"title":"Plan","overview":"Overview","markdown":"# Steps\r\nOriginal content",
        "todos":[{"id":"todo","description":"Implement","status":"pending","phaseId":"phase"}],
        "phases":[{"id":"phase","title":"Implementation","description":"Work"}]}),
    )
}
fn question(client: &str) -> ToolRequest {
    parse(
        ASK_USER,
        json!({"schemaVersion":1,"clientRequestId":client,"questions":[{
        "questionId":"q","prompt":"Choose or explain","options":[{"optionId":"a","label":"Same"},{"optionId":"b","label":"Same"}],
        "multiSelect":false,"allowFreeText":true,"required":true,"allowSkip":false,"maxTextLength":64}]}),
    )
}
fn parse(tool: &str, value: Value) -> ToolRequest {
    ToolRequest::parse(tool, &serde_json::to_vec(&value).unwrap()).unwrap()
}
fn binding(scope: &Scope, call: &str) -> CallBinding {
    CallBinding {
        scope: scope.clone(),
        call: NativeCall {
            conversation_id: "conversation".into(),
            tool_use_id: call.into(),
        },
    }
}
fn activate(
    model: &mut Model,
    scope: &Scope,
    payload: ToolRequest,
    call: &str,
    now: u64,
) -> Submission {
    match model
        .execute(
            Command::Activate {
                binding: binding(scope, call),
                payload,
                issued_at_ms: now,
                client_deadline_ms: now + 1_000_000,
            },
            now,
        )
        .unwrap()
    {
        Response::Registered { submission } => submission,
        other => panic!("unexpected {other:?}"),
    }
}
fn owner(request: &Submission) -> Owner {
    Owner {
        connection_id: format!("connection-{}", request.request_id),
        rpc_id: RpcId::Number(0),
    }
}
fn enter(model: &mut Model, request: &Submission, now: u64) -> Submission {
    let binding = model.request(&request.request_id).unwrap().binding.clone();
    match model
        .execute(
            Command::Enter {
                request_id: request.request_id.clone(),
                binding,
                owner: owner(request),
                client_deadline_ms: now + 1_000_000,
            },
            now,
        )
        .unwrap()
    {
        Response::Entered { submission } => submission,
        other => panic!("unexpected {other:?}"),
    }
}
fn plan_decision(action: PlanAction) -> Decision {
    Decision::Plan {
        action,
        feedback: String::new(),
    }
}
fn decide(model: &mut Model, request: &Submission, action: PlanAction, now: u64) {
    model
        .execute(
            Command::Decide {
                submission: request.clone(),
                decision: plan_decision(action),
            },
            now,
        )
        .unwrap();
}
fn claim(model: &mut Model, scope: &Scope, request: &Submission, now: u64) -> String {
    match model
        .execute(
            Command::Claim {
                request_id: request.request_id.clone(),
                scope: scope.clone(),
                owner: owner(request),
            },
            now,
        )
        .unwrap()
    {
        Response::Claimed { lease_id, .. } => lease_id,
        other => panic!("unexpected {other:?}"),
    }
}
fn written(model: &mut Model, request: &Submission, lease: &str, now: u64) -> Response {
    model
        .execute(
            Command::ReportDelivery {
                request_id: request.request_id.clone(),
                lease_id: lease.into(),
                outcome: DeliveryOutcome::Written,
            },
            now,
        )
        .unwrap()
}
fn answer(option: &str) -> Decision {
    Decision::Answered {
        answers: vec![Answer {
            question_id: "q".into(),
            selected_option_ids: vec![option.into()],
            text: None,
            skipped: false,
        }],
    }
}

#[test]
fn business_schema_removes_only_ticket_and_preserves_original_content() {
    let original = plan("a", "p", 0, None);
    let mut raw = serde_json::to_value(&original).unwrap()["business"].clone();
    raw["bridgeTicket"] = json!("secret-never-persisted");
    let supplied = parse(REVIEW_PLAN, raw.clone());
    assert_eq!(supplied, original);
    assert!(!serde_json::to_string(&supplied)
        .unwrap()
        .contains("secret-never-persisted"));
    raw["bridgeTicket"] = json!(42);
    assert!(ToolRequest::parse(REVIEW_PLAN, &serde_json::to_vec(&raw).unwrap()).is_err());
    assert!(ToolRequest::parse(ASK_USER, br#"{"schemaVersion":1,"schemaVersion":1}"#).is_err());
    raw.as_object_mut().unwrap().remove("bridgeTicket");
    raw["markdown"] = json!("# Steps\nOriginal content");
    assert_ne!(
        parse(REVIEW_PLAN, raw).arguments_digest(),
        original.arguments_digest()
    );
}

#[test]
fn questions_use_ids_validate_required_text_and_reject_duplicate_or_unknown_answers() {
    let request = question("a");
    request.validate_decision(&answer("b")).unwrap();
    assert!(request.validate_decision(&answer("Same")).is_err());
    let empty = Answer {
        question_id: "q".into(),
        selected_option_ids: vec![],
        text: Some(" ".into()),
        skipped: false,
    };
    assert_eq!(
        request.validate_decision(&Decision::Answered {
            answers: vec![empty.clone()]
        }),
        Err("ANSWER_REQUIRED")
    );
    let text = Answer {
        text: Some("Free text without choosing an option".into()),
        ..empty.clone()
    };
    request
        .validate_decision(&Decision::Answered {
            answers: vec![text],
        })
        .unwrap();
    assert!(request
        .validate_decision(&Decision::Answered { answers: vec![] })
        .is_err());
    assert!(request
        .validate_decision(&Decision::Answered {
            answers: vec![empty.clone(), empty.clone()]
        })
        .is_err());
    assert!(request
        .validate_decision(&Decision::Answered {
            answers: vec![Answer {
                selected_option_ids: vec!["a".into(), "a".into()],
                ..empty.clone()
            }]
        })
        .is_err());
    assert!(request
        .validate_decision(&Decision::Answered {
            answers: vec![Answer {
                text: None,
                skipped: true,
                ..empty
            }]
        })
        .is_err());
}

#[test]
fn text_only_questions_and_explicit_skip_have_distinct_valid_answers() {
    let mut raw = serde_json::to_value(question("a")).unwrap()["business"].clone();
    raw["questions"][0]["options"] = json!([]);
    raw["questions"][0]["allowSkip"] = json!(true);
    raw["questions"][0]["maxTextLength"] = json!(2);
    let request = parse(ASK_USER, raw);
    let answer = Answer {
        question_id: "q".into(),
        selected_option_ids: vec![],
        text: Some("你好".into()),
        skipped: false,
    };
    request
        .validate_decision(&Decision::Answered {
            answers: vec![answer.clone()],
        })
        .unwrap();
    request
        .validate_decision(&Decision::Answered {
            answers: vec![Answer {
                text: None,
                skipped: true,
                ..answer.clone()
            }],
        })
        .unwrap();
    assert!(request
        .validate_decision(&Decision::Answered {
            answers: vec![Answer {
                text: Some("你好呀".into()),
                ..answer.clone()
            }]
        })
        .is_err());
    assert!(request
        .validate_decision(&Decision::Answered {
            answers: vec![Answer {
                skipped: true,
                ..answer
            }]
        })
        .is_err());
}

#[test]
fn plan_schema_checks_phase_links_and_digest_includes_every_reviewed_field() {
    let original = plan("a", "p", 0, None);
    let raw = serde_json::to_value(&original).unwrap()["business"].clone();
    for field in ["title", "overview", "markdown"] {
        let mut changed = raw.clone();
        changed[field] = json!("Changed");
        assert_ne!(
            parse(REVIEW_PLAN, changed).plan_content_digest(),
            original.plan_content_digest()
        );
    }
    let mut changed = raw.clone();
    changed["todos"][0]["status"] = json!("completed");
    assert_ne!(
        parse(REVIEW_PLAN, changed).plan_content_digest(),
        original.plan_content_digest()
    );
    let mut changed = raw.clone();
    changed["todos"][0]["phaseId"] = json!("missing");
    assert!(ToolRequest::parse(REVIEW_PLAN, &serde_json::to_vec(&changed).unwrap()).is_err());
    let mut changed = raw;
    changed["unexpected"] = json!(true);
    assert!(ToolRequest::parse(REVIEW_PLAN, &serde_json::to_vec(&changed).unwrap()).is_err());
}

#[test]
fn exact_retry_precedes_revision_checks_and_changed_input_conflicts_without_mutation() {
    let (mut model, scope) = setup();
    let payload = plan("a", "p", 0, None);
    let first = activate(&mut model, &scope, payload.clone(), "call-a", 1);
    let gate = model.gate_revision();
    let deadline = model.request(&first.request_id).unwrap().deadline_ms;
    let response = model
        .execute(
            Command::Activate {
                binding: binding(&scope, "retry"),
                payload,
                issued_at_ms: 2,
                client_deadline_ms: 2_000_000,
            },
            2,
        )
        .unwrap();
    assert!(matches!(
        response,
        Response::Duplicate {
            status: DuplicateStatus::Pending,
            current_revision: Some(1),
            ..
        }
    ));
    assert_eq!(model.request_count(), 1);
    assert_eq!(model.gate_revision(), gate);
    assert_eq!(
        model.request(&first.request_id).unwrap().deadline_ms,
        deadline
    );
    assert_eq!(
        model.execute(
            Command::Activate {
                binding: binding(&scope, "retry"),
                payload: plan("a", "other", 0, None),
                issued_at_ms: 2,
                client_deadline_ms: 999999
            },
            2
        ),
        Err("REQUEST_ID_CONFLICT")
    );
    assert_eq!(model.request_count(), 1);
}

#[test]
fn same_content_new_request_requires_new_revision_and_new_human_decision() {
    let (mut model, scope) = setup();
    let first = activate(&mut model, &scope, plan("a", "p", 0, None), "call-a", 1);
    let entered = enter(&mut model, &first, 2);
    decide(&mut model, &entered, PlanAction::Approved, 3);
    assert_eq!(model.gate(&scope, false), Gate::ReviewPending);
    let lease = claim(&mut model, &scope, &first, 4);
    written(&mut model, &first, &lease, 5);
    assert_eq!(model.gate(&scope, false), Gate::NormalPolicy);
    let second = activate(
        &mut model,
        &scope,
        plan("b", "p", 1, Some("a")),
        "call-b",
        6,
    );
    assert_eq!(
        first.plan.as_ref().unwrap().content_digest,
        second.plan.as_ref().unwrap().content_digest
    );
    assert_eq!(second.plan.as_ref().unwrap().revision, 2);
    assert_ne!(first.request_id, second.request_id);
    assert_eq!(model.request(&second.request_id).unwrap().decision, None);
    assert_eq!(model.gate(&scope, false), Gate::ReviewPending);
    assert_eq!(
        model.request(&first.request_id).unwrap().transport,
        Transport::Written
    );
}

#[test]
fn approving_one_plan_does_not_release_other_plans_or_other_workflows() {
    let (mut model, scope) = setup();
    let first = activate(&mut model, &scope, plan("a", "p", 0, None), "call-a", 1);
    activate(&mut model, &scope, plan("b", "other", 0, None), "call-b", 2);
    let entered = enter(&mut model, &first, 3);
    decide(&mut model, &entered, PlanAction::Approved, 4);
    let lease = claim(&mut model, &scope, &first, 5);
    written(&mut model, &first, &lease, 6);
    assert_eq!(model.gate(&scope, false), Gate::ReviewPending);
    assert_eq!(model.gate(&scope, true), Gate::NormalPolicy);
    model
        .execute(
            Command::ConfirmGeneration {
                root: "other".into(),
                generation: "generation".into(),
            },
            7,
        )
        .unwrap();
    let independent = model.scope("other", "generation", &["project"]).unwrap();
    assert_eq!(model.gate(&independent, false), Gate::NormalPolicy);
}

#[test]
fn disabled_projects_and_stale_project_revisions_reject_all_live_writes() {
    let (mut model, scope) = setup();
    let first = activate(&mut model, &scope, plan("a", "p", 0, None), "call-a", 1);
    let entered = enter(&mut model, &first, 2);
    model
        .execute(
            Command::SetProject {
                project_key: "project".into(),
                enabled: false,
            },
            3,
        )
        .unwrap();
    assert_eq!(model.gate(&scope, false), Gate::Unavailable);
    assert_eq!(
        model.execute(
            Command::Decide {
                submission: entered,
                decision: plan_decision(PlanAction::Approved)
            },
            4
        ),
        Err("PROJECT_NOT_ENABLED")
    );
    model
        .execute(
            Command::SetProject {
                project_key: "project".into(),
                enabled: true,
            },
            5,
        )
        .unwrap();
    assert_eq!(
        model.validate_scope(&scope),
        Err("PROJECT_REVISION_CHANGED")
    );
    assert_eq!(
        model.request(&first.request_id).unwrap().phase,
        Phase::Disconnected
    );
    let current = model
        .scope("conversation", "generation", &["project"])
        .unwrap();
    assert_eq!(model.gate(&current, false), Gate::NormalPolicy);
}

#[test]
fn entry_timeout_is_committed_even_when_a_late_entry_command_fails() {
    let (mut model, scope) = setup();
    let first = activate(&mut model, &scope, plan("a", "p", 0, None), "call-a", 0);
    assert_eq!(
        model.execute(
            Command::Enter {
                request_id: first.request_id.clone(),
                binding: binding(&scope, "call-a"),
                owner: owner(&first),
                client_deadline_ms: 1_000_000
            },
            60_000
        ),
        Err("REQUEST_NOT_WAITING_FOR_ENTRY")
    );
    assert_eq!(
        model.request(&first.request_id).unwrap().phase,
        Phase::Expired
    );
    assert_eq!(
        model.request(&first.request_id).unwrap().native_evidence,
        NativeEvidence::Unconfirmed
    );
    assert_eq!(model.gate(&scope, false), Gate::NormalPolicy);
    let second = activate(
        &mut model,
        &scope,
        plan("b", "p", 1, Some("a")),
        "call-b",
        60_001,
    );
    assert_eq!(second.plan.unwrap().revision, 2);
}

#[test]
fn entering_mcp_switches_deadlines_and_ticket_ttl_cannot_expire_human_wait() {
    let (mut model, scope) = setup();
    let response = model
        .execute(
            Command::Activate {
                binding: binding(&scope, "call-a"),
                payload: plan("a", "p", 0, None),
                issued_at_ms: 0,
                client_deadline_ms: 1_000_000,
            },
            590_000,
        )
        .unwrap();
    let Response::Registered { submission } = response else {
        panic!()
    };
    assert_eq!(
        model.request(&submission.request_id).unwrap().deadline_ms,
        600_000
    );
    enter(&mut model, &submission, 599_000);
    model.execute(Command::Tick, 601_000).unwrap();
    assert_eq!(
        model.request(&submission.request_id).unwrap().phase,
        Phase::Pending
    );
    assert_eq!(
        model.request(&submission.request_id).unwrap().deadline_ms,
        899_000
    );
}

#[test]
fn insufficient_entry_budget_ends_wait_without_creating_pending_form() {
    let (mut model, scope) = setup();
    let first = activate(&mut model, &scope, plan("a", "p", 0, None), "call-a", 0);
    let response = model
        .execute(
            Command::Enter {
                request_id: first.request_id.clone(),
                binding: binding(&scope, "call-a"),
                owner: owner(&first),
                client_deadline_ms: 39_999,
            },
            0,
        )
        .unwrap();
    assert_eq!(
        response,
        Response::EntryRejected {
            code: "INSUFFICIENT_WAIT_BUDGET".into()
        }
    );
    assert_eq!(
        model.request(&first.request_id).unwrap().phase,
        Phase::Expired
    );
    assert_eq!(model.gate(&scope, false), Gate::NormalPolicy);
}

#[test]
fn decisions_compare_full_plan_snapshot_and_repeats_never_change_original_choice() {
    let (mut model, scope) = setup();
    let first = activate(&mut model, &scope, plan("a", "p", 0, None), "call-a", 1);
    let entered = enter(&mut model, &first, 2);
    let mut stale = entered.clone();
    stale.plan.as_mut().unwrap().content_digest = "old".into();
    assert_eq!(
        model.execute(
            Command::Decide {
                submission: stale,
                decision: plan_decision(PlanAction::Approved)
            },
            3
        ),
        Err("STALE_CONTENT")
    );
    assert_eq!(
        model.execute(
            Command::Decide {
                submission: first,
                decision: plan_decision(PlanAction::Approved)
            },
            3
        ),
        Err("STALE_REQUEST")
    );
    decide(&mut model, &entered, PlanAction::Approved, 4);
    let version = model.request(&entered.request_id).unwrap().version;
    assert_eq!(
        model
            .execute(
                Command::Decide {
                    submission: entered.clone(),
                    decision: plan_decision(PlanAction::Approved)
                },
                5
            )
            .unwrap(),
        Response::DecisionRecorded {
            already_recorded: true
        }
    );
    assert_eq!(model.request(&entered.request_id).unwrap().version, version);
    assert_eq!(
        model.execute(
            Command::Decide {
                submission: entered,
                decision: plan_decision(PlanAction::Rejected)
            },
            6
        ),
        Err("DECISION_CONFLICT")
    );
}

#[test]
fn rpc_zero_has_one_owner_and_same_native_call_cannot_create_two_requests() {
    let (mut model, scope) = setup();
    let first = activate(&mut model, &scope, question("a"), "call-a", 0);
    let entered = enter(&mut model, &first, 1);
    assert_eq!(
        model.execute(
            Command::Activate {
                binding: binding(&scope, "call-a"),
                payload: question("b"),
                issued_at_ms: 1,
                client_deadline_ms: 999999
            },
            1
        ),
        Err("NATIVE_CALL_ALREADY_BOUND")
    );
    let second = activate(&mut model, &scope, question("b"), "call-b", 2);
    assert_eq!(
        model.execute(
            Command::Enter {
                request_id: second.request_id.clone(),
                binding: binding(&scope, "call-b"),
                owner: owner(&first),
                client_deadline_ms: 999999
            },
            3
        ),
        Err("RPC_ID_ALREADY_BOUND")
    );
    model
        .execute(
            Command::Decide {
                submission: entered,
                decision: answer("a"),
            },
            4,
        )
        .unwrap();
    assert_eq!(
        model.execute(
            Command::Claim {
                request_id: first.request_id.clone(),
                scope: scope.clone(),
                owner: owner(&second)
            },
            5
        ),
        Err("NOT_RESULT_OWNER")
    );
    let lease = claim(&mut model, &scope, &first, 5);
    written(&mut model, &first, &lease, 6);
    assert_eq!(
        model.request(&first.request_id).unwrap().native_evidence,
        NativeEvidence::Unconfirmed
    );
    assert_eq!(
        model.execute(
            Command::Claim {
                request_id: first.request_id.clone(),
                scope,
                owner: owner(&first)
            },
            7
        ),
        Err("RESULT_NOT_CLAIMABLE")
    );
}

#[test]
fn retry_native_failure_cannot_cancel_the_original_business_owner() {
    let (mut model, scope) = setup();
    let first = activate(&mut model, &scope, question("a"), "call-a", 0);
    let retry = model
        .execute(
            Command::Activate {
                binding: binding(&scope, "retry"),
                payload: question("a"),
                issued_at_ms: 1,
                client_deadline_ms: 999999,
            },
            1,
        )
        .unwrap();
    assert!(matches!(retry, Response::Duplicate { .. }));
    assert_eq!(
        model.execute(
            Command::NativeTermination {
                request_id: first.request_id.clone(),
                binding: binding(&scope, "retry"),
                evidence: NativeEvidence::BlockedObserved
            },
            2
        ),
        Err("CALL_NOT_ASSOCIATED")
    );
    assert_eq!(
        model.request(&first.request_id).unwrap().phase,
        Phase::AwaitingMcpEntry
    );
}

#[test]
fn cancellation_before_claim_prevents_delivery_and_after_claim_preserves_unknown() {
    for after_claim in [false, true] {
        let (mut model, scope) = setup();
        let first = activate(&mut model, &scope, plan("a", "p", 0, None), "call-a", 0);
        let entered = enter(&mut model, &first, 1);
        decide(&mut model, &entered, PlanAction::Approved, 2);
        let lease = after_claim.then(|| claim(&mut model, &scope, &first, 3));
        model
            .execute(
                Command::NativeTermination {
                    request_id: first.request_id.clone(),
                    binding: binding(&scope, "call-a"),
                    evidence: NativeEvidence::CancelledObserved,
                },
                4,
            )
            .unwrap();
        assert_eq!(
            model.request(&first.request_id).unwrap().transport,
            if after_claim {
                Transport::OutcomeUnknown
            } else {
                Transport::NotStarted
            }
        );
        assert_eq!(
            model.execute(
                Command::Claim {
                    request_id: first.request_id.clone(),
                    scope: scope.clone(),
                    owner: owner(&first)
                },
                5
            ),
            Err("RESULT_NOT_CLAIMABLE")
        );
        if let Some(lease) = lease {
            assert_eq!(
                written(&mut model, &first, &lease, 6),
                Response::DeliveryRecorded { historical: true }
            );
        }
    }
}

#[test]
fn old_claimed_approval_cannot_release_replacement_gate_when_late_write_arrives() {
    let (mut model, scope) = setup();
    let first = activate(&mut model, &scope, plan("a", "p", 0, None), "call-a", 0);
    let entered = enter(&mut model, &first, 1);
    decide(&mut model, &entered, PlanAction::Approved, 2);
    let lease = claim(&mut model, &scope, &first, 3);
    let second = activate(
        &mut model,
        &scope,
        plan("b", "p", 1, Some("a")),
        "call-b",
        4,
    );
    let revision = model.gate_revision();
    assert_eq!(
        written(&mut model, &first, &lease, 5),
        Response::DeliveryRecorded { historical: true }
    );
    assert_eq!(model.gate_revision(), revision);
    assert_eq!(model.gate(&scope, false), Gate::ReviewPending);
    assert_eq!(
        model.request(&second.request_id).unwrap().phase,
        Phase::AwaitingMcpEntry
    );
}

#[test]
fn human_revision_survives_failed_replacement_and_only_new_written_approval_clears_it() {
    let (mut model, scope) = setup();
    let first = activate(&mut model, &scope, plan("a", "p", 0, None), "call-a", 0);
    let entered = enter(&mut model, &first, 1);
    decide(&mut model, &entered, PlanAction::Revise, 2);
    let lease = claim(&mut model, &scope, &first, 3);
    written(&mut model, &first, &lease, 4);
    let second = activate(
        &mut model,
        &scope,
        plan("b", "p", 1, Some("a")),
        "call-b",
        5,
    );
    model.execute(Command::Tick, 60_005).unwrap();
    assert_eq!(
        model.request(&second.request_id).unwrap().phase,
        Phase::Expired
    );
    assert_eq!(model.gate(&scope, false), Gate::RevisionRequired);
    let third = activate(
        &mut model,
        &scope,
        plan("c", "p", 2, Some("b")),
        "call-c",
        60_006,
    );
    let entered = enter(&mut model, &third, 60_007);
    decide(&mut model, &entered, PlanAction::Approved, 60_008);
    assert_eq!(model.gate(&scope, false), Gate::RevisionRequired);
    let lease = claim(&mut model, &scope, &third, 60_009);
    written(&mut model, &third, &lease, 60_010);
    assert_eq!(model.gate(&scope, false), Gate::NormalPolicy);
}

#[test]
fn lease_timeout_never_reassigns_or_replays_a_possibly_written_result() {
    let (mut model, scope) = setup();
    let first = activate(&mut model, &scope, plan("a", "p", 0, None), "call-a", 0);
    let entered = enter(&mut model, &first, 1);
    decide(&mut model, &entered, PlanAction::Approved, 2);
    let lease = claim(&mut model, &scope, &first, 3);
    model.execute(Command::Tick, 5_003).unwrap();
    assert_eq!(
        model.request(&first.request_id).unwrap().transport,
        Transport::OutcomeUnknown
    );
    assert_eq!(
        model.execute(
            Command::Claim {
                request_id: first.request_id.clone(),
                scope: scope.clone(),
                owner: owner(&first)
            },
            5_004
        ),
        Err("RESULT_NOT_CLAIMABLE")
    );
    let response = model
        .execute(
            Command::Activate {
                binding: binding(&scope, "retry"),
                payload: plan("a", "p", 0, None),
                issued_at_ms: 5_004,
                client_deadline_ms: 999999,
            },
            5_004,
        )
        .unwrap();
    assert!(matches!(
        response,
        Response::Duplicate {
            status: DuplicateStatus::Terminated,
            ..
        }
    ));
    assert!(!serde_json::to_string(&response)
        .unwrap()
        .contains("approved"));
    assert_eq!(
        written(&mut model, &first, &lease, 5_005),
        Response::DeliveryRecorded { historical: true }
    );
}

#[test]
fn confirmed_generation_switch_retires_old_calls_without_late_events_touching_new_gate() {
    let (mut model, scope) = setup();
    let old = activate(&mut model, &scope, plan("a", "p", 0, None), "call-a", 0);
    model
        .execute(
            Command::ConfirmGeneration {
                root: "conversation".into(),
                generation: "new".into(),
            },
            1,
        )
        .unwrap();
    let current = model.scope("conversation", "new", &["project"]).unwrap();
    let new = activate(&mut model, &current, plan("a", "p", 0, None), "call-new", 2);
    model
        .execute(
            Command::NativeTermination {
                request_id: old.request_id,
                binding: binding(&scope, "call-a"),
                evidence: NativeEvidence::BlockedObserved,
            },
            3,
        )
        .unwrap();
    assert_eq!(model.gate(&current, false), Gate::ReviewPending);
    assert_eq!(
        model.request(&new.request_id).unwrap().phase,
        Phase::AwaitingMcpEntry
    );
    assert_eq!(
        model.execute(
            Command::ConfirmGeneration {
                root: "conversation".into(),
                generation: "generation".into()
            },
            4
        ),
        Err("GENERATION_RETIRED")
    );
    assert_eq!(
        model
            .execute(
                Command::EndGeneration {
                    root: "conversation".into(),
                    generation: "generation".into()
                },
                4
            )
            .unwrap(),
        Response::Unchanged
    );
}

#[test]
fn restart_preserves_read_only_decisions_but_never_old_epoch_authority_or_transport() {
    let (mut model, scope) = setup();
    let first = activate(&mut model, &scope, plan("a", "p", 0, None), "call-a", 0);
    let entered = enter(&mut model, &first, 1);
    decide(&mut model, &entered, PlanAction::Approved, 2);
    let lease = claim(&mut model, &scope, &first, 3);
    let bytes = serde_json::to_vec(&model).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains(&lease));
    assert!(!String::from_utf8_lossy(&bytes).contains(&owner(&first).connection_id));
    let saved = serde_json::from_slice(&bytes).unwrap();
    let mut recovered = Model::recover(saved, "installation").unwrap();
    assert_ne!(recovered.app_epoch(), model.app_epoch());
    assert_eq!(
        recovered.request(&first.request_id).unwrap().transport,
        Transport::OutcomeUnknown
    );
    assert_eq!(
        recovered.request(&first.request_id).unwrap().phase,
        Phase::Disconnected
    );
    assert_eq!(
        recovered.execute(
            Command::Decide {
                submission: entered,
                decision: plan_decision(PlanAction::Approved)
            },
            0
        ),
        Err("APP_RESTARTED")
    );
    assert_eq!(
        recovered.execute(
            Command::ReportDelivery {
                request_id: first.request_id.clone(),
                lease_id: lease,
                outcome: DeliveryOutcome::Written
            },
            0
        ),
        Err("DELIVERY_LEASE_INVALID")
    );
    recovered
        .execute(
            Command::ConfirmGeneration {
                root: "conversation".into(),
                generation: "generation".into(),
            },
            0,
        )
        .unwrap();
    let current = recovered
        .scope("conversation", "generation", &["project"])
        .unwrap();
    let new = activate(
        &mut recovered,
        &current,
        plan("a", "p", 0, None),
        "call-new",
        1,
    );
    assert_ne!(new.request_id, first.request_id);
    assert_eq!(new.plan.unwrap().revision, 1);
}

#[test]
fn monotonic_clock_regression_does_not_extend_or_mutate_deadlines() {
    let (mut model, scope) = setup();
    activate(&mut model, &scope, question("a"), "call-a", 10);
    let before = model.clone();
    assert_eq!(
        model.execute(Command::Tick, 9),
        Err("MONOTONIC_CLOCK_REGRESSED")
    );
    assert_eq!(model, before);
}

#[test]
fn concurrent_retries_create_one_owner_and_competing_plan_revisions_have_one_winner() {
    use std::sync::{Arc, Mutex};
    let (model, scope) = setup();
    let model = Arc::new(Mutex::new(model));
    let register = |clients: [&str; 2], base: u64, previous: Option<&str>| {
        std::thread::scope(|threads| {
            clients
                .into_iter()
                .enumerate()
                .map(|(index, client)| {
                    let model = Arc::clone(&model);
                    let scope = scope.clone();
                    threads.spawn(move || {
                        model.lock().unwrap().execute(
                            Command::Activate {
                                binding: binding(&scope, &format!("call-{base}-{index}")),
                                payload: plan(client, "p", base, previous),
                                issued_at_ms: base,
                                client_deadline_ms: 999999,
                            },
                            base,
                        )
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>()
        })
    };
    let first = register(["a", "a"], 0, None);
    assert_eq!(
        first
            .iter()
            .filter(|r| matches!(r, Ok(Response::Registered { .. })))
            .count(),
        1
    );
    assert_eq!(
        first
            .iter()
            .filter(|r| matches!(r, Ok(Response::Duplicate { .. })))
            .count(),
        1
    );
    let updates = register(["b", "c"], 1, Some("a"));
    assert_eq!(
        updates
            .iter()
            .filter(|r| matches!(r, Ok(Response::Registered { .. })))
            .count(),
        1
    );
    assert_eq!(
        updates
            .iter()
            .filter(|r| **r == Err("REVISION_CONFLICT"))
            .count(),
        1
    );
    assert_eq!(model.lock().unwrap().request_count(), 2);
}

#[test]
fn configuration_changes_interrupt_waits_but_do_not_erase_human_revision_requirement() {
    let (mut model, scope) = setup();
    let first = activate(&mut model, &scope, plan("a", "p", 0, None), "call-a", 0);
    let entered = enter(&mut model, &first, 1);
    decide(&mut model, &entered, PlanAction::Rejected, 2);
    model.execute(Command::ChangeConfiguration, 3).unwrap();
    assert_eq!(model.gate(&scope, false), Gate::Unavailable);
    let current = model
        .scope("conversation", "generation", &["project"])
        .unwrap();
    assert_eq!(model.gate(&current, false), Gate::RevisionRequired);
}

#[test]
fn simultaneous_opposite_decisions_record_exactly_one_immutable_choice() {
    use std::sync::{Arc, Mutex};
    let (mut model, scope) = setup();
    let first = activate(&mut model, &scope, plan("a", "p", 0, None), "call-a", 0);
    let entered = enter(&mut model, &first, 1);
    let model = Arc::new(Mutex::new(model));
    let outcomes = std::thread::scope(|threads| {
        [PlanAction::Approved, PlanAction::Rejected]
            .into_iter()
            .map(|action| {
                let model = Arc::clone(&model);
                let submission = entered.clone();
                threads.spawn(move || {
                    model.lock().unwrap().execute(
                        Command::Decide {
                            submission,
                            decision: plan_decision(action),
                        },
                        2,
                    )
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(
                outcome,
                Ok(Response::DecisionRecorded {
                    already_recorded: false
                })
            ))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| **outcome == Err("DECISION_CONFLICT"))
            .count(),
        1
    );
}

#[test]
fn each_reported_project_must_remain_enabled_at_submission() {
    let (mut model, _) = setup();
    model
        .execute(
            Command::SetProject {
                project_key: "second".into(),
                enabled: true,
            },
            0,
        )
        .unwrap();
    let scope = model
        .scope("conversation", "generation", &["project", "second"])
        .unwrap();
    let first = activate(&mut model, &scope, question("a"), "call-a", 1);
    let entered = enter(&mut model, &first, 2);
    model
        .execute(
            Command::SetProject {
                project_key: "second".into(),
                enabled: false,
            },
            3,
        )
        .unwrap();
    assert_eq!(
        model.execute(
            Command::Decide {
                submission: entered,
                decision: answer("a")
            },
            4
        ),
        Err("PROJECT_NOT_ENABLED")
    );
    assert_eq!(
        model.request(&first.request_id).unwrap().phase,
        Phase::Disconnected
    );
}

#[test]
fn restart_rejects_old_termination_commands_and_corrupt_plan_metadata() {
    let (mut model, scope) = setup();
    let first = activate(&mut model, &scope, plan("a", "p", 0, None), "call-a", 0);
    let saved = serde_json::to_value(&model).unwrap();
    let mut recovered = Model::recover(model, "installation").unwrap();
    let before = recovered.clone();
    assert_eq!(
        recovered.execute(
            Command::NativeTermination {
                request_id: first.request_id.clone(),
                binding: binding(&scope, "call-a"),
                evidence: NativeEvidence::BlockedObserved
            },
            0
        ),
        Err("APP_RESTARTED")
    );
    assert_eq!(recovered, before);
    let mut corrupt = saved.clone();
    corrupt["requests"][&first.request_id]["version"] = json!(u64::MAX);
    assert!(Model::recover(serde_json::from_value(corrupt).unwrap(), "installation").is_err());
    let mut corrupt = saved;
    corrupt["requests"][&first.request_id]["plan"]["contentDigest"] =
        json!("not-the-reviewed-content");
    assert_eq!(
        Model::recover(serde_json::from_value(corrupt).unwrap(), "installation"),
        Err("INVALID_SAVED_PLAN")
    );
}

#[test]
fn multiple_questions_cannot_be_answered_twice_under_one_question_id() {
    let mut raw = serde_json::to_value(question("a")).unwrap()["business"].clone();
    let mut second = raw["questions"][0].clone();
    second["questionId"] = json!("second");
    raw["questions"].as_array_mut().unwrap().push(second);
    let payload = parse(ASK_USER, raw);
    let Decision::Answered { answers } = answer("a") else {
        panic!()
    };
    assert_eq!(
        payload.validate_decision(&Decision::Answered {
            answers: vec![answers[0].clone(), answers[0].clone()]
        }),
        Err("DUPLICATE_ANSWER")
    );
    let mut second = answers[0].clone();
    second.question_id = "second".into();
    payload
        .validate_decision(&Decision::Answered {
            answers: vec![second, answers[0].clone()],
        })
        .unwrap();
}

#[test]
fn rust_digests_match_shared_javascript_fixtures_without_normalizing_content() {
    let fixtures: Value = serde_json::from_str(include_str!(
        "../../protocol/cursor/codecraft-v1/fixtures/business-digests.json"
    ))
    .unwrap();
    assert_eq!(fixtures["evidenceKind"], "synthetic");
    assert_eq!(fixtures["cursorClientExecuted"], false);
    for case in fixtures["cases"].as_array().unwrap() {
        let input = ToolRequest::parse(
            case["toolName"].as_str().unwrap(),
            &serde_json::to_vec(&case["arguments"]).unwrap(),
        )
        .unwrap();
        assert_eq!(
            input.arguments_digest(),
            case["expected"]["argumentsDigest"].as_str().unwrap(),
            "{}",
            case["name"]
        );
        assert_eq!(
            input.plan_content_digest(),
            case["expected"]["planContentDigest"].as_str(),
            "{}",
            case["name"]
        );
    }
}
