use crate::AgentAction;
use anyhow::Result;
use sectiond::SectiondControlPlane;
use serde_json::json;
use std::path::Path;

pub fn run(config_path: Option<&Path>, action: AgentAction, json_mode: bool) -> Result<()> {
    let control_plane = SectiondControlPlane::load(config_path)?;

    match action {
        AgentAction::Register { name } => {
            let agent = control_plane.agent_register(&name)?;
            if json_mode {
                println!(
                    "{}",
                    json!({
                        "ok": true,
                        "agent": agent,
                    })
                );
            } else {
                println!("Agent '{}' registered as {}.", agent.name, agent.agent_id);
            }
        }
        AgentAction::Identify => {
            let agent = control_plane.agent_identify()?;
            if json_mode {
                println!(
                    "{}",
                    json!({
                        "ok": agent.is_some(),
                        "agent": agent,
                    })
                );
            } else if let Some(agent) = agent {
                println!("{}  ({})", agent.agent_id, agent.name);
            } else {
                println!("No agent registered.");
            }
        }
    }

    Ok(())
}
