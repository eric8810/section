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

    // 2. Let the shared runtime boundary materialize and probe the merged source view.
    let runtime = SectiondRuntime::from_config_and_store(config, store)?;
    let status = runtime.status_snapshot(mount_point, is_mounted)?;

    if json_mode {
        let output = json!({
            "mount": {
                "path": status.mount_path.to_string_lossy(),
                "active": status.mount_active,
            },
            "sources": status.sources,
        });
        println!("{}", serde_json::to_string(&output)?);
    } else {
        if status.mount_active {
            println!("Mount: {} (active)", status.mount_path.display());
        } else {
            println!("Mount: not mounted");
        }

        println!("Sources:");
        if status.sources.is_empty() {
            println!("  (none configured)");
            return Ok(());
        }

        for source in status.sources {
            let connectivity = if source.connected {
                "\u{2713} connected"
            } else {
                "\u{2717} unreachable"
            };

            println!(
                "  {name:<16}(provider: {provider:<10}, origin: {origin}) {status}",
                name = source.name,
                provider = source.provider,
                origin = source.origin.as_str(),
                status = connectivity,
            );
        }
    }

    Ok(())
}
