//! Desktop-only installation coordinator. The active configuration participates
//! in the same durable transaction as Cursor's files; the manifest commits last.
use crate::{files, install, process, protocol, schema::Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub const SERVER: &str = "codecraft_cursor";
pub const REASON: &str = "CodeCraft 独立工具审批、问答与计划审阅；不与 Cursor 原生审批同步，计划结果不拦截未关联的原生工具。";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    pub schema_version: u32,
    pub installation_id: String,
    pub revision: u64,
    pub enabled: bool,
    pub executable: Option<PathBuf>,
    pub cursor_executable: Option<PathBuf>,
    pub hooks: BTreeMap<String, Vec<Value>>,
    pub mcp: Option<Value>,
    pub created_directories: Vec<PathBuf>,
}

pub fn root() -> Result<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(|p| PathBuf::from(p).join("CodeCraft/cursor"))
        .ok_or("LOCAL_DATA_DIRECTORY_UNAVAILABLE")
}
pub fn cursor_home() -> Result<PathBuf> {
    std::env::var_os("USERPROFILE")
        .map(|p| PathBuf::from(p).join(".cursor"))
        .ok_or("USER_DIRECTORY_UNAVAILABLE")
}
pub fn initialize(root: &Path) -> Result<()> {
    if !root.is_absolute() {
        return Err("ABSOLUTE_DIRECTORY_REQUIRED");
    }
    files::reject_link_path(root)?;
    // Cursor starts before other integrations on a fresh installation. Create
    // the shared app container without changing its ownership or permissions.
    fs::create_dir_all(root.parent().ok_or("DIRECTORY_CREATE_FAILED")?)
        .map_err(|_| "DIRECTORY_CREATE_FAILED")?;
    files::private_directory(root)?;
    let identity = root.join("installation.json");
    if !identity.exists() {
        files::atomic_json(
            &identity,
            &json!({"installationId":uuid::Uuid::new_v4().to_string()}),
            false,
        )?;
    }
    Ok(())
}
pub fn load(root: &Path) -> Result<Config> {
    let path = root.join("configuration.json");
    if !path.exists() {
        let installation_id = if root.join("installation.json").exists() {
            files::read_json(&root.join("installation.json"))?["installationId"]
                .as_str()
                .filter(|id| crate::schema::valid_id(id))
                .ok_or("INVALID_INSTALLATION")?
                .to_owned()
        } else {
            "cursor-local".into()
        };
        return Ok(Config {
            schema_version: 1,
            installation_id,
            revision: 0,
            enabled: false,
            executable: None,
            cursor_executable: process::detect_cursor(),
            hooks: BTreeMap::new(),
            mcp: None,
            created_directories: Vec::new(),
        });
    }
    let config: Config =
        serde_json::from_value(files::read_json(&path)?).map_err(|_| "CURSOR_CONFIG_CORRUPT")?;
    if config.schema_version != 1 || !crate::schema::valid_id(&config.installation_id) {
        return Err("CURSOR_CONFIG_CORRUPT");
    }
    // A partially written configuration never activates the integration.
    let manifest: install::Manifest =
        serde_json::from_value(files::read_json(&root.join("installer/manifest.json"))?)
            .map_err(|_| "INSTALL_MANIFEST_INVALID")?;
    if manifest.installation_id != config.installation_id
        || manifest.config_revision != config.revision
    {
        return Err("INSTALL_RECOVERY_REQUIRED");
    }
    Ok(config)
}

pub fn recover(root: &Path) -> Result<()> {
    if root.join("installer/transaction.json").exists() {
        let mut tx = install::Transaction::open(&root.join("installer"))?;
        // A committed installation is checked by owned-entry CAS, not the hash
        // of the entire user file: unrelated user additions must remain valid.
        if matches!(
            tx.journal().phase,
            install::Phase::Committed | install::Phase::RolledBack
        ) {
            return Ok(());
        }
        let phase = tx.recover()?;
        if phase == install::Phase::Conflict {
            return Err("INSTALL_RECOVERY_CONFLICT");
        }
    }
    Ok(())
}

fn config_bytes(path: &Path) -> Result<Option<Vec<u8>>> {
    files::reject_link_path(path)?;
    match fs::File::open(path) {
        Ok(f) => protocol::read_bytes(f)
            .map(Some)
            .map_err(|_| "CONFIG_UNREADABLE"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("CONFIG_UNREADABLE"),
    }
}
fn bytes(path: &Path) -> Result<Vec<u8>> {
    Ok(config_bytes(path)?.unwrap_or_else(|| b"{}".to_vec()))
}
fn quoted(path: &Path) -> Result<String> {
    let value = path
        .to_str()
        .ok_or("NON_UNICODE_PATH")?
        .strip_prefix("\\\\?\\")
        .unwrap_or(path.to_str().unwrap());
    if value.contains(['"', '%', '!', '$', '`', '\r', '\n']) {
        return Err("UNSAFE_COMMAND_PATH");
    }
    Ok(format!("\"{value}\""))
}
fn handlers(executable: &Path) -> Result<BTreeMap<String, Vec<Value>>> {
    let exe = quoted(executable)?;
    Ok(protocol::EVENTS.iter().map(|event| (event.to_string(), vec![json!({
        "command":format!("{exe} --codecraft-cursor-hook --event {event}"), "timeout":if matches!(*event,"preToolUse"|"beforeMCPExecution") {60}else{5}, "failClosed":false
    })])).collect())
}

fn configuration_changes(
    config: &Config,
    home: &Path,
    executable: &Path,
    enabled: bool,
) -> Result<Vec<install::Change>> {
    let replacement = if enabled {
        handlers(executable)?
    } else {
        BTreeMap::new()
    };
    let mcp = enabled.then(|| json!({"command":executable,"args":["--codecraft-cursor-mcp"]}));
    let mut changes = Vec::new();
    for name in ["hooks.json", "mcp.json"] {
        let target = home.join(name);
        let before = config_bytes(&target)?;
        let input = before.as_deref().unwrap_or(b"{}");
        let after = if name == "hooks.json" {
            let mut expected = config.hooks.clone();
            if enabled {
                let current = protocol::strict_json(input).map_err(|_| "INVALID_HOOK_CONFIG")?;
                // Missing event lists can be restored by the normal installer.
                // A changed, nonempty list still needs the exact ownership CAS.
                expected.retain(|event, _| {
                    current["hooks"]
                        .get(event)
                        .is_some_and(|handlers| !handlers.as_array().is_some_and(Vec::is_empty))
                });
            }
            install::merge_hooks(input, &expected, &replacement)?
        } else {
            let current = protocol::strict_json(input).map_err(|_| "INVALID_MCP_CONFIG")?;
            let expected = if enabled && current["mcpServers"].get(SERVER).is_none() {
                None
            } else {
                config.mcp.as_ref()
            };
            install::merge_mcp(input, SERVER, expected, mcp.clone())?
        };
        // Bind the CAS hash to the exact bytes used for merging. A subsequent
        // user save must reject this plan rather than silently losing new keys.
        changes.push(install::Change {
            target,
            before_hash: before.as_deref().map(protocol::sha256),
            after: Some(after),
        });
    }
    Ok(changes)
}

pub fn preview_at(
    root: &Path,
    home: &Path,
    executable: &Path,
    enabled: bool,
    cursor: Option<PathBuf>,
) -> Result<Value> {
    let config = load(root)?;
    let cursor = cursor
        .or(config
            .cursor_executable
            .clone()
            .filter(|path| process::cursor_version(path).is_ok()))
        .or_else(process::detect_cursor);
    let version = cursor
        .as_deref()
        .and_then(|p| process::cursor_version(p).ok());
    let files = configuration_changes(&config, home, executable, enabled)?
        .into_iter()
        .map(|change| Ok(json!({"path":change.target,"content":protocol::strict_json(change.after.as_deref().ok_or("INVALID_INSTALL_PLAN")?).map_err(|_| "CONFIG_INVALID")?})))
        .collect::<Result<Vec<_>>>()?;
    Ok(
        json!({"enabled":enabled,"cursorExecutable":cursor,"cursorVersion":version,"captureScope":"observedOnly",
        "reason":REASON,"files":files,
        "projectsEnabled":false,"capabilities":{"hookOnly":enabled,"nativeShared":false,"askUser":enabled,"reviewPlan":enabled}}),
    )
}

pub fn configure_at(
    root: &Path,
    home: &Path,
    executable: &Path,
    enabled: bool,
    cursor: Option<PathBuf>,
) -> Result<Value> {
    configure_with_projects_at(root, home, executable, enabled, cursor, Vec::new())
}

pub fn configure_with_projects_at(
    root: &Path,
    home: &Path,
    executable: &Path,
    enabled: bool,
    cursor: Option<PathBuf>,
    projects: Vec<PathBuf>,
) -> Result<Value> {
    initialize(root)?;
    let workflow = files::private_directory(&root.join("workflow"))?;
    let _writer = files::lock(&workflow.join("broker.lock"))?;
    recover(root)?;
    let config_hash = install::file_hash(&root.join("configuration.json"))?;
    let mut config = load(root)?;
    let cursor = cursor
        .or(config
            .cursor_executable
            .clone()
            .filter(|path| process::cursor_version(path).is_ok()))
        .or_else(process::detect_cursor);
    if enabled {
        process::cursor_version(cursor.as_deref().ok_or("CURSOR_NOT_FOUND")?)?;
    }
    if !executable.is_file() {
        return Err("CODECRAFT_EXECUTABLE_REQUIRED");
    }
    let mut changes = configuration_changes(&config, home, executable, enabled)?;
    changes.extend(crate::project_rules::installation_changes(
        root,
        &config.installation_id,
        enabled,
        projects,
    )?);
    files::reject_link_path(home)?;
    if !home.exists() {
        // Only the known .cursor directory can be created; its parent must exist.
        crate::projects::resolve(home.parent().ok_or("INVALID_INSTALL_TARGET")?)?;
        fs::create_dir(home).map_err(|_| "CURSOR_DIRECTORY_CREATE_FAILED")?;
        config.created_directories.push(home.to_path_buf());
    }
    config.revision = config
        .revision
        .checked_add(1)
        .ok_or("INSTALL_REVISION_OVERFLOW")?;
    if root.join("installer/transaction.json").exists() {
        let previous = install::Transaction::open(&root.join("installer"))?;
        config.revision = config.revision.max(
            previous
                .journal()
                .config_revision
                .checked_add(1)
                .ok_or("INSTALL_REVISION_OVERFLOW")?,
        );
    }
    config.enabled = enabled;
    config.executable = Some(executable.to_path_buf());
    config.cursor_executable = cursor;
    config.hooks = if enabled {
        handlers(executable)?
    } else {
        BTreeMap::new()
    };
    config.mcp = enabled.then(|| json!({"command":executable,"args":["--codecraft-cursor-mcp"]}));
    let target = root.join("configuration.json");
    changes.push(install::Change {
        before_hash: config_hash,
        target,
        after: Some(serde_json::to_vec_pretty(&config).map_err(|_| "INSTALL_ENCODE_FAILED")?),
    });
    let mut tx = install::Transaction::prepare(
        &root.join("installer"),
        &config.installation_id,
        config.revision,
        changes,
    )?;
    let result = (|| {
        while tx.advance()? != install::Phase::Verifying {}
        tx.commit(|_| {
            if enabled && !executable.is_file() {
                return Err("CODECRAFT_EXECUTABLE_REQUIRED");
            }
            Ok(())
        })?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = tx.recover();
        return Err(error);
    }
    // Never recursively remove user directories, even when we originally created them.
    status_at(root, home)
}

pub fn status_at(root: &Path, home: &Path) -> Result<Value> {
    let config = load(root)?;
    let cursor_executable = config
        .cursor_executable
        .clone()
        .filter(|path| process::cursor_version(path).is_ok())
        .or_else(process::detect_cursor);
    let version = cursor_executable
        .as_deref()
        .and_then(|p| process::cursor_version(p).ok());
    let rules_intact = crate::project_rules::installation_intact(root, &config.installation_id)?;
    let intact = if config.enabled {
        install::merge_hooks(
            &bytes(&home.join("hooks.json"))?,
            &config.hooks,
            &config.hooks,
        )
        .is_ok()
            && install::merge_mcp(
                &bytes(&home.join("mcp.json"))?,
                SERVER,
                config.mcp.as_ref(),
                config.mcp.clone(),
            )
            .is_ok()
            && config.executable.as_ref().is_some_and(|p| p.is_file())
            && rules_intact
    } else {
        false
    };
    Ok(
        json!({"enabled":config.enabled,"filesInstalled":intact,"detected":version.is_some(),"cursorVersion":version,
        "cursorExecutable":cursor_executable,"installPath":home,"configRevision":config.revision,
        "state":if !config.enabled {"disabled"} else if intact {"installed"} else {"conflict"},
        "projectRulesInstalled":config.enabled && rules_intact,
        "reason":REASON,"error":if config.enabled && !rules_intact {Some("CURSOR_PROJECT_RULES_CHANGED")} else if config.enabled && !intact {Some("CURSOR_CONFIGURATION_CHANGED")} else {None}}),
    )
}

pub fn status() -> Result<Value> {
    status_at(&root()?, &cursor_home()?)
}
pub fn repair_moved_executable(root: &Path, home: &Path, executable: &Path) -> Result<()> {
    recover(root)?;
    let config = load(root)?;
    if !config.enabled
        && crate::project_rules::has_registered_projects(root, &config.installation_id)?
    {
        // Finish removals left behind by versions that uninstalled only Hook/MCP.
        configure_at(root, home, executable, false, config.cursor_executable)?;
        return Ok(());
    }
    if config.enabled
        && config
            .executable
            .as_deref()
            .and_then(|p| p.canonicalize().ok())
            != executable.canonicalize().ok()
    {
        configure_at(root, home, executable, true, config.cursor_executable)?;
    }
    Ok(())
}
pub fn preview(enabled: bool, cursor: Option<PathBuf>) -> Result<Value> {
    preview_at(
        &root()?,
        &cursor_home()?,
        &std::env::current_exe().map_err(|_| "EXECUTABLE_UNAVAILABLE")?,
        enabled,
        cursor,
    )
}

#[cfg(all(test, windows))]
pub(crate) mod tests {
    use super::*;
    #[test]
    fn first_launch_creates_missing_shared_data_container_before_private_state() {
        let (base, _, _, _) = fixture();
        let root = base.join("fresh-local/CodeCraft/cursor");
        assert!(!root.parent().unwrap().exists());
        initialize(&root).unwrap();
        let first = load(&root).unwrap().installation_id;
        assert!(crate::schema::valid_id(&first));
        initialize(&root).unwrap();
        assert_eq!(load(&root).unwrap().installation_id, first);
    }
    pub(crate) fn fixture() -> (PathBuf, PathBuf, PathBuf, PathBuf) {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("runs")
            .join(format!("product-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(base.join("client/resources/app")).unwrap();
        let root = base.join("state");
        let home = base.join(".cursor");
        let cursor = base.join("client/Cursor.exe");
        fs::write(&cursor, b"synthetic-client-metadata-only").unwrap();
        fs::write(
            base.join("client/resources/app/package.json"),
            br#"{"version":"3.21.16"}"#,
        )
        .unwrap();
        (root, home, std::env::current_exe().unwrap(), cursor)
    }
    #[test]
    fn hook_mcp_rules_and_project_registration_install_and_uninstall_together() {
        let (root, home, exe, cursor) = fixture();
        let project = root.parent().unwrap().join("project");
        fs::create_dir(&project).unwrap();
        let installed = configure_with_projects_at(
            &root,
            &home,
            &exe,
            true,
            Some(cursor),
            vec![project.clone()],
        )
        .unwrap();
        assert_eq!(installed["filesInstalled"], true);
        assert_eq!(installed["projectRulesInstalled"], true);
        let rule = project.join(".cursor/rules/codecraft-workflow.mdc");
        let rule_bytes = fs::read(&rule).unwrap();
        let id = load(&root).unwrap().installation_id;
        assert!(crate::project_rules::has_registered_projects(&root, &id).unwrap());

        let removed = configure_at(&root, &home, &exe, false, None).unwrap();
        assert_eq!(removed["enabled"], false);
        assert_eq!(removed["filesInstalled"], false);
        assert!(!rule.exists());
        assert!(!crate::project_rules::has_registered_projects(&root, &id).unwrap());
        assert!(
            files::read_json(&home.join("mcp.json")).unwrap()["mcpServers"]
                .get(SERVER)
                .is_none()
        );
        let backups = fs::read_dir(rule.parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "bak"))
            .collect::<Vec<_>>();
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read(backups[0].path()).unwrap(), rule_bytes);

        // Global removal does not turn an ordinary project into a permanent opt-out.
        let restored =
            configure_with_projects_at(&root, &home, &exe, true, None, vec![project.clone()])
                .unwrap();
        assert_eq!(restored["filesInstalled"], true);
        assert!(rule.exists());
        let store = crate::store::Store::open(&root.join("workflow"), &id).unwrap();
        assert!(store
            .model()
            .project_registry()
            .entries()
            .all(|(_, project)| project.enabled));
    }

    #[test]
    fn project_rule_conflicts_prevent_partial_installation_and_uninstallation() {
        let (root, home, exe, cursor) = fixture();
        let project = root.parent().unwrap().join("project");
        let rule = project.join(".cursor/rules/codecraft-workflow.mdc");
        fs::create_dir_all(rule.parent().unwrap()).unwrap();
        fs::write(&rule, b"user rule").unwrap();
        assert_eq!(
            configure_with_projects_at(
                &root,
                &home,
                &exe,
                true,
                Some(cursor.clone()),
                vec![project.clone()],
            )
            .unwrap_err(),
            "RULE_CONFLICT"
        );
        assert!(!home.join("hooks.json").exists());
        assert!(!home.join("mcp.json").exists());
        assert!(!load(&root).unwrap().enabled);

        fs::remove_file(&rule).unwrap();
        configure_with_projects_at(&root, &home, &exe, true, Some(cursor), vec![project]).unwrap();
        fs::remove_file(&rule).unwrap();
        assert_eq!(status_at(&root, &home).unwrap()["filesInstalled"], false);
        assert_eq!(
            status_at(&root, &home).unwrap()["error"],
            "CURSOR_PROJECT_RULES_CHANGED"
        );
        configure_at(&root, &home, &exe, true, None).unwrap();
        assert!(rule.exists());
        fs::write(&rule, b"edited rule").unwrap();
        let hooks = fs::read(home.join("hooks.json")).unwrap();
        let mcp = fs::read(home.join("mcp.json")).unwrap();
        let state = fs::read(root.join("workflow/state.json")).unwrap();
        assert_eq!(
            configure_at(&root, &home, &exe, false, None).unwrap_err(),
            "RULE_CONFLICT"
        );
        assert!(load(&root).unwrap().enabled);
        assert_eq!(fs::read(home.join("hooks.json")).unwrap(), hooks);
        assert_eq!(fs::read(home.join("mcp.json")).unwrap(), mcp);
        assert_eq!(fs::read(root.join("workflow/state.json")).unwrap(), state);
        assert_eq!(fs::read(rule).unwrap(), b"edited rule");
    }

    #[test]
    fn startup_finishes_project_cleanup_left_by_older_uninstallers() {
        let (root, home, exe, cursor) = fixture();
        let project = root.parent().unwrap().join("project");
        fs::create_dir(&project).unwrap();
        configure_with_projects_at(
            &root,
            &home,
            &exe,
            true,
            Some(cursor),
            vec![project.clone()],
        )
        .unwrap();
        let rule = project.join(".cursor/rules/codecraft-workflow.mdc");
        let old_rule = fs::read(&rule).unwrap();
        let old_state = fs::read(root.join("workflow/state.json")).unwrap();
        configure_at(&root, &home, &exe, false, None).unwrap();
        fs::write(&rule, old_rule).unwrap();
        fs::write(root.join("workflow/state.json"), old_state).unwrap();
        repair_moved_executable(&root, &home, &exe).unwrap();
        assert!(!rule.exists());
        let config = load(&root).unwrap();
        assert!(!config.enabled);
        assert!(
            !crate::project_rules::has_registered_projects(&root, &config.installation_id).unwrap()
        );
        let revision = config.revision;
        repair_moved_executable(&root, &home, &exe).unwrap();
        assert_eq!(load(&root).unwrap().revision, revision);
    }

    #[test]
    fn install_upgrade_uninstall_commit_with_backups_and_preserve_third_party_entries() {
        let (root, home, exe, cursor) = fixture();
        fs::create_dir(&home).unwrap();
        fs::write(home.join("hooks.json"), br#"{"version":1,"custom":true,"hooks":{"preToolUse":[{"command":"third-party.exe"}]}}"#).unwrap();
        fs::write(
            home.join("mcp.json"),
            br#"{"custom":true,"mcpServers":{"other":{"command":"other.exe"}}}"#,
        )
        .unwrap();
        let preview = preview_at(&root, &home, &exe, true, Some(cursor.clone())).unwrap();
        assert_eq!(preview["capabilities"]["nativeShared"], false);
        let installed = configure_at(&root, &home, &exe, true, Some(cursor.clone())).unwrap();
        assert_eq!(installed["filesInstalled"], true);
        assert!(load(&root).unwrap().enabled);
        assert!(fs::read_dir(&home)
            .unwrap()
            .filter_map(std::result::Result::ok)
            .any(|e| e.file_name().to_string_lossy().ends_with(".bak")));
        let updated = configure_at(&root, &home, &exe, true, Some(cursor)).unwrap();
        assert!(
            updated["configRevision"].as_u64().unwrap()
                > installed["configRevision"].as_u64().unwrap()
        );
        configure_at(&root, &home, &exe, false, None).unwrap();
        let hooks = protocol::strict_json(&fs::read(home.join("hooks.json")).unwrap()).unwrap();
        let mcp = protocol::strict_json(&fs::read(home.join("mcp.json")).unwrap()).unwrap();
        assert_eq!(
            hooks["hooks"]["preToolUse"],
            json!([{"command":"third-party.exe"}])
        );
        assert_eq!(hooks["custom"], true);
        assert_eq!(mcp["mcpServers"], json!({"other":{"command":"other.exe"}}));
        assert_eq!(mcp["custom"], true);
        assert!(!load(&root).unwrap().enabled);
    }
    #[test]
    fn portable_update_rewrites_owned_commands_and_retains_new_unrelated_entries() {
        let (root, home, exe, cursor) = fixture();
        configure_at(&root, &home, &exe, true, Some(cursor)).unwrap();
        let path = home.join("mcp.json");
        let mut value = protocol::strict_json(&fs::read(&path).unwrap()).unwrap();
        value["mcpServers"]["added-later"] = json!({"command":"user-added.exe"});
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let moved = root.parent().unwrap().join("new-codecraft.exe");
        fs::write(&moved, b"synthetic-executable").unwrap();
        repair_moved_executable(&root, &home, &moved).unwrap();
        let current = load(&root).unwrap();
        assert_eq!(current.executable.as_ref(), Some(&moved));
        assert_eq!(
            protocol::strict_json(&fs::read(path).unwrap()).unwrap()["mcpServers"]["added-later"],
            json!({"command":"user-added.exe"})
        );
    }
    #[test]
    fn reinstall_repairs_missing_hooks_and_mcp_without_removing_other_servers() {
        let (root, home, exe, cursor) = fixture();
        configure_at(&root, &home, &exe, true, Some(cursor)).unwrap();
        fs::remove_file(home.join("hooks.json")).unwrap();
        fs::write(
            home.join("mcp.json"),
            br#"{"mcpServers":{"other":{"command":"user.exe"}}}"#,
        )
        .unwrap();
        assert_eq!(status_at(&root, &home).unwrap()["filesInstalled"], false);
        let repaired = configure_at(&root, &home, &exe, true, None).unwrap();
        assert_eq!(repaired["filesInstalled"], true);
        let mcp = files::read_json(&home.join("mcp.json")).unwrap();
        assert_eq!(mcp["mcpServers"]["other"]["command"], "user.exe");
        assert!(mcp["mcpServers"][SERVER].is_object());
        let mut hooks = files::read_json(&home.join("hooks.json")).unwrap();
        hooks["hooks"]["preToolUse"] = json!([]);
        fs::write(home.join("hooks.json"), serde_json::to_vec(&hooks).unwrap()).unwrap();
        assert_eq!(
            configure_at(&root, &home, &exe, true, None).unwrap()["filesInstalled"],
            true
        );
    }
    #[test]
    fn user_edits_block_repair_and_uninstall_without_overwriting_them() {
        let (root, home, exe, cursor) = fixture();
        configure_at(&root, &home, &exe, true, Some(cursor)).unwrap();
        let path = home.join("mcp.json");
        let mut value = protocol::strict_json(&fs::read(&path).unwrap()).unwrap();
        value["mcpServers"][SERVER]["command"] = "user-edited.exe".into();
        let edited = serde_json::to_vec_pretty(&value).unwrap();
        fs::write(&path, &edited).unwrap();
        assert_eq!(
            configure_at(&root, &home, &exe, true, None).unwrap_err(),
            "MCP_ENTRY_CONFLICT"
        );
        assert_eq!(
            configure_at(&root, &home, &exe, false, None).unwrap_err(),
            "MCP_ENTRY_CONFLICT"
        );
        assert_eq!(fs::read(path).unwrap(), edited);
        assert_eq!(status_at(&root, &home).unwrap()["state"], "conflict");
    }
    #[test]
    fn user_save_after_merge_rejects_the_stale_plan_and_preserves_new_entries() {
        let (root, home, exe, cursor) = fixture();
        configure_at(&root, &home, &exe, true, Some(cursor)).unwrap();
        let config = load(&root).unwrap();
        let changes = configuration_changes(&config, &home, &exe, false).unwrap();
        let path = home.join("mcp.json");
        let mut value = protocol::strict_json(&fs::read(&path).unwrap()).unwrap();
        value["mcpServers"]["saved-concurrently"] = json!({"command":"user-added.exe"});
        let edited = serde_json::to_vec(&value).unwrap();
        fs::write(&path, &edited).unwrap();
        assert!(matches!(
            install::Transaction::prepare(
                &root.join("installer"),
                &config.installation_id,
                config.revision + 1,
                changes,
            ),
            Err("INSTALL_FILE_CONFLICT")
        ));
        assert_eq!(fs::read(&path).unwrap(), edited);
        assert!(status_at(&root, &home).unwrap()["filesInstalled"] == true);
        configure_at(&root, &home, &exe, false, None).unwrap();
        assert_eq!(
            files::read_json(&path).unwrap()["mcpServers"]["saved-concurrently"],
            json!({"command":"user-added.exe"})
        );
    }
    #[test]
    fn interrupted_configuration_is_rolled_back_before_loading_and_revision_advances() {
        let (root, home, exe, cursor) = fixture();
        configure_at(&root, &home, &exe, true, Some(cursor)).unwrap();
        let previous = load(&root).unwrap();
        let mut changed = previous.clone();
        changed.enabled = false;
        changed.revision += 1;
        let path = root.join("configuration.json");
        let mut tx = install::Transaction::prepare(
            &root.join("installer"),
            &previous.installation_id,
            changed.revision,
            vec![install::Change {
                before_hash: install::file_hash(&path).unwrap(),
                target: path,
                after: Some(serde_json::to_vec(&changed).unwrap()),
            }],
        )
        .unwrap();
        while tx.advance().unwrap() != install::Phase::Verifying {}
        drop(tx);
        assert!(load(&root).is_err());
        recover(&root).unwrap();
        assert_eq!(load(&root).unwrap().revision, previous.revision);
        configure_at(&root, &home, &exe, false, None).unwrap();
        assert!(load(&root).unwrap().revision > changed.revision);
    }
}
