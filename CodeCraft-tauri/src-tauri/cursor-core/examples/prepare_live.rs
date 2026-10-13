//! Prepare an isolated online acceptance run using the production installer.
//! Does not start a broker, submit decisions, or assert any Cursor test result.
use codecraft_cursor::{product, project_rules, store::Store, workflow::Command};
use std::{env, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    if args.len() != 3 {
        return Err(
            "usage: prepare_live <existing run directory> <product exe> <Cursor.exe>".into(),
        );
    }
    let run = args[0].canonicalize()?;
    let allowed = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/cursor-probe/runs")
        .canonicalize()?;
    if !run.starts_with(&allowed) || run == allowed {
        return Err("run must be an existing child of scripts/cursor-probe/runs".into());
    }
    let root = run.join("local/CodeCraft/cursor");
    let home = run.join("home/.cursor");
    let project = run.join("project");
    product::initialize(&root)?;
    product::configure_at(&root, &home, &args[1], true, Some(args[2].clone()))?;
    let config = product::load(&root)?;
    project_rules::configure(&root, &project, true)?;
    let mut store = Store::open(&root.join("workflow"), &config.installation_id)?;
    store.execute(Command::SetProjectPath {
        path: project,
        enabled: true,
    })?;
    println!("Isolated production configuration prepared; no runtime verdict recorded.");
    Ok(())
}
