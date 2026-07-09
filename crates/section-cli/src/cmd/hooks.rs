use crate::HooksAction;
use anyhow::Result;
use sectiond::SectiondControlPlane;
use serde_json::json;
use std::path::Path;

pub fn run(config_path: Option<&Path>, action: HooksAction, json_mode: bool) -> Result<()> {
    let result = run_inner(config_path, action, json_mode);
    if let Err(err) = &result {
        if json_mode {
            super::print_agentfs_json_error(err, "hooks");
        }
    }
    result
}

fn run_inner(config_path: Option<&Path>, action: HooksAction, json_mode: bool) -> Result<()> {
    let control_plane = SectiondControlPlane::load(config_path)?;

    match action {
        HooksAction::Add { fs, name, command } => {
            let hook = control_plane.hooks_add(&fs, &name, command)?;
            if json_mode {
                println!("{}", json!({ "ok": true, "hook": hook }));
            } else {
                println!("Added hook {} on '{}'.", hook.hook_id, fs);
            }
        }
        HooksAction::List { fs } => {
            let hooks = control_plane.hooks_list(&fs)?;
            if json_mode {
                println!("{}", json!({ "ok": true, "hooks": hooks }));
            } else if hooks.is_empty() {
                println!("No hooks.");
            } else {
                for hook in hooks {
                    println!("  {}  {}  {:?}", hook.hook_id, hook.name, hook.command);
                }
            }
        }
        HooksAction::Remove { fs, hook_id } => {
            let hook = control_plane.hooks_remove(&fs, &hook_id)?;
            if json_mode {
                println!("{}", json!({ "ok": true, "hook": hook }));
            } else {
                println!("Removed hook {} from '{}'.", hook.hook_id, fs);
            }
        }
    }

    Ok(())
}
