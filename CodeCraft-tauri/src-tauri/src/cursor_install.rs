//! Recoverable file transactions for the Cursor installer. This module does
//! not select user files, register handlers, or grant runtime capabilities.
//! Targets and expected hashes come from a reviewed desktop installation plan.
use crate::{files, projects, protocol, schema::Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};
use uuid::Uuid;

const MAX_STEPS: usize = 32;

#[derive(Clone, Debug)]
pub struct Change {
    pub target: PathBuf,
    pub before_hash: Option<String>,
    pub after: Option<Vec<u8>>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Prepared,
    Writing,
    Verifying,
    Committed,
    RollbackRequired,
    RolledBack,
    Conflict,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Step {
    pub target: PathBuf,
    pub before_hash: Option<String>,
    pub after_hash: Option<String>,
    pub backup: Option<PathBuf>,
    parent_identity: projects::Identity,
    after_file: Option<String>,
    completed: bool,
    restored: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Journal {
    schema_version: u32,
    pub operation_id: String,
    pub installation_id: String,
    pub config_revision: u64,
    pub phase: Phase,
    pub steps: Vec<Step>,
    pub conflicts: Vec<PathBuf>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub operation_id: String,
    pub installation_id: String,
    pub config_revision: u64,
}

pub struct Transaction {
    root: PathBuf,
    _lock: File,
    journal: Journal,
    unavailable: bool,
}

fn bounded_bytes(path: &Path) -> Result<Option<Vec<u8>>> {
    files::reject_link_path(path)?;
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("INSTALL_FILE_UNREADABLE"),
    };
    if !file
        .metadata()
        .map_err(|_| "INSTALL_FILE_UNREADABLE")?
        .is_file()
    {
        return Err("INSTALL_TARGET_NOT_FILE");
    }
    let mut bytes = Vec::new();
    file.take(protocol::MAX_MESSAGE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "INSTALL_FILE_UNREADABLE")?;
    if bytes.len() > protocol::MAX_MESSAGE_BYTES {
        return Err("INSTALL_FILE_TOO_LARGE");
    }
    Ok(Some(bytes))
}
pub fn file_hash(path: &Path) -> Result<Option<String>> {
    Ok(bounded_bytes(path)?.as_deref().map(protocol::sha256))
}
fn save(root: &Path, journal: &Journal) -> Result<()> {
    files::atomic_json(
        &files::child(root, "transaction.json")?,
        &serde_json::to_value(journal).map_err(|_| "INSTALL_ENCODE_FAILED")?,
        true,
    )
}

impl Transaction {
    pub fn prepare(
        root: &Path,
        installation_id: &str,
        config_revision: u64,
        changes: Vec<Change>,
    ) -> Result<Self> {
        if !crate::schema::valid_id(installation_id)
            || config_revision == 0
            || changes.is_empty()
            || changes.len() > MAX_STEPS
        {
            return Err("INVALID_INSTALL_PLAN");
        }
        let root = files::private_directory(root)?;
        let lock = files::lock(&files::child(&root, "install.lock")?)?;
        let journal_path = files::child(&root, "transaction.json")?;
        if journal_path.exists() {
            let previous: Journal = serde_json::from_value(files::read_json(&journal_path)?)
                .map_err(|_| "INSTALL_JOURNAL_CORRUPT")?;
            if previous.schema_version != 1
                || previous.steps.is_empty()
                || previous.steps.len() > MAX_STEPS
                || uuid::Uuid::parse_str(&previous.operation_id).is_err()
            {
                return Err("INSTALL_JOURNAL_CORRUPT");
            }
            if !matches!(previous.phase, Phase::Committed | Phase::RolledBack) {
                return Err("INSTALL_RECOVERY_REQUIRED");
            }
            if config_revision <= previous.config_revision
                || installation_id != previous.installation_id
            {
                return Err("INSTALL_REVISION_CONFLICT");
            }
        }
        let operation_id = Uuid::new_v4().to_string();
        let mut names = BTreeSet::new();
        let mut staged = Vec::new();
        // Validate all targets and outputs before creating backups or a journal.
        for change in changes {
            if !change.target.is_absolute() {
                return Err("INVALID_INSTALL_TARGET");
            }
            files::reject_link_path(&change.target)?;
            let parent =
                projects::resolve(change.target.parent().ok_or("INVALID_INSTALL_TARGET")?)?;
            let target = files::child(
                &parent.final_path,
                change
                    .target
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or("INVALID_INSTALL_TARGET")?,
            )?;
            if target.starts_with(&root) || !names.insert(target.to_string_lossy().to_lowercase()) {
                return Err("INVALID_INSTALL_TARGET");
            }
            let before = bounded_bytes(&target)?;
            if before.as_deref().map(protocol::sha256) != change.before_hash {
                return Err("INSTALL_FILE_CONFLICT");
            }
            if change
                .after
                .as_ref()
                .is_some_and(|bytes| bytes.len() > protocol::MAX_MESSAGE_BYTES)
            {
                return Err("INSTALL_FILE_TOO_LARGE");
            }
            staged.push((target, parent.identity, before, change.after));
        }
        let mut steps = Vec::new();
        for (index, (target, parent_identity, before, after)) in staged.into_iter().enumerate() {
            let backup = if let Some(bytes) = &before {
                let name = target
                    .file_name()
                    .ok_or("INVALID_INSTALL_TARGET")?
                    .to_string_lossy();
                let backup = target.with_file_name(format!("{name}.{operation_id}.bak"));
                files::atomic_write(&backup, bytes, false)?;
                Some(backup)
            } else {
                None
            };
            let after_file = if let Some(bytes) = &after {
                let name = format!("{operation_id}-{index}.stage");
                files::atomic_write(&files::child(&root, &name)?, bytes, false)?;
                Some(name)
            } else {
                None
            };
            steps.push(Step {
                target,
                parent_identity,
                backup,
                before_hash: before.as_deref().map(protocol::sha256),
                after_hash: after.as_deref().map(protocol::sha256),
                after_file,
                completed: false,
                restored: false,
            });
        }
        let journal = Journal {
            schema_version: 1,
            operation_id,
            installation_id: installation_id.into(),
            config_revision,
            phase: Phase::Prepared,
            steps,
            conflicts: Vec::new(),
        };
        save(&root, &journal)?;
        Ok(Self {
            root,
            _lock: lock,
            journal,
            unavailable: false,
        })
    }
    pub fn open(root: &Path) -> Result<Self> {
        let root = files::private_directory(root)?;
        let lock = files::lock(&files::child(&root, "install.lock")?)?;
        let journal: Journal =
            serde_json::from_value(files::read_json(&files::child(&root, "transaction.json")?)?)
                .map_err(|_| "INSTALL_JOURNAL_CORRUPT")?;
        if journal.schema_version != 1
            || journal.steps.is_empty()
            || journal.steps.len() > MAX_STEPS
            || Uuid::parse_str(&journal.operation_id).is_err()
            || !crate::schema::valid_id(&journal.installation_id)
            || journal.config_revision == 0
        {
            return Err("INSTALL_JOURNAL_CORRUPT");
        }
        for (index, step) in journal.steps.iter().enumerate() {
            let expected = format!("{}-{index}.stage", journal.operation_id);
            if !step.target.is_absolute()
                || step.target.starts_with(&root)
                || step
                    .after_file
                    .as_ref()
                    .is_some_and(|name| name != &expected)
                || step.after_hash.is_some() != step.after_file.is_some()
                || step.before_hash.is_some() != step.backup.is_some()
            {
                return Err("INSTALL_JOURNAL_CORRUPT");
            }
            if let Some(backup) = &step.backup {
                let name = step
                    .target
                    .file_name()
                    .ok_or("INSTALL_JOURNAL_CORRUPT")?
                    .to_string_lossy();
                if *backup
                    != step
                        .target
                        .with_file_name(format!("{name}.{}.bak", journal.operation_id))
                {
                    return Err("INSTALL_JOURNAL_CORRUPT");
                }
            }
        }
        Ok(Self {
            root,
            _lock: lock,
            journal,
            unavailable: false,
        })
    }
    pub fn journal(&self) -> &Journal {
        &self.journal
    }
    fn available(&self) -> Result<()> {
        if self.unavailable {
            Err("INSTALL_STORAGE_UNAVAILABLE")
        } else {
            Ok(())
        }
    }
    fn persist(&mut self) -> Result<()> {
        let result = save(&self.root, &self.journal);
        if result.is_err() {
            self.unavailable = true;
        }
        result
    }
    fn manifest(&self) -> Manifest {
        Manifest {
            schema_version: 1,
            operation_id: self.journal.operation_id.clone(),
            installation_id: self.journal.installation_id.clone(),
            config_revision: self.journal.config_revision,
        }
    }
    fn manifest_matches(&self) -> Result<bool> {
        let path = files::child(&self.root, "manifest.json")?;
        if !path.exists() {
            return Ok(false);
        }
        let manifest: Manifest = serde_json::from_value(files::read_json(&path)?)
            .map_err(|_| "INSTALL_MANIFEST_CORRUPT")?;
        Ok(manifest == self.manifest())
    }
    fn verify_parent(step: &Step) -> Result<()> {
        files::reject_link_path(&step.target)?;
        if projects::resolve(step.target.parent().ok_or("INVALID_INSTALL_TARGET")?)?.identity
            != step.parent_identity
        {
            return Err("INSTALL_PARENT_CHANGED");
        }
        Ok(())
    }
    fn write_step(&self, index: usize) -> Result<()> {
        let step = &self.journal.steps[index];
        Self::verify_parent(step)?;
        if file_hash(&step.target)? != step.before_hash {
            return Err("INSTALL_FILE_CONFLICT");
        }
        match &step.after_file {
            Some(name) => {
                let bytes = bounded_bytes(&files::child(&self.root, name)?)?
                    .ok_or("INSTALL_STAGE_MISSING")?;
                if Some(protocol::sha256(&bytes)) != step.after_hash {
                    return Err("INSTALL_STAGE_CHANGED");
                }
                // Recheck after reading the stage. Cross-process user writes
                // are not a multi-file transaction; recovery never overwrites
                // content that no longer matches our recorded output.
                if file_hash(&step.target)? != step.before_hash {
                    return Err("INSTALL_FILE_CONFLICT");
                }
                files::atomic_write(&step.target, &bytes, step.before_hash.is_some())?;
            }
            None => {
                if step.before_hash.is_some() {
                    fs::remove_file(&step.target).map_err(|_| "INSTALL_DELETE_FAILED")?;
                }
            }
        }
        Ok(())
    }
    /// Incremental steps permit a UI to show durable progress. On restart,
    /// use recover, never blindly resume a partially written installation.
    pub fn advance(&mut self) -> Result<Phase> {
        self.available()?;
        match self.journal.phase {
            Phase::Prepared => {
                self.journal.phase = Phase::Writing;
                self.persist()?;
            }
            Phase::Writing => {
                if let Some(index) = self.journal.steps.iter().position(|step| !step.completed) {
                    if let Err(error) = self.write_step(index) {
                        self.journal.phase = Phase::RollbackRequired;
                        self.persist()?;
                        return Err(error);
                    }
                    self.journal.steps[index].completed = true;
                    self.persist()?;
                } else {
                    self.journal.phase = Phase::Verifying;
                    self.persist()?;
                }
            }
            Phase::Verifying => return Err("INSTALL_READINESS_REQUIRED"),
            Phase::Committed | Phase::RolledBack => {}
            _ => return Err("INSTALL_RECOVERY_REQUIRED"),
        }
        Ok(self.journal.phase.clone())
    }
    fn verify_outputs(&self) -> Result<()> {
        for step in &self.journal.steps {
            Self::verify_parent(step)?;
            if file_hash(&step.target)? != step.after_hash {
                return Err("INSTALL_FILE_CONFLICT");
            }
        }
        Ok(())
    }
    /// The trusted caller verifies helper binaries, structured configuration
    /// and local readiness here. A committed installation still needs a fresh
    /// compatibility evaluation at every runtime handshake.
    pub fn commit(
        &mut self,
        verify_readiness: impl FnOnce(&[Step]) -> Result<()>,
    ) -> Result<Manifest> {
        self.available()?;
        if self.journal.phase != Phase::Verifying {
            return Err("INSTALL_NOT_READY");
        }
        if let Err(error) = self
            .verify_outputs()
            .and_then(|()| verify_readiness(&self.journal.steps))
            .and_then(|()| self.verify_outputs())
        {
            self.journal.phase = Phase::RollbackRequired;
            self.persist()?;
            return Err(error);
        }
        let manifest = self.manifest();
        let path = files::child(&self.root, "manifest.json")?;
        // Preserve the previous manifest for repair/audit, like external files.
        if let Some(bytes) = bounded_bytes(&path)? {
            files::atomic_write(
                &files::child(
                    &self.root,
                    &format!("manifest.{}.bak", self.journal.operation_id),
                )?,
                &bytes,
                false,
            )?;
        }
        files::atomic_json(
            &path,
            &serde_json::to_value(&manifest).map_err(|_| "INSTALL_ENCODE_FAILED")?,
            true,
        )?;
        self.journal.phase = Phase::Committed;
        self.persist()?;
        Ok(manifest)
    }
    /// The manifest is the activation commit point. If it was not reached,
    /// restore only unchanged installer output, in reverse order. A conflict
    /// remains explicit and can be retried after a reviewed user resolution.
    pub fn recover(&mut self) -> Result<Phase> {
        self.available()?;
        if self.manifest_matches()? {
            if self.verify_outputs().is_ok() {
                self.journal.phase = Phase::Committed;
            } else {
                self.journal.phase = Phase::Conflict;
            }
            self.persist()?;
            return Ok(self.journal.phase.clone());
        }
        if self.journal.phase == Phase::Committed {
            return Err("INSTALL_MANIFEST_CONFLICT");
        }
        self.journal.phase = Phase::RollbackRequired;
        self.journal.conflicts.clear();
        self.persist()?;
        for index in (0..self.journal.steps.len()).rev() {
            let step = &self.journal.steps[index];
            let result = (|| {
                Self::verify_parent(step)?;
                let current = file_hash(&step.target)?;
                if current == step.before_hash {
                    return Ok(());
                }
                if current != step.after_hash {
                    return Err("INSTALL_FILE_CONFLICT");
                }
                if let Some(backup) = &step.backup {
                    let bytes = bounded_bytes(backup)?.ok_or("INSTALL_BACKUP_MISSING")?;
                    if Some(protocol::sha256(&bytes)) != step.before_hash {
                        return Err("INSTALL_BACKUP_CHANGED");
                    }
                    if file_hash(&step.target)? != step.after_hash {
                        return Err("INSTALL_FILE_CONFLICT");
                    }
                    files::atomic_write(&step.target, &bytes, current.is_some())?;
                } else if current.is_some() {
                    fs::remove_file(&step.target).map_err(|_| "INSTALL_DELETE_FAILED")?;
                }
                Ok(())
            })();
            if result.is_err() {
                self.journal.conflicts.push(step.target.clone());
            } else {
                self.journal.steps[index].restored = true;
            }
            self.persist()?;
        }
        self.journal.phase = if self.journal.conflicts.is_empty() {
            Phase::RolledBack
        } else {
            Phase::Conflict
        };
        self.persist()?;
        Ok(self.journal.phase.clone())
    }
    pub fn active_manifest(&self) -> Result<Manifest> {
        self.available()?;
        if self.journal.phase != Phase::Committed || !self.manifest_matches()? {
            return Err("INSTALL_INCOMPLETE");
        }
        self.verify_outputs()?;
        Ok(self.manifest())
    }
}

/// Compare-and-swap only one owned MCP registration, preserving every other
/// field. A matching name alone is not ownership; expected is the exact entry
/// last committed by this installation. None means create only if absent.
pub fn merge_mcp(
    bytes: &[u8],
    name: &str,
    expected: Option<&Value>,
    replacement: Option<Value>,
) -> Result<Vec<u8>> {
    let mut value = protocol::strict_json(bytes).map_err(|_| "INVALID_MCP_CONFIG")?;
    let object = value.as_object_mut().ok_or("INVALID_MCP_CONFIG")?;
    let servers = object
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or("INVALID_MCP_CONFIG")?;
    if servers.get(name) != expected {
        return Err("MCP_ENTRY_CONFLICT");
    }
    if let Some(replacement) = replacement {
        servers.insert(name.into(), replacement);
    } else {
        servers.remove(name);
    }
    serde_json::to_vec_pretty(&value).map_err(|_| "INSTALL_ENCODE_FAILED")
}

/// Structured CAS for exactly the Hook entries recorded by our prior install.
/// It never replaces another source's array or treats a command substring as
/// ownership. Unknown events and top-level settings survive the merge.
pub fn merge_hooks(
    bytes: &[u8],
    expected: &std::collections::BTreeMap<String, Vec<Value>>,
    replacement: &std::collections::BTreeMap<String, Vec<Value>>,
) -> Result<Vec<u8>> {
    let mut value = protocol::strict_json(bytes).map_err(|_| "INVALID_HOOK_CONFIG")?;
    let object = value.as_object_mut().ok_or("INVALID_HOOK_CONFIG")?;
    if object.get("version").is_some_and(|version| version != 1) {
        return Err("INVALID_HOOK_VERSION");
    }
    object.insert("version".into(), Value::from(1));
    let hooks = object
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or("INVALID_HOOK_CONFIG")?;
    let names = expected
        .keys()
        .chain(replacement.keys())
        .collect::<BTreeSet<_>>();
    for name in names {
        if !protocol::EVENTS.contains(&name.as_str()) {
            return Err("INVALID_HOOK_EVENT");
        }
        let mut handlers = match hooks.get(name) {
            Some(value) => value.as_array().ok_or("INVALID_HOOK_CONFIG")?.clone(),
            None => Vec::new(),
        };
        for old in expected.get(name).into_iter().flatten() {
            let matching = handlers
                .iter()
                .enumerate()
                .filter(|(_, value)| *value == old)
                .map(|(i, _)| i)
                .collect::<Vec<_>>();
            if matching.len() != 1 {
                return Err("HOOK_ENTRY_CONFLICT");
            }
            handlers.remove(matching[0]);
        }
        for new in replacement.get(name).into_iter().flatten() {
            if !new.is_object() || handlers.contains(new) {
                return Err("HOOK_ENTRY_CONFLICT");
            }
            handlers.push(new.clone());
        }
        hooks.insert(name.clone(), Value::Array(handlers));
    }
    serde_json::to_vec_pretty(&value).map_err(|_| "INSTALL_ENCODE_FAILED")
}

pub const PROJECT_RULE: &str = "---\ndescription: CodeCraft human questions and plan reviews\nalwaysApply: true\n---\n\nUse codecraft_ask_user for questions that require a human answer. Use codecraft_review_plan before implementing work that requires plan review. Wait for the current request's human result before proceeding.\n\nUse a fresh clientRequestId for each new question or review, including reviewing identical content again. For plan updates, keep planId, use the broker's current baseRevision and supersedesClientRequestId, and send the complete plan. An exact retry with the same clientRequestId returns status only and does not grant a new approval.\n\nIf a call expires, disconnects or reports an error, describe the interruption. Do not treat it as approval. Ask again with a new request when human confirmation is required. A plan approval applies only to that reviewed version and does not operate Cursor's native Build control.\n";

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn hook_merge_changes_only_owned_entries_and_rejects_modified_or_duplicate_ownership() {
        let third = json!({"command":"other.exe", "timeout":10});
        let old = json!({"command":"codecraft.exe --cursor-hook", "failClosed":false});
        let new = json!({"command":"codecraft.exe --cursor-hook v2", "failClosed":false});
        let value = json!({"version":1,"setting":"keep","hooks":{"preToolUse":[third.clone(),old.clone()],"futureEvent":[{"custom":true}]}});
        let expected = std::collections::BTreeMap::from([("preToolUse".into(), vec![old.clone()])]);
        let replacement =
            std::collections::BTreeMap::from([("preToolUse".into(), vec![new.clone()])]);
        let updated: Value = serde_json::from_slice(
            &merge_hooks(
                &serde_json::to_vec(&value).unwrap(),
                &expected,
                &replacement,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(updated["hooks"]["preToolUse"], json!([third, new]));
        assert_eq!(
            updated["hooks"]["futureEvent"],
            value["hooks"]["futureEvent"]
        );
        assert_eq!(updated["setting"], "keep");
        let mut duplicate = value.clone();
        duplicate["hooks"]["preToolUse"]
            .as_array_mut()
            .unwrap()
            .push(old);
        assert!(merge_hooks(
            &serde_json::to_vec(&duplicate).unwrap(),
            &expected,
            &replacement
        )
        .is_err());
        assert!(merge_hooks(
            &serde_json::to_vec(&updated).unwrap(),
            &expected,
            &replacement
        )
        .is_err());
    }
    #[test]
    fn journal_write_failure_stops_the_instance_before_more_targets_can_change() {
        let (root, changes) = setup();
        let mut tx = Transaction::prepare(&root, "installation", 1, changes.clone()).unwrap();
        fs::remove_file(root.join("transaction.json")).unwrap();
        fs::create_dir(root.join("transaction.json")).unwrap();
        assert!(tx.advance().is_err());
        assert_eq!(tx.advance(), Err("INSTALL_STORAGE_UNAVAILABLE"));
        assert_eq!(tx.recover(), Err("INSTALL_STORAGE_UNAVAILABLE"));
        assert!(tx.active_manifest().is_err());
        for change in changes {
            assert_eq!(file_hash(&change.target).unwrap(), change.before_hash);
        }
    }
    fn setup() -> (PathBuf, Vec<Change>) {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("runs")
            .join(format!("install-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let a = root.join("a.json");
        let b = root.join("b.json");
        let c = root.join("c.json");
        fs::write(&a, b"original a").unwrap();
        fs::write(&c, b"original c").unwrap();
        (
            root.join("journal"),
            vec![
                Change {
                    target: a,
                    before_hash: Some(protocol::sha256(b"original a")),
                    after: Some(b"new a".to_vec()),
                },
                Change {
                    target: b,
                    before_hash: None,
                    after: Some(b"new b".to_vec()),
                },
                Change {
                    target: c,
                    before_hash: Some(protocol::sha256(b"original c")),
                    after: None,
                },
            ],
        )
    }
    #[test]
    fn interrupted_installation_rolls_back_at_every_durable_step_with_unique_backups() {
        for stop in 0..6 {
            let (root, changes) = setup();
            let mut tx = Transaction::prepare(&root, "installation", 1, changes.clone()).unwrap();
            for _ in 0..stop {
                tx.advance().unwrap();
            }
            drop(tx);
            let mut recovered = Transaction::open(&root).unwrap();
            assert_eq!(recovered.recover().unwrap(), Phase::RolledBack);
            for change in &changes {
                assert_eq!(file_hash(&change.target).unwrap(), change.before_hash);
            }
            for step in &recovered.journal.steps {
                if let Some(backup) = &step.backup {
                    assert_eq!(file_hash(backup).unwrap(), step.before_hash);
                    assert_eq!(backup.extension().unwrap(), "bak");
                }
            }
            assert!(recovered.active_manifest().is_err());
        }
    }
    #[test]
    fn crash_between_file_write_and_journal_update_is_recovered_by_hash() {
        let (root, changes) = setup();
        let mut tx = Transaction::prepare(&root, "installation", 1, changes.clone()).unwrap();
        tx.advance().unwrap();
        tx.write_step(0).unwrap();
        drop(tx);
        let mut tx = Transaction::open(&root).unwrap();
        assert!(!tx.journal.steps[0].completed);
        assert_eq!(tx.recover().unwrap(), Phase::RolledBack);
        assert_eq!(
            file_hash(&changes[0].target).unwrap(),
            changes[0].before_hash
        );
    }
    #[test]
    fn user_edits_and_changed_backups_are_never_overwritten_during_recovery() {
        let (root, changes) = setup();
        let mut tx = Transaction::prepare(&root, "installation", 1, changes.clone()).unwrap();
        tx.advance().unwrap();
        tx.advance().unwrap();
        tx.advance().unwrap();
        fs::write(&changes[1].target, "user edit").unwrap();
        fs::write(
            tx.journal.steps[0].backup.as_ref().unwrap(),
            "changed backup",
        )
        .unwrap();
        assert_eq!(tx.recover().unwrap(), Phase::Conflict);
        assert_eq!(tx.journal.conflicts.len(), 2);
        assert_eq!(fs::read(&changes[1].target).unwrap(), b"user edit");
        assert_eq!(fs::read(&changes[0].target).unwrap(), b"new a");
    }
    #[test]
    fn activation_is_last_and_restart_after_manifest_publication_preserves_commit() {
        let (root, changes) = setup();
        let mut tx = Transaction::prepare(&root, "installation", 1, changes).unwrap();
        while tx.advance().unwrap() != Phase::Verifying {}
        assert!(tx.active_manifest().is_err());
        let manifest = tx.commit(|_| Ok(())).unwrap();
        assert_eq!(tx.active_manifest().unwrap(), manifest);
        tx.journal.phase = Phase::Verifying;
        save(&tx.root, &tx.journal).unwrap();
        drop(tx);
        let mut tx = Transaction::open(&root).unwrap();
        assert_eq!(tx.recover().unwrap(), Phase::Committed);
        assert_eq!(tx.active_manifest().unwrap(), manifest);
        fs::write(&tx.journal.steps[0].target, "later user edit").unwrap();
        assert!(tx.active_manifest().is_err());
        assert_eq!(tx.recover().unwrap(), Phase::Conflict);
        assert_eq!(
            fs::read(&tx.journal.steps[0].target).unwrap(),
            b"later user edit"
        );
    }
    #[test]
    fn preparation_and_write_recheck_conflicts_and_readiness_failure_cannot_activate() {
        let (root, changes) = setup();
        let mut wrong = changes.clone();
        wrong[0].before_hash = None;
        assert!(Transaction::prepare(&root, "installation", 1, wrong).is_err());
        let mut tx = Transaction::prepare(&root, "installation", 1, changes.clone()).unwrap();
        tx.advance().unwrap();
        fs::write(&changes[0].target, "changed").unwrap();
        assert_eq!(tx.advance(), Err("INSTALL_FILE_CONFLICT"));
        assert!(tx.active_manifest().is_err());
        let (root, changes) = setup();
        let mut tx = Transaction::prepare(&root, "installation", 1, changes).unwrap();
        while tx.advance().unwrap() != Phase::Verifying {}
        assert_eq!(
            tx.commit(|_| Err("HELPER_NOT_READY")),
            Err("HELPER_NOT_READY")
        );
        assert!(tx.active_manifest().is_err());
        assert_eq!(tx.recover().unwrap(), Phase::RolledBack);
    }
    #[test]
    fn mcp_merge_preserves_unknown_configuration_and_rejects_name_collisions() {
        let original = json!({"other": [1,2], "mcpServers": {"thirdParty": {"url":"https://example.invalid"}}});
        let entry = json!({"command":"C:/CodeCraft.exe", "args":["--cursor-mcp"]});
        let merged = merge_mcp(
            &serde_json::to_vec(&original).unwrap(),
            "codecraft",
            None,
            Some(entry.clone()),
        )
        .unwrap();
        let value: Value = serde_json::from_slice(&merged).unwrap();
        assert_eq!(value["other"], original["other"]);
        assert_eq!(
            value["mcpServers"]["thirdParty"],
            original["mcpServers"]["thirdParty"]
        );
        assert!(merge_mcp(&merged, "codecraft", None, Some(entry.clone())).is_err());
        let removed: Value =
            serde_json::from_slice(&merge_mcp(&merged, "codecraft", Some(&entry), None).unwrap())
                .unwrap();
        assert_eq!(removed, original);
    }
}
