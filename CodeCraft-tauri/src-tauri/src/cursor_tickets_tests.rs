use super::*;
use crate::{
    schema::{Decision, PlanAction, REVIEW_PLAN},
    workflow::{Gate, NativeCall, NativeEvidence, Phase as RequestPhase, RpcId, Submission},
};
use serde_json::json;
use std::{fs, path::PathBuf};

fn setup() -> (Model, Candidate) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("runs")
        .join(format!("tickets-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&path).unwrap();
    let mut model = Model::new("installation").unwrap();
    model
        .execute(
            Command::SetProjectPath {
                path: path.clone(),
                enabled: true,
            },
            0,
        )
        .unwrap();
    model
        .execute(
            Command::ConfirmGeneration {
                root: "root".into(),
                generation: "generation".into(),
            },
            0,
        )
        .unwrap();
    let key = crate::projects::resolve(&path).unwrap().identity.key();
    let scope = model.scope("root", "generation", &[&key]).unwrap();
    let payload = ToolRequest::parse(REVIEW_PLAN, &serde_json::to_vec(&json!({"schemaVersion":1,"clientRequestId":"review-a","planId":"p","baseRevision":0,"title":"Plan","overview":"","markdown":"Review me","todos":[],"phases":[]})).unwrap()).unwrap();
    let text = path.to_str().unwrap().to_string();
    let candidate = Candidate {
        binding: CallBinding {
            scope,
            call: NativeCall {
                conversation_id: "root".into(),
                tool_use_id: "call-a".into(),
            },
        },
        candidate_event_id: "event-a".into(),
        server: ServerIdentity {
            registration_id: "CodeCraft".into(),
            command_digest: protocol::sha256(b"command and binary"),
            configuration_digest: protocol::sha256(b"effective config"),
        },
        roots: vec![text.clone()],
        cwd: text,
        payload,
        client_deadline_ms: 1_000_000,
    };
    (model, candidate)
}
fn issue(model: &mut Model, candidate: &Candidate, now: u64) -> Secret {
    let Response::CandidateIssued { ticket } = model
        .execute(
            Command::IssueTicket {
                candidate: Box::new(candidate.clone()),
            },
            now,
        )
        .unwrap()
    else {
        panic!()
    };
    ticket
}
fn activate(
    model: &mut Model,
    candidate: &Candidate,
    ticket: &Secret,
    now: u64,
) -> Result<Response> {
    model.execute(
        Command::ActivateTicket {
            ticket: ticket.clone(),
            binding: candidate.binding.clone(),
            server: candidate.server.clone(),
            payload: candidate.payload.clone(),
            roots: candidate.roots.clone(),
            cwd: candidate.cwd.clone(),
        },
        now,
    )
}
fn consume(
    model: &mut Model,
    candidate: &Candidate,
    ticket: &Secret,
    rpc: i64,
    now: u64,
) -> Result<Response> {
    model.execute(
        Command::ConsumeTicket {
            ticket: ticket.clone(),
            server: candidate.server.clone(),
            payload: candidate.payload.clone(),
            roots: candidate.roots.clone(),
            cwd: candidate.cwd.clone(),
            owner: Owner {
                connection_id: "connection".into(),
                rpc_id: RpcId::Number(rpc),
            },
        },
        now,
    )
}
fn entered(response: Response) -> Submission {
    let Response::Entered { submission } = response else {
        panic!()
    };
    submission
}
fn retry(candidate: &Candidate, suffix: &str) -> Candidate {
    let mut candidate = candidate.clone();
    candidate.binding.call.tool_use_id = format!("call-{suffix}");
    candidate.candidate_event_id = format!("event-{suffix}");
    candidate
}

#[test]
fn candidate_alone_has_no_gate_and_wrong_server_or_changed_arguments_cannot_activate() {
    let (mut model, candidate) = setup();
    let ticket = issue(&mut model, &candidate, 0);
    assert_eq!(model.request_count(), 0);
    assert_eq!(
        model.gate(&candidate.binding.scope, false),
        Gate::NormalPolicy
    );
    assert_eq!(
        consume(&mut model, &candidate, &ticket, 0, 1),
        Err("TICKET_NOT_ACTIVE")
    );
    let mut wrong = candidate.clone();
    wrong.server.command_digest = protocol::sha256(b"different command");
    assert_eq!(
        activate(&mut model, &wrong, &ticket, 2),
        Err("TICKET_SERVER_MISMATCH")
    );
    let mut business = serde_json::to_value(&candidate.payload).unwrap()["business"].clone();
    business["markdown"] = json!("changed");
    wrong = candidate.clone();
    wrong.payload =
        ToolRequest::parse(REVIEW_PLAN, &serde_json::to_vec(&business).unwrap()).unwrap();
    assert_eq!(
        activate(&mut model, &wrong, &ticket, 3),
        Err("TICKET_ARGUMENTS_CHANGED")
    );
    assert_eq!(model.request_count(), 0);
    assert!(matches!(
        activate(&mut model, &candidate, &ticket, 4),
        Ok(Response::Registered { .. })
    ));
    assert_eq!(
        model.gate(&candidate.binding.scope, false),
        Gate::ReviewPending
    );
    assert_eq!(
        activate(&mut model, &candidate, &ticket, 5),
        Err("TICKET_ALREADY_ACTIVATED")
    );
}
#[test]
fn one_time_consumption_binds_rpc_zero_and_never_serializes_a_raw_ticket() {
    let (mut model, candidate) = setup();
    let ticket = issue(&mut model, &candidate, 0);
    activate(&mut model, &candidate, &ticket, 1).unwrap();
    consume(&mut model, &candidate, &ticket, 0, 2).unwrap();
    assert_eq!(
        consume(&mut model, &candidate, &ticket, 0, 3).unwrap(),
        Response::ProtocolDuplicate
    );
    assert_eq!(
        consume(&mut model, &candidate, &ticket, 1, 4),
        Err("TICKET_USED")
    );
    let retry = retry(&candidate, "retry");
    let second = issue(&mut model, &retry, 5);
    activate(&mut model, &retry, &second, 6).unwrap();
    assert_eq!(
        consume(&mut model, &retry, &second, 0, 7),
        Err("RPC_ID_CONFLICT")
    );
    assert!(!serde_json::to_string(&model)
        .unwrap()
        .contains(ticket.for_injection()));
    assert!(!format!("{ticket:?}").contains(ticket.for_injection()));
    assert_eq!(serde_json::to_value(&ticket).unwrap(), json!("[redacted]"));
}
#[test]
fn status_only_retry_never_inherits_answers_owner_or_native_cancellation_effects() {
    let (mut model, candidate) = setup();
    let ticket = issue(&mut model, &candidate, 0);
    activate(&mut model, &candidate, &ticket, 1).unwrap();
    let submission = entered(consume(&mut model, &candidate, &ticket, 0, 2).unwrap());
    let second = retry(&candidate, "second");
    let second_ticket = issue(&mut model, &second, 3);
    assert!(matches!(
        activate(&mut model, &second, &second_ticket, 4).unwrap(),
        Response::Duplicate { .. }
    ));
    model
        .execute(
            Command::TerminateTicketCall {
                binding: second.binding,
                evidence: NativeEvidence::BlockedObserved,
            },
            5,
        )
        .unwrap();
    assert_eq!(
        model.request(&submission.request_id).unwrap().phase,
        RequestPhase::Pending
    );
    let third = retry(&candidate, "third");
    let third_ticket = issue(&mut model, &third, 6);
    activate(&mut model, &third, &third_ticket, 7).unwrap();
    let response = consume(&mut model, &third, &third_ticket, 1, 8).unwrap();
    assert!(matches!(response, Response::Duplicate { .. }));
    model
        .execute(
            Command::Decide {
                submission: submission.clone(),
                decision: Decision::Plan {
                    action: PlanAction::Approved,
                    feedback: "private answer".into(),
                },
            },
            9,
        )
        .unwrap();
    let fourth = retry(&candidate, "fourth");
    let fourth_ticket = issue(&mut model, &fourth, 10);
    activate(&mut model, &fourth, &fourth_ticket, 11).unwrap();
    let response = consume(&mut model, &fourth, &fourth_ticket, 2, 12).unwrap();
    assert!(!serde_json::to_string(&response)
        .unwrap()
        .contains("private answer"));
    assert_eq!(model.request_count(), 1);
    assert_eq!(
        model.execute(
            Command::Claim {
                request_id: submission.request_id.clone(),
                scope: candidate.binding.scope.clone(),
                owner: Owner {
                    connection_id: "connection".into(),
                    rpc_id: RpcId::Number(1)
                }
            },
            13
        ),
        Err("NOT_RESULT_OWNER")
    );
    assert!(matches!(
        model
            .execute(
                Command::Claim {
                    request_id: submission.request_id,
                    scope: candidate.binding.scope,
                    owner: Owner {
                        connection_id: "connection".into(),
                        rpc_id: RpcId::Number(0)
                    }
                },
                14
            )
            .unwrap(),
        Response::Claimed { .. }
    ));
}
#[test]
fn native_termination_expires_the_original_gate_and_candidate_cannot_revive() {
    let (mut model, candidate) = setup();
    let ticket = issue(&mut model, &candidate, 0);
    let Response::Registered { submission } = activate(&mut model, &candidate, &ticket, 1).unwrap()
    else {
        panic!()
    };
    model
        .execute(
            Command::TerminateTicketCall {
                binding: candidate.binding.clone(),
                evidence: NativeEvidence::CancelledObserved,
            },
            2,
        )
        .unwrap();
    assert_eq!(
        consume(&mut model, &candidate, &ticket, 0, 3),
        Err("TICKET_REVOKED")
    );
    assert_eq!(
        model.request(&submission.request_id).unwrap().phase,
        RequestPhase::Cancelled
    );
    assert_eq!(
        model.gate(&candidate.binding.scope, false),
        Gate::NormalPolicy
    );
}
#[test]
fn consumed_ticket_ttl_does_not_truncate_human_wait_and_restart_drops_all_tickets() {
    let (mut model, candidate) = setup();
    let ticket = issue(&mut model, &candidate, 0);
    activate(&mut model, &candidate, &ticket, 500_000).unwrap();
    let submission = entered(consume(&mut model, &candidate, &ticket, 0, 559_000).unwrap());
    model.execute(Command::Tick, 600_001).unwrap();
    assert_eq!(
        model.request(&submission.request_id).unwrap().phase,
        RequestPhase::Pending
    );
    let mut recovered = Model::recover(model, "installation").unwrap();
    assert_eq!(
        consume(&mut recovered, &candidate, &ticket, 0, 1),
        Err("TICKET_UNKNOWN")
    );
    assert_eq!(
        recovered.request(&submission.request_id).unwrap().phase,
        RequestPhase::Disconnected
    );
}
#[test]
fn project_replacement_after_entry_rejects_human_submission_and_lease_claim() {
    let (mut model, candidate) = setup();
    let ticket = issue(&mut model, &candidate, 0);
    activate(&mut model, &candidate, &ticket, 1).unwrap();
    let submission = entered(consume(&mut model, &candidate, &ticket, 0, 2).unwrap());
    let path = PathBuf::from(&candidate.cwd);
    let renamed = path.with_extension("renamed");
    fs::rename(&path, &renamed).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(model
        .execute(
            Command::Decide {
                submission,
                decision: Decision::Plan {
                    action: PlanAction::Approved,
                    feedback: String::new()
                }
            },
            3
        )
        .is_err());
    assert_eq!(
        model.gate(&candidate.binding.scope, false),
        Gate::Unavailable
    );
    let (mut model, candidate) = setup();
    let ticket = issue(&mut model, &candidate, 0);
    activate(&mut model, &candidate, &ticket, 1).unwrap();
    let submission = entered(consume(&mut model, &candidate, &ticket, 0, 2).unwrap());
    model
        .execute(
            Command::Decide {
                submission: submission.clone(),
                decision: Decision::Plan {
                    action: PlanAction::Approved,
                    feedback: String::new(),
                },
            },
            3,
        )
        .unwrap();
    let path = PathBuf::from(&candidate.cwd);
    fs::rename(&path, path.with_extension("renamed")).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(model
        .execute(
            Command::Claim {
                request_id: submission.request_id,
                scope: candidate.binding.scope,
                owner: Owner {
                    connection_id: "connection".into(),
                    rpc_id: RpcId::Number(0)
                }
            },
            4
        )
        .is_err());
}

#[test]
fn entry_expiry_and_same_rpc_changed_payload_cannot_reactivate_or_replace_a_request() {
    let (mut model, candidate) = setup();
    let ticket = issue(&mut model, &candidate, 0);
    activate(&mut model, &candidate, &ticket, 1).unwrap();
    assert_eq!(
        consume(&mut model, &candidate, &ticket, 0, 60_001),
        Err("TICKET_EXPIRED")
    );
    assert_eq!(
        model.gate(&candidate.binding.scope, false),
        Gate::NormalPolicy
    );
    let (mut model, candidate) = setup();
    let ticket = issue(&mut model, &candidate, 0);
    activate(&mut model, &candidate, &ticket, 1).unwrap();
    consume(&mut model, &candidate, &ticket, 0, 2).unwrap();
    let mut changed = candidate.clone();
    let mut value = serde_json::to_value(&candidate.payload).unwrap()["business"].clone();
    value["markdown"] = json!("changed");
    changed.payload =
        ToolRequest::parse(REVIEW_PLAN, &serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(
        consume(&mut model, &changed, &ticket, 0, 3),
        Err("RPC_ID_CONFLICT")
    );
    assert_eq!(model.request_count(), 1);
}

#[test]
fn durable_store_commits_ticket_and_registration_together_and_restart_removes_authority() {
    let (_, mut candidate) = setup();
    let path = PathBuf::from(&candidate.cwd).join("store");
    let mut store = crate::store::Store::open(&path, "installation").unwrap();
    store
        .execute(Command::SetProjectPath {
            path: PathBuf::from(&candidate.cwd),
            enabled: true,
        })
        .unwrap();
    store
        .execute(Command::ConfirmGeneration {
            root: "root".into(),
            generation: "generation".into(),
        })
        .unwrap();
    let key = crate::projects::resolve(&PathBuf::from(&candidate.cwd))
        .unwrap()
        .identity
        .key();
    candidate.binding.scope = store.model().scope("root", "generation", &[&key]).unwrap();
    let Response::CandidateIssued { ticket } = store
        .execute(Command::IssueTicket {
            candidate: Box::new(candidate.clone()),
        })
        .unwrap()
    else {
        panic!()
    };
    let Response::Registered { submission } = store
        .execute(Command::ActivateTicket {
            ticket: ticket.clone(),
            binding: candidate.binding.clone(),
            server: candidate.server.clone(),
            payload: candidate.payload.clone(),
            roots: candidate.roots.clone(),
            cwd: candidate.cwd.clone(),
        })
        .unwrap()
    else {
        panic!()
    };
    let bytes = fs::read(path.join("state.json")).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains(ticket.for_injection()));
    let saved: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(saved["tickets"][ticket.hash()]["phase"], "active");
    assert_eq!(
        saved["requests"][&submission.request_id]["phase"],
        "awaitingMcpEntry"
    );
    drop(store);
    let store = crate::store::Store::open(&path, "installation").unwrap();
    assert_eq!(
        store.model().request(&submission.request_id).unwrap().phase,
        RequestPhase::Disconnected
    );
    assert!(serde_json::to_value(store.model()).unwrap()["tickets"]
        .as_object()
        .unwrap()
        .is_empty());
}
