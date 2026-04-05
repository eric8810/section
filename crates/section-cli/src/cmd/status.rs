use anyhow::Result;
use section_core::SectionConfig;
use section_provider::ProviderStore;
use sectiond::SectiondRuntime;
use serde_json::json;

use super::mount::is_mount_active;

pub fn run(config: &SectionConfig, store: &ProviderStore, json_mode: bool) -> Result<()> {
    // 1. Check mount status
    let mount_point = &config.mount_point;
    let is_mounted = is_mount_active(mount_point);

    // 2. Let the shared runtime boundary materialize the merged source view.
    let runtime = SectiondRuntime::from_config_and_store(config, store)?;
    let router = runtime.router();
    let rt = tokio::runtime::Runtime::new()?;

    // Collect and sort source names for deterministic output
    let source_names = runtime.sources();

    if json_mode {
        let sources_json: Vec<serde_json::Value> = source_names
            .iter()
            .map(|source| {
                let connected = match router.get_operator(&source.name) {
                    Ok(op) => match rt.block_on(op.stat("/")) {
                        Ok(_) => true,
                        Err(_) => rt.block_on(op.list("/")).is_ok(),
                    },
                    Err(_) => false,
                };

                json!({
                    "name": source.name,
                    "provider": source.provider,
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
        if source_names.is_empty() {
            println!("  (none configured)");
            return Ok(());
        }

        for source in source_names {
            let status = match router.get_operator(&source.name) {
                Ok(op) => match rt.block_on(op.stat("/")) {
                    Ok(_) => "\u{2713} connected".to_string(),
                    Err(_) => match rt.block_on(op.list("/")) {
                        Ok(_) => "\u{2713} connected".to_string(),
                        Err(_) => "\u{2717} unreachable".to_string(),
                    },
                },
                Err(_) => "\u{2717} unreachable".to_string(),
            };

            println!(
                "  {name:<16}(provider: {provider:<10}, origin: {origin}) {status}",
                name = source.name,
                provider = source.provider,
                origin = source.origin.as_str(),
            );
        }
    }

    Ok(())
}
