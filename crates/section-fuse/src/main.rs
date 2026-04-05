use anyhow::Result;
use clap::Parser;
use sectiond::SectiondRuntime;
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

    let runtime = SectiondRuntime::load(args.config.as_deref())?;
    let (config, router) = runtime.into_parts();

    let mount_point = args.mount_point;
    std::fs::create_dir_all(&mount_point)?;

    tracing::info!("mounting section filesystem at {}", mount_point.display());

    let section_fs = fs::SectionFs::new(&config, router);
    #[cfg(target_os = "linux")]
    let options = vec![
        fuser::MountOption::FSName("section".to_string()),
        fuser::MountOption::AutoUnmount,
        fuser::MountOption::AllowOther,
    ];

    #[cfg(not(target_os = "linux"))]
    let options = vec![
        fuser::MountOption::FSName("section".to_string()),
        fuser::MountOption::AutoUnmount,
    ];

    fuser::mount2(section_fs, &mount_point, &options)?;

    Ok(())
}
