use crate::{FsAction, FsRoleArg};
use anyhow::Result;
use sectiond::{AgentFsError, AgentFsRole, SectiondControlPlane};
use serde_json::json;
use std::collections::HashMap;
use std::path::Path;

pub fn run(config_path: Option<&Path>, action: FsAction, json_mode: bool) -> Result<()> {
    let result = run_inner(config_path, action, json_mode);
    if let Err(err) = &result {
        if json_mode {
            if let Some(agentfs_error) = err.downcast_ref::<AgentFsError>() {
                println!("{}", json!({ "error": agentfs_error.payload() }));
            }
        }
    }
    result
}

fn run_inner(config_path: Option<&Path>, action: FsAction, json_mode: bool) -> Result<()> {
    let control_plane = SectiondControlPlane::load(config_path)?;

    match action {
        FsAction::Create {
            name,
            provider,
            opt,
        } => {
            let options: HashMap<String, String> = opt.into_iter().collect();
            let fs = control_plane.fs_create(&name, &provider, options)?;
            if json_mode {
                println!("{}", json!({ "ok": true, "fs": fs }));
            } else {
                println!("AgentFS '{}' created as {}.", fs.name, fs.fs_id);
            }
        }
        FsAction::List => {
            let filesystems = control_plane.fs_list()?;
            if json_mode {
                println!("{}", serde_json::to_string(&filesystems)?);
            } else if filesystems.is_empty() {
                println!("No AgentFS filesystems found.");
            } else {
                for fs in filesystems {
                    println!(
                        "  {}  ({}, source: {}, owner: {})",
                        fs.name, fs.fs_id, fs.source_name, fs.owner_agent_id
                    );
                }
            }
        }
        FsAction::Grant { fs, agent_id, role } => {
            let grant = control_plane.fs_grant(&fs, &agent_id, role.into())?;
            if json_mode {
                println!("{}", json!({ "ok": true, "grant": grant }));
            } else {
                println!(
                    "Granted {:?} on '{}' to {}.",
                    grant.role, fs, grant.agent_id
                );
            }
        }
        FsAction::Revoke { fs, agent_id } => {
            let revoked = control_plane.fs_revoke(&fs, &agent_id)?;
            if json_mode {
                println!("{}", json!({ "ok": true, "revoked": revoked }));
            } else {
                println!(
                    "Revoked {} active grant(s) on '{}' for {}.",
                    revoked.len(),
                    fs,
                    agent_id
                );
            }
        }
        FsAction::Attach { fs, local_root } => {
            let attached = control_plane.fs_attach(&fs, &local_root)?;
            if json_mode {
                println!("{}", json!({ "ok": true, "attach": attached }));
            } else {
                println!(
                    "Attached '{}' to {}.",
                    attached.fs.name,
                    attached.local_root.display()
                );
            }
        }
        FsAction::Status { fs } => {
            let status = control_plane.fs_status(&fs)?;
            if json_mode {
                println!("{}", json!({ "ok": true, "status": status }));
            } else {
                println!(
                    "{}  ({}) head={}",
                    status.fs.name,
                    status.fs.fs_id,
                    status.head.commit_id.as_deref().unwrap_or("<empty>")
                );
            }
        }
    }

    Ok(())
}

impl From<FsRoleArg> for AgentFsRole {
    fn from(value: FsRoleArg) -> Self {
        match value {
            FsRoleArg::Reader => Self::Reader,
            FsRoleArg::Writer => Self::Writer,
            FsRoleArg::Manager => Self::Manager,
        }
    }
}
