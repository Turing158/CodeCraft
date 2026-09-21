//! Synthetic process harness. Never shipped as a CodeCraft entry point.
use codecraft_trae::{protocol::Capabilities, store, wire};
use std::{io::BufRead, path::PathBuf};
fn main() {
    wire::limit_cores().unwrap();
    let args = std::env::args().collect::<Vec<_>>();
    let root = PathBuf::from(args.get(2).expect("test root"));
    if matches!(args[1].as_str(), "store" | "store-bundled") {
        let caps = if args[1] == "store-bundled" {
            Capabilities::bundled(Some("3.3.102"))
        } else {
            Capabilities {
                tool_input_mappings_verified: true,
                tool_approval: true,
                mcp_questions: true,
                mcp_plan_review: true,
                native_observation: true,
                hook_timeout_seconds: Some(codecraft_trae::protocol::HOOK_TIMEOUT_SECONDS),
                verified_hook_timeout_seconds: Some(150),
                verified_mcp_timeout_seconds: Some(270),
                verified_version: Some("synthetic".into()),
                reason: String::new(),
            }
        };
        store::start_at(root, caps).unwrap();
        for line in std::io::stdin().lock().lines() {
            if line.unwrap() == "snapshot" {
                println!("{}", store::snapshot());
            }
        }
    } else if args[1] == "transport" {
        let client = codecraft_trae::transport::Client::connect(&root).unwrap();
        for line in std::io::stdin().lock().lines() {
            let command: serde_json::Value = serde_json::from_str(&line.unwrap()).unwrap();
            let result = client.call(command);
            println!(
                "{}",
                serde_json::json!({"result":result.as_ref().ok(),"error":result.as_ref().err()})
            );
        }
    } else {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .max_blocking_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let result = runtime.block_on(codecraft_trae::mcp::serve(&root));
        runtime.shutdown_timeout(std::time::Duration::from_secs(3));
        if let Err(e) = result {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
