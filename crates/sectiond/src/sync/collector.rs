use super::{normalize_source_path, observe_local_entry, observe_remote_entry, EntryKind};
use anyhow::Result;
use opendal::{Entry, Operator};
use section_provider::PathSyncStateRecord;
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone)]
pub(crate) struct ObservedEntry {
    pub(crate) kind: EntryKind,
    pub(crate) version: Option<String>,
    // TODO(#002): use cached size for local mtime/size fast path.
    #[allow(dead_code)]
    pub(crate) size: Option<u64>,
    // TODO(#002): use cached mtime for incremental local/remote scans.
    #[allow(dead_code)]
    pub(crate) mtime_ms: Option<i64>,
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
        previous: &HashMap<String, PathSyncStateRecord>,
    ) -> Result<HashMap<String, ObservedEntry>>;

    fn collect_remote(
        &self,
        op: &Operator,
        previous: &HashMap<String, PathSyncStateRecord>,
    ) -> Result<HashMap<String, ObservedEntry>>;

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
        previous: &HashMap<String, PathSyncStateRecord>,
    ) -> Result<HashMap<String, ObservedEntry>> {
        // Reserved for Issue #002: current implementation is still a full scan.
        let _ = previous;
        let mut entries = HashMap::new();

        fn walk(
            current: &Path,
            root: &Path,
            entries: &mut HashMap<String, ObservedEntry>,
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

                let observed = observe_local_entry(&path)?;
                let descend = observed.kind.as_str() == "dir";
                entries.insert(source_path.clone(), observed);

                if descend {
                    walk(&path, root, entries)?;
                }
            }

            Ok(())
        }

        if root.exists() {
            walk(root, root, &mut entries)?;
        }

        Ok(entries)
    }

    fn collect_remote(
        &self,
        op: &Operator,
        previous: &HashMap<String, PathSyncStateRecord>,
    ) -> Result<HashMap<String, ObservedEntry>> {
        // Reserved for Issue #002: current implementation is still a full scan.
        let _ = previous;
        let entries: Vec<Entry> = self
            .rt
            .block_on(async { op.list_with("").recursive(true).await })?;
        let mut result = HashMap::new();

        for entry in entries {
            let path = normalize_source_path(entry.path());
            if path.is_empty() {
                continue;
            }

            let observed = observe_remote_entry(self.rt, op, &path, entry.metadata())?;
            result.insert(path, observed);
        }

        Ok(result)
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

#[cfg(test)]
mod tests {
    use super::{DefaultSnapshotCollector, SnapshotCollector};
    use crate::sync::EntryKind;
    use section_provider::PathSyncStateRecord;
    use std::collections::HashMap;
    use std::fs;

    #[test]
    fn collect_local_new_file_records_version() {
        let tmp = tempfile::tempdir().expect("tempdir");
        fs::write(tmp.path().join("new.txt"), "hello").expect("write file");
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let collector = DefaultSnapshotCollector::new(&rt);

        let local = collector
            .collect_local(tmp.path(), &HashMap::new())
            .expect("collect local");

        let entry = local.get("new.txt").expect("new.txt entry");
        assert_eq!(entry.kind, EntryKind::File);
        assert!(entry.version.is_some());
        assert_eq!(entry.size, Some(5));
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
