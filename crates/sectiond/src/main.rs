use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "sectiond",
    about = "Section runtime boundary and future daemon skeleton"
)]
#[command(version)]
struct Cli {
    /// Config file path
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,

    /// Output results as JSON
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Print the current sectiond runtime contract and merged source snapshot
    Inspect,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    match cli.command {
        Commands::Inspect => inspect(cli.config.as_deref(), cli.json),
    }
}

fn inspect(config_path: Option<&std::path::Path>, json_mode: bool) -> Result<()> {
    let runtime = sectiond::SectiondRuntime::load(config_path)?;
    let snapshot = runtime.snapshot(config_path);

    if json_mode {
        println!("{}", serde_json::to_string_pretty(&snapshot)?);
        return Ok(());
    }

    println!("sectiond runtime snapshot");
    println!("  mount point: {}", snapshot.mount_point.display());
    println!("  data dir: {}", snapshot.data_dir.display());
    println!("  source registry mode: {}", snapshot.source_registry_mode);
    println!("  sources:");
    if snapshot.sources.is_empty() {
        println!("    (none)");
    } else {
        for source in snapshot.sources {
            println!(
                "    - {} (provider: {}, origin: {}, metadata_ttl_secs: {}, content_ttl_secs: {})",
                source.name,
                source.provider,
                source.origin.as_str(),
                source.metadata_ttl_secs,
                source.content_ttl_secs
            );
        }
    }
    println!("  control plane:");
    for item in snapshot.contract.control_plane {
        println!("    - {item}");
    }
    println!("  data plane:");
    for item in snapshot.contract.data_plane {
        println!("    - {item}");
    }

    Ok(())
}
