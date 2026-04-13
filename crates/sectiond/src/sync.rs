mod collector;
mod coordinator;
mod transport;

use anyhow::{anyhow, bail, Result};
use collector::{DefaultSnapshotCollector, ObservedEntry, PathSyncInput, SnapshotCollector};
use coordinator::{
    DefaultSyncCoordinator, PathSyncPlan, PlannedRecordSpec, SyncCoordinator, SyncPlan,
};
use opendal::{EntryMode, Metadata, Operator};
use ring::digest::{digest, SHA256};
use section_provider::{PathSyncStateRecord, ProviderStore, SyncEventRecord};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use transport::{OpenDalTransport, Transport, TransportConfig, TransportOutcome};

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
enum EntryKind {
    File,
    Dir,
}

impl EntryKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Dir => "dir",
        }
    }
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

pub fn sync_source(
    runtime: &SectiondRuntime,
    store: &ProviderStore,
    source_id: &str,
    local_root: &Path,
) -> Result<SourceSyncResult> {
    let operator = runtime.router().get_operator(source_id)?;
    let rt = tokio::runtime::Runtime::new()?;
    fs::create_dir_all(local_root)?;

    let existing = store
        .list_path_sync_states(source_id)?
        .into_iter()
        .map(|record| (record.path.clone(), record))
        .collect::<HashMap<_, _>>();

    let collector = DefaultSnapshotCollector::new(&rt);
    let local = collector.collect_local(local_root, &existing)?;
    let remote = collector.collect_remote(operator, &existing)?;
    let inputs = collector.build_inputs(existing, local, remote);

    let coordinator = DefaultSyncCoordinator::default();
    let SyncPlan { paths } = coordinator.plan(source_id, inputs)?;

    let transport = OpenDalTransport::new(
        &rt,
        operator.clone(),
        local_root.to_path_buf(),
        TransportConfig::default(),
    );

    let mut pulled = 0;
    let mut pushed = 0;
    let mut conflicts = 0;
    let mut events_emitted = 0;

    for path_plan in paths {
        let outcome = transport.execute(&path_plan)?;
        persist_path_plan(store, source_id, &path_plan, outcome, &mut events_emitted)?;
        pulled += path_plan.pulled;
        pushed += path_plan.pushed;
        conflicts += path_plan.conflicts;
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

fn persist_path_plan(
    store: &ProviderStore,
    source_id: &str,
    path_plan: &PathSyncPlan,
    outcome: TransportOutcome,
    events_emitted: &mut usize,
) -> Result<()> {
    match materialize_record(source_id, &path_plan.path, &path_plan.record_spec, outcome)? {
        Some(record) => store.upsert_path_sync_state(&record)?,
        None => store.remove_path_sync_state(source_id, &path_plan.path)?,
    }

    for event in &path_plan.events {
        store.append_sync_event(
            &event.source_name,
            &event.path,
            &event.kind,
            &event.state,
            event.created_at_ms,
        )?;
        *events_emitted += 1;
    }

    Ok(())
}

fn materialize_record(
    source_id: &str,
    source_path: &str,
    record_spec: &PlannedRecordSpec,
    outcome: TransportOutcome,
) -> Result<Option<PathSyncStateRecord>> {
    match record_spec {
        PlannedRecordSpec::Remove => Ok(None),
        PlannedRecordSpec::Static(record) => Ok(Some(record.clone())),
        PlannedRecordSpec::ReadyFromPulledRemote {
            kind,
            remote_version,
        } => Ok(Some(ready_record(
            source_id,
            source_path,
            *kind,
            outcome.local_version,
            remote_version.clone(),
            true,
        ))),
    }
}

fn apply_use_local(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
    source_id: &str,
    local_root: &Path,
    source_path: &str,
    local: Option<ObservedEntry>,
    remote: Option<ObservedEntry>,
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
    _local: Option<ObservedEntry>,
    remote: Option<ObservedEntry>,
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
    kind: EntryKind,
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
    local: &ObservedEntry,
    remote: &ObservedEntry,
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
    kind: EntryKind,
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

fn inspect_local_entry(local_path: &Path) -> Result<Option<ObservedEntry>> {
    if !local_path.exists() {
        return Ok(None);
    }

    observe_local_entry(local_path).map(Some)
}

fn inspect_remote_entry(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
    source_path: &str,
) -> Result<Option<ObservedEntry>> {
    if source_path.is_empty() {
        return Ok(None);
    }

    for candidate in [source_path.to_string(), normalized_dir(source_path)] {
        let meta = match rt.block_on(operator.stat(&candidate)) {
            Ok(meta) => meta,
            Err(err) if err.kind() == opendal::ErrorKind::NotFound => continue,
            Err(err) => return Err(err.into()),
        };
        return observe_remote_entry(rt, operator, source_path, &meta).map(Some);
    }

    Ok(None)
}

fn observe_local_entry(local_path: &Path) -> Result<ObservedEntry> {
    let metadata = fs::metadata(local_path)?;
    if metadata.is_dir() {
        Ok(ObservedEntry {
            kind: EntryKind::Dir,
            version: None,
            size: None,
            mtime_ms: local_mtime_ms(&metadata),
        })
    } else if metadata.is_file() {
        Ok(ObservedEntry {
            kind: EntryKind::File,
            version: Some(hash_bytes(&fs::read(local_path)?)),
            size: Some(metadata.len()),
            mtime_ms: local_mtime_ms(&metadata),
        })
    } else {
        bail!("unsupported local entry type for {}", local_path.display());
    }
}

fn observe_remote_entry(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
    source_path: &str,
    meta: &Metadata,
) -> Result<ObservedEntry> {
    let kind = metadata_entry_kind(meta)?;
    let version = match kind {
        EntryKind::File => Some(remote_file_version(rt, operator, source_path, meta)?),
        EntryKind::Dir => None,
    };
    Ok(ObservedEntry {
        kind,
        version,
        size: match kind {
            EntryKind::File => Some(meta.content_length()),
            EntryKind::Dir => None,
        },
        mtime_ms: remote_mtime_ms(meta),
    })
}

fn metadata_entry_kind(meta: &Metadata) -> Result<EntryKind> {
    match meta.mode() {
        EntryMode::FILE => Ok(EntryKind::File),
        EntryMode::DIR => Ok(EntryKind::Dir),
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
    local: &ObservedEntry,
) -> Result<Option<String>> {
    match local.kind {
        EntryKind::Dir => {
            ensure_remote_dir(rt, operator, source_path)?;
            Ok(None)
        }
        EntryKind::File => {
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
    remote: &ObservedEntry,
) -> Result<Option<String>> {
    let local_path = local_root.join(source_path);

    match remote.kind {
        EntryKind::Dir => {
            fs::create_dir_all(&local_path)?;
            Ok(None)
        }
        EntryKind::File => {
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
    kind: EntryKind,
) -> Result<()> {
    match kind {
        EntryKind::Dir => {
            rt.block_on(operator.remove_all(&normalized_dir(source_path)))?;
        }
        EntryKind::File => {
            rt.block_on(operator.delete(source_path))?;
        }
    }
    Ok(())
}

fn delete_local_entry(local_root: &Path, source_path: &str, kind: EntryKind) -> Result<()> {
    let local_path = local_root.join(source_path);
    if !local_path.exists() {
        return Ok(());
    }

    match kind {
        EntryKind::Dir => fs::remove_dir_all(local_path)?,
        EntryKind::File => fs::remove_file(local_path)?,
    }
    Ok(())
}

fn infer_local_kind_from_path(path: &Path) -> Result<EntryKind> {
    let metadata = fs::metadata(path)?;
    if metadata.is_dir() {
        Ok(EntryKind::Dir)
    } else if metadata.is_file() {
        Ok(EntryKind::File)
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

fn local_mtime_ms(metadata: &fs::Metadata) -> Option<i64> {
    metadata.modified().ok().and_then(system_time_to_ms)
}

fn remote_mtime_ms(meta: &Metadata) -> Option<i64> {
    meta.last_modified().and_then(|timestamp| {
        let system_time: SystemTime = timestamp.into();
        system_time_to_ms(system_time)
    })
}

fn system_time_to_ms(time: SystemTime) -> Option<i64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as i64)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock drift")
        .as_millis() as i64
}
