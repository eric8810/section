use crate::sync::{
    compare_path as sync_compare_path, list_watch_events, resolve_path as sync_resolve_path,
    sync_source as run_source_sync, PathCompareSnapshot, PathResolveResult, PathResolveStrategy,
    SourceSyncResult,
};
use crate::{SectiondRuntime, SourceOrigin, StatusSnapshot};
use anyhow::{bail, Result};
use section_core::config::{CacheConfig, SourceConfig};
use section_core::SectionConfig;
use section_provider::{PathSyncStateRecord, ProviderStore, SyncEventRecord};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;

const REFRESH_XATTR_NAME: &str = "section.refresh";
const REFRESH_XATTR_NAME_LINUX: &str = "user.section.refresh";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SourceRegistryEntry {
    pub name: String,
    pub provider: String,
    pub origin: SourceOrigin,
    pub metadata_ttl_secs: u64,
    pub content_ttl_secs: u64,
    pub local_root: Option<PathBuf>,
    pub options: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RootDiscoveryMarker {
    pub source_id: String,
    pub local_root: PathBuf,
    pub control_plane_endpoint: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PathDetailSnapshot {
    pub local_present: bool,
    pub dirty_local: bool,
    pub dirty_remote: bool,
    pub pinned: bool,
    pub stale: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PathInspectSnapshot {
    pub source_id: String,
    pub local_root: PathBuf,
    pub local_path: PathBuf,
    pub source_path: String,
    pub state: String,
    pub detail: PathDetailSnapshot,
    pub base_remote_version: Option<String>,
    pub current_remote_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RefreshResult {
    pub mount_active: bool,
    pub message: String,
}

pub struct SectiondControlPlane {
    config: SectionConfig,
    store: ProviderStore,
}

impl SectiondControlPlane {
    pub fn load(config_path: Option<&Path>) -> Result<Self> {
        let config = SectionConfig::load(config_path)?;
        config.ensure_dirs()?;
        let store = ProviderStore::open(&config.data_dir)?;
        Ok(Self { config, store })
    }

    pub fn source_add(
        &self,
        name: &str,
        provider: &str,
        options: HashMap<String, String>,
    ) -> Result<SourceRegistryEntry> {
        self.ensure_name_is_not_config_owned(name, "change")?;

        let source = SourceConfig {
            provider: provider.to_string(),
            options,
            cache: CacheConfig::default(),
        };
        self.store.add_source(name, &source)?;
        self.find_source(name)?
            .ok_or_else(|| anyhow::anyhow!("source {name} was added but could not be reloaded"))
    }

    pub fn source_remove(&self, name: &str) -> Result<()> {
        self.ensure_name_is_not_config_owned(name, "remove")?;
        let previous_root = self.store.get_source_local_root(name)?;
        self.store.remove_source(name)?;
        if let Some(previous_root) = previous_root {
            remove_root_marker(&previous_root)?;
        }
        Ok(())
    }

    pub fn list_sources(&self) -> Result<Vec<SourceRegistryEntry>> {
        let store_sources = self.store.load_all()?;
        let binding_map = self
            .store
            .list_source_local_roots()?
            .into_iter()
            .map(|binding| (binding.source_name, binding.local_root))
            .collect::<HashMap<_, _>>();
        let mut source_names = BTreeSet::new();
        source_names.extend(self.config.sources.keys().cloned());
        source_names.extend(store_sources.keys().cloned());

        let mut entries = Vec::with_capacity(source_names.len());
        for name in source_names {
            let origin = match (
                self.config.sources.contains_key(&name),
                store_sources.contains_key(&name),
            ) {
                (true, true) => SourceOrigin::ConfigPreferredWithStoreFallback,
                (true, false) => SourceOrigin::ConfigFile,
                (false, true) => SourceOrigin::ProviderStore,
                (false, false) => unreachable!("source should exist in config or store"),
            };

            let effective_source = match origin {
                SourceOrigin::ConfigFile | SourceOrigin::ConfigPreferredWithStoreFallback => self
                    .config
                    .sources
                    .get(&name)
                    .expect("config-owned source should exist in config"),
                SourceOrigin::ProviderStore => store_sources
                    .get(&name)
                    .expect("store-owned source should exist in provider store"),
            };

            entries.push(SourceRegistryEntry {
                local_root: binding_map.get(&name).cloned(),
                name,
                provider: effective_source.provider.clone(),
                origin,
                metadata_ttl_secs: effective_source.cache.metadata_ttl_secs,
                content_ttl_secs: effective_source.cache.content_ttl_secs,
                options: effective_source
                    .options
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
            });
        }

        Ok(entries)
    }

    pub fn source_bind_local_root(
        &self,
        name: &str,
        local_root: &Path,
    ) -> Result<SourceRegistryEntry> {
        self.ensure_source_exists(name)?;

        let local_root = absolutize_path(local_root)?;
        let previous_root = self.store.get_source_local_root(name)?;
        self.store.set_source_local_root(name, &local_root)?;
        write_root_marker(name, &local_root)?;

        if let Some(previous_root) = previous_root {
            if previous_root != local_root {
                remove_root_marker(&previous_root)?;
            }
        }

        self.find_source(name)?
            .ok_or_else(|| anyhow::anyhow!("source {name} was bound but could not be reloaded"))
    }

    pub fn source_unbind_local_root(&self, name: &str) -> Result<()> {
        self.ensure_source_exists(name)?;

        if let Some(previous_root) = self.store.get_source_local_root(name)? {
            self.store.remove_source_local_root(name)?;
            remove_root_marker(&previous_root)?;
        }

        Ok(())
    }

    pub fn path_inspect(&self, input_path: &Path) -> Result<PathInspectSnapshot> {
        let local_path = absolutize_path(input_path)?;
        let marker = discover_root_marker(&local_path)?;
        let authoritative_root = self
            .store
            .get_source_local_root(&marker.source_id)?
            .ok_or_else(|| {
                anyhow::anyhow!("source {} has no bound local root", marker.source_id)
            })?;

        let source_path = relative_source_path(&authoritative_root, &local_path)?;
        let state = self
            .store
            .get_path_sync_state(&marker.source_id, &source_path)?
            .unwrap_or_else(|| default_path_state(&marker.source_id, &source_path, &local_path));

        Ok(PathInspectSnapshot {
            source_id: marker.source_id,
            local_root: authoritative_root,
            local_path,
            source_path: display_source_path(&source_path),
            state: state.public_state,
            detail: PathDetailSnapshot {
                local_present: state.local_present,
                dirty_local: state.dirty_local,
                dirty_remote: state.dirty_remote,
                pinned: state.pinned,
                stale: state.stale,
            },
            base_remote_version: state.base_remote_version,
            current_remote_version: state.current_remote_version,
        })
    }

    pub fn source_sync(&self, name: &str) -> Result<SourceSyncResult> {
        self.ensure_source_exists(name)?;
        let local_root = self
            .store
            .get_source_local_root(name)?
            .ok_or_else(|| anyhow::anyhow!("source {name} has no bound local root"))?;
        let runtime = self.runtime()?;
        run_source_sync(&runtime, &self.store, name, &local_root)
    }

    pub fn path_compare(&self, input_path: &Path) -> Result<PathCompareSnapshot> {
        let (source_id, local_root, local_path, source_path) =
            self.resolve_local_path(input_path)?;
        let runtime = self.runtime()?;
        sync_compare_path(
            &runtime,
            &self.store,
            &source_id,
            &local_root,
            &local_path,
            &source_path,
        )
    }

    pub fn path_resolve(
        &self,
        input_path: &Path,
        strategy: PathResolveStrategy,
    ) -> Result<PathResolveResult> {
        let (source_id, local_root, local_path, source_path) =
            self.resolve_local_path(input_path)?;
        let runtime = self.runtime()?;
        sync_resolve_path(
            &runtime,
            &self.store,
            &source_id,
            &local_root,
            &local_path,
            &source_path,
            strategy,
        )
    }

    pub fn watch_path(&self, input_path: &Path, after_id: i64) -> Result<Vec<SyncEventRecord>> {
        let (source_id, _local_root, _local_path, source_path) =
            self.resolve_local_path(input_path)?;
        list_watch_events(&self.store, &source_id, &source_path, after_id)
    }

    pub fn status_snapshot(&self) -> Result<StatusSnapshot> {
        let mount_active = is_mount_active(&self.config.mount_point);
        self.runtime()?
            .status_snapshot(&self.config.mount_point, mount_active)
    }

    pub fn refresh_path(&self, path: &str) -> Result<RefreshResult> {
        let mount_active = is_mount_active(&self.config.mount_point);
        let message = if mount_active {
            let mount_target = refresh_mount_target(&self.config.mount_point, path);
            let data = trigger_refresh_xattr(&mount_target)?;
            let response = String::from_utf8_lossy(&data);
            if response.trim().is_empty() || response.trim() == "ok" {
                format!("Cache refreshed for {path}")
            } else {
                format!("Cache refreshed for {path}: {}", response.trim())
            }
        } else {
            format!(
                "No active mount at {}; CLI has no persistent cache to invalidate for {path}",
                self.config.mount_point.display()
            )
        };

        Ok(RefreshResult {
            mount_active,
            message,
        })
    }

    fn runtime(&self) -> Result<SectiondRuntime> {
        SectiondRuntime::from_config_and_store(&self.config, &self.store)
    }

    fn find_source(&self, name: &str) -> Result<Option<SourceRegistryEntry>> {
        Ok(self
            .list_sources()?
            .into_iter()
            .find(|entry| entry.name == name))
    }

    fn ensure_source_exists(&self, name: &str) -> Result<()> {
        if self.find_source(name)?.is_none() {
            bail!("source {name} is not registered");
        }
        Ok(())
    }

    fn resolve_local_path(&self, input_path: &Path) -> Result<(String, PathBuf, PathBuf, String)> {
        let local_path = absolutize_path(input_path)?;
        let marker = discover_root_marker(&local_path)?;
        let local_root = self
            .store
            .get_source_local_root(&marker.source_id)?
            .ok_or_else(|| {
                anyhow::anyhow!("source {} has no bound local root", marker.source_id)
            })?;
        let source_path = relative_source_path(&local_root, &local_path)?;
        Ok((marker.source_id, local_root, local_path, source_path))
    }

    fn ensure_name_is_not_config_owned(&self, name: &str, action: &str) -> Result<()> {
        if self.config.sources.contains_key(name) {
            bail!(
                "Source '{name}' is defined in the config file and cannot be changed via the control plane. Edit the config file to {action} it."
            );
        }

        Ok(())
    }
}

fn absolutize_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn write_root_marker(source_id: &str, local_root: &Path) -> Result<()> {
    let marker = RootDiscoveryMarker {
        source_id: source_id.to_string(),
        local_root: local_root.to_path_buf(),
        control_plane_endpoint: "sectiond://local".to_string(),
    };
    let marker_dir = local_root.join(".section");
    std::fs::create_dir_all(&marker_dir)?;
    std::fs::write(
        marker_dir.join("root.json"),
        serde_json::to_vec_pretty(&marker)?,
    )?;
    Ok(())
}

fn remove_root_marker(local_root: &Path) -> Result<()> {
    let marker_dir = local_root.join(".section");
    let marker_path = marker_dir.join("root.json");
    if marker_path.exists() {
        std::fs::remove_file(&marker_path)?;
    }
    if marker_dir.exists() {
        let mut entries = std::fs::read_dir(&marker_dir)?;
        if entries.next().is_none() {
            std::fs::remove_dir(&marker_dir)?;
        }
    }
    Ok(())
}

fn discover_root_marker(path: &Path) -> Result<RootDiscoveryMarker> {
    let mut current = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| path.to_path_buf())
    };

    loop {
        let marker_path = current.join(".section").join("root.json");
        if marker_path.exists() {
            let marker = std::fs::read(&marker_path)?;
            return Ok(serde_json::from_slice(&marker)?);
        }

        if !current.pop() {
            bail!(
                "no Section local-root marker found above {}",
                path.display()
            );
        }
    }
}

fn relative_source_path(local_root: &Path, local_path: &Path) -> Result<String> {
    let relative = local_path.strip_prefix(local_root).map_err(|_| {
        anyhow::anyhow!(
            "{} is outside the bound local root {}",
            local_path.display(),
            local_root.display()
        )
    })?;

    let raw = relative
        .iter()
        .map(|segment| segment.to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");

    Ok(raw)
}

fn display_source_path(path: &str) -> String {
    if path.is_empty() {
        "/".to_string()
    } else {
        path.to_string()
    }
}

fn default_path_state(source_id: &str, path: &str, local_path: &Path) -> PathSyncStateRecord {
    PathSyncStateRecord {
        source_name: source_id.to_string(),
        path: path.to_string(),
        entry_kind: if local_path.is_dir() {
            "dir".to_string()
        } else {
            "file".to_string()
        },
        public_state: "ready".to_string(),
        local_present: local_path.exists(),
        dirty_local: false,
        dirty_remote: false,
        pinned: false,
        stale: false,
        last_local_version: None,
        base_remote_version: None,
        current_remote_version: None,
    }
}

fn refresh_attr_names() -> &'static [&'static str] {
    #[cfg(target_os = "linux")]
    {
        &[REFRESH_XATTR_NAME_LINUX, REFRESH_XATTR_NAME]
    }

    #[cfg(not(target_os = "linux"))]
    {
        &[REFRESH_XATTR_NAME, REFRESH_XATTR_NAME_LINUX]
    }
}

fn refresh_mount_target(mount_point: &Path, path: &str) -> PathBuf {
    let mut target = mount_point.to_path_buf();
    for segment in path.split('/') {
        if !segment.is_empty() {
            target.push(segment);
        }
    }
    target
}

fn trigger_refresh_xattr(path: &Path) -> Result<Vec<u8>> {
    let mut last_error = None;

    for attr_name in refresh_attr_names() {
        match xattr::get(path, attr_name) {
            Ok(Some(data)) => return Ok(data),
            Ok(None) => {
                last_error = Some(anyhow::anyhow!(
                    "refresh xattr {attr_name} returned no data for {}",
                    path.display()
                ));
            }
            Err(err) => {
                last_error = Some(anyhow::anyhow!(
                    "failed to read refresh xattr {attr_name} on {}: {err}",
                    path.display()
                ));
            }
        }
    }

    Err(last_error
        .unwrap_or_else(|| anyhow::anyhow!("refresh xattr not available for {}", path.display())))
}

fn is_mount_active(mount_point: &Path) -> bool {
    check_proc_mounts(mount_point) || check_mount_command(mount_point)
}

fn check_proc_mounts(mount_point: &Path) -> bool {
    let mount_str = mount_point.to_string_lossy();

    if let Ok(mounts) = std::fs::read_to_string("/proc/mounts") {
        for line in mounts.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 && parts[1] == mount_str.as_ref() {
                return true;
            }
        }
    }

    false
}

fn check_mount_command(mount_point: &Path) -> bool {
    let output = match Command::new("mount").output() {
        Ok(output) if output.status.success() => output,
        _ => return false,
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    mount_output_contains_target(&stdout, mount_point)
}

fn mount_output_contains_target(output: &str, mount_point: &Path) -> bool {
    let mount_str = mount_point.to_string_lossy();

    output.lines().any(|line| {
        let Some((_, rest)) = line.split_once(" on ") else {
            return false;
        };

        rest == mount_str.as_ref()
            || rest.starts_with(&format!("{mount_str} "))
            || rest.starts_with(&format!("{mount_str} ("))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_config(data_dir: &Path, path: &Path, source_name: &str) {
        let config = format!(
            "data_dir = {:?}\nmount_point = \"/tmp/section-mount\"\n\n[sources.{source_name}]\nprovider = \"fs\"\n\n[sources.{source_name}.options]\nroot = \"/tmp/from-config\"\n",
            data_dir.to_string_lossy()
        );
        fs::write(path, config).expect("write config");
    }

    #[test]
    fn list_sources_returns_merged_effective_registry() {
        let temp_dir = TempDir::new().expect("temp dir");
        let config_path = temp_dir.path().join("section.toml");
        write_config(temp_dir.path(), &config_path, "config-only");

        let control = SectiondControlPlane::load(Some(&config_path)).expect("control plane");
        control
            .source_add(
                "store-only",
                "fs",
                HashMap::from([("root".to_string(), "/tmp/from-store".to_string())]),
            )
            .expect("add source");

        let sources = control.list_sources().expect("list sources");
        assert_eq!(sources.len(), 2);

        let config_source = sources
            .iter()
            .find(|entry| entry.name == "config-only")
            .expect("config source");
        assert_eq!(config_source.origin, SourceOrigin::ConfigFile);
        assert_eq!(
            config_source.options.get("root").map(String::as_str),
            Some("/tmp/from-config")
        );

        let store_source = sources
            .iter()
            .find(|entry| entry.name == "store-only")
            .expect("store source");
        assert_eq!(store_source.origin, SourceOrigin::ProviderStore);
        assert_eq!(
            store_source.options.get("root").map(String::as_str),
            Some("/tmp/from-store")
        );
    }

    #[test]
    fn source_bind_local_root_writes_marker_and_updates_registry_entry() {
        let temp_dir = TempDir::new().expect("temp dir");
        let config_path = temp_dir.path().join("section.toml");
        write_config(temp_dir.path(), &config_path, "config-only");

        let control = SectiondControlPlane::load(Some(&config_path)).expect("control plane");
        control
            .source_add(
                "store-only",
                "fs",
                HashMap::from([("root".to_string(), "/tmp/from-store".to_string())]),
            )
            .expect("add source");

        let local_root = temp_dir.path().join("bound-root");
        let entry = control
            .source_bind_local_root("store-only", &local_root)
            .expect("bind local root");

        assert_eq!(entry.local_root, Some(local_root.clone()));

        let marker =
            std::fs::read(local_root.join(".section").join("root.json")).expect("read root marker");
        let marker: RootDiscoveryMarker =
            serde_json::from_slice(&marker).expect("parse root marker");
        assert_eq!(marker.source_id, "store-only");
        assert_eq!(marker.local_root, local_root);
        assert_eq!(marker.control_plane_endpoint, "sectiond://local");
    }

    #[test]
    fn path_inspect_resolves_local_path_and_detail_state() {
        let temp_dir = TempDir::new().expect("temp dir");
        let config_path = temp_dir.path().join("section.toml");
        write_config(temp_dir.path(), &config_path, "config-only");

        let control = SectiondControlPlane::load(Some(&config_path)).expect("control plane");
        control
            .source_add(
                "store-only",
                "fs",
                HashMap::from([("root".to_string(), "/tmp/from-store".to_string())]),
            )
            .expect("add source");

        let local_root = temp_dir.path().join("bound-root");
        control
            .source_bind_local_root("store-only", &local_root)
            .expect("bind local root");

        let nested = local_root.join("notes").join("todo.txt");
        std::fs::create_dir_all(nested.parent().expect("parent")).expect("create parent");
        std::fs::write(&nested, "hello").expect("write local file");
        control
            .store
            .upsert_path_sync_state(&PathSyncStateRecord {
                source_name: "store-only".to_string(),
                path: "notes/todo.txt".to_string(),
                entry_kind: "file".to_string(),
                public_state: "conflict".to_string(),
                local_present: true,
                dirty_local: true,
                dirty_remote: true,
                pinned: false,
                stale: true,
                last_local_version: Some("l1".to_string()),
                base_remote_version: Some("v1".to_string()),
                current_remote_version: Some("v2".to_string()),
            })
            .expect("seed path state");

        let inspect = control.path_inspect(&nested).expect("path inspect");
        assert_eq!(inspect.source_id, "store-only");
        assert_eq!(inspect.local_root, local_root);
        assert_eq!(inspect.local_path, nested);
        assert_eq!(inspect.source_path, "notes/todo.txt");
        assert_eq!(inspect.state, "conflict");
        assert!(inspect.detail.local_present);
        assert!(inspect.detail.dirty_local);
        assert!(inspect.detail.dirty_remote);
        assert!(inspect.detail.stale);
        assert_eq!(inspect.base_remote_version.as_deref(), Some("v1"));
        assert_eq!(inspect.current_remote_version.as_deref(), Some("v2"));
    }

    #[test]
    fn source_remove_cleans_up_root_marker() {
        let temp_dir = TempDir::new().expect("temp dir");
        let config_path = temp_dir.path().join("section.toml");
        write_config(temp_dir.path(), &config_path, "config-only");

        let control = SectiondControlPlane::load(Some(&config_path)).expect("control plane");
        control
            .source_add(
                "store-only",
                "fs",
                HashMap::from([("root".to_string(), "/tmp/from-store".to_string())]),
            )
            .expect("add source");

        let local_root = temp_dir.path().join("bound-root");
        control
            .source_bind_local_root("store-only", &local_root)
            .expect("bind local root");

        control
            .source_remove("store-only")
            .expect("remove store-owned source");

        assert!(
            !local_root.join(".section").join("root.json").exists(),
            "root marker should be removed"
        );
    }

    #[test]
    fn source_add_rejects_config_owned_name() {
        let temp_dir = TempDir::new().expect("temp dir");
        let config_path = temp_dir.path().join("section.toml");
        write_config(temp_dir.path(), &config_path, "shared");

        let control = SectiondControlPlane::load(Some(&config_path)).expect("control plane");
        let err = control
            .source_add(
                "shared",
                "fs",
                HashMap::from([("root".to_string(), "/tmp/override".to_string())]),
            )
            .expect_err("config-owned source should fail");

        assert!(err
            .to_string()
            .contains("defined in the config file and cannot be changed via the control plane"));
    }

    #[test]
    fn source_remove_rejects_config_owned_name() {
        let temp_dir = TempDir::new().expect("temp dir");
        let config_path = temp_dir.path().join("section.toml");
        write_config(temp_dir.path(), &config_path, "shared");

        let control = SectiondControlPlane::load(Some(&config_path)).expect("control plane");
        let err = control
            .source_remove("shared")
            .expect_err("config-owned source should fail");

        assert!(err
            .to_string()
            .contains("defined in the config file and cannot be changed via the control plane"));
    }
}
