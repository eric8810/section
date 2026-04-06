use anyhow::Result;
use clap::{Parser, Subcommand};
use section_core::SectionConfig;
use section_provider::ProviderStore;
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

mod cmd;

#[derive(Parser)]
#[command(name = "section", about = "Section - Agent-first unified data layer")]
#[command(version)]
struct Cli {
    /// Config file path
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,

    /// Output results as JSON (for programmatic consumption)
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Manage data sources
    Source {
        #[command(subcommand)]
        action: SourceAction,
    },
    /// Inspect source/path sync state through local paths
    Path {
        #[command(subcommand)]
        action: PathAction,
    },
    /// List files
    Ls {
        /// Path (e.g., my-s3/documents/)
        path: Option<String>,
        /// Show richer metadata output
        #[arg(short = 'l', long)]
        long: bool,
    },
    /// Copy files
    Cp {
        /// Copy directories recursively
        #[arg(short, long)]
        recursive: bool,
        /// Source path
        src: String,
        /// Destination path
        dst: String,
    },
    /// Read file content to stdout
    Cat {
        /// Path to file
        path: String,
    },
    /// Remove file or directory
    Rm {
        /// Path to remove
        path: String,
        /// Recursive removal
        #[arg(short, long)]
        recursive: bool,
    },
    /// Mount the FUSE filesystem
    Mount {
        /// Mount point
        #[arg(default_value = "/mnt/section")]
        path: PathBuf,
    },
    /// Unmount the FUSE filesystem
    Unmount {
        /// Mount point
        #[arg(default_value = "/mnt/section")]
        path: PathBuf,
    },
    /// Force refresh cache for a path
    Refresh {
        /// Path to refresh
        path: String,
    },
    /// Execute a file from a data source
    Exec {
        /// Path to the file to execute (e.g., scripts/hello.sh)
        path: String,
        /// Arguments to pass to the script (after --)
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Write stdin content to a file in a data source
    Write {
        /// Destination path (e.g., store/new-file.txt)
        path: String,
    },
    /// Show mount status and source connectivity
    Status,
    /// Interactive setup for your first data source
    Init,
}

#[derive(Subcommand)]
pub enum SourceAction {
    /// Add a data source
    Add {
        /// Source name (e.g., "work-s3", "my-gdrive", "office-nas")
        name: String,
        /// Provider type (e.g., "s3", "fs", "webdav", "gdrive")
        #[arg(long)]
        provider: String,
        /// Provider-specific options as key=value pairs
        #[arg(long, value_parser = parse_key_val)]
        opt: Vec<(String, String)>,
    },
    /// Bind a source to a local root directory
    Bind {
        /// Source name
        name: String,
        /// Local root path
        local_root: PathBuf,
    },
    /// Remove the local-root binding for a source
    Unbind {
        /// Source name
        name: String,
    },
    /// Remove a data source
    Remove {
        /// Source name
        name: String,
    },
    /// List all data sources
    List,
}

#[derive(Subcommand)]
pub enum PathAction {
    /// Inspect sync state for a local path under a bound root
    Inspect {
        /// Local filesystem path
        path: PathBuf,
    },
}

fn parse_key_val(s: &str) -> Result<(String, String), String> {
    let pos = s
        .find('=')
        .ok_or_else(|| format!("invalid key=value: no '=' found in '{s}'"))?;
    Ok((s[..pos].to_string(), s[pos + 1..].to_string()))
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let json = cli.json;

    match cli.command {
        Commands::Source { action } => cmd::source::run(cli.config.as_deref(), action, json),
        Commands::Path { action } => cmd::path::run(cli.config.as_deref(), action, json),
        Commands::Ls { path, long } => {
            let (config, store) = load_config_and_store(cli.config.as_deref())?;
            cmd::file::ls(&config, &store, path.as_deref(), json, long)
        }
        Commands::Cp {
            recursive,
            src,
            dst,
        } => {
            let (config, store) = load_config_and_store(cli.config.as_deref())?;
            cmd::file::cp(&config, &store, &src, &dst, recursive, json)
        }
        Commands::Cat { path } => {
            let (config, store) = load_config_and_store(cli.config.as_deref())?;
            cmd::file::cat(&config, &store, &path, json)
        }
        Commands::Rm { path, recursive } => {
            let (config, store) = load_config_and_store(cli.config.as_deref())?;
            cmd::file::rm(&config, &store, &path, recursive, json)
        }
        Commands::Mount { path } => {
            let config = load_config(cli.config.as_deref())?;
            cmd::mount::mount(&config, cli.config.as_deref(), &path)
        }
        Commands::Unmount { path } => cmd::mount::unmount(&path),
        Commands::Refresh { path } => cmd::file::refresh(cli.config.as_deref(), &path, json),
        Commands::Exec { path, args } => {
            let (config, store) = load_config_and_store(cli.config.as_deref())?;
            cmd::file::exec(&config, &store, &path, &args, json)
        }
        Commands::Write { path } => {
            let (config, store) = load_config_and_store(cli.config.as_deref())?;
            cmd::file::write_stdin(&config, &store, &path, json)
        }
        Commands::Status => cmd::status::run(cli.config.as_deref(), json),
        Commands::Init => {
            let (config, store) = load_config_and_store(cli.config.as_deref())?;
            let _ = config;
            cmd::init::run(&store)
        }
    }
}

fn load_config(config_path: Option<&std::path::Path>) -> Result<SectionConfig> {
    let config = SectionConfig::load(config_path)?;
    config.ensure_dirs()?;
    Ok(config)
}

fn load_config_and_store(
    config_path: Option<&std::path::Path>,
) -> Result<(SectionConfig, ProviderStore)> {
    let config = load_config(config_path)?;
    let store = ProviderStore::open(&config.data_dir)?;
    Ok((config, store))
}
