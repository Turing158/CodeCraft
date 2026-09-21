use crate::{files, protocol::*, wire};
use serde_json::{json, Value};
use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

fn configuration() -> Result<PathBuf> {
    Ok(PathBuf::from(
        env::var_os("USERPROFILE")
            .or_else(|| env::var_os("HOME"))
            .ok_or_else(|| invalid("User profile missing"))?,
    )
    .join(".trae-cn/hooks.json"))
}
pub fn detect() -> Value {
    let mut candidates = Vec::<PathBuf>::new();
    if let Ok(v) = files::read(&files::root().join("installation.json"), 16384) {
        if let Some(p) = v["executable"].as_str() {
            candidates.push(p.into());
        }
    }
    #[cfg(windows)]
    {
        use winreg::{enums::*, RegKey};
        for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
            let hive = RegKey::predef(hive);
            if let Ok(key) = hive
                .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\Trae CN.exe")
            {
                if let Ok(p) = key.get_value::<String, _>("") {
                    candidates.push(p.into());
                }
            }
            for branch in [
                "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
                "Software\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
            ] {
                if let Ok(key) = hive.open_subkey(branch) {
                    for name in key.enum_keys().flatten() {
                        if let Ok(entry) = key.open_subkey(name) {
                            if entry
                                .get_value::<String, _>("DisplayName")
                                .ok()
                                .is_some_and(|n| n.contains("Trae CN") || n.contains("TraeCode CN"))
                            {
                                if let Ok(path) = entry.get_value::<String, _>("InstallLocation") {
                                    candidates.push(PathBuf::from(path).join("Trae CN.exe"));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    if let Some(local) = env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(local).join("Programs/Trae CN/Trae CN.exe"));
    }
    if let Some(path) = env::var_os("PATH") {
        for dir in env::split_paths(&path) {
            candidates.push(dir.join("Trae CN.exe"));
            candidates.push(dir.join("../Trae CN.exe"));
        }
    }
    for exe in candidates {
        if !exe.is_file() {
            continue;
        }
        let exe = match exe.canonicalize() {
            Ok(p) => p,
            Err(_) => continue,
        };
        let product = exe.parent().unwrap().join("resources/app/product.json");
        if let Ok(v) = files::read(&product, 1024 * 1024) {
            if v["nameShort"] == "Trae CN" {
                return json!({"detected":true,"executable":exe,"productVersion":v["appVersion"],"baseVersion":v["version"]});
            }
        }
    }
    json!({"detected":false,"productVersion":null,"baseVersion":null})
}
pub fn set_installation(path: &str) -> Result<Value> {
    let path = Path::new(path).canonicalize()?;
    if path
        .file_name()
        .is_none_or(|v| !v.to_string_lossy().eq_ignore_ascii_case("Trae CN.exe"))
    {
        return Err(invalid("Select Trae CN.exe"));
    }
    let product = files::read(
        &path.parent().unwrap().join("resources/app/product.json"),
        1024 * 1024,
    )?;
    if product["nameShort"] != "Trae CN" {
        return Err(invalid("Not a Trae CN installation"));
    }
    let client = files::Client::connect(&files::root())?;
    client.call(json!({"kind":"refresh_capabilities","suspend":true}))?;
    files::initialize(&files::root())?;
    let file = files::root().join("installation.json");
    files::backup(&file)?;
    files::atomic(&file, &json!({"executable":path}), true)?;
    client.call(json!({"kind":"refresh_capabilities"}))?;
    Ok(detect())
}
fn groups(exe: &Path) -> Vec<(String, Value)> {
    wire::EVENTS.iter().map(|event|{let command=format!("& '{}' --codecraft-trae-hook --event {event}",exe.to_string_lossy().replace('\'',"''"));let mut group=json!({"hooks":[{"type":"command","command":command,"timeout":if *event=="PreToolUse"{HOOK_TIMEOUT_SECONDS}else{30}}]});if matches!(*event,"PreToolUse"|"PostToolUse"|"Notification"){group["matcher"]=json!(".*");}(event.to_string(),group)}).collect()
}
pub fn merge(config: &mut Value, old: &[(String, Value)], new: &[(String, Value)]) -> Result<()> {
    let object = config
        .as_object_mut()
        .ok_or_else(|| invalid("Hook config must be an object"))?;
    if object.get("version").is_some_and(|v| v != 1) {
        return Err(ApiError::new(
            ErrorCode::UnsupportedVersion,
            "Trae hooks schema version is unsupported",
        ));
    }
    object.entry("version").or_insert(json!(1));
    let hooks = object
        .entry("hooks")
        .or_insert(json!({}))
        .as_object_mut()
        .ok_or_else(|| invalid("hooks must be an object"))?;
    for (event, owned) in old {
        if let Some(groups) = hooks.get_mut(event) {
            let groups = groups
                .as_array_mut()
                .ok_or_else(|| invalid("Hook event must contain an array"))?;
            let command = &owned["hooks"][0]["command"];
            if groups.iter().any(|g| {
                g != owned
                    && g["hooks"]
                        .as_array()
                        .is_some_and(|h| h.iter().any(|h| h["command"] == *command))
            }) {
                return Err(ApiError::new(
                    ErrorCode::RequestConflict,
                    "A managed Trae Hook was changed by the user",
                ));
            }
            groups.retain(|g| g != owned);
        }
    }
    hooks.retain(|event, groups| {
        !old.iter().any(|(owned, _)| owned == event)
            || !groups.as_array().is_some_and(|v| v.is_empty())
    });
    for (event, group) in new {
        let groups = hooks
            .entry(event)
            .or_insert(json!([]))
            .as_array_mut()
            .ok_or_else(|| invalid("Hook event must contain an array"))?;
        if !groups.contains(group) {
            if groups.iter().any(|g| {
                g["hooks"].as_array().is_some_and(|hs| {
                    hs.iter().any(|h| {
                        h["command"]
                            .as_str()
                            .is_some_and(|s| s.contains("--codecraft-trae-hook"))
                    })
                })
            }) {
                return Err(ApiError::new(
                    ErrorCode::RequestConflict,
                    "Unowned CodeCraft Trae Hook already exists",
                ));
            }
            groups.push(group.clone());
        }
    }
    Ok(())
}
pub fn install(exe: &Path, enabled: bool) -> Result<Value> {
    install_at(&files::root(), &configuration()?, exe, enabled)?;
    status()
}
fn install_at(root: &Path, path: &Path, exe: &Path, enabled: bool) -> Result<()> {
    files::initialize(root)?;
    fs::create_dir_all(path.parent().unwrap())?;
    let _lock = files::lock(&path.with_extension("codecraft.lock"))?;
    let record = root.join("installation-record.json");
    let pending_path = root.join("installation-pending.json");
    if pending_path.exists() {
        let pending = files::read(&pending_path, 8 * 1024 * 1024)?;
        let current = if path.exists() {
            files::read(path, wire::MAX_IPC)?
        } else {
            json!({"version":1,"hooks":{}})
        };
        if current == pending["after"] {
            files::backup(&record)?;
            files::atomic(&record, &pending["record"], true)?;
        } else if current != pending["before"] {
            return Err(ApiError::new(
                ErrorCode::RequestConflict,
                "Trae configuration changed during an interrupted install",
            ));
        }
        fs::remove_file(&pending_path)?;
    }
    let prior = if record.exists() {
        files::read(&record, 1024 * 1024)?
    } else {
        json!({"groups":[]})
    };
    let owned: Vec<(String, Value)> = serde_json::from_value(prior["groups"].clone())
        .map_err(|_| invalid("Trae ownership record is corrupt"))?;
    let config_created = prior["configCreated"].as_bool().unwrap_or(!path.exists());
    let mut config = if path.exists() {
        files::read(path, wire::MAX_IPC)?
    } else {
        json!({"version":1,"hooks":{}})
    };
    let before = config.clone();
    let empty_events = prior["emptyEvents"].as_array().cloned().unwrap_or_else(|| {
        before["hooks"]
            .as_object()
            .into_iter()
            .flat_map(|hooks| hooks.iter())
            .filter(|(_, groups)| groups.as_array().is_some_and(Vec::is_empty))
            .map(|(event, _)| json!(event))
            .collect()
    });
    let new = if enabled { groups(exe) } else { vec![] };
    merge(&mut config, &owned, &new)?;
    for event in empty_events.iter().filter_map(Value::as_str) {
        config["hooks"]
            .as_object_mut()
            .unwrap()
            .entry(event)
            .or_insert(json!([]));
    }
    let next_record = json!({"groups":new,"enabled":enabled,"emptyEvents":empty_events,"configCreated":config_created,"installationId":prior["installationId"].as_str().map(str::to_string).unwrap_or_else(id)});
    files::atomic(
        &pending_path,
        &json!({"before":before,"after":config,"record":next_record}),
        true,
    )?;
    if config != before {
        files::backup(path)?;
        files::atomic(path, &config, true)?;
    }
    files::backup(&record)?;
    files::atomic(&record, &next_record, true)?;
    if !enabled && config_created && config == json!({"version":1,"hooks":{}}) && path.exists() {
        files::backup(path)?;
        fs::remove_file(path)?;
    }
    fs::remove_file(pending_path)?;
    Ok(())
}
pub fn status() -> Result<Value> {
    let env = detect();
    let path = configuration()?;
    let path_record = files::root().join("installation-record.json");
    let record = if path_record.exists() {
        files::read(&path_record, 1024 * 1024)?
    } else {
        json!({"groups":[]})
    };
    let groups: Vec<(String, Value)> = serde_json::from_value(record["groups"].clone())
        .map_err(|_| invalid("Trae ownership record is corrupt"))?;
    let current = if path.exists() {
        files::read(&path, wire::MAX_IPC)?
    } else {
        json!({})
    };
    let present = !groups.is_empty()
        && groups.iter().all(|(event, group)| {
            current["hooks"][event]
                .as_array()
                .is_some_and(|gs| gs.contains(group))
        });
    let snapshot = crate::store::snapshot();
    let recent = snapshot["sessions"].as_array().is_some_and(|s| {
        s.iter().any(|s| {
            s["updatedAt"]
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .is_some_and(|s| chrono::Utc::now().signed_duration_since(s).num_seconds() < 30)
        })
    });
    Ok(
        json!({"environment":env,"filesInstalled":present,"enabled":record["enabled"]==true,"traeEnabled":if recent{Some(true)}else{None},"recentEvents":recent,"state":if present{"installed"}else if groups.is_empty(){"notInstalled"}else{"modified"},"installPath":path,"capabilities":Capabilities::bundled(env["productVersion"].as_str())}),
    )
}
pub fn template() -> Result<Value> {
    let exe = env::current_exe()?;
    Ok(
        json!({"mcp":{"mcpServers":{"codecraft":{"command":exe,"args":["--codecraft-trae-mcp"]}}},"workflow":include_str!("../../protocol/trae/codecraft-v1/workflow.md"),"note":"Add the server in the intended Trae agent/project. Native questions and Plan/Spec confirmations remain in Trae."}),
    )
}
pub fn suppress_imported_claude() -> bool {
    if env::var_os("TRAE_PROJECT_DIR").is_none() {
        return false;
    }
    wire::host_evidence()
        .ok()
        .is_some_and(|v| v["candidate"].is_object())
        && status()
            .ok()
            .is_some_and(|v| v["filesInstalled"] == true && v["enabled"] == true)
}
fn source(input: &Value) -> Result<Value> {
    if env::var_os("TRAE_PROJECT_DIR").is_none()
        && !wire::workspace_less_observation(input["hook_event_name"].as_str().unwrap_or(""), input)
    {
        return Err(ApiError::new(
            ErrorCode::StateUnavailable,
            "Trae source evidence is missing",
        ));
    }
    let evidence = wire::host_evidence().map_err(invalid)?;
    let candidate = &evidence["candidate"];
    let exe = candidate["executable"].as_str().ok_or_else(|| {
        ApiError::new(
            ErrorCode::StateUnavailable,
            "Cannot identify Trae host process",
        )
    })?;
    let environment = detect();
    if environment["executable"]
        .as_str()
        .and_then(|p| Path::new(p).canonicalize().ok())
        != Path::new(exe).canonicalize().ok()
    {
        return Err(ApiError::new(
            ErrorCode::StateUnavailable,
            "Trae host does not match the detected installation",
        ));
    }
    let record = files::read(&files::root().join("installation-record.json"), 1024 * 1024)?;
    let installation = record["installationId"]
        .as_str()
        .ok_or_else(|| invalid("Trae installation is not registered"))?;
    Ok(
        json!({"installationId":installation,"traeInstanceId":wire::hash(&json!([candidate["pid"],candidate["createdFileTime"]])).map_err(invalid)?}),
    )
}
fn invoke_hook(event: &str) -> Result<Value> {
    let input = wire::strict_json(
        &wire::read_bounded(io::stdin().lock(), wire::MAX_NATIVE).map_err(invalid)?,
        wire::MAX_NATIVE,
    )
    .map_err(invalid)?;
    process_hook_input(event, &input, |input| forward_hook(event, input))
}
fn process_hook_input(
    event: &str,
    input: &Value,
    forward: impl FnOnce(&Value) -> Result<Value>,
) -> Result<Value> {
    wire::validate_hook(event, input).map_err(invalid)?;
    match forward(input) {
        Err(error) if wire::workspace_less_observation(event, input) => {
            // Recording standalone chat is best-effort. Missing source/project
            // evidence or an offline bridge must not block an ordinary message.
            eprintln!("Could not record Trae activity: {error}");
            Ok(json!({}))
        }
        result => result,
    }
}
fn forward_hook(event: &str, input: &Value) -> Result<Value> {
    let identity = source(input)?;
    let root = files::root();
    let key = wire::hash(&json!([
        identity["installationId"],
        identity["traeInstanceId"],
        input["session_id"]
    ]))
    .map_err(invalid)?;
    let client = match crate::transport::Client::connect(&root) {
        Ok(c) => c,
        Err(e) => {
            let ordinary = files::read(&root.join("bindings.json"), wire::MAX_IPC)
                .ok()
                .is_some_and(|v| v["sessions"][&key]["protected"] == false);
            if event == "PreToolUse"
                && ordinary
                && own_tool(input["tool_name"].as_str().unwrap_or("")).is_none()
            {
                return Ok(wire::permission("ask", &e.to_string(), None));
            }
            return Err(e);
        }
    };
    let fallback = |e: ApiError| -> Result<Value> {
        let ordinary = files::read(&root.join("bindings.json"), wire::MAX_IPC)
            .ok()
            .is_some_and(|v| {
                v["appEpoch"] == client.epoch && v["sessions"][&key]["protected"] == false
            });
        if event == "PreToolUse"
            && ordinary
            && own_tool(input["tool_name"].as_str().unwrap_or("")).is_none()
            && matches!(
                e.error.code,
                ErrorCode::RequestExpired
                    | ErrorCode::BridgeUnavailable
                    | ErrorCode::StateUnavailable
                    | ErrorCode::QueueFull
            )
        {
            return Ok(wire::permission("ask", &e.to_string(), None));
        }
        Err(e)
    };
    let started = Instant::now();
    let result = loop {
        match client.call(json!({"kind":"hook","input":input,"identity":identity})) {
            Ok(v) if v["waitForPlan"] == true => {
                if started.elapsed() >= Duration::from_secs(5) {
                    return Err(ApiError::new(
                        ErrorCode::DeliveryUnconfirmed,
                        "Plan approval delivery was not confirmed within five seconds",
                    ));
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            Ok(v) => break v,
            Err(e) => return fallback(e),
        }
    };
    if result.get("output").is_some() {
        return Ok(result["output"].clone());
    }
    let op = result["requestId"]
        .as_str()
        .ok_or_else(|| invalid("Missing tool request identity"))?;
    let start = Instant::now();
    loop {
        let state = match client.call(json!({"kind":"poll","operation":op})) {
            Ok(v) => v,
            Err(e) => return fallback(e),
        };
        if state["state"] != "pending" {
            let prepared = match client.call(json!({"kind":"prepare","operation":op})) {
                Ok(v) => v,
                Err(e) => return fallback(e),
            };
            let output = &prepared["output"];
            write(output)?;
            let ack = client.call(json!({"kind":"ack","operation":op,"lease":prepared["lease"]}));
            if let Err(e) = ack {
                eprintln!("Trae output flushed; confirmation failed: {e}");
            }
            return Ok(Value::Null);
        }
        if start.elapsed() > Duration::from_secs(120) {
            return Err(ApiError::new(
                ErrorCode::RequestExpired,
                "Tool approval timed out",
            ));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}
fn write(value: &Value) -> Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|e| invalid(e.to_string()))?;
    let mut out = io::stdout().lock();
    out.write_all(&bytes)?;
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}
pub fn capture() -> i32 {
    let args = env::args().collect::<Vec<_>>();
    let event = args
        .windows(2)
        .find(|w| w[0] == "--event")
        .map(|w| w[1].as_str())
        .unwrap_or("");
    if !wire::EVENTS.contains(&event) {
        eprintln!("Invalid fixed Trae Hook event");
        return 2;
    }
    let result = std::panic::catch_unwind(|| {
        wire::limit_cores().map_err(invalid)?;
        invoke_hook(event)
    });
    let output = match result {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            eprintln!("{e}");
            wire::failure(event, &e.to_string())
        }
        Err(_) => return 2,
    };
    if output.is_null() {
        return 0;
    }
    if write(&output).is_err() {
        2
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn chat_input() -> Value {
        serde_json::from_str::<Value>(include_str!(
            "../../protocol/trae/3.3.102/fixtures/workspace-less-chat.json"
        ))
        .unwrap()["input"]
            .clone()
    }
    #[test]
    fn workspace_less_chat_does_not_block_when_observation_is_unavailable() {
        for agent in ["chat", "solo_agent"] {
            for event in ["UserPromptSubmit", "SessionStart", "Notification", "Stop"] {
                let mut input = chat_input();
                input["agent_type"] = json!(agent);
                input["agent_id"] = json!(agent);
                input["hook_event_name"] = json!(event);
                for code in [ErrorCode::BridgeUnavailable, ErrorCode::StateUnavailable] {
                    let output = process_hook_input(event, &input, |_| {
                        Err(ApiError::new(code, "Observation unavailable"))
                    })
                    .unwrap();
                    assert_eq!(output, json!({}));
                }
            }
        }
    }
    #[test]
    fn chat_exception_preserves_tool_and_envelope_validation() {
        let input = chat_input();
        for event in ["PreToolUse", "PostToolUse"] {
            let mut tool = input.clone();
            tool["hook_event_name"] = json!(event);
            tool["tool_name"] = json!("RunCommand");
            tool["llm_tool_name"] = json!("RunCommand");
            tool["tool_use_id"] = json!("chat-tool");
            tool["tool_input"] = json!({"command":"echo test"});
            assert!(
                process_hook_input(event, &tool, |_| panic!("Unscoped tool forwarded")).is_err()
            );
        }
        for (field, bad) in [
            ("hook_event_name", json!("PreToolUse")),
            ("session_id", json!("")),
            ("prompt", Value::Null),
            ("agent_type", json!("builder")),
            ("workspace_roots", Value::Null),
            ("workspace_roots", json!(["relative"])),
            ("cwd", json!("..")),
        ] {
            let mut invalid = input.clone();
            invalid[field] = bad;
            assert!(process_hook_input("UserPromptSubmit", &invalid, |_| {
                panic!("Invalid event forwarded")
            })
            .is_err());
        }
        let mut scoped = input;
        scoped["cwd"] = json!(env!("CARGO_MANIFEST_DIR"));
        scoped["workspace_roots"] = json!([env!("CARGO_MANIFEST_DIR")]);
        assert!(process_hook_input("UserPromptSubmit", &scoped, |_| {
            Err(ApiError::new(ErrorCode::BridgeUnavailable, "offline"))
        })
        .is_err());
    }
    #[test]
    fn installation_backup_repair_uninstall_and_corruption_are_reversible() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/trae-installer-tests")
            .join(id());
        let state = root.join("state");
        let config = root.join("profile/hooks.json");
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        let original = json!({"version":1,"custom":"retain","hooks":{"Stop":[{"hooks":[{"command":"user-script"}]}],"Notification":[]}});
        files::atomic(&config, &original, true).unwrap();
        install_at(
            &state,
            &config,
            Path::new("C:/中文 ' app/CodeCraft.exe"),
            true,
        )
        .unwrap();
        let installed = files::read(&config, wire::MAX_IPC).unwrap();
        install_at(
            &state,
            &config,
            Path::new("C:/中文 ' app/CodeCraft.exe"),
            true,
        )
        .unwrap();
        assert_eq!(installed, files::read(&config, wire::MAX_IPC).unwrap());
        assert!(fs::read_dir(config.parent().unwrap())
            .unwrap()
            .flatten()
            .any(|e| e.path().extension().is_some_and(|v| v == "bak")));
        install_at(&state, &config, Path::new("C:/next/CodeCraft.exe"), true).unwrap();
        install_at(&state, &config, Path::new("C:/next/CodeCraft.exe"), false).unwrap();
        let uninstalled = files::read(&config, wire::MAX_IPC).unwrap();
        assert_eq!(uninstalled, original);
        assert_eq!(uninstalled["custom"], "retain");
        assert_eq!(uninstalled["hooks"]["Stop"], original["hooks"]["Stop"]);
        fs::write(state.join("installation-record.json"), "corrupt").unwrap();
        assert!(install_at(&state, &config, Path::new("C:/app.exe"), true).is_err());
        assert_eq!(files::read(&config, wire::MAX_IPC).unwrap(), uninstalled);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn uninstall_removes_only_a_new_empty_config_and_recovers_interrupted_install() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/trae-installer-tests")
            .join(id());
        let state = root.join("state");
        let config = root.join("profile/hooks.json");
        install_at(&state, &config, Path::new("C:/app.exe"), true).unwrap();
        let installed = files::read(&config, wire::MAX_IPC).unwrap();
        let record = files::read(&state.join("installation-record.json"), wire::MAX_IPC).unwrap();
        files::atomic(
            &state.join("installation-pending.json"),
            &json!({"before":{"version":1,"hooks":{}},"after":installed,"record":record}),
            true,
        )
        .unwrap();
        fs::remove_file(state.join("installation-record.json")).unwrap();
        install_at(&state, &config, Path::new("C:/app.exe"), false).unwrap();
        assert!(!config.exists());
        assert_eq!(
            files::read(&state.join("installation-record.json"), wire::MAX_IPC).unwrap()["enabled"],
            false
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn config_merge_is_precise_and_reversible() {
        let mut c =
            json!({"unknown":true,"hooks":{"PreToolUse":[{"hooks":[{"command":"user-hook"}]}]}});
        let original = c.clone();
        let g = groups(Path::new("C:/中文 ' folder/CodeCraft.exe"));
        merge(&mut c, &[], &g).unwrap();
        let first = c.clone();
        merge(&mut c, &g, &g).unwrap();
        assert_eq!(c, first);
        merge(&mut c, &g, &[]).unwrap();
        assert_eq!(c["unknown"], original["unknown"]);
        assert_eq!(c["hooks"]["PreToolUse"], original["hooks"]["PreToolUse"]);
    }
    #[test]
    fn modified_managed_hook_is_not_overwritten() {
        let g = groups(Path::new("C:/app.exe"));
        let mut c = json!({"hooks":{}});
        merge(&mut c, &[], &g).unwrap();
        c["hooks"]["PreToolUse"][0]["hooks"][0]["timeout"] = json!(42);
        assert!(merge(&mut c, &g, &g).is_err());
    }
}
