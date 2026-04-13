use super::{
    hash_bytes, local_mtime_ms, metadata_entry_kind, normalize_source_path, remote_file_token,
    remote_mtime_ms, EntryKind,
};
use anyhow::Result;
use opendal::{Entry, Metadata, Operator};
use section_provider::{LocalScanCacheRecord, PathSyncStateRecord, RemoteManifestRecord};
use serde::Serialize;
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::Path;

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
}

impl<'a> DefaultSnapshotCollector<'a> {
    pub(crate) fn new(rt: &'a tokio::runtime::Runtime) -> Self {
        Self { rt }
    }
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
        let collector = DefaultSnapshotCollector::new(&rt);

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
        let collector = DefaultSnapshotCollector::new(&rt);

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
        let collector = DefaultSnapshotCollector::new(&rt);

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
    fn build_inputs_preserves_previous_state() {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let collector = DefaultSnapshotCollector::new(&rt);
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
