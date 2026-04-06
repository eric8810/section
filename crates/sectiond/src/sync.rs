use anyhow::{anyhow, bail, Result};
use opendal::{Entry, EntryMode, Metadata, Operator};
use ring::digest::{digest, SHA256};
use section_provider::{PathSyncStateRecord, ProviderStore, SyncEventRecord};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::SectiondRuntime;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SourceSyncResult {
    pub source_id: String,
    pub local_root: PathBuf,
    pub pulled: usize,
    pub pushed: usize,
    pub conflicts: usize,
    pub events_emitted: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PathCompareSnapshot {
    pub source_id: String,
    pub local_root: PathBuf,
    pub local_path: PathBuf,
    pub source_path: String,
    pub entry_kind: Option<String>,
    pub state: String,
    pub local_present: bool,
    pub remote_present: bool,
    pub local_version: Option<String>,
    pub base_remote_version: Option<String>,
    pub current_remote_version: Option<String>,
    pub local_matches_base: bool,
    pub local_matches_current_remote: bool,
    pub stale: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PathResolveStrategy {
    UseLocal,
    UseRemote,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PathResolveResult {
    pub source_id: String,
    pub local_root: PathBuf,
    pub local_path: PathBuf,
    pub source_path: String,
    pub strategy: String,
    pub state: String,
    pub base_remote_version: Option<String>,
    pub current_remote_version: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryKindRepr {
    File,
    Dir,
}

impl EntryKindRepr {
    fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Dir => "dir",
        }
    }
}

#[derive(Debug, Clone)]
struct LocalEntrySnapshot {
    kind: EntryKindRepr,
    version: Option<String>,
}

#[derive(Debug, Clone)]
struct RemoteEntrySnapshot {
    kind: EntryKindRepr,
    version: Option<String>,
}

pub fn sync_source(
    runtime: &SectiondRuntime,
    store: &ProviderStore,
    source_id: &str,
    local_root: &Path,
) -> Result<SourceSyncResult> {
    let operator = runtime.router().get_operator(source_id)?;
    let rt = tokio::runtime::Runtime::new()?;
    fs::create_dir_all(local_root)?;

    let local_entries = scan_local_tree(local_root)?;
    let remote_entries = scan_remote_tree(&rt, operator)?;
    let existing = store
        .list_path_sync_states(source_id)?
        .into_iter()
        .map(|record| (record.path.clone(), record))
        .collect::<HashMap<_, _>>();

    let mut all_paths = BTreeSet::new();
    all_paths.extend(local_entries.keys().cloned());
    all_paths.extend(remote_entries.keys().cloned());
    all_paths.extend(existing.keys().cloned());

    let mut pulled = 0;
    let mut pushed = 0;
    let mut conflicts = 0;
    let mut events_emitted = 0;

    for source_path in all_paths {
        let local = local_entries.get(&source_path);
        let remote = remote_entries.get(&source_path);
        let previous = existing.get(&source_path);

        let outcome = reconcile_path(
            &rt,
            operator,
            source_id,
            local_root,
            &source_path,
            previous.cloned(),
            local.cloned(),
            remote.cloned(),
        )?;

        if let Some(record) = outcome.record {
            store.upsert_path_sync_state(&record)?;
        } else {
            store.remove_path_sync_state(source_id, &source_path)?;
        }

        for event in outcome.events {
            store.append_sync_event(
                &event.source_name,
                &event.path,
                &event.kind,
                &event.state,
                event.created_at_ms,
            )?;
            events_emitted += 1;
        }

        pulled += outcome.pulled;
        pushed += outcome.pushed;
        conflicts += outcome.conflicts;
    }

    Ok(SourceSyncResult {
        source_id: source_id.to_string(),
        local_root: local_root.to_path_buf(),
        pulled,
        pushed,
        conflicts,
        events_emitted,
    })
}

pub fn compare_path(
    runtime: &SectiondRuntime,
    store: &ProviderStore,
    source_id: &str,
    local_root: &Path,
    local_path: &Path,
    source_path: &str,
) -> Result<PathCompareSnapshot> {
    let operator = runtime.router().get_operator(source_id)?;
    let rt = tokio::runtime::Runtime::new()?;
    let local = inspect_local_entry(local_path)?;
    let remote = inspect_remote_entry(&rt, operator, source_path)?;
    let stored = store.get_path_sync_state(source_id, source_path)?;

    let entry_kind = local
        .as_ref()
        .map(|entry| entry.kind.as_str().to_string())
        .or_else(|| remote.as_ref().map(|entry| entry.kind.as_str().to_string()))
        .or_else(|| stored.as_ref().map(|record| record.entry_kind.clone()));

    let state = if let Some(record) = &stored {
        record.public_state.clone()
    } else if local.is_some() || remote.is_some() {
        "ready".to_string()
    } else {
        "error".to_string()
    };

    let local_version = local.as_ref().and_then(|entry| entry.version.clone());
    let current_remote_version = remote.as_ref().and_then(|entry| entry.version.clone());
    let base_remote_version = stored
        .as_ref()
        .and_then(|record| record.base_remote_version.clone());
    let stale = stored.as_ref().map(|record| record.stale).unwrap_or(false);

    Ok(PathCompareSnapshot {
        source_id: source_id.to_string(),
        local_root: local_root.to_path_buf(),
        local_path: local_path.to_path_buf(),
        source_path: display_source_path(source_path),
        entry_kind,
        state,
        local_present: local.is_some(),
        remote_present: remote.is_some(),
        local_version: local_version.clone(),
        base_remote_version: base_remote_version.clone(),
        current_remote_version: current_remote_version.clone(),
        local_matches_base: local_version.is_some() && local_version == base_remote_version,
        local_matches_current_remote: local_version.is_some()
            && local_version == current_remote_version,
        stale,
    })
}

pub fn resolve_path(
    runtime: &SectiondRuntime,
    store: &ProviderStore,
    source_id: &str,
    local_root: &Path,
    local_path: &Path,
    source_path: &str,
    strategy: PathResolveStrategy,
) -> Result<PathResolveResult> {
    let operator = runtime.router().get_operator(source_id)?;
    let rt = tokio::runtime::Runtime::new()?;
    let compare = compare_path(
        runtime,
        store,
        source_id,
        local_root,
        local_path,
        source_path,
    )?;
    let existing = store
        .get_path_sync_state(source_id, source_path)?
        .ok_or_else(|| anyhow!("path {} is not tracked", display_source_path(source_path)))?;

    if existing.public_state != "conflict" {
        bail!(
            "path {} is not in conflict",
            display_source_path(source_path)
        );
    }

    let local = inspect_local_entry(local_path)?;
    let remote = inspect_remote_entry(&rt, operator, source_path)?;

    let resolved = match strategy {
        PathResolveStrategy::UseLocal => apply_use_local(
            &rt,
            operator,
            source_id,
            local_root,
            source_path,
            local,
            remote,
        )?,
        PathResolveStrategy::UseRemote => apply_use_remote(
            &rt,
            operator,
            source_id,
            local_root,
            source_path,
            local_path,
            local,
            remote,
        )?,
    };

    store.upsert_path_sync_state(&resolved.record)?;
    for event in resolved.events {
        store.append_sync_event(
            &event.source_name,
            &event.path,
            &event.kind,
            &event.state,
            event.created_at_ms,
        )?;
    }

    Ok(PathResolveResult {
        source_id: source_id.to_string(),
        local_root: local_root.to_path_buf(),
        local_path: compare.local_path,
        source_path: display_source_path(source_path),
        strategy: match strategy {
            PathResolveStrategy::UseLocal => "use-local".to_string(),
            PathResolveStrategy::UseRemote => "use-remote".to_string(),
        },
        state: "ready".to_string(),
        base_remote_version: resolved.record.base_remote_version,
        current_remote_version: resolved.record.current_remote_version,
    })
}

pub fn list_watch_events(
    store: &ProviderStore,
    source_id: &str,
    source_prefix: &str,
    after_id: i64,
) -> Result<Vec<SyncEventRecord>> {
    let events = store.list_sync_events_after(source_id, after_id)?;
    let prefix = source_prefix.trim_matches('/');
    if prefix.is_empty() {
        return Ok(events);
    }

    let nested_prefix = format!("{prefix}/");
    Ok(events
        .into_iter()
        .filter(|event| event.path == prefix || event.path.starts_with(&nested_prefix))
        .collect())
}

#[derive(Debug)]
struct ReconcileOutcome {
    record: Option<PathSyncStateRecord>,
    events: Vec<PendingSyncEvent>,
    pulled: usize,
    pushed: usize,
    conflicts: usize,
}

#[derive(Debug, Clone)]
struct PendingSyncEvent {
    source_name: String,
    path: String,
    kind: String,
    state: String,
    created_at_ms: i64,
}

#[derive(Debug)]
struct ResolveOutcome {
    record: PathSyncStateRecord,
    events: Vec<PendingSyncEvent>,
}

fn reconcile_path(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
    source_id: &str,
    local_root: &Path,
    source_path: &str,
    previous: Option<PathSyncStateRecord>,
    local: Option<LocalEntrySnapshot>,
    remote: Option<RemoteEntrySnapshot>,
) -> Result<ReconcileOutcome> {
    if local.is_none() && remote.is_none() {
        return Ok(ReconcileOutcome {
            record: None,
            events: Vec::new(),
            pulled: 0,
            pushed: 0,
            conflicts: 0,
        });
    }

    if let (Some(local), Some(remote)) = (&local, &remote) {
        if local.kind != remote.kind {
            let record = conflict_record(source_id, source_path, local, remote, previous.as_ref());
            let event = event_for(source_id, source_path, "conflict_detected", "conflict");
            return Ok(ReconcileOutcome {
                record: Some(record),
                events: vec![event],
                pulled: 0,
                pushed: 0,
                conflicts: 1,
            });
        }
    }

    let prev_base_remote = previous
        .as_ref()
        .and_then(|record| record.base_remote_version.clone());
    let prev_local_version = previous
        .as_ref()
        .and_then(|record| record.last_local_version.clone());
    let prev_local_present = previous
        .as_ref()
        .map(|record| record.local_present)
        .unwrap_or(false);

    let local_changed = match &local {
        Some(entry) => entry.version != prev_local_version || !prev_local_present,
        None => prev_local_present,
    };
    let remote_version = remote.as_ref().and_then(|entry| entry.version.clone());
    let remote_changed = remote_version != prev_base_remote;

    match (local, remote) {
        (Some(local), Some(remote)) => {
            if !local_changed && !remote_changed {
                Ok(ReconcileOutcome {
                    record: Some(ready_record(
                        source_id,
                        source_path,
                        local.kind,
                        local.version.clone(),
                        remote.version.clone(),
                        true,
                    )),
                    events: Vec::new(),
                    pulled: 0,
                    pushed: 0,
                    conflicts: 0,
                })
            } else if local_changed && !remote_changed {
                let remote_version =
                    push_local_entry(rt, operator, local_root, source_path, &local)?;
                let event = event_for(source_id, source_path, "synced_to_remote", "ready");
                Ok(ReconcileOutcome {
                    record: Some(ready_record(
                        source_id,
                        source_path,
                        local.kind,
                        local.version.clone(),
                        remote_version,
                        true,
                    )),
                    events: vec![event],
                    pulled: 0,
                    pushed: 1,
                    conflicts: 0,
                })
            } else if !local_changed && remote_changed {
                let local_version =
                    pull_remote_entry(rt, operator, local_root, source_path, &remote)?;
                let event = event_for(source_id, source_path, "synced_from_remote", "ready");
                Ok(ReconcileOutcome {
                    record: Some(ready_record(
                        source_id,
                        source_path,
                        remote.kind,
                        local_version,
                        remote.version.clone(),
                        true,
                    )),
                    events: vec![event],
                    pulled: 1,
                    pushed: 0,
                    conflicts: 0,
                })
            } else {
                let record =
                    conflict_record(source_id, source_path, &local, &remote, previous.as_ref());
                let event = event_for(source_id, source_path, "conflict_detected", "conflict");
                Ok(ReconcileOutcome {
                    record: Some(record),
                    events: vec![event],
                    pulled: 0,
                    pushed: 0,
                    conflicts: 1,
                })
            }
        }
        (None, Some(remote)) => {
            if prev_local_present {
                if remote_changed {
                    let record = absent_conflict_record(
                        source_id,
                        source_path,
                        remote.kind,
                        false,
                        remote.version.clone(),
                        previous.as_ref(),
                    );
                    let event = event_for(source_id, source_path, "conflict_detected", "conflict");
                    Ok(ReconcileOutcome {
                        record: Some(record),
                        events: vec![event],
                        pulled: 0,
                        pushed: 0,
                        conflicts: 1,
                    })
                } else {
                    delete_remote_entry(rt, operator, source_path, remote.kind)?;
                    let event = event_for(source_id, source_path, "synced_to_remote", "ready");
                    Ok(ReconcileOutcome {
                        record: None,
                        events: vec![event],
                        pulled: 0,
                        pushed: 1,
                        conflicts: 0,
                    })
                }
            } else {
                let local_version =
                    pull_remote_entry(rt, operator, local_root, source_path, &remote)?;
                let event = event_for(source_id, source_path, "synced_from_remote", "ready");
                Ok(ReconcileOutcome {
                    record: Some(ready_record(
                        source_id,
                        source_path,
                        remote.kind,
                        local_version,
                        remote.version.clone(),
                        true,
                    )),
                    events: vec![event],
                    pulled: 1,
                    pushed: 0,
                    conflicts: 0,
                })
            }
        }
        (Some(local), None) => {
            if prev_base_remote.is_some() {
                if local_changed {
                    let record = absent_conflict_record(
                        source_id,
                        source_path,
                        local.kind,
                        true,
                        None,
                        previous.as_ref(),
                    );
                    let event = event_for(source_id, source_path, "conflict_detected", "conflict");
                    Ok(ReconcileOutcome {
                        record: Some(record),
                        events: vec![event],
                        pulled: 0,
                        pushed: 0,
                        conflicts: 1,
                    })
                } else {
                    delete_local_entry(local_root, source_path, local.kind)?;
                    let event = event_for(source_id, source_path, "synced_from_remote", "ready");
                    Ok(ReconcileOutcome {
                        record: None,
                        events: vec![event],
                        pulled: 1,
                        pushed: 0,
                        conflicts: 0,
                    })
                }
            } else {
                let remote_version =
                    push_local_entry(rt, operator, local_root, source_path, &local)?;
                let event = event_for(source_id, source_path, "synced_to_remote", "ready");
                Ok(ReconcileOutcome {
                    record: Some(ready_record(
                        source_id,
                        source_path,
                        local.kind,
                        local.version.clone(),
                        remote_version,
                        true,
                    )),
                    events: vec![event],
                    pulled: 0,
                    pushed: 1,
                    conflicts: 0,
                })
            }
        }
        (None, None) => unreachable!("handled above"),
    }
}

fn apply_use_local(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
    source_id: &str,
    local_root: &Path,
    source_path: &str,
    local: Option<LocalEntrySnapshot>,
    remote: Option<RemoteEntrySnapshot>,
) -> Result<ResolveOutcome> {
    let now = now_ms();
    match local {
        Some(local) => {
            let remote_version = push_local_entry(rt, operator, local_root, source_path, &local)?;
            Ok(ResolveOutcome {
                record: ready_record(
                    source_id,
                    source_path,
                    local.kind,
                    local.version,
                    remote_version,
                    true,
                ),
                events: vec![PendingSyncEvent {
                    source_name: source_id.to_string(),
                    path: source_path.to_string(),
                    kind: "resolved".to_string(),
                    state: "ready".to_string(),
                    created_at_ms: now,
                }],
            })
        }
        None => {
            if let Some(remote) = remote {
                delete_remote_entry(rt, operator, source_path, remote.kind)?;
            }
            Ok(ResolveOutcome {
                record: ready_absent_record(source_id, source_path),
                events: vec![PendingSyncEvent {
                    source_name: source_id.to_string(),
                    path: source_path.to_string(),
                    kind: "resolved".to_string(),
                    state: "ready".to_string(),
                    created_at_ms: now,
                }],
            })
        }
    }
}

fn apply_use_remote(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
    source_id: &str,
    local_root: &Path,
    source_path: &str,
    local_path: &Path,
    _local: Option<LocalEntrySnapshot>,
    remote: Option<RemoteEntrySnapshot>,
) -> Result<ResolveOutcome> {
    let now = now_ms();
    match remote {
        Some(remote) => {
            let local_version = pull_remote_entry(rt, operator, local_root, source_path, &remote)?;
            Ok(ResolveOutcome {
                record: ready_record(
                    source_id,
                    source_path,
                    remote.kind,
                    local_version,
                    remote.version,
                    true,
                ),
                events: vec![PendingSyncEvent {
                    source_name: source_id.to_string(),
                    path: source_path.to_string(),
                    kind: "resolved".to_string(),
                    state: "ready".to_string(),
                    created_at_ms: now,
                }],
            })
        }
        None => {
            if local_path.exists() {
                delete_local_entry(
                    local_root,
                    source_path,
                    infer_local_kind_from_path(local_path)?,
                )?;
            }
            Ok(ResolveOutcome {
                record: ready_absent_record(source_id, source_path),
                events: vec![PendingSyncEvent {
                    source_name: source_id.to_string(),
                    path: source_path.to_string(),
                    kind: "resolved".to_string(),
                    state: "ready".to_string(),
                    created_at_ms: now,
                }],
            })
        }
    }
}

fn ready_record(
    source_id: &str,
    source_path: &str,
    kind: EntryKindRepr,
    local_version: Option<String>,
    remote_version: Option<String>,
    local_present: bool,
) -> PathSyncStateRecord {
    PathSyncStateRecord {
        source_name: source_id.to_string(),
        path: source_path.to_string(),
        entry_kind: kind.as_str().to_string(),
        public_state: "ready".to_string(),
        local_present,
        dirty_local: false,
        dirty_remote: false,
        pinned: false,
        stale: false,
        last_local_version: local_version,
        base_remote_version: remote_version.clone(),
        current_remote_version: remote_version,
    }
}

fn ready_absent_record(source_id: &str, source_path: &str) -> PathSyncStateRecord {
    PathSyncStateRecord {
        source_name: source_id.to_string(),
        path: source_path.to_string(),
        entry_kind: "file".to_string(),
        public_state: "ready".to_string(),
        local_present: false,
        dirty_local: false,
        dirty_remote: false,
        pinned: false,
        stale: false,
        last_local_version: None,
        base_remote_version: None,
        current_remote_version: None,
    }
}

fn conflict_record(
    source_id: &str,
    source_path: &str,
    local: &LocalEntrySnapshot,
    remote: &RemoteEntrySnapshot,
    previous: Option<&PathSyncStateRecord>,
) -> PathSyncStateRecord {
    PathSyncStateRecord {
        source_name: source_id.to_string(),
        path: source_path.to_string(),
        entry_kind: local.kind.as_str().to_string(),
        public_state: "conflict".to_string(),
        local_present: true,
        dirty_local: true,
        dirty_remote: true,
        pinned: false,
        stale: true,
        last_local_version: local.version.clone(),
        base_remote_version: previous.and_then(|record| record.base_remote_version.clone()),
        current_remote_version: remote.version.clone(),
    }
}

fn absent_conflict_record(
    source_id: &str,
    source_path: &str,
    kind: EntryKindRepr,
    local_present: bool,
    current_remote_version: Option<String>,
    previous: Option<&PathSyncStateRecord>,
) -> PathSyncStateRecord {
    PathSyncStateRecord {
        source_name: source_id.to_string(),
        path: source_path.to_string(),
        entry_kind: kind.as_str().to_string(),
        public_state: "conflict".to_string(),
        local_present,
        dirty_local: local_present,
        dirty_remote: current_remote_version
            != previous.and_then(|record| record.base_remote_version.clone()),
        pinned: false,
        stale: true,
        last_local_version: previous.and_then(|record| record.last_local_version.clone()),
        base_remote_version: previous.and_then(|record| record.base_remote_version.clone()),
        current_remote_version,
    }
}

fn event_for(source_id: &str, source_path: &str, kind: &str, state: &str) -> PendingSyncEvent {
    PendingSyncEvent {
        source_name: source_id.to_string(),
        path: source_path.to_string(),
        kind: kind.to_string(),
        state: state.to_string(),
        created_at_ms: now_ms(),
    }
}

fn scan_local_tree(local_root: &Path) -> Result<HashMap<String, LocalEntrySnapshot>> {
    let mut entries = HashMap::new();

    fn walk(
        current: &Path,
        root: &Path,
        entries: &mut HashMap<String, LocalEntrySnapshot>,
    ) -> Result<()> {
        let mut dir_entries = fs::read_dir(current)?.collect::<std::result::Result<Vec<_>, _>>()?;
        dir_entries.sort_by_key(|entry| entry.file_name());

        for entry in dir_entries {
            let path = entry.path();
            if current == root && entry.file_name() == ".section" {
                continue;
            }

            let metadata = entry.metadata()?;
            let relative = path.strip_prefix(root).map_err(|err| {
                anyhow!(
                    "failed to derive relative path for {}: {err}",
                    path.display()
                )
            })?;
            let source_path = relative
                .iter()
                .map(|segment| segment.to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");

            if metadata.is_dir() {
                entries.insert(
                    source_path.clone(),
                    LocalEntrySnapshot {
                        kind: EntryKindRepr::Dir,
                        version: None,
                    },
                );
                walk(&path, root, entries)?;
            } else if metadata.is_file() {
                entries.insert(
                    source_path,
                    LocalEntrySnapshot {
                        kind: EntryKindRepr::File,
                        version: Some(hash_bytes(&fs::read(&path)?)),
                    },
                );
            }
        }

        Ok(())
    }

    if local_root.exists() {
        walk(local_root, local_root, &mut entries)?;
    }

    Ok(entries)
}

fn scan_remote_tree(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
) -> Result<HashMap<String, RemoteEntrySnapshot>> {
    let entries: Vec<Entry> =
        rt.block_on(async { operator.list_with("").recursive(true).await })?;
    let mut result = HashMap::new();

    for entry in entries {
        let path = normalize_source_path(entry.path());
        if path.is_empty() {
            continue;
        }

        let kind = metadata_entry_kind(entry.metadata())?;
        let version = match kind {
            EntryKindRepr::File => {
                Some(remote_file_version(rt, operator, &path, entry.metadata())?)
            }
            EntryKindRepr::Dir => None,
        };
        result.insert(path.clone(), RemoteEntrySnapshot { kind, version });
    }

    Ok(result)
}

fn inspect_local_entry(local_path: &Path) -> Result<Option<LocalEntrySnapshot>> {
    if !local_path.exists() {
        return Ok(None);
    }

    let metadata = fs::metadata(local_path)?;
    if metadata.is_dir() {
        Ok(Some(LocalEntrySnapshot {
            kind: EntryKindRepr::Dir,
            version: None,
        }))
    } else if metadata.is_file() {
        Ok(Some(LocalEntrySnapshot {
            kind: EntryKindRepr::File,
            version: Some(hash_bytes(&fs::read(local_path)?)),
        }))
    } else {
        bail!("unsupported local entry type for {}", local_path.display());
    }
}

fn inspect_remote_entry(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
    source_path: &str,
) -> Result<Option<RemoteEntrySnapshot>> {
    if source_path.is_empty() {
        return Ok(None);
    }

    for candidate in [source_path.to_string(), normalized_dir(source_path)] {
        let meta = match rt.block_on(operator.stat(&candidate)) {
            Ok(meta) => meta,
            Err(err) if err.kind() == opendal::ErrorKind::NotFound => continue,
            Err(err) => return Err(err.into()),
        };
        let kind = metadata_entry_kind(&meta)?;
        let version = match kind {
            EntryKindRepr::File => Some(remote_file_version(rt, operator, source_path, &meta)?),
            EntryKindRepr::Dir => None,
        };
        return Ok(Some(RemoteEntrySnapshot { kind, version }));
    }

    Ok(None)
}

fn metadata_entry_kind(meta: &Metadata) -> Result<EntryKindRepr> {
    match meta.mode() {
        EntryMode::FILE => Ok(EntryKindRepr::File),
        EntryMode::DIR => Ok(EntryKindRepr::Dir),
        _ => bail!("unsupported entry mode {:?}", meta.mode()),
    }
}

fn remote_file_version(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
    source_path: &str,
    meta: &Metadata,
) -> Result<String> {
    if let Some(etag) = meta.etag() {
        if !etag.is_empty() {
            return Ok(etag.to_string());
        }
    }

    let data = rt.block_on(operator.read(source_path))?;
    Ok(hash_bytes(data.to_bytes().as_ref()))
}

fn push_local_entry(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
    local_root: &Path,
    source_path: &str,
    local: &LocalEntrySnapshot,
) -> Result<Option<String>> {
    match local.kind {
        EntryKindRepr::Dir => {
            ensure_remote_dir(rt, operator, source_path)?;
            Ok(None)
        }
        EntryKindRepr::File => {
            let local_path = local_root.join(source_path);
            let data = fs::read(&local_path)?;
            ensure_remote_parent_dirs(rt, operator, source_path)?;
            rt.block_on(operator.write(source_path, data))?;
            Ok(local.version.clone())
        }
    }
}

fn pull_remote_entry(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
    local_root: &Path,
    source_path: &str,
    remote: &RemoteEntrySnapshot,
) -> Result<Option<String>> {
    let local_path = local_root.join(source_path);

    match remote.kind {
        EntryKindRepr::Dir => {
            fs::create_dir_all(&local_path)?;
            Ok(None)
        }
        EntryKindRepr::File => {
            if let Some(parent) = local_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let data = rt.block_on(operator.read(source_path))?;
            fs::write(&local_path, data.to_bytes().as_ref())?;
            Ok(Some(hash_bytes(data.to_bytes().as_ref())))
        }
    }
}

fn delete_remote_entry(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
    source_path: &str,
    kind: EntryKindRepr,
) -> Result<()> {
    match kind {
        EntryKindRepr::Dir => {
            rt.block_on(operator.remove_all(&normalized_dir(source_path)))?;
        }
        EntryKindRepr::File => {
            rt.block_on(operator.delete(source_path))?;
        }
    }
    Ok(())
}

fn delete_local_entry(local_root: &Path, source_path: &str, kind: EntryKindRepr) -> Result<()> {
    let local_path = local_root.join(source_path);
    if !local_path.exists() {
        return Ok(());
    }

    match kind {
        EntryKindRepr::Dir => fs::remove_dir_all(local_path)?,
        EntryKindRepr::File => fs::remove_file(local_path)?,
    }
    Ok(())
}

fn infer_local_kind_from_path(path: &Path) -> Result<EntryKindRepr> {
    let metadata = fs::metadata(path)?;
    if metadata.is_dir() {
        Ok(EntryKindRepr::Dir)
    } else if metadata.is_file() {
        Ok(EntryKindRepr::File)
    } else {
        bail!("unsupported local entry type for {}", path.display())
    }
}

fn ensure_remote_parent_dirs(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
    source_path: &str,
) -> Result<()> {
    if let Some((parent, _)) = source_path.rsplit_once('/') {
        let mut current = String::new();
        for segment in parent.split('/') {
            if segment.is_empty() {
                continue;
            }
            if !current.is_empty() {
                current.push('/');
            }
            current.push_str(segment);
            rt.block_on(operator.create_dir(&normalized_dir(&current)))?;
        }
    }
    Ok(())
}

fn ensure_remote_dir(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
    source_path: &str,
) -> Result<()> {
    ensure_remote_parent_dirs(rt, operator, source_path)?;
    rt.block_on(operator.create_dir(&normalized_dir(source_path)))?;
    Ok(())
}

fn normalize_source_path(path: &str) -> String {
    path.trim_matches('/').to_string()
}

fn normalized_dir(path: &str) -> String {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}/")
    }
}

fn display_source_path(path: &str) -> String {
    if path.is_empty() {
        "/".to_string()
    } else {
        path.to_string()
    }
}

fn hash_bytes(bytes: &[u8]) -> String {
    let digest = digest(&SHA256, bytes);
    digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock drift")
        .as_millis() as i64
}
