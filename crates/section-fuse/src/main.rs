use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

mod fs;
mod inode;

#[derive(Parser)]
#[command(name = "section-fuse", about = "Section FUSE filesystem daemon")]
struct Args {
    /// Mount point
    #[arg(short, long, default_value = "/mnt/section")]
    mount_point: PathBuf,

    /// Config file path
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Run in foreground (don't daemonize)
    #[arg(short, long, default_value_t = true)]
    foreground: bool,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let args = Args::parse();

    let mut config = section_core::SectionConfig::load(args.config.as_deref())?;
    let store = section_provider::ProviderStore::open(&config.data_dir)?;

    // Merge DB sources into config
    let db_sources = store.load_all()?;
    for (name, source) in db_sources {
        config.sources.entry(name).or_insert(source);
    }

    let router = section_core::Router::from_config(&config)?;

    let mount_point = args.mount_point;
    std::fs::create_dir_all(&mount_point)?;

    tracing::info!("mounting section filesystem at {}", mount_point.display());

    let section_fs = fs::SectionFs::new(router);
    let mut options = vec![
        fuser::MountOption::FSName("section".to_string()),
        fuser::MountOption::AutoUnmount,
    ];

    #[cfg(target_os = "linux")]
    {
        options.push(fuser::MountOption::AllowOther);
    }

    fuser::mount2(section_fs, &mount_point, &options)?;

    Ok(())
}
