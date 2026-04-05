use crate::SourceAction;
use anyhow::Result;
use sectiond::SectiondControlPlane;
use serde_json::json;
use std::collections::HashMap;
use std::path::Path;

/// Mask sensitive option values for display.
///
/// - Keys containing "secret", "password", "token", or equal to "private_key"
///   are fully masked as `***`.
/// - Keys containing "key_id" or "access_key" (but not "secret") are partially
///   masked: first 4 characters shown followed by `***`, or `***` if the value
///   is 4 characters or shorter.
/// - All other keys are shown in plain text.
fn mask_value(key: &str, value: &str) -> String {
    let lower = key.to_lowercase();
    if lower.contains("secret")
        || lower.contains("password")
        || lower.contains("token")
        || lower == "private_key"
    {
        return "***".to_string();
    }
    if lower.contains("key_id") || lower.contains("access_key") {
        if value.len() <= 4 {
            return "***".to_string();
        }
        return format!("{}***", &value[..4]);
    }
    value.to_string()
}

pub fn run(config_path: Option<&Path>, action: SourceAction, json_mode: bool) -> Result<()> {
    let control_plane = SectiondControlPlane::load(config_path)?;

    match action {
        SourceAction::Add {
            name,
            provider,
            opt,
        } => {
            let options: HashMap<String, String> = opt.into_iter().collect();
            let entry = control_plane.source_add(&name, &provider, options)?;
            if json_mode {
                println!(
                    "{}",
                    json!({
                        "ok": true,
                        "message": format!("Source '{name}' added (provider: {provider})."),
                        "source": entry,
                    })
                );
            } else {
                println!("Source '{name}' added (provider: {provider}).");
            }
        }
        SourceAction::Remove { name } => {
            control_plane.source_remove(&name)?;
            if json_mode {
                println!(
                    "{}",
                    json!({"ok": true, "message": format!("Source '{name}' removed.")})
                );
            } else {
                println!("Source '{name}' removed.");
            }
        }
        SourceAction::List => {
            let sources = control_plane.list_sources()?;
            if json_mode {
                let arr: Vec<serde_json::Value> = sources
                    .iter()
                    .map(|source| {
                        let masked_options: serde_json::Map<String, serde_json::Value> = source
                            .options
                            .iter()
                            .map(|(k, v)| (k.clone(), json!(mask_value(k, v))))
                            .collect();
                        json!({
                            "name": source.name,
                            "provider": source.provider,
                            "origin": source.origin,
                            "metadata_ttl_secs": source.metadata_ttl_secs,
                            "content_ttl_secs": source.content_ttl_secs,
                            "options": masked_options,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string(&arr)?);
            } else if sources.is_empty() {
                println!("No sources configured.");
            } else {
                for source in &sources {
                    println!(
                        "  {}  (provider: {}, origin: {}, metadata_ttl_secs: {}, content_ttl_secs: {})",
                        source.name,
                        source.provider,
                        source.origin.as_str(),
                        source.metadata_ttl_secs,
                        source.content_ttl_secs,
                    );
                    let mut keys: Vec<&String> = source.options.keys().collect();
                    keys.sort();
                    for key in keys {
                        let value = &source.options[key];
                        let display_value = mask_value(key, value);
                        println!("    {key} = {display_value}");
                    }
                }
            }
        }
    }
    Ok(())
}
