use crate::{FsAction, FsRoleArg};
use anyhow::{bail, Result};
use sectiond::{AgentFsRole, SectiondControlPlane};
use serde_json::json;
use std::path::Path;

pub fn run(config_path: Option<&Path>, action: FsAction, json_mode: bool) -> Result<()> {
    let result = run_inner(config_path, action, json_mode);
    if let Err(err) = &result {
        if json_mode {
            super::print_agentfs_json_error(err, "fs");
        }
    }
    result
}

fn run_inner(config_path: Option<&Path>, action: FsAction, json_mode: bool) -> Result<()> {
    let control_plane = SectiondControlPlane::load(config_path)?;

    match action {
        FsAction::Create {
            name,
            source_profile,
            provider,
            opt,
        } => {
            if provider.is_some() || !opt.is_empty() {
                bail!("fs create now uses Section Control Service SourceProfiles; pass --source-profile <profile>");
            }
            let source_profile = source_profile
                .ok_or_else(|| anyhow::anyhow!("fs create requires --source-profile <profile>"))?;
            let fs = control_plane.fs_create(&name, &source_profile)?;
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
        FsAction::Grant {
            fs,
            agent_id,
            role,
            scopes,
        } => {
            let path_scopes = if scopes.is_empty() {
                None
            } else {
                Some(scopes)
            };
            let grant = control_plane.fs_grant(&fs, &agent_id, role.into(), path_scopes)?;
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
        FsAction::Share { fs, agent_id } => {
            let share = control_plane.fs_share(&fs, &agent_id)?;
            if json_mode {
                println!(
                    "{}",
                    json!({ "ok": true, "share": share.share, "fs": share.fs, "source_profile": share.source_profile })
                );
            } else {
                println!(
                    "Shared '{}' with {} as share {}.",
                    share.fs.name, share.share.target_agent_id, share.share.share_id
                );
            }
        }
        FsAction::Available => {
            let shares = control_plane.fs_available()?;
            if json_mode {
                println!("{}", json!({ "ok": true, "available": shares }));
            } else if shares.is_empty() {
                println!("No AgentFS shares available.");
            } else {
                for share in shares {
                    println!(
                        "  {}  ({}, role: {:?}, share: {})",
                        share.fs.name, share.fs.fs_id, share.share.role, share.share.share_id
                    );
                }
            }
        }
        FsAction::Accept { share_id } => {
            let accepted = control_plane.fs_accept(&share_id)?;
            if json_mode {
                println!(
                    "{}",
                    json!({
                        "ok": true,
                        "share": accepted.share,
                        "fs": accepted.fs,
                        "source_profile": accepted.source_profile,
                        "credential_binding": accepted.credential_binding,
                    })
                );
            } else {
                println!(
                    "Accepted share {} for '{}'.",
                    accepted.share.share_id, accepted.fs.name
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
        FsAction::Events { fs, after, limit } => {
            let events = control_plane.fs_events(&fs, after.as_deref(), limit)?;
            if json_mode {
                println!("{}", json!({ "ok": true, "events": events }));
            } else if events.is_empty() {
                println!("No AgentFS events.");
            } else {
                for event in events {
                    println!(
                        "[{}] {} {} {}",
                        event.seq, event.kind, event.fs_id, event.subject_id
                    );
                }
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
            FsRoleArg::Contributor => Self::Contributor,
        }
    }
}
