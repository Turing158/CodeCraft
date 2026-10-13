//! Owned project guidance. A rule cannot authorize a workflow. Every write uses
//! the existing recovery transaction with a unique same-directory .bak.
use crate::{files, install, projects, protocol, schema::Result};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

fn saved_model(root: &Path, installation_id: &str) -> Result<Option<crate::workflow::Model>> {
    let path = root.join("workflow/state.json");
    if !path.exists() {
        return Ok(None);
    }
    let model: crate::workflow::Model =
        serde_json::from_value(files::read_json(&path)?).map_err(|_| "STATE_CORRUPT")?;
    if model.installation_id() != installation_id {
        return Err("STATE_INSTALLATION_MISMATCH");
    }
    Ok(Some(model))
}

pub fn has_registered_projects(root: &Path, installation_id: &str) -> Result<bool> {
    Ok(saved_model(root, installation_id)?
        .is_some_and(|model| model.project_registry().entries().next().is_some()))
}

pub fn installation_intact(root: &Path, installation_id: &str) -> Result<bool> {
    let Some(model) = saved_model(root, installation_id)? else {
        return Ok(true);
    };
    for (_, project) in model
        .project_registry()
        .entries()
        .filter(|(_, p)| p.enabled)
    {
        let path = &project.directory.final_path;
        if !path.exists() {
            continue;
        }
        if projects::resolve(path)?.identity != project.directory.identity
            || status(path)["state"] != "installed"
        {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Called with the workflow writer lock held. Rules and project registrations
/// commit in the same recovery transaction as the Hook, MCP and enabled flag.
pub fn installation_changes(
    root: &Path,
    installation_id: &str,
    enabled: bool,
    discovered: Vec<PathBuf>,
) -> Result<Vec<install::Change>> {
    let state_path = root.join("workflow/state.json");
    let state_hash = install::file_hash(&state_path)?;
    let saved = saved_model(root, installation_id)?;
    let mut model = match saved {
        Some(model) => crate::workflow::Model::recover(model, installation_id)?,
        None => crate::workflow::Model::new(installation_id)?,
    };
    let previous_registry = model.project_registry().clone();
    let mut targets = BTreeMap::new();
    for (id, project) in previous_registry.entries() {
        if enabled && !project.enabled {
            continue;
        }
        let path = &project.directory.final_path;
        if !path.exists() {
            continue;
        }
        if projects::resolve(path)?.identity != project.directory.identity {
            return Err("PROJECT_SCOPE_CHANGED");
        }
        targets.insert(id.clone(), path.clone());
    }
    if enabled {
        for path in discovered {
            let directory = projects::resolve(&path)?;
            let key = directory.identity.key();
            if previous_registry.project(&key).is_some_and(|p| !p.enabled) {
                continue;
            }
            targets.insert(key, directory.final_path);
        }
    }
    let mut changes = Vec::new();
    for path in targets.into_values() {
        let target = path.join(".cursor/rules/codecraft-workflow.mdc");
        files::reject_link_path(&target)?;
        let before = match fs::read(&target) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err("RULE_UNREADABLE"),
        };
        if before
            .as_ref()
            .is_some_and(|bytes| bytes != RULE.as_bytes())
        {
            return Err("RULE_CONFLICT");
        }
        if enabled {
            model.execute(
                crate::workflow::Command::SetProjectPath {
                    path,
                    enabled: true,
                },
                0,
            )?;
        }
        if before.is_some() != enabled {
            changes.push(install::Change {
                target,
                before_hash: before.as_deref().map(protocol::sha256),
                after: enabled.then(|| RULE.as_bytes().to_vec()),
            });
        }
    }
    if !enabled {
        model.remove_project_registrations()?;
    }
    if model.project_registry() != &previous_registry {
        changes.push(install::Change {
            target: state_path,
            before_hash: state_hash,
            after: Some(serde_json::to_vec(&model).map_err(|_| "STATE_ENCODE_FAILED")?),
        });
    }
    // Preflight all existing files before creating any missing rule directories.
    if enabled {
        for change in &changes {
            let parent = change.target.parent().ok_or("RULE_DIRECTORY_FAILED")?;
            fs::create_dir_all(parent).map_err(|_| "RULE_DIRECTORY_FAILED")?;
            files::reject_link_path(parent)?;
        }
    }
    Ok(changes)
}

/// Only the locally saved open windows are installation targets. Conversation
/// payloads, MCP arguments and historical workspace lists cannot select paths.
pub fn installation_projects(storage: &Path) -> Result<Vec<PathBuf>> {
    if !storage.exists() {
        return Ok(Vec::new());
    }
    let state = files::read_json(storage).map_err(|_| "CURSOR_WORKSPACES_UNAVAILABLE")?;
    let windows = &state["windowsState"];
    let mut paths = Vec::new();
    for window in std::iter::once(&windows["lastActiveWindow"])
        .chain(windows["openedWindows"].as_array().into_iter().flatten())
        .take(64)
    {
        if let Some(path) = window["folder"].as_str().and_then(local_path) {
            paths.push(path);
        }
        if let Some(workspace) = window["workspace"]["configPath"]
            .as_str()
            .and_then(local_path)
        {
            let Ok(document) = files::read_json(&workspace) else {
                continue;
            };
            let Some(parent) = workspace.parent() else {
                continue;
            };
            for folder in document["folders"]
                .as_array()
                .into_iter()
                .flatten()
                .take(64)
            {
                if let Some(path) = folder["path"].as_str() {
                    paths.push(parent.join(path));
                } else if let Some(path) = folder["uri"].as_str().and_then(local_path) {
                    paths.push(path);
                }
            }
        }
    }
    let mut directories = BTreeMap::new();
    for path in paths {
        // Reject missing directories, network workspaces and invalid paths;
        // aliases of one local directory are registered only once.
        if let Ok(directory) = projects::resolve(&path) {
            directories.insert(directory.identity.key(), directory.final_path);
        }
    }
    Ok(directories.into_values().collect())
}

fn local_path(value: &str) -> Option<PathBuf> {
    let uri = url::Url::parse(value).ok()?;
    if uri.scheme() != "file"
        || uri
            .host_str()
            .is_some_and(|host| !host.is_empty() && host != "localhost")
    {
        return None;
    }
    uri.to_file_path().ok()
}

pub fn install_for_projects(
    root: &Path,
    store: &mut crate::store::Store,
    discovered: Vec<PathBuf>,
) -> Result<()> {
    let registry = store.model().project_registry();
    let mut targets = BTreeMap::new();
    for (id, project) in registry.entries().filter(|(_, project)| project.enabled) {
        let path = &project.directory.final_path;
        if !path.exists() {
            continue;
        }
        if projects::resolve(path)?.identity != project.directory.identity {
            return Err("PROJECT_SCOPE_CHANGED");
        }
        targets.insert(id.clone(), path.clone());
    }
    for path in discovered {
        let directory = projects::resolve(&path)?;
        let id = directory.identity.key();
        // Keep an existing explicit opt-out across reinstalls.
        if registry
            .project(&id)
            .is_some_and(|project| !project.enabled)
        {
            continue;
        }
        targets.insert(id, directory.final_path);
    }
    for path in targets.into_values() {
        configure(root, &path, true)?;
        store.execute(crate::workflow::Command::SetProjectPath {
            path,
            enabled: true,
        })?;
    }
    Ok(())
}

const RULE: &str = "---\ndescription: CodeCraft independent human review workflow\nalwaysApply: true\n---\n\n# CodeCraft independent review\n\nUse the CodeCraft MCP tools for human questions and plan review when available. First call codecraft_list_projects, then codecraft_start_workflow with the appropriate enabled project and a descriptive title. Keep the returned workflowId for this task only. Do not reuse another task's workflow.\n\nUse codecraft_ask_user or codecraft_review_plan with a new clientRequestId for each new human review. A pending response is not approval: poll codecraft_get_result about once per second until a human decision or terminal error arrives. Never invent answers or approval. If a plan is revised, use the returned current revision and explicitly supersede the prior clientRequestId. After unknown delivery or reconnection request a fresh human review. Cancel abandoned requests with codecraft_cancel_request. When the task finishes or is abandoned, call codecraft_end_workflow to release the workflow.\n\nThese are independent CodeCraft reviews, not Cursor native approvals. Only proceed after the requested human decision; CodeCraft does not enforce this guidance across native Cursor tools. Native permissions and other hooks remain in effect.\n";
pub fn status(path: &Path) -> Value {
    let target = path.join(".cursor/rules/codecraft-workflow.mdc");
    let state = match fs::read(&target) {
        Ok(v) if v == RULE.as_bytes() => "installed",
        Ok(_) => "conflict",
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => "missing",
        Err(_) => "unavailable",
    };
    json!({"state":state,"path":target})
}
pub fn configure(root: &Path, path: &Path, enabled: bool) -> Result<()> {
    let project = projects::resolve(path)?;
    let target = project
        .final_path
        .join(".cursor/rules/codecraft-workflow.mdc");
    files::reject_link_path(&target)?;
    let before = match fs::read(&target) {
        Ok(v) => Some(v),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Err("RULE_UNREADABLE"),
    };
    if before.as_ref().is_some_and(|b| b != RULE.as_bytes()) {
        return Err("RULE_CONFLICT");
    }
    if (enabled && before.is_some()) || (!enabled && before.is_none()) {
        return Ok(());
    }
    let parent = target.parent().ok_or("RULE_PATH_INVALID")?;
    if enabled {
        fs::create_dir_all(parent).map_err(|_| "RULE_DIRECTORY_FAILED")?;
        files::reject_link_path(parent)?;
    }
    let transaction_root = root.join("project-rules").join(project.identity.key());
    fs::create_dir_all(transaction_root.parent().ok_or("RULE_PATH_INVALID")?)
        .map_err(|_| "RULE_DIRECTORY_FAILED")?;
    let revision = if transaction_root.join("transaction.json").exists() {
        let mut old = install::Transaction::open(&transaction_root)?;
        if !matches!(
            old.journal().phase,
            install::Phase::Committed | install::Phase::RolledBack
        ) && !matches!(
            old.recover()?,
            install::Phase::Committed | install::Phase::RolledBack
        ) {
            return Err("RULE_RECOVERY_CONFLICT");
        }
        old.journal().config_revision + 1
    } else {
        1
    };
    let mut tx = install::Transaction::prepare(
        &transaction_root,
        &project.identity.key(),
        revision,
        vec![install::Change {
            target,
            before_hash: before.as_deref().map(protocol::sha256),
            after: enabled.then(|| RULE.as_bytes().to_vec()),
        }],
    )?;
    while tx.advance()? != install::Phase::Verifying {}
    tx.commit(|_| Ok(()))?;
    Ok(())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn installation_uses_local_open_windows_and_workspace_roots_only() {
        let (root, _, _, _) = crate::product::tests::fixture();
        fs::create_dir_all(&root).unwrap();
        let project = root.join("project with spaces");
        let second = root.join("second");
        let historical = root.join("historical");
        for path in [&project, &second, &historical] {
            fs::create_dir(path).unwrap();
        }
        let workspace = root.join("current.code-workspace");
        files::atomic_json(&workspace, &json!({"folders":[{"path":"second"}]}), false).unwrap();
        let storage = root.join("storage.json");
        files::atomic_json(&storage, &json!({
            "windowsState": {
                "lastActiveWindow":{"folder":url::Url::from_directory_path(&project).unwrap().to_string()},
                "openedWindows":[
                    {"folder":url::Url::from_directory_path(&project).unwrap().to_string()},
                    {"workspace":{"configPath":url::Url::from_file_path(&workspace).unwrap().to_string()}},
                    {"folder":"vscode-remote://ssh-remote+server/home/project"},
                    {"folder":"file://server/share/project"}
                ]
            },
            "backupWorkspaces":{"folders":[{"folderUri":url::Url::from_directory_path(&historical).unwrap().to_string()}]}
        }), false).unwrap();
        let paths = installation_projects(&storage).unwrap();
        assert_eq!(paths.len(), 2);
        assert!(paths.contains(&projects::resolve(&project).unwrap().final_path));
        assert!(paths.contains(&projects::resolve(&second).unwrap().final_path));
        assert!(!paths.contains(&projects::resolve(&historical).unwrap().final_path));
        assert!(installation_projects(&root.join("missing.json"))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn hook_install_enables_projects_repairs_rules_and_preserves_opt_outs_and_edits() {
        let (root, _, _, _) = crate::product::tests::fixture();
        crate::product::initialize(&root).unwrap();
        let project = root.parent().unwrap().join("project");
        fs::create_dir(&project).unwrap();
        let mut store =
            crate::store::Store::open(&root.join("workflow"), "installer-test").unwrap();
        install_for_projects(&root, &mut store, vec![project.clone()]).unwrap();
        let key = projects::resolve(&project).unwrap().identity.key();
        assert!(
            store
                .model()
                .project_registry()
                .project(&key)
                .unwrap()
                .enabled
        );
        assert_eq!(status(&project)["state"], "installed");
        let rule = project.join(".cursor/rules/codecraft-workflow.mdc");
        fs::remove_file(&rule).unwrap();
        install_for_projects(&root, &mut store, Vec::new()).unwrap();
        assert_eq!(status(&project)["state"], "installed");
        fs::write(&rule, b"user-edited-rule").unwrap();
        assert_eq!(
            install_for_projects(&root, &mut store, vec![project.clone()]),
            Err("RULE_CONFLICT")
        );
        assert_eq!(fs::read(&rule).unwrap(), b"user-edited-rule");
        store
            .execute(crate::workflow::Command::SetProjectPath {
                path: project.clone(),
                enabled: false,
            })
            .unwrap();
        install_for_projects(&root, &mut store, vec![project]).unwrap();
        assert!(
            !store
                .model()
                .project_registry()
                .project(&key)
                .unwrap()
                .enabled
        );
    }
}
