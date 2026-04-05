use crate::{SectiondRuntime, SourceOrigin, StatusSnapshot};
use anyhow::{bail, Result};
use section_core::config::{CacheConfig, SourceConfig};
use section_core::SectionConfig;
use section_provider::ProviderStore;
use serde::Serialize;
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
    pub options: BTreeMap<String, String>,
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
        self.store.remove_source(name)?;
        Ok(())
    }

    pub fn list_sources(&self) -> Result<Vec<SourceRegistryEntry>> {
        let store_sources = self.store.load_all()?;
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

    fn ensure_name_is_not_config_owned(&self, name: &str, action: &str) -> Result<()> {
        if self.config.sources.contains_key(name) {
            bail!(
                "Source '{name}' is defined in the config file and cannot be changed via the control plane. Edit the config file to {action} it."
            );
        }

        Ok(())
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
