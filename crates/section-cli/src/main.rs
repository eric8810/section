use anyhow::Result;
use clap::{error::ErrorKind, Parser, Subcommand, ValueEnum};
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
    /// Manage local agent identity
    Agent {
        #[command(subcommand)]
        action: AgentAction,
    },
    /// Manage AgentFS filesystems
    Fs {
        #[command(subcommand)]
        action: FsAction,
    },
    /// Inspect and apply AgentFS commits
    Commit {
        #[command(subcommand)]
        action: CommitAction,
    },
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
    /// Subscribe to source/path state-change events from a local path
    Watch {
        /// Local filesystem path or bound root
        path: PathBuf,
        /// Watch AgentFS governance events instead of low-level source/path events
        #[arg(long)]
        agentfs: bool,
        /// Exit after printing currently available events
        #[arg(long)]
        once: bool,
        /// Poll interval while following
        #[arg(long, default_value_t = 250)]
        interval_ms: u64,
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
    /// Run source/path sync for a bound source
    Sync {
        /// Source name
        name: String,
        /// Keep syncing in a loop
        #[arg(long)]
        watch: bool,
        /// Path and transport concurrency
        #[arg(long, default_value_t = 8)]
        concurrency: usize,
        /// Poll interval in seconds while watching
        #[arg(long, default_value_t = 2)]
        interval_secs: u64,
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
pub enum AgentAction {
    /// Log in through the Section Control Service
    Login {
        /// Agent display name
        name: String,
    },
    /// Register or rename the local agent identity
    Register {
        /// Agent display name
        name: String,
    },
    /// Show the local agent identity
    Identify,
}

#[derive(Subcommand)]
pub enum FsAction {
    /// Create an agent-owned filesystem
    Create {
        /// Filesystem name
        name: String,
        /// Server-managed SourceProfile name
        #[arg(long)]
        source_profile: Option<String>,
        /// Deprecated provider type; use --source-profile
        #[arg(long)]
        provider: Option<String>,
        /// Deprecated provider-specific options; use --source-profile
        #[arg(long, value_parser = parse_key_val)]
        opt: Vec<(String, String)>,
    },
    /// List AgentFS filesystems visible through local sources
    List,
    /// Grant another agent access to an FS
    Grant {
        /// FS name, fs_id, or source name
        fs: String,
        /// Agent id to grant
        agent_id: String,
        /// Role to grant
        #[arg(long, value_enum)]
        role: FsRoleArg,
    },
    /// Revoke an agent's active grants on an FS
    Revoke {
        /// FS name, fs_id, or source name
        fs: String,
        /// Agent id to revoke
        agent_id: String,
    },
    /// Create a server-side share for a granted agent
    Share {
        /// FS name, fs_id, or source name
        fs: String,
        /// Agent id to share with
        agent_id: String,
    },
    /// List server-side shares available to the logged-in agent
    Available,
    /// Accept a server-side share
    Accept {
        /// Share id
        share_id: String,
    },
    /// Attach an FS to a local working directory
    Attach {
        /// FS name, fs_id, or source name
        fs: String,
        /// Local working directory
        local_root: PathBuf,
    },
    /// Show AgentFS status
    Status {
        /// FS name, fs_id, source name, or attached local path
        fs: String,
    },
    /// Replay AgentFS governance events
    Events {
        /// FS name, fs_id, source name, or attached local path
        fs: String,
        /// Resume after an event seq or event_id
        #[arg(long)]
        after: Option<String>,
        /// Maximum events to print
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum FsRoleArg {
    Reader,
    Writer,
    Manager,
}

#[derive(Subcommand)]
pub enum CommitAction {
    /// Show dirty paths and stale-base state for an attached AgentFS root
    Status {
        /// Attached local path or root
        path: PathBuf,
    },
    /// Accept and materialize all dirty paths under an attached AgentFS root
    Apply {
        /// Attached local path or root
        path: PathBuf,
        /// Commit summary
        #[arg(long)]
        message: String,
    },
    /// Retry materialization for an accepted pending or failed commit
    Repair {
        /// FS name, fs_id, source name, or attached local path
        fs: String,
        /// Commit id to repair. Defaults to current head.
        #[arg(long)]
        commit: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum PathAction {
    /// Inspect sync state for a local path under a bound root
    Inspect {
        /// Local filesystem path
        path: PathBuf,
    },
    /// Compare local truth against remote truth for a local path
    Compare {
        /// Local filesystem path
        path: PathBuf,
    },
    /// Resolve a conflicted path
    Resolve {
        /// Local filesystem path
        path: PathBuf,
        /// Resolution strategy
        #[arg(long, value_enum)]
        strategy: ResolveStrategyArg,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum ResolveStrategyArg {
    UseLocal,
    UseRemote,
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

    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            if json_arg_requested()
                && !matches!(
                    err.kind(),
                    ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
                )
            {
                print_json_parse_error(&err);
                std::process::exit(err.exit_code());
            }
            err.exit();
        }
    };
    let json = cli.json;

    match cli.command {
        Commands::Agent { action } => cmd::agent::run(cli.config.as_deref(), action, json),
        Commands::Fs { action } => cmd::fs::run(cli.config.as_deref(), action, json),
        Commands::Commit { action } => cmd::commit::run(cli.config.as_deref(), action, json),
        Commands::Source { action } => cmd::source::run(cli.config.as_deref(), action, json),
        Commands::Path { action } => cmd::path::run(cli.config.as_deref(), action, json),
        Commands::Watch {
            path,
            agentfs,
            once,
            interval_ms,
        } => cmd::watch::run(
            cli.config.as_deref(),
            &path,
            agentfs,
            once,
            interval_ms,
            json,
        ),
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

fn json_arg_requested() -> bool {
    std::env::args_os().any(|arg| arg == "--json")
}

fn print_json_parse_error(err: &clap::Error) {
    println!(
        "{}",
        serde_json::json!({
            "error": {
                "code": "invalid_arguments",
                "message": err.to_string(),
                "retryable": false,
                "details": {
                    "kind": format!("{:?}", err.kind()),
                },
            }
        })
    );
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
