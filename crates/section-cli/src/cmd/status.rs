use anyhow::Result;
use sectiond::SectiondControlPlane;
use serde_json::json;
use std::path::Path;

pub fn run(config_path: Option<&Path>, json_mode: bool) -> Result<()> {
    let status = SectiondControlPlane::load(config_path)?.status_snapshot()?;

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
                "  {name:<16}(provider: {provider:<10}, origin: {origin}, local_root: {local_root}) {status}",
                name = source.name,
                provider = source.provider,
                origin = source.origin.as_str(),
                local_root = source
                    .local_root
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "-".to_string()),
                status = connectivity,
            );
        }
    }

    Ok(())
}
