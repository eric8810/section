use anyhow::Result;
use section_core::{Router, SectionConfig};
use section_provider::ProviderStore;
use serde_json::json;
use std::path::Path;

pub fn run(config: &SectionConfig, store: &ProviderStore, json_mode: bool) -> Result<()> {
    // 1. Check mount status
    let mount_point = &config.mount_point;
    let is_mounted = check_mounted(mount_point);

    // 2. Merge file-based and DB-based sources
    let mut full_config = config.clone();
    let db_sources = store.load_all()?;
    for (name, source) in db_sources {
        full_config.sources.entry(name).or_insert(source);
    }

    // Build router (this creates operators for all sources)
    let router = if full_config.sources.is_empty() {
        None
    } else {
        Some(Router::from_config(&full_config)?)
    };
    let rt = tokio::runtime::Runtime::new()?;

    // Collect and sort source names for deterministic output
    let mut source_names: Vec<&String> = full_config.sources.keys().collect();
    source_names.sort();

    if json_mode {
        let sources_json: Vec<serde_json::Value> = source_names
            .iter()
            .map(|name| {
                let source_cfg = &full_config.sources[*name];
                let provider = &source_cfg.provider;

                let connected = match &router {
                    Some(r) => match r.get_operator(name) {
                        Ok(op) => match rt.block_on(op.stat("/")) {
                            Ok(_) => true,
                            Err(_) => rt.block_on(op.list("/")).is_ok(),
                        },
                        Err(_) => false,
                    },
                    None => false,
                };

                json!({
                    "name": name,
                    "provider": provider,
                    "connected": connected,
                })
            })
            .collect();

        let output = json!({
            "mount": {
                "path": mount_point.to_string_lossy(),
                "active": is_mounted,
            },
            "sources": sources_json,
        });
        println!("{}", serde_json::to_string(&output)?);
    } else {
        if is_mounted {
            println!("Mount: {} (active)", mount_point.display());
        } else {
            println!("Mount: not mounted");
        }

        println!("Sources:");
        if full_config.sources.is_empty() {
            println!("  (none configured)");
            return Ok(());
        }

        for name in source_names {
            let source_cfg = &full_config.sources[name];
            let provider = &source_cfg.provider;

            let status = match &router {
                Some(r) => match r.get_operator(name) {
                    Ok(op) => {
                        match rt.block_on(op.stat("/")) {
                            Ok(_) => "\u{2713} connected".to_string(),
                            Err(_) => {
                                match rt.block_on(op.list("/")) {
                                    Ok(_) => "\u{2713} connected".to_string(),
                                    Err(_) => "\u{2717} unreachable".to_string(),
                                }
                            }
                        }
                    }
                    Err(_) => "\u{2717} unreachable".to_string(),
                },
                None => "\u{2717} unreachable".to_string(),
            };

            println!("  {name:<16}(provider: {provider:<10}) {status}");
        }
    }

    Ok(())
}

fn check_mounted(mount_point: &Path) -> bool {
    if let Ok(mounts) = std::fs::read_to_string("/proc/mounts") {
        let mount_str = mount_point.to_string_lossy();
        for line in mounts.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 && parts[1] == mount_str.as_ref() {
                return true;
            }
        }
    }
    false
}
