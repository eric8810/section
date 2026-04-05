use crate::SourceAction;
use anyhow::Result;
use section_core::config::{CacheConfig, SourceConfig};
use section_provider::ProviderStore;
use serde_json::json;
use std::collections::HashMap;

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

pub fn run(action: SourceAction, store: &ProviderStore, json_mode: bool) -> Result<()> {
    match action {
        SourceAction::Add {
            name,
            provider,
            opt,
        } => {
            let options: HashMap<String, String> = opt.into_iter().collect();
            let source = SourceConfig {
                provider: provider.clone(),
                options,
                cache: CacheConfig::default(),
            };
            store.add_source(&name, &source)?;
            if json_mode {
                println!(
                    "{}",
                    json!({"ok": true, "message": format!("Source '{name}' added (provider: {provider}).")})
                );
            } else {
                println!("Source '{name}' added (provider: {provider}).");
            }
        }
        SourceAction::Remove { name } => {
            store.remove_source(&name)?;
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
            let sources = store.list_sources()?;
            if json_mode {
                let arr: Vec<serde_json::Value> = sources
                    .iter()
                    .map(|(name, provider, options)| {
                        let masked_options: serde_json::Map<String, serde_json::Value> = options
                            .iter()
                            .map(|(k, v)| (k.clone(), json!(mask_value(k, v))))
                            .collect();
                        json!({
                            "name": name,
                            "provider": provider,
                            "options": masked_options,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string(&arr)?);
            } else if sources.is_empty() {
                println!("No sources configured.");
            } else {
                for (name, provider, options) in &sources {
                    println!("  {name}  (provider: {provider})");
                    let mut keys: Vec<&String> = options.keys().collect();
                    keys.sort();
                    for key in keys {
                        let value = &options[key];
                        let display_value = mask_value(key, value);
                        println!("    {key} = {display_value}");
                    }
                }
            }
        }
    }
    Ok(())
}
