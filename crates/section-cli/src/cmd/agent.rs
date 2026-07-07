use crate::AgentAction;
use anyhow::Result;
use sectiond::SectiondControlPlane;
use serde_json::json;
use std::path::Path;

pub fn run(config_path: Option<&Path>, action: AgentAction, json_mode: bool) -> Result<()> {
    let control_plane = SectiondControlPlane::load(config_path)?;

    match action {
        AgentAction::Login { name } => {
            let agent = control_plane.agent_login(&name)?;
            if json_mode {
                println!(
                    "{}",
                    json!({
                        "ok": true,
                        "agent": agent,
                        "installation": {
                            "installation_id": agent.installation_id,
                            "agent_id": agent.agent_id,
                        },
                    })
                );
            } else {
                println!(
                    "Agent '{}' logged in as {} on installation {}.",
                    agent.name, agent.agent_id, agent.installation_id
                );
            }
        }
        AgentAction::Register { name } => {
            let agent = control_plane.agent_register(&name)?;
            if json_mode {
                println!(
                    "{}",
                    json!({
                        "ok": true,
                        "agent": agent,
                        "installation": {
                            "installation_id": agent.installation_id,
                            "agent_id": agent.agent_id,
                        },
                    })
                );
            } else {
                println!(
                    "Agent '{}' logged in as {} on installation {}.",
                    agent.name, agent.agent_id, agent.installation_id
                );
            }
        }
        AgentAction::Identify => {
            let agent = control_plane.agent_identify()?;
            if json_mode {
                let installation = agent.as_ref().map(|agent| {
                    json!({
                        "installation_id": agent.installation_id,
                        "agent_id": agent.agent_id,
                    })
                });
                println!(
                    "{}",
                    json!({
                        "ok": agent.is_some(),
                        "agent": agent,
                        "installation": installation,
                    })
                );
            } else if let Some(agent) = agent {
                println!(
                    "{}  ({}) installation={}",
                    agent.agent_id, agent.name, agent.installation_id
                );
            } else {
                println!("No agent registered.");
            }
        }
    }

    Ok(())
}
