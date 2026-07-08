use crate::CommitAction;
use anyhow::Result;
use sectiond::SectiondControlPlane;
use serde_json::json;
use std::path::Path;

pub fn run(config_path: Option<&Path>, action: CommitAction, json_mode: bool) -> Result<()> {
    let result = run_inner(config_path, action, json_mode);
    if let Err(err) = &result {
        if json_mode {
            super::print_agentfs_json_error(err, "commit");
        }
    }
    result
}

fn run_inner(config_path: Option<&Path>, action: CommitAction, json_mode: bool) -> Result<()> {
    let control_plane = SectiondControlPlane::load(config_path)?;

    match action {
        CommitAction::Status { path } => {
            let status = control_plane.commit_status(&path)?;
            if json_mode {
                println!("{}", json!({ "ok": true, "status": status }));
            } else if status.dirty_paths.is_empty() {
                println!("No dirty paths.");
            } else {
                for dirty in status.dirty_paths {
                    println!("{}  {}  {}", dirty.op, dirty.kind, dirty.path);
                }
            }
        }
        CommitAction::Apply { path, message } => {
            let result = control_plane.commit_apply(&path, &message)?;
            if json_mode {
                println!(
                    "{}",
                    json!({ "ok": true, "commit": result.commit, "sync": result.sync, "warnings": result.warnings })
                );
            } else {
                println!(
                    "Commit {} accepted and materialized.",
                    result.commit.commit_id
                );
                print_warnings(&result.warnings);
            }
        }
        CommitAction::Repair { fs, commit } => {
            let result = control_plane.commit_repair(&fs, commit.as_deref())?;
            if json_mode {
                println!(
                    "{}",
                    json!({ "ok": true, "commit": result.commit, "sync": result.sync, "warnings": result.warnings })
                );
            } else {
                println!("Commit {} materialized.", result.commit.commit_id);
                print_warnings(&result.warnings);
            }
        }
    }

    Ok(())
}

fn print_warnings(warnings: &[String]) {
    for warning in warnings {
        eprintln!("warning: {warning}");
    }
}
