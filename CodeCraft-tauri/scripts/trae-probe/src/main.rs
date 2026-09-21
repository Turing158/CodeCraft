mod mcp;
mod protocol;
mod store;

use protocol::*;
use serde_json::{json, Value};
use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use store::{Client, Command, Identity};

fn option(args: &[String], name: &str) -> Result<String, String> {
    let indices = args
        .iter()
        .enumerate()
        .filter(|(_, v)| *v == name)
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    if indices.len() != 1 {
        return Err(format!("Expected exactly one {name}"));
    }
    args.get(indices[0] + 1)
        .filter(|v| !v.starts_with("--"))
        .cloned()
        .ok_or_else(|| format!("Missing {name} value"))
}
fn write_json(value: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    let mut stdout = io::stdout().lock();
    stdout
        .write_all(&bytes)
        .and_then(|_| stdout.write_all(b"\n"))
        .and_then(|_| stdout.flush())
        .map_err(|e| e.to_string())
}
fn ps_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn initialize(root: &Path) -> Result<Value, String> {
    if !root.is_absolute() {
        return Err("Run root must be absolute".into());
    }
    fs::create_dir(root).map_err(|e| format!("Create a NEW disposable run directory: {e}"))?;
    for dir in [
        "inbox",
        "processing",
        "replies",
        "journal",
        "events",
        "project/.trae",
    ] {
        fs::create_dir_all(root.join(dir)).map_err(|e| e.to_string())?;
    }
    store::atomic_new(
        &root.join("probe-run.json"),
        &json!({"schemaVersion":1,"kind":"codecraft-trae-p0","createdAtMs":store::wall_ms(),"capabilitiesVerified":false}),
    )?;
    store::atomic_new(
        &root.join("control.json"),
        &json!({"case":"observe","delayMs":0}),
    )?;
    store::atomic_new(
        &root.join("transport.json"),
        &json!({"beforeWriteMs":0,"afterFlushMs":0,"dropAck":false}),
    )?;
    let exe = env::current_exe().map_err(|e| e.to_string())?;
    let mut hooks = serde_json::Map::new();
    for event in EVENTS {
        let command = format!(
            "& {} hook --root {} --event {}",
            ps_quote(&exe.to_string_lossy()),
            ps_quote(&root.to_string_lossy()),
            event
        );
        let mut group = json!({"hooks":[{"type":"command","command":command,"timeout":if event=="PreToolUse" {150} else {30}}]});
        if matches!(event, "PreToolUse" | "PostToolUse" | "Notification") {
            group["matcher"] = json!("*");
        }
        hooks.insert(event.into(), json!([group]));
    }
    store::atomic_new(
        &root.join("project/.trae/hooks.json"),
        &json!({"version":1,"hooks":hooks}),
    )?;
    store::atomic_new(
        &root.join("mcp.preview.json"),
        &json!({"mcpServers":{"codecraft_probe":{"command":exe,"args":["mcp","--root",root]}}}),
    )?;
    fs::write(
        root.join("project/fixture.txt"),
        "CodeCraft Trae P0 fixture. Safe to read.\n",
    )
    .map_err(|e| e.to_string())?;
    fs::write(root.join("project/README.md"),"# Trae P0 isolated project\n\nUse only for CodeCraft protocol experiments. Never open a real project with the probe Hook. MCP configuration is a preview; add it in Trae settings and verify the server name. Keep native confirmation and sandbox settings unchanged.\n").map_err(|e|e.to_string())?;
    Ok(
        json!({"root":root,"project":root.join("project"),"mcpPreview":root.join("mcp.preview.json"),"status":"prepared_not_verified"}),
    )
}

fn scope(root: &Path, input: &Value) -> Result<String, String> {
    let project = root
        .join("project")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let cwd = Path::new(input["cwd"].as_str().unwrap())
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let roots = input["workspace_roots"].as_array().unwrap();
    if cwd != project
        || roots.len() != 1
        || Path::new(roots[0].as_str().unwrap())
            .canonicalize()
            .map_err(|e| e.to_string())?
            != project
    {
        return Err("FORBIDDEN: probe only runs in its isolated, single-root test project".into());
    }
    Ok(project.to_string_lossy().into())
}

fn hook(root: &Path, event: &str, synthetic: bool) -> Result<Value, String> {
    store::assert_run(root)?;
    let bytes = read_bounded(io::stdin().lock(), MAX_NATIVE)?;
    let input = strict_json(&bytes, MAX_NATIVE)?;
    validate_hook(event, &input)?;
    let canonical_root = scope(root, &input)?;
    let evidence = if synthetic {
        json!({"candidate":{"pid":1,"createdFileTime":"synthetic"},"verified":false})
    } else {
        host_evidence()?
    };
    let captured = json!({"evidenceKind":if synthetic {"synthetic"} else {"runtime_capture_unverified"},"capturedAtMs":store::wall_ms(),"host":evidence,"input":input});
    // Raw captures are kept in ignored disposable runs. Never publish these
    // automatically: prompts, paths and bridge tickets require redaction.
    if fs::read_dir(root.join("events"))
        .map_err(|e| e.to_string())?
        .count()
        >= 1024
    {
        return Err("QUEUE_FULL: raw event capture limit".into());
    }
    store::atomic_new(
        &root.join("events").join(format!("{}.json", store::id())),
        &captured,
    )?;
    let candidate = evidence["candidate"]
        .as_object()
        .ok_or("STATE_UNAVAILABLE: no Trae host ancestor; inspect capture")?;
    if !synthetic && env::var_os("TRAE_PROJECT_DIR").is_none() {
        return Err("STATE_UNAVAILABLE: missing Trae source evidence".into());
    }
    let identity = Identity {
        host: hash(&Value::Object(candidate.clone()))?,
        session: input["session_id"].as_str().unwrap().into(),
        root: canonical_root,
        call: input["tool_use_id"].as_str().unwrap_or("").into(),
    };
    let client = Client::connect(root)?;
    if event != "PreToolUse" {
        let observed = client.call(Command::Observe {
            identity,
            event: event.into(),
            prompt: input["prompt"].as_str().map(str::to_string),
        })?;
        if let Some(context) = observed["additionalContext"].as_str() {
            return Ok(
                json!({"hookSpecificOutput":{"hookEventName":event,"additionalContext":context}}),
            );
        }
        return Ok(json!({}));
    }
    let control = strict_json(
        &read_bounded(
            File::open(root.join("control.json")).map_err(|e| e.to_string())?,
            4096,
        )?,
        4096,
    )?;
    let case = control["case"]
        .as_str()
        .ok_or("INVALID_ARGUMENT: probe case")?;
    match case {
        "inject" => {
            let tool = own_tool(input["tool_name"].as_str().unwrap())
                .ok_or("INVALID_ARGUMENT: injection only applies to exact probe MCP tools")?;
            let reply = client.call(Command::Register {
                identity,
                tool: tool.into(),
                arguments: input["tool_input"].clone(),
            })?;
            let mut updated = input["tool_input"].clone();
            updated["bridgeTicket"] = reply["bridgeTicket"].clone();
            Ok(permission(
                "allow",
                "CodeCraft P0 ticket injection; native confirmation remains applicable",
                Some(updated),
            ))
        }
        "allow" | "deny" | "ask" => Ok(permission(case, "CodeCraft P0 explicit test case", None)),
        "delay" => {
            let ms = control["delayMs"]
                .as_u64()
                .filter(|v| *v <= 180_000)
                .ok_or("INVALID_ARGUMENT: delayMs must be <= 180000")?;
            std::thread::sleep(Duration::from_millis(ms));
            Ok(permission("deny", "CodeCraft P0 delay finished", None))
        }
        "exit2" => {
            eprintln!("CodeCraft P0 intentional exit 2");
            std::process::exit(2)
        }
        "error" => {
            eprintln!("CodeCraft P0 intentional non-blocking error");
            std::process::exit(1)
        }
        "observe" => Ok(permission(
            "ask",
            "CodeCraft P0 observation; decide in Trae",
            None,
        )),
        _ => Err("INVALID_ARGUMENT: unknown probe case".into()),
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let root = PathBuf::from(option(args, "--root")?);
    let sub = args.first().map(String::as_str).unwrap_or("");
    let result=match sub {
        "init"=>initialize(&root)?,
        "coordinator"=>{store::coordinate(&root)?; return Ok(())},
        "mcp"=>{
            let runtime=tokio::runtime::Builder::new_multi_thread().worker_threads(2).max_blocking_threads(2).enable_all().build().map_err(|e|e.to_string())?;
            let result=runtime.block_on(mcp::serve(&root));
            // Tokio's stdin uses a blocking reader that cannot be cancelled.
            // Bound shutdown when stdout fails while the caller keeps stdin open.
            runtime.shutdown_timeout(Duration::from_secs(3));
            result?;
            return Ok(())
        }
        "arm"=>Client::connect(&root)?.call(Command::Arm)?,
        "inspect"=>Client::connect(&root)?.call(Command::Inspect)?,
        "release"=>Client::connect(&root)?.call(Command::Release {operation:option(args,"--operation")?})?,
        "transport"=>{
            store::assert_run(&root)?;
            let millis=|flag| -> Result<u64,String> {if args.iter().any(|v|v==flag) {option(args,flag)?.parse::<u64>().map_err(|e|e.to_string()).and_then(|v|if v<=10_000 {Ok(v)} else {Err("transport delay must be <=10000ms".into())})} else {Ok(0)}};
            let value=json!({"beforeWriteMs":millis("--before-write-ms")?,"afterFlushMs":millis("--after-flush-ms")?,"dropAck":args.iter().any(|v|v=="--drop-ack")});
            let mut file=OpenOptions::new().write(true).truncate(true).open(root.join("transport.json")).map_err(|e|e.to_string())?;
            file.write_all(value.to_string().as_bytes()).and_then(|_|file.sync_all()).map_err(|e|e.to_string())?;
            value
        }
        "case"=>{
            store::assert_run(&root)?;
            let case=option(args,"--case")?;
            if !["observe","inject","allow","deny","ask","delay","exit2","error"].contains(&case.as_str()) {return Err("Unknown case".into());}
            let delay=if args.iter().any(|v|v=="--delay-ms") {option(args,"--delay-ms")?.parse::<u64>().map_err(|e|e.to_string())?} else {0};
            if delay>180_000 {return Err("delay must be <= 180000ms".into());}
            // A run is explicitly disposable and owns this file. It must never
            // be pointed at a user's existing Trae configuration directory.
            let value=json!({"case":case,"delayMs":delay});
            let mut file=OpenOptions::new().write(true).truncate(true).open(root.join("control.json")).map_err(|e|e.to_string())?;
            file.write_all(value.to_string().as_bytes()).and_then(|_|file.sync_all()).map_err(|e|e.to_string())?;
            value
        }
        _=>return Err("Usage: codecraft-trae-probe init|coordinator|mcp|hook|case|arm|inspect|release --root <absolute disposable run>".into()),
    };
    write_json(&result)
}

fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.first().is_some_and(|v| v == "hook") {
        // Event identity comes from argv even when stdin is damaged. Any panic
        // or serialization/write failure exits 2, never an accidental success.
        let event = option(&args, "--event").unwrap_or_default();
        let result = std::panic::catch_unwind(|| {
            limit_cores()?;
            let root = PathBuf::from(option(&args, "--root")?);
            hook(&root, &event, args.iter().any(|v| v == "--synthetic"))
        });
        let output = match result {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => {
                eprintln!("{e}");
                failure(&event, &e)
            }
            Err(_) => {
                eprintln!("CodeCraft P0 Hook panic");
                std::process::exit(2)
            }
        };
        if !EVENTS.contains(&event.as_str()) || write_json(&output).is_err() {
            std::process::exit(2);
        }
        return;
    }
    if let Err(error) = limit_cores().and_then(|_| run(&args)) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
