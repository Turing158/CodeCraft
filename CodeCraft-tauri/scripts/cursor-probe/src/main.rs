use codecraft_cursor_probe::{
    capture,
    compatibility::{Capability, Compatibility, RuntimeIdentity},
    limit_cpu, mcp, protocol,
};
use serde_json::json;
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
};

fn argument(args: &[String], name: &str) -> Result<String, String> {
    let positions = args
        .iter()
        .enumerate()
        .filter(|(_, s)| s.as_str() == name)
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    if positions.len() != 1 {
        return Err(format!("EXPECTED_ARGUMENT:{name}"));
    }
    args.get(positions[0] + 1)
        .filter(|s| !s.starts_with("--"))
        .cloned()
        .ok_or_else(|| format!("EXPECTED_ARGUMENT:{name}"))
}

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let affinity = limit_cpu();
    // Known Hook events always produce their legal passive response, even if
    // capture, input parsing or affinity setup fails. Never fall back to deny.
    if args.first().is_some_and(|arg| arg == "hook") {
        let result = (|| {
            let event = argument(&args, "--event")?;
            let response = protocol::passive_response(&event)?;
            let capture_result = affinity.and_then(|_| {
                let root = PathBuf::from(argument(&args, "--root")?);
                let bytes = protocol::read_bytes(io::stdin().lock())?;
                let input = protocol::hook_json(&bytes).inspect_err(|_error| {
                    // Observe transport encoding without saving any raw input,
                    // credentials, tickets or even fragments of those strings.
                    let _ = capture::record(&root,"hookStdin","inputRejected",&serde_json::Value::Null,json!({
                        "controlledEvent":event,"byteLength":bytes.len(),
                        "utf8Bom":bytes.starts_with(&[0xef,0xbb,0xbf]),
                        "utf16LeBom":bytes.starts_with(&[0xff,0xfe]),
                        "utf16BeBom":bytes.starts_with(&[0xfe,0xff]),
                        "validUtf8":std::str::from_utf8(&bytes).is_ok(),"containsNul":bytes.contains(&0)
                    }));
                })?;
                capture::capture_hook(&root, &event, &input, bytes.starts_with(&[0xef, 0xbb, 0xbf]))
            });
            if let Err(error) = capture_result {
                eprintln!("Cursor P0 capture: {error}");
            }
            let mut output = io::stdout().lock();
            serde_json::to_writer(&mut output, &response).map_err(|_| "STDOUT_FAILED")?;
            output
                .write_all(b"\n")
                .and_then(|_| output.flush())
                .map_err(|_| "STDOUT_FAILED")?;
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            eprintln!("Cursor P0 hook: {error}");
            std::process::exit(1);
        }
        return;
    }
    let result = affinity.and_then(|_| run(&args));
    if let Err(error) = result {
        eprintln!("Cursor P0: {error}");
        std::process::exit(1);
    }
}

fn run(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("init") => {
            let root = PathBuf::from(argument(args, "--root")?);
            capture::initialize(&root)?;
            println!(
                "{}",
                json!({"prepared":true,"root":root,"cursorClientExecuted":false,"capabilitiesEnabled":false})
            );
        }
        Some("mcp") => mcp::serve(
            Path::new(&argument(args, "--root")?),
            &argument(args, "--server")?,
            io::stdin().lock(),
            io::stdout().lock(),
        )?,
        Some("inspect") => {
            let root = PathBuf::from(argument(args, "--root")?);
            let events = capture::event_paths(&root)?
                .iter()
                .map(|p| {
                    protocol::read_input(std::fs::File::open(p).map_err(|_| "EVENT_UNREADABLE")?)
                })
                .collect::<Result<Vec<_>, String>>()?;
            println!("{}",serde_json::to_string_pretty(&json!({"evidenceKind":"probeCapture","capabilitiesEnabled":false,"events":events})).map_err(|_| "ENCODE_FAILED")?);
        }
        Some("compatibility") => {
            let path = PathBuf::from(argument(args, "--record")?);
            let record = Compatibility::load(&path)?;
            let runtime_path = argument(args, "--runtime")?;
            let runtime: RuntimeIdentity = serde_json::from_value(protocol::read_input(
                std::fs::File::open(runtime_path).map_err(|_| "RUNTIME_MISSING")?,
            )?)
            .map_err(|_| "INVALID_RUNTIME")?;
            let tool = argument(args, "--tool")?;
            let results = [
                Capability::HookOnly,
                Capability::NativeShared,
                Capability::AskUser,
                Capability::ReviewPlan,
            ]
            .map(|capability| {
                record.evaluate(
                    &runtime,
                    capability,
                    &tool,
                    path.parent().unwrap_or(Path::new(".")),
                )
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&results).map_err(|_| "ENCODE_FAILED")?
            );
        }
        _ => return Err("Use init, hook, mcp, inspect or compatibility; see README.md".into()),
    }
    Ok(())
}
