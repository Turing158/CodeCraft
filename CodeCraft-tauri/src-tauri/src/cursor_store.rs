//! Single-writer persistence for the isolated P1 model in an owned, private
//! directory. Importing this module does not create a production broker.
use crate::{
    files, protocol,
    workflow::{Command, Gate, Model, Response, Result, Scope},
};
use std::{
    fs::File,
    path::{Path, PathBuf},
    time::Instant,
};

pub struct Store {
    root: PathBuf,
    _lock: File,
    model: Model,
    started: Instant,
    unavailable: bool,
    revision: u64,
}

impl Store {
    pub fn open(root: &Path, installation_id: &str) -> Result<Self> {
        let root = files::private_directory(root)?;
        let lock_path = files::child(&root, "broker.lock")?;
        let lock = files::lock(&lock_path).map_err(|error| match error {
            "FILE_ALREADY_LOCKED" => "BROKER_ALREADY_RUNNING",
            _ => "BROKER_LOCK_UNAVAILABLE",
        })?;
        let path = files::child(&root, "state.json")?;
        let model = match File::open(&path) {
            Ok(file) => {
                let value = protocol::read_input(file).map_err(|_| "STATE_CORRUPT")?;
                let saved = serde_json::from_value(value).map_err(|_| "STATE_CORRUPT")?;
                Model::recover(saved, installation_id)?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Model::new(installation_id)?
            }
            Err(_) => return Err("STATE_UNREADABLE"),
        };
        let mut store = Self {
            root,
            _lock: lock,
            model: model.clone(),
            started: Instant::now(),
            unavailable: false,
            revision: 0,
        };
        // Publish the new epoch before accepting any command, including on the
        // first run. Orphan temporary files are never treated as committed data.
        store.commit(model)?;
        Ok(store)
    }
    /// Read-only inspection, not a current authorization check. Use `gate` to
    /// account for elapsed deadlines and store failure before a policy decision.
    pub fn model(&self) -> &Model {
        &self.model
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn gate(&mut self, scope: &Scope, verified_read_only: bool) -> Gate {
        if self.execute(Command::Tick).is_err() {
            return Gate::Unavailable;
        }
        self.model.gate(scope, verified_read_only)
    }
    pub fn is_available(&self) -> bool {
        !self.unavailable
    }
    pub fn execute(&mut self, command: Command) -> Result<Response> {
        if self.unavailable {
            return Err("BROKER_UNAVAILABLE");
        }
        let mut candidate = self.model.clone();
        let result = candidate.execute(
            command,
            self.started.elapsed().as_millis().min(u64::MAX as u128) as u64,
        );
        // Even a rejected command may have observed deadline expiry. Publish
        // that transition before replying, and never acknowledge uncommitted work.
        if candidate != self.model {
            self.commit(candidate)?;
        }
        result
    }
    fn commit(&mut self, candidate: Model) -> Result<()> {
        let bytes = serde_json::to_vec(&candidate).map_err(|_| "STATE_ENCODE_FAILED")?;
        if bytes.len() > protocol::MAX_MESSAGE_BYTES {
            self.unavailable = true;
            return Err("STATE_BACKPRESSURE");
        }
        if let Err(error) = files::child(&self.root, "state.json")
            .and_then(|path| files::atomic_write(&path, &bytes, true))
        {
            // If a replacement failed after some filesystem work, its outcome
            // may be uncertain. Require reopen/new epoch instead of proceeding
            // with potentially divergent memory and disk authority.
            self.unavailable = true;
            return Err(error);
        }
        self.model = candidate;
        self.revision += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        schema::{ToolRequest, REVIEW_PLAN},
        workflow::*,
    };
    use serde_json::json;
    use std::{fs, process::Command as ProcessCommand};
    use uuid::Uuid;

    fn root() -> PathBuf {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("runs")
            .join(format!("state-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        files::private_directory(&root).unwrap()
    }
    fn populate(store: &mut Store) -> Submission {
        store
            .execute(Command::SetProject {
                project_key: "project".into(),
                enabled: true,
            })
            .unwrap();
        store
            .execute(Command::ConfirmGeneration {
                root: "conversation".into(),
                generation: "generation".into(),
            })
            .unwrap();
        let scope = store
            .model()
            .scope("conversation", "generation", &["project"])
            .unwrap();
        let payload = ToolRequest::parse(REVIEW_PLAN, &serde_json::to_vec(&json!({"schemaVersion":1,"clientRequestId":"a","planId":"p","baseRevision":0,"title":"Plan","overview":"","markdown":"Review me","todos":[],"phases":[]})).unwrap()).unwrap();
        let response = store
            .execute(Command::Activate {
                binding: CallBinding {
                    scope,
                    call: NativeCall {
                        conversation_id: "conversation".into(),
                        tool_use_id: "native-call".into(),
                    },
                },
                payload,
                issued_at_ms: 0,
                client_deadline_ms: 1_000_000,
            })
            .unwrap();
        let Response::Registered { submission } = response else {
            panic!()
        };
        submission
    }
    #[test]
    fn one_live_broker_owns_the_directory_and_reopen_starts_a_new_epoch() {
        let root = root();
        let mut first = Store::open(&root, "installation").unwrap();
        assert!(matches!(
            Store::open(&root, "installation"),
            Err("BROKER_ALREADY_RUNNING")
        ));
        let request = populate(&mut first);
        let epoch = first.model().app_epoch().to_string();
        drop(first);
        let reopened = Store::open(&root, "installation").unwrap();
        assert_ne!(reopened.model().app_epoch(), epoch);
        assert_eq!(
            reopened.model().request(&request.request_id).unwrap().phase,
            Phase::Disconnected
        );
        drop(reopened);
        let before = fs::read(root.join("state.json")).unwrap();
        assert!(matches!(
            Store::open(&root, "other-installation"),
            Err("INVALID_SAVED_STATE")
        ));
        assert_eq!(fs::read(root.join("state.json")).unwrap(), before);
    }
    #[test]
    fn corrupt_or_oversized_committed_state_is_not_silently_reset() {
        for bytes in [
            b"{bad".to_vec(),
            vec![b' '; protocol::MAX_MESSAGE_BYTES + 1],
        ] {
            let root = root();
            fs::write(root.join("state.json"), &bytes).unwrap();
            assert!(matches!(
                Store::open(&root, "installation"),
                Err("STATE_CORRUPT")
            ));
            assert_eq!(fs::read(root.join("state.json")).unwrap(), bytes);
        }
    }
    #[test]
    fn partial_temporary_snapshot_does_not_override_last_committed_state() {
        let root = root();
        let mut store = Store::open(&root, "installation").unwrap();
        let request = populate(&mut store);
        drop(store);
        fs::write(root.join("orphan.tmp"), b"{partial").unwrap();
        let recovered = Store::open(&root, "installation").unwrap();
        assert_eq!(recovered.model().request_count(), 1);
        assert_eq!(
            recovered
                .model()
                .request(&request.request_id)
                .unwrap()
                .reason
                .as_deref(),
            Some("APP_RESTARTED")
        );
    }
    #[test]
    fn failed_disk_commit_returns_no_success_and_disables_further_commands() {
        let root = root();
        let mut store = Store::open(&root, "installation").unwrap();
        let request = populate(&mut store);
        let scope = store
            .model()
            .request(&request.request_id)
            .unwrap()
            .binding
            .scope
            .clone();
        let before = store.model().clone();
        // A directory at the fixed snapshot target forces replacement failure
        // without affecting unrelated files or requiring a full test disk.
        fs::remove_file(root.join("state.json")).unwrap();
        fs::create_dir(root.join("state.json")).unwrap();
        assert!(store
            .execute(Command::SetProject {
                project_key: "project".into(),
                enabled: true
            })
            .is_err());
        assert_eq!(store.model(), &before);
        assert!(!store.is_available());
        assert_eq!(store.gate(&scope, false), Gate::Unavailable);
        assert_eq!(store.execute(Command::Tick), Err("BROKER_UNAVAILABLE"));
    }
    #[test]
    fn state_capacity_never_evicts_pending_work_to_accept_more() {
        let root = root();
        let mut store = Store::open(&root, "installation").unwrap();
        let request = populate(&mut store);
        let before = store.model().clone();
        let mut value = serde_json::to_value(&before).unwrap();
        // Fill a harmless metadata map to test the persistence limit; the
        // original pending request and committed bytes must stay untouched.
        for i in 0..20000 {
            value["projects"][format!("long-project-key-{i:0128}")] =
                json!({"revision":1,"enabled":false});
        }
        let candidate: Model = serde_json::from_value(value).unwrap();
        assert_eq!(store.commit(candidate), Err("STATE_BACKPRESSURE"));
        assert_eq!(store.model(), &before);
        assert_eq!(
            store.model().request(&request.request_id).unwrap().phase,
            Phase::AwaitingMcpEntry
        );
        assert_eq!(
            serde_json::from_slice::<Model>(&fs::read(root.join("state.json")).unwrap()).unwrap(),
            before
        );
    }
    #[test]
    fn writer_process_helper() {
        let Some(root) = std::env::var_os("CODECRAFT_CURSOR_STATE_TEST_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        let mut store = Store::open(&root, "installation").unwrap();
        let submission = populate(&mut store);
        let binding = store
            .model()
            .request(&submission.request_id)
            .unwrap()
            .binding
            .clone();
        let owner = Owner {
            connection_id: "connection".into(),
            rpc_id: RpcId::Number(0),
        };
        let Response::Entered { submission } = store
            .execute(Command::Enter {
                request_id: submission.request_id.clone(),
                binding: binding.clone(),
                owner: owner.clone(),
                client_deadline_ms: 999999,
            })
            .unwrap()
        else {
            panic!()
        };
        store
            .execute(Command::Decide {
                submission: submission.clone(),
                decision: crate::schema::Decision::Plan {
                    action: crate::schema::PlanAction::Approved,
                    feedback: String::new(),
                },
            })
            .unwrap();
        store
            .execute(Command::Claim {
                request_id: submission.request_id,
                scope: binding.scope,
                owner,
            })
            .unwrap();
        // Exit without Drop, releasing the OS lock but not producing a fake
        // successful delivery report for the durable claimed decision.
        std::process::exit(17);
    }
    #[test]
    fn process_exit_after_claim_recovers_unknown_without_replaying_approval() {
        let root = root();
        let mut command = ProcessCommand::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "store::tests::writer_process_helper",
                "--test-threads=1",
            ])
            .env("CODECRAFT_CURSOR_STATE_TEST_ROOT", &root);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let output = command.output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(17),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let saved: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("state.json")).unwrap()).unwrap();
        let request_id = saved["requests"]
            .as_object()
            .unwrap()
            .keys()
            .next()
            .unwrap();
        assert_eq!(saved["requests"][request_id]["transport"], "claimed");
        let recovered = Store::open(&root, "installation").unwrap();
        let request = recovered.model().request(request_id).unwrap();
        assert_eq!(request.transport, Transport::OutcomeUnknown);
        assert_eq!(request.phase, Phase::Disconnected);
        assert!(request.decision.is_some());
        assert_ne!(
            recovered.model().app_epoch(),
            saved["appEpoch"].as_str().unwrap()
        );
    }
}
