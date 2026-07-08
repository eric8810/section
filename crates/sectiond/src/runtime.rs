use crate::contract::SectiondContract;
use anyhow::Result;
use section_core::{Router, SectionConfig};
use section_provider::ProviderStore;
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceOrigin {
    ConfigFile,
    ProviderStore,
    ConfigPreferredWithStoreFallback,
}

impl SourceOrigin {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ConfigFile => "config_file",
            Self::ProviderStore => "provider_store",
            Self::ConfigPreferredWithStoreFallback => "config_preferred_with_store_fallback",
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SourceSnapshot {
    pub name: String,
    pub provider: String,
    pub metadata_ttl_secs: u64,
    pub content_ttl_secs: u64,
    pub origin: SourceOrigin,
    pub local_root: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimeSnapshot {
    pub config_path: Option<PathBuf>,
    pub data_dir: PathBuf,
    pub mount_point: PathBuf,
    pub source_registry_mode: String,
    pub sources: Vec<SourceSnapshot>,
    pub contract: SectiondContract,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SourceStatusSnapshot {
    pub name: String,
    pub provider: String,
    pub origin: SourceOrigin,
    pub local_root: Option<PathBuf>,
    pub connected: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct StatusSnapshot {
    pub mount_path: PathBuf,
    pub mount_active: bool,
    pub sources: Vec<SourceStatusSnapshot>,
}

pub struct SectiondRuntime {
    config: SectionConfig,
    router: Router,
    sources: Vec<SourceSnapshot>,
    contract: SectiondContract,
}

impl SectiondRuntime {
    pub fn load(config_path: Option<&Path>) -> Result<Self> {
        let config = SectionConfig::load(config_path)?;
        config.ensure_dirs()?;
        let store = ProviderStore::open(&config.data_dir)?;
        Self::from_config_and_store(&config, &store)
    }

    pub fn from_config_and_store(config: &SectionConfig, store: &ProviderStore) -> Result<Self> {
        Self::from_config_and_store_with_agentfs(config, store, false)
    }

    pub fn from_config_and_store_including_agentfs(
        config: &SectionConfig,
        store: &ProviderStore,
    ) -> Result<Self> {
        Self::from_config_and_store_with_agentfs(config, store, true)
    }

    fn from_config_and_store_with_agentfs(
        config: &SectionConfig,
        store: &ProviderStore,
        include_agentfs: bool,
    ) -> Result<Self> {
        let mut file_sources = config.sources.clone();
        let mut store_sources = store.load_all()?;
        for name in config.sources.keys() {
            if store.is_agentfs_source(name)? {
                file_sources.remove(name);
            }
        }
        if !include_agentfs {
            let store_source_names = store_sources.keys().cloned().collect::<Vec<_>>();
            for name in store_source_names {
                if store.is_agentfs_source(&name)? {
                    store_sources.remove(&name);
                }
            }
        }
        let binding_map = store
            .list_source_local_roots()?
            .into_iter()
            .map(|binding| (binding.source_name, binding.local_root))
            .collect::<std::collections::HashMap<_, _>>();

        let mut merged = config.clone();
        merged.sources = file_sources.clone();
        for (name, source) in &store_sources {
            merged.sources.entry(name.clone()).or_insert(source.clone());
        }

        let mut source_names = BTreeSet::new();
        source_names.extend(file_sources.keys().cloned());
        source_names.extend(store_sources.keys().cloned());

        let mut sources = Vec::with_capacity(source_names.len());
        for name in source_names {
            let source = merged
                .sources
                .get(&name)
                .expect("merged source should exist for every discovered name");
            let local_root = binding_map.get(&name).cloned();
            let origin = match (
                file_sources.contains_key(&name),
                store_sources.contains_key(&name),
            ) {
                (true, true) => SourceOrigin::ConfigPreferredWithStoreFallback,
                (true, false) => SourceOrigin::ConfigFile,
                (false, true) => SourceOrigin::ProviderStore,
                (false, false) => {
                    unreachable!("source must come from file config or provider store")
                }
            };
            sources.push(SourceSnapshot {
                name,
                provider: source.provider.clone(),
                metadata_ttl_secs: source.cache.metadata_ttl_secs,
                content_ttl_secs: source.cache.content_ttl_secs,
                origin,
                local_root,
            });
        }

        let router = Router::from_config(&merged)?;
        Ok(Self {
            config: merged,
            router,
            sources,
            contract: SectiondContract::default(),
        })
    }

    pub fn config(&self) -> &SectionConfig {
        &self.config
    }

    pub fn router(&self) -> &Router {
        &self.router
    }

    pub fn sources(&self) -> &[SourceSnapshot] {
        &self.sources
    }

    pub fn contract(&self) -> &SectiondContract {
        &self.contract
    }

    pub fn snapshot(&self, config_path: Option<&Path>) -> RuntimeSnapshot {
        RuntimeSnapshot {
            config_path: config_path.map(Path::to_path_buf),
            data_dir: self.config.data_dir.clone(),
            mount_point: self.config.mount_point.clone(),
            source_registry_mode: "transitional_config_plus_provider_store".to_string(),
            sources: self.sources.clone(),
            contract: self.contract.clone(),
        }
    }

    pub fn status_snapshot(&self, mount_path: &Path, mount_active: bool) -> Result<StatusSnapshot> {
        let rt = tokio::runtime::Runtime::new()?;
        let mut sources = Vec::with_capacity(self.sources.len());

        for source in &self.sources {
            let connected = match self.router.get_operator(&source.name) {
                Ok(op) => match rt.block_on(op.stat("/")) {
                    Ok(_) => true,
                    Err(_) => rt.block_on(op.list("/")).is_ok(),
                },
                Err(_) => false,
            };

            sources.push(SourceStatusSnapshot {
                name: source.name.clone(),
                provider: source.provider.clone(),
                origin: source.origin,
                local_root: source.local_root.clone(),
                connected,
            });
        }

        Ok(StatusSnapshot {
            mount_path: mount_path.to_path_buf(),
            mount_active,
            sources,
        })
    }

    pub fn into_parts(self) -> (SectionConfig, Router) {
        (self.config, self.router)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use section_core::config::{CacheConfig, SourceConfig};
    use section_provider::AcceptedFilesystemRecord;
    use std::collections::HashMap;
    use tempfile::TempDir;

    fn fs_source(root: &str, metadata_ttl_secs: u64, content_ttl_secs: u64) -> SourceConfig {
        let mut options = HashMap::new();
        options.insert("root".to_string(), root.to_string());
        SourceConfig {
            provider: "fs".to_string(),
            options,
            cache: CacheConfig {
                metadata_ttl_secs,
                content_ttl_secs,
            },
        }
    }

    #[test]
    fn runtime_snapshot_classifies_origins_and_sorts_sources() {
        let temp_dir = TempDir::new().expect("temp dir");
        let store = ProviderStore::open(temp_dir.path()).expect("store");
        store
            .add_source("db-only", &fs_source("/tmp/db-only", 30, 45))
            .expect("add db-only");
        store
            .add_source("shared", &fs_source("/tmp/from-db", 11, 22))
            .expect("add shared");

        let mut config = SectionConfig {
            data_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        };
        config.sources.insert(
            "config-only".to_string(),
            fs_source("/tmp/config-only", 60, 300),
        );
        config
            .sources
            .insert("shared".to_string(), fs_source("/tmp/from-config", 99, 100));

        let runtime = SectiondRuntime::from_config_and_store(&config, &store).expect("runtime");
        let snapshot = runtime.snapshot(None);

        assert_eq!(
            snapshot
                .sources
                .iter()
                .map(|source| source.name.as_str())
                .collect::<Vec<_>>(),
            vec!["config-only", "db-only", "shared"]
        );

        let config_only = snapshot
            .sources
            .iter()
            .find(|source| source.name == "config-only")
            .expect("config-only");
        assert_eq!(config_only.origin, SourceOrigin::ConfigFile);

        let db_only = snapshot
            .sources
            .iter()
            .find(|source| source.name == "db-only")
            .expect("db-only");
        assert_eq!(db_only.origin, SourceOrigin::ProviderStore);

        let shared = snapshot
            .sources
            .iter()
            .find(|source| source.name == "shared")
            .expect("shared");
        assert_eq!(
            shared.origin,
            SourceOrigin::ConfigPreferredWithStoreFallback
        );
        assert_eq!(shared.metadata_ttl_secs, 99);
        assert_eq!(shared.content_ttl_secs, 100);
    }

    #[test]
    fn runtime_router_contains_merged_sources() {
        let temp_dir = TempDir::new().expect("temp dir");
        let store = ProviderStore::open(temp_dir.path()).expect("store");
        store
            .add_source("db-only", &fs_source("/tmp/db-only", 30, 45))
            .expect("add db-only");

        let mut config = SectionConfig {
            data_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        };
        config.sources.insert(
            "config-only".to_string(),
            fs_source("/tmp/config-only", 60, 300),
        );

        let runtime = SectiondRuntime::from_config_and_store(&config, &store).expect("runtime");
        assert_eq!(
            runtime.router().sources(),
            vec!["config-only".to_string(), "db-only".to_string()]
        );
    }

    #[test]
    fn public_runtime_filters_agentfs_sources_and_internal_runtime_uses_store_source() {
        let temp_dir = TempDir::new().expect("temp dir");
        let public_root = TempDir::new().expect("public root");
        let service_root = TempDir::new().expect("service root");
        let collision_root = TempDir::new().expect("collision root");
        let store = ProviderStore::open(temp_dir.path()).expect("store");
        store
            .add_source(
                "fs_service",
                &fs_source(service_root.path().to_str().expect("utf8 service"), 30, 45),
            )
            .expect("add agentfs source");
        store
            .cache_accepted_filesystem(&AcceptedFilesystemRecord {
                fs_id: "fs_service".to_string(),
                name: "project".to_string(),
                owner_agent_id: "agt_owner".to_string(),
                source_profile_id: "srcp_profile".to_string(),
                source_name: "fs_service".to_string(),
                accepted_at_ms: 1,
            })
            .expect("mark accepted agentfs source");

        let mut config = SectionConfig {
            data_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        };
        config.sources.insert(
            "public".to_string(),
            fs_source(public_root.path().to_str().expect("utf8 public"), 60, 300),
        );
        config.sources.insert(
            "fs_service".to_string(),
            fs_source(
                collision_root.path().to_str().expect("utf8 collision"),
                99,
                100,
            ),
        );

        let public_runtime =
            SectiondRuntime::from_config_and_store(&config, &store).expect("public runtime");
        assert_eq!(
            public_runtime.router().sources(),
            vec!["public".to_string()]
        );
        assert!(public_runtime.router().get_operator("fs_service").is_err());
        assert_eq!(
            public_runtime
                .snapshot(None)
                .sources
                .iter()
                .map(|source| source.name.as_str())
                .collect::<Vec<_>>(),
            vec!["public"]
        );

        let internal_runtime =
            SectiondRuntime::from_config_and_store_including_agentfs(&config, &store)
                .expect("internal runtime");
        assert_eq!(
            internal_runtime.router().sources(),
            vec!["fs_service".to_string(), "public".to_string()]
        );
        let agentfs_source = internal_runtime
            .snapshot(None)
            .sources
            .into_iter()
            .find(|source| source.name == "fs_service")
            .expect("agentfs source");
        assert_eq!(agentfs_source.origin, SourceOrigin::ProviderStore);
        assert_eq!(agentfs_source.metadata_ttl_secs, 30);
        assert_eq!(agentfs_source.content_ttl_secs, 45);
    }

    #[test]
    fn status_snapshot_reports_mount_state_and_connectivity() {
        let temp_dir = TempDir::new().expect("temp dir");
        let source_root = TempDir::new().expect("source root");
        let store = ProviderStore::open(temp_dir.path()).expect("store");

        let mut config = SectionConfig {
            data_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        };
        config.sources.insert(
            "config-only".to_string(),
            fs_source(source_root.path().to_str().expect("utf8 path"), 60, 300),
        );

        let runtime = SectiondRuntime::from_config_and_store(&config, &store).expect("runtime");
        let snapshot = runtime
            .status_snapshot(Path::new("/mnt/section"), true)
            .expect("status snapshot");

        assert_eq!(snapshot.mount_path, PathBuf::from("/mnt/section"));
        assert!(snapshot.mount_active);
        assert_eq!(snapshot.sources.len(), 1);
        assert_eq!(snapshot.sources[0].name, "config-only");
        assert!(snapshot.sources[0].connected);
    }
}
