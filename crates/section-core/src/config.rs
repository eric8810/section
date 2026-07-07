use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Top-level Section configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectionConfig {
    /// Mount point for the FUSE filesystem.
    #[serde(default = "default_mount_point")]
    pub mount_point: PathBuf,

    /// Data directory for metadata, cache, credentials.
    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,

    /// Registered sources (name -> config).
    #[serde(default)]
    pub sources: HashMap<String, SourceConfig>,

    /// Section Control Service configuration for AgentFS governance.
    #[serde(default)]
    pub control_service: ControlServiceConfig,
}

/// A source is an instance of a provider with bound credentials.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceConfig {
    /// Provider type (e.g., "s3", "fs", "webdav", "gdrive", "samba").
    pub provider: String,

    /// Provider-specific options (credentials + connection params).
    /// Passed directly to the OpenDAL operator builder.
    #[serde(default)]
    pub options: HashMap<String, String>,

    /// Cache settings for this source.
    #[serde(default)]
    pub cache: CacheConfig,
}

/// File-backed Section Control Service configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ControlServiceConfig {
    /// SQLite database path for the file-backed control service harness.
    pub path: Option<PathBuf>,

    /// Server-managed backing source profiles available to AgentFS.
    #[serde(default)]
    pub source_profiles: HashMap<String, SourceConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    /// Metadata (directory listing) TTL in seconds. 0 = no cache.
    #[serde(default = "default_metadata_ttl")]
    pub metadata_ttl_secs: u64,

    /// Content TTL in seconds. 0 = no cache.
    #[serde(default = "default_content_ttl")]
    pub content_ttl_secs: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            metadata_ttl_secs: default_metadata_ttl(),
            content_ttl_secs: default_content_ttl(),
        }
    }
}

fn default_mount_point() -> PathBuf {
    PathBuf::from("/mnt/section")
}

fn default_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("/var/lib/section"))
        .join("section")
}

fn default_metadata_ttl() -> u64 {
    60
}

fn default_content_ttl() -> u64 {
    300
}

impl SectionConfig {
    /// Load config from the default or specified path.
    pub fn load(path: Option<&Path>) -> crate::Result<Self> {
        let config_path = match path {
            Some(p) => p.to_path_buf(),
            None => Self::default_config_path(),
        };

        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            let config: Self = toml::from_str(&content)
                .map_err(|e| anyhow::anyhow!("failed to parse config: {e}"))?;
            Ok(config)
        } else {
            Ok(Self::default())
        }
    }

    /// Default config file location.
    pub fn default_config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("/etc"))
            .join("section")
            .join("config.toml")
    }

    /// Ensure data directory exists.
    pub fn ensure_dirs(&self) -> crate::Result<()> {
        std::fs::create_dir_all(&self.data_dir)?;
        Ok(())
    }
}

impl Default for SectionConfig {
    fn default() -> Self {
        Self {
            mount_point: default_mount_point(),
            data_dir: default_data_dir(),
            sources: HashMap::new(),
            control_service: ControlServiceConfig::default(),
        }
    }
}
