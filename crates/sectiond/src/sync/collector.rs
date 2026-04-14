use super::{
    hash_bytes, local_mtime_ms, metadata_entry_kind, normalize_source_path, remote_file_token,
    remote_mtime_ms, EntryKind,
};
use anyhow::Result;
use opendal::{Entry, Metadata, Operator};
use section_provider::{LocalScanCacheRecord, PathSyncStateRecord, RemoteManifestRecord};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::Path;
use tracing::warn;

#[derive(Debug, Clone)]
pub(crate) struct ObservedEntry {
    pub(crate) kind: EntryKind,
    pub(crate) version: Option<String>,
    // TODO(#002): use observed size in planner heuristics and transfer scheduling.
    #[allow(dead_code)]
    pub(crate) size: Option<u64>,
    pub(crate) mtime_ms: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct LocalScanStats {
    pub files: usize,
    pub cache_hits: usize,
    pub cache_misses: usize,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct RemoteScanStats {
    pub files: usize,
    pub metadata_hits: usize,
    pub stat_fallbacks: usize,
    pub body_fallbacks: usize,
    pub accelerator: Option<String>,
    pub accelerated_entries: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct LocalSnapshot {
    pub(crate) entries: HashMap<String, ObservedEntry>,
    pub(crate) cache_records: Vec<LocalScanCacheRecord>,
    pub(crate) stats: LocalScanStats,
}

#[derive(Debug, Clone)]
pub(crate) struct RemoteSnapshot {
    pub(crate) entries: HashMap<String, ObservedEntry>,
    pub(crate) manifest_records: Vec<RemoteManifestRecord>,
    pub(crate) stats: RemoteScanStats,
}

#[derive(Debug, Clone)]
pub(crate) struct PathSyncInput {
    pub(crate) path: String,
    pub(crate) previous: Option<PathSyncStateRecord>,
    pub(crate) local: Option<ObservedEntry>,
    pub(crate) remote: Option<ObservedEntry>,
}

pub(crate) trait SnapshotCollector {
    fn collect_local(
        &self,
        root: &Path,
        cache: &HashMap<String, LocalScanCacheRecord>,
    ) -> Result<LocalSnapshot>;

    fn collect_remote(
        &self,
        op: &Operator,
        manifest: &HashMap<String, RemoteManifestRecord>,
    ) -> Result<RemoteSnapshot>;

    fn build_inputs(
        &self,
        previous: HashMap<String, PathSyncStateRecord>,
        local: HashMap<String, ObservedEntry>,
        remote: HashMap<String, ObservedEntry>,
    ) -> Vec<PathSyncInput>;
}

pub(crate) struct DefaultSnapshotCollector<'a> {
    rt: &'a tokio::runtime::Runtime,
    inventory_manifest_path: Option<String>,
}

impl<'a> DefaultSnapshotCollector<'a> {
    pub(crate) fn new(
        rt: &'a tokio::runtime::Runtime,
        inventory_manifest_path: Option<String>,
    ) -> Self {
        Self {
            rt,
            inventory_manifest_path,
        }
    }
}

#[derive(Debug, Deserialize)]
struct InventoryManifestEntry {
    path: String,
    #[serde(default = "default_inventory_entry_kind", alias = "entry_kind")]
    kind: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    etag: Option<String>,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    mtime_ms: Option<i64>,
}

impl SnapshotCollector for DefaultSnapshotCollector<'_> {
    fn collect_local(
        &self,
        root: &Path,
        cache: &HashMap<String, LocalScanCacheRecord>,
    ) -> Result<LocalSnapshot> {
        let mut entries = HashMap::new();
        let mut cache_records = Vec::new();
        let mut stats = LocalScanStats::default();

        fn walk(
            current: &Path,
            root: &Path,
            cache: &HashMap<String, LocalScanCacheRecord>,
            entries: &mut HashMap<String, ObservedEntry>,
            cache_records: &mut Vec<LocalScanCacheRecord>,
            stats: &mut LocalScanStats,
        ) -> Result<()> {
            let mut dir_entries =
                fs::read_dir(current)?.collect::<std::result::Result<Vec<_>, _>>()?;
            dir_entries.sort_by_key(|entry| entry.file_name());

            for entry in dir_entries {
                let path = entry.path();
                if current == root && entry.file_name() == ".section" {
                    continue;
                }

                let relative = path.strip_prefix(root)?;
                let source_path = relative
                    .iter()
                    .map(|segment| segment.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");

                let cached = cache.get(&source_path);
                let (observed, record) =
                    observe_local_entry_with_cache(&path, &source_path, cached, stats)?;
                let descend = observed.kind == EntryKind::Dir;
                entries.insert(source_path, observed);
                cache_records.push(record);

                if descend {
                    walk(&path, root, cache, entries, cache_records, stats)?;
                }
            }

            Ok(())
        }

        if root.exists() {
            walk(
                root,
                root,
                cache,
                &mut entries,
                &mut cache_records,
                &mut stats,
            )?;
        }

        Ok(LocalSnapshot {
            entries,
            cache_records,
            stats,
        })
    }

    fn collect_remote(
        &self,
        op: &Operator,
        manifest: &HashMap<String, RemoteManifestRecord>,
    ) -> Result<RemoteSnapshot> {
        if let Some(inventory_manifest_path) = &self.inventory_manifest_path {
            match self.collect_remote_from_inventory_manifest(op, manifest, inventory_manifest_path)
            {
                Ok(snapshot) => return Ok(snapshot),
                Err(err) => {
                    warn!(
                        manifest_path = %inventory_manifest_path,
                        error = %err,
                        "remote inventory accelerator failed; falling back to recursive list"
                    );
                }
            }
        }

        let mut listed_entries: Vec<Entry> = self
            .rt
            .block_on(async { op.list_with("").recursive(true).await })?;
        listed_entries.sort_by_key(|entry| entry.path().to_string());

        let mut entries = HashMap::new();
        let mut manifest_records = Vec::new();
        let mut stats = RemoteScanStats::default();

        for entry in listed_entries {
            let path = normalize_source_path(entry.path());
            if path.is_empty() {
                continue;
            }

            let cached = manifest.get(&path);
            let (observed, record) = observe_remote_entry_with_manifest(
                self.rt,
                op,
                &path,
                entry.metadata(),
                cached,
                &mut stats,
            )?;
            entries.insert(path, observed);
            manifest_records.push(record);
        }

        Ok(RemoteSnapshot {
            entries,
            manifest_records,
            stats,
        })
    }

    fn build_inputs(
        &self,
        previous: HashMap<String, PathSyncStateRecord>,
        local: HashMap<String, ObservedEntry>,
        remote: HashMap<String, ObservedEntry>,
    ) -> Vec<PathSyncInput> {
        let mut paths = BTreeSet::new();
        paths.extend(previous.keys().cloned());
        paths.extend(local.keys().cloned());
        paths.extend(remote.keys().cloned());

        paths
            .into_iter()
            .map(|path| PathSyncInput {
                previous: previous.get(&path).cloned(),
                local: local.get(&path).cloned(),
                remote: remote.get(&path).cloned(),
                path,
            })
            .collect()
    }
}

impl DefaultSnapshotCollector<'_> {
    fn collect_remote_from_inventory_manifest(
        &self,
        op: &Operator,
        manifest: &HashMap<String, RemoteManifestRecord>,
        inventory_manifest_path: &str,
    ) -> Result<RemoteSnapshot> {
        let data = self.rt.block_on(op.read(inventory_manifest_path))?;
        let mut inventory_entries = parse_inventory_manifest(data.to_bytes().as_ref())?;
        inventory_entries.sort_by(|left, right| left.path.cmp(&right.path));
        let manifest_path = normalize_source_path(inventory_manifest_path);

        let mut entries = HashMap::new();
        let mut manifest_records = Vec::new();
        let mut stats = RemoteScanStats {
            accelerator: Some("inventory_manifest".to_string()),
            ..RemoteScanStats::default()
        };

        for inventory_entry in inventory_entries {
            let path = normalize_source_path(&inventory_entry.path);
            if path.is_empty() || path == manifest_path {
                continue;
            }

            let cached = manifest.get(&path);
            let (observed, record) = observe_remote_inventory_entry(
                self.rt,
                op,
                &path,
                inventory_entry,
                cached,
                &mut stats,
            )?;
            entries.insert(path, observed);
            manifest_records.push(record);
            stats.accelerated_entries += 1;
        }

        Ok(RemoteSnapshot {
            entries,
            manifest_records,
            stats,
        })
    }
}

fn observe_local_entry_with_cache(
    local_path: &Path,
    source_path: &str,
    cached: Option<&LocalScanCacheRecord>,
    stats: &mut LocalScanStats,
) -> Result<(ObservedEntry, LocalScanCacheRecord)> {
    let metadata = fs::metadata(local_path)?;
    if metadata.is_dir() {
        let observed = ObservedEntry {
            kind: EntryKind::Dir,
            version: None,
            size: None,
            mtime_ms: local_mtime_ms(&metadata),
        };
        return Ok((
            observed.clone(),
            LocalScanCacheRecord {
                path: source_path.to_string(),
                entry_kind: observed.kind.as_str().to_string(),
                version: None,
                size: None,
                mtime_ms: observed.mtime_ms,
            },
        ));
    }

    if !metadata.is_file() {
        anyhow::bail!("unsupported local entry type for {}", local_path.display());
    }

    let size = Some(metadata.len());
    let mtime_ms = local_mtime_ms(&metadata);
    stats.files += 1;

    let version = match cached {
        Some(record)
            if record.entry_kind == "file"
                && record.version.is_some()
                && mtime_ms.is_some()
                && record.size == size
                && record.mtime_ms == mtime_ms =>
        {
            stats.cache_hits += 1;
            record.version.clone()
        }
        _ => {
            stats.cache_misses += 1;
            Some(hash_bytes(&fs::read(local_path)?))
        }
    };

    let observed = ObservedEntry {
        kind: EntryKind::File,
        version: version.clone(),
        size,
        mtime_ms,
    };
    Ok((
        observed,
        LocalScanCacheRecord {
            path: source_path.to_string(),
            entry_kind: "file".to_string(),
            version,
            size,
            mtime_ms,
        },
    ))
}

fn observe_remote_entry_with_manifest(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
    source_path: &str,
    listed_meta: &Metadata,
    cached: Option<&RemoteManifestRecord>,
    stats: &mut RemoteScanStats,
) -> Result<(ObservedEntry, RemoteManifestRecord)> {
    let kind = metadata_entry_kind(listed_meta)?;
    let listed_size = remote_metadata_size(listed_meta);
    let listed_mtime_ms = remote_mtime_ms(listed_meta);

    if kind == EntryKind::Dir {
        let observed = ObservedEntry {
            kind,
            version: None,
            size: None,
            mtime_ms: listed_mtime_ms,
        };
        return Ok((
            observed.clone(),
            RemoteManifestRecord {
                path: source_path.to_string(),
                entry_kind: observed.kind.as_str().to_string(),
                version: None,
                size: None,
                mtime_ms: observed.mtime_ms,
            },
        ));
    }

    stats.files += 1;

    if let Some(version) = remote_file_token(listed_meta) {
        stats.metadata_hits += 1;
        return Ok(remote_file_observation(
            source_path,
            version.to_string(),
            listed_size,
            listed_mtime_ms,
        ));
    }

    if let Some(version) = cached_manifest_version(cached, listed_size, listed_mtime_ms) {
        stats.metadata_hits += 1;
        return Ok(remote_file_observation(
            source_path,
            version,
            listed_size,
            listed_mtime_ms,
        ));
    }

    stats.stat_fallbacks += 1;
    let stat_meta = rt.block_on(operator.stat(source_path))?;
    let stat_size = remote_metadata_size(&stat_meta);
    let stat_mtime_ms = remote_mtime_ms(&stat_meta);

    if let Some(version) = remote_file_token(&stat_meta) {
        stats.metadata_hits += 1;
        return Ok(remote_file_observation(
            source_path,
            version.to_string(),
            stat_size,
            stat_mtime_ms,
        ));
    }

    if let Some(version) = cached_manifest_version(cached, stat_size, stat_mtime_ms) {
        stats.metadata_hits += 1;
        return Ok(remote_file_observation(
            source_path,
            version,
            stat_size,
            stat_mtime_ms,
        ));
    }

    stats.body_fallbacks += 1;
    let data = rt.block_on(operator.read(source_path))?;
    Ok(remote_file_observation(
        source_path,
        hash_bytes(data.to_bytes().as_ref()),
        stat_size,
        stat_mtime_ms,
    ))
}

fn observe_remote_inventory_entry(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
    source_path: &str,
    inventory_entry: InventoryManifestEntry,
    cached: Option<&RemoteManifestRecord>,
    stats: &mut RemoteScanStats,
) -> Result<(ObservedEntry, RemoteManifestRecord)> {
    let kind = inventory_entry_kind(&inventory_entry.kind)?;
    if kind == EntryKind::Dir {
        let observed = ObservedEntry {
            kind,
            version: None,
            size: None,
            mtime_ms: inventory_entry.mtime_ms,
        };
        return Ok((
            observed.clone(),
            RemoteManifestRecord {
                path: source_path.to_string(),
                entry_kind: observed.kind.as_str().to_string(),
                version: None,
                size: None,
                mtime_ms: observed.mtime_ms,
            },
        ));
    }

    stats.files += 1;
    if let Some(version) = inventory_entry.version.or(inventory_entry.etag) {
        stats.metadata_hits += 1;
        return Ok(remote_file_observation(
            source_path,
            version,
            inventory_entry.size,
            inventory_entry.mtime_ms,
        ));
    }

    if let Some(version) =
        cached_manifest_version(cached, inventory_entry.size, inventory_entry.mtime_ms)
    {
        stats.metadata_hits += 1;
        return Ok(remote_file_observation(
            source_path,
            version,
            inventory_entry.size,
            inventory_entry.mtime_ms,
        ));
    }

    stats.stat_fallbacks += 1;
    let stat_meta = rt.block_on(operator.stat(source_path))?;
    let stat_size = remote_metadata_size(&stat_meta);
    let stat_mtime_ms = remote_mtime_ms(&stat_meta);

    if let Some(version) = remote_file_token(&stat_meta) {
        stats.metadata_hits += 1;
        return Ok(remote_file_observation(
            source_path,
            version,
            stat_size,
            stat_mtime_ms,
        ));
    }

    if let Some(version) = cached_manifest_version(cached, stat_size, stat_mtime_ms) {
        stats.metadata_hits += 1;
        return Ok(remote_file_observation(
            source_path,
            version,
            stat_size,
            stat_mtime_ms,
        ));
    }

    stats.body_fallbacks += 1;
    let data = rt.block_on(operator.read(source_path))?;
    Ok(remote_file_observation(
        source_path,
        hash_bytes(data.to_bytes().as_ref()),
        stat_size,
        stat_mtime_ms,
    ))
}

fn parse_inventory_manifest(bytes: &[u8]) -> Result<Vec<InventoryManifestEntry>> {
    let payload = std::str::from_utf8(bytes)?;
    let trimmed = payload.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    if trimmed.starts_with('[') {
        return Ok(serde_json::from_str(trimmed)?);
    }

    trimmed
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).map_err(Into::into))
        .collect()
}

fn inventory_entry_kind(raw: &str) -> Result<EntryKind> {
    match raw {
        "file" => Ok(EntryKind::File),
        "dir" => Ok(EntryKind::Dir),
        other => anyhow::bail!("unsupported inventory entry kind {other}"),
    }
}

fn default_inventory_entry_kind() -> String {
    "file".to_string()
}

fn remote_file_observation(
    source_path: &str,
    version: String,
    size: Option<u64>,
    mtime_ms: Option<i64>,
) -> (ObservedEntry, RemoteManifestRecord) {
    let observed = ObservedEntry {
        kind: EntryKind::File,
        version: Some(version.clone()),
        size,
        mtime_ms,
    };
    let record = RemoteManifestRecord {
        path: source_path.to_string(),
        entry_kind: "file".to_string(),
        version: Some(version),
        size,
        mtime_ms,
    };
    (observed, record)
}

fn cached_manifest_version(
    cached: Option<&RemoteManifestRecord>,
    size: Option<u64>,
    mtime_ms: Option<i64>,
) -> Option<String> {
    cached.and_then(|record| {
        (record.entry_kind == "file"
            && record.version.is_some()
            && size.is_some()
            && mtime_ms.is_some()
            && record.size == size
            && record.mtime_ms == mtime_ms)
            .then(|| record.version.clone())
            .flatten()
    })
}

fn remote_metadata_size(meta: &Metadata) -> Option<u64> {
    let size = meta.content_length();
    if size > 0 || remote_file_token(meta).is_some() {
        Some(size)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{DefaultSnapshotCollector, SnapshotCollector};
    use crate::sync::EntryKind;
    use opendal::services;
    use opendal::Operator;
    use section_provider::PathSyncStateRecord;
    use std::collections::HashMap;
    use std::fs;

    fn fs_operator(root: &std::path::Path) -> Operator {
        let builder = services::Fs::default().root(root.to_str().expect("utf8 path"));
        Operator::new(builder).expect("operator").finish()
    }

    #[test]
    fn collect_local_new_file_records_version() {
        let tmp = tempfile::tempdir().expect("tempdir");
        fs::write(tmp.path().join("new.txt"), "hello").expect("write file");
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let collector = DefaultSnapshotCollector::new(&rt, None);

        let snapshot = collector
            .collect_local(tmp.path(), &HashMap::new())
            .expect("collect local");

        let entry = snapshot.entries.get("new.txt").expect("new.txt entry");
        assert_eq!(entry.kind, EntryKind::File);
        assert!(entry.version.is_some());
        assert_eq!(entry.size, Some(5));
        assert_eq!(snapshot.stats.cache_hits, 0);
        assert_eq!(snapshot.stats.cache_misses, 1);
    }

    #[test]
    fn collect_local_reuses_cached_hash_when_metadata_matches() {
        let tmp = tempfile::tempdir().expect("tempdir");
        fs::write(tmp.path().join("new.txt"), "hello").expect("write file");
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let collector = DefaultSnapshotCollector::new(&rt, None);

        let first = collector
            .collect_local(tmp.path(), &HashMap::new())
            .expect("first collect");
        let cache = first
            .cache_records
            .into_iter()
            .map(|record| (record.path.clone(), record))
            .collect::<HashMap<_, _>>();

        let second = collector
            .collect_local(tmp.path(), &cache)
            .expect("second collect");

        assert_eq!(second.stats.cache_hits, 1);
        assert_eq!(second.stats.cache_misses, 0);
        assert_eq!(
            second
                .entries
                .get("new.txt")
                .and_then(|entry| entry.version.clone()),
            Some("2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824".to_string())
        );
    }

    #[test]
    fn collect_remote_reuses_manifest_version_when_metadata_matches() {
        let remote = tempfile::tempdir().expect("remote tempdir");
        fs::write(remote.path().join("notes.txt"), "hello").expect("write remote");
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let collector = DefaultSnapshotCollector::new(&rt, None);

        let first = collector
            .collect_remote(&fs_operator(remote.path()), &HashMap::new())
            .expect("first collect remote");
        let manifest = first
            .manifest_records
            .into_iter()
            .map(|record| (record.path.clone(), record))
            .collect::<HashMap<_, _>>();

        let second = collector
            .collect_remote(&fs_operator(remote.path()), &manifest)
            .expect("second collect remote");

        assert_eq!(second.stats.body_fallbacks, 0);
        assert!(
            second.stats.metadata_hits >= 1,
            "expected metadata-based remote reuse"
        );
        assert_eq!(
            second
                .entries
                .get("notes.txt")
                .and_then(|entry| entry.version.clone()),
            manifest
                .get("notes.txt")
                .and_then(|record| record.version.clone())
        );
    }

    #[test]
    fn collect_remote_can_use_inventory_manifest_accelerator() {
        let remote = tempfile::tempdir().expect("remote tempdir");
        fs::write(remote.path().join("notes.txt"), "hello").expect("write remote");
        fs::write(
            remote.path().join("inventory.jsonl"),
            r#"{"path":"notes.txt","kind":"file","version":"inventory-v1","size":5,"mtime_ms":1}"#,
        )
        .expect("write inventory");
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let collector = DefaultSnapshotCollector::new(&rt, Some("inventory.jsonl".to_string()));

        let snapshot = collector
            .collect_remote(&fs_operator(remote.path()), &HashMap::new())
            .expect("collect remote via inventory");

        assert_eq!(
            snapshot.stats.accelerator.as_deref(),
            Some("inventory_manifest")
        );
        assert_eq!(snapshot.stats.accelerated_entries, 1);
        assert_eq!(
            snapshot
                .entries
                .get("notes.txt")
                .and_then(|entry| entry.version.clone()),
            Some("inventory-v1".to_string())
        );
    }

    #[test]
    fn collect_remote_inventory_manifest_skips_manifest_object() {
        let remote = tempfile::tempdir().expect("remote tempdir");
        fs::write(remote.path().join("notes.txt"), "hello").expect("write remote");
        fs::write(
            remote.path().join("inventory.jsonl"),
            concat!(
                r#"{"path":"notes.txt","kind":"file","version":"inventory-v1","size":5,"mtime_ms":1}"#,
                "\n",
                r#"{"path":"inventory.jsonl","kind":"file","version":"inventory-self","size":2,"mtime_ms":1}"#
            ),
        )
        .expect("write inventory");
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let collector = DefaultSnapshotCollector::new(&rt, Some("inventory.jsonl".to_string()));

        let snapshot = collector
            .collect_remote(&fs_operator(remote.path()), &HashMap::new())
            .expect("collect remote via inventory");

        assert_eq!(snapshot.stats.accelerated_entries, 1);
        assert!(snapshot.entries.contains_key("notes.txt"));
        assert!(!snapshot.entries.contains_key("inventory.jsonl"));
    }

    #[test]
    fn build_inputs_preserves_previous_state() {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let collector = DefaultSnapshotCollector::new(&rt, None);
        let mut previous = HashMap::new();
        previous.insert(
            "docs/readme.txt".to_string(),
            PathSyncStateRecord {
                source_name: "local".to_string(),
                path: "docs/readme.txt".to_string(),
                entry_kind: "file".to_string(),
                public_state: "ready".to_string(),
                local_present: true,
                dirty_local: false,
                dirty_remote: false,
                pinned: false,
                stale: false,
                last_local_version: Some("local-v1".to_string()),
                base_remote_version: Some("remote-v1".to_string()),
                current_remote_version: Some("remote-v1".to_string()),
            },
        );

        let inputs = collector.build_inputs(previous, HashMap::new(), HashMap::new());
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].path, "docs/readme.txt");
        assert!(inputs[0].previous.is_some());
    }
}
