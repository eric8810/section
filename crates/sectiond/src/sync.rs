mod collector;
mod coordinator;
mod delta;
mod transport;

use anyhow::{anyhow, bail, Result};
use collector::{DefaultSnapshotCollector, ObservedEntry, PathSyncInput, SnapshotCollector};
use coordinator::{
    DefaultSyncCoordinator, PathSyncPlan, PlannedRecordSpec, SyncCoordinator, SyncPlan,
};
use opendal::{EntryMode, Metadata, Operator};
use ring::digest::{digest, SHA256};
use section_provider::{
    LocalScanCacheRecord, PathSyncStateRecord, ProviderStore, RemoteManifestRecord, SyncEventRecord,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};
use transport::{
    OpenDalTransport, Transport, TransportConfig, TransportOutcome, TransportProgress,
    TransportProgressObserver,
};

use crate::SectiondRuntime;

pub use collector::{LocalScanStats, RemoteScanStats};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SourceSyncResult {
    pub source_id: String,
    pub local_root: PathBuf,
    pub pulled: usize,
    pub pushed: usize,
    pub conflicts: usize,
    pub events_emitted: usize,
    pub local_scan: LocalScanStats,
    pub remote_scan: RemoteScanStats,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceSyncOptions {
    pub path_concurrency: usize,
    pub transfer_concurrency: usize,
    pub http_concurrency: usize,
    pub max_retries: usize,
    pub emit_syncing_events: bool,
}

impl Default for SourceSyncOptions {
    fn default() -> Self {
        Self {
            path_concurrency: 8,
            transfer_concurrency: 8,
            http_concurrency: 8,
            max_retries: 3,
            emit_syncing_events: true,
        }
    }
}

impl SourceSyncOptions {
    fn transport_config(&self) -> TransportConfig {
        TransportConfig {
            concurrency: self.transfer_concurrency.max(1),
            http_concurrency: self.http_concurrency.max(1),
            max_retries: self.max_retries,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncLifecycleStage {
    Queued,
    Running,
    Progress,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncLifecycleEvent {
    pub source_id: String,
    pub path: String,
    pub stage: SyncLifecycleStage,
    pub bytes_complete: Option<u64>,
    pub bytes_total: Option<u64>,
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
pub(crate) enum EntryKind {
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

#[derive(Debug)]
struct MaterializedPathPlan {
    source_id: String,
    path: String,
    record: Option<PathSyncStateRecord>,
    events: Vec<PendingSyncEvent>,
}

#[derive(Debug, Default)]
struct AppliedSyncTotals {
    pulled: usize,
    pushed: usize,
    conflicts: usize,
    events_emitted: usize,
}

pub type SyncLifecycleObserver = Arc<dyn Fn(SyncLifecycleEvent) + Send + Sync + 'static>;

pub fn sync_source(
    runtime: &SectiondRuntime,
    store: &ProviderStore,
    source_id: &str,
    local_root: &Path,
) -> Result<SourceSyncResult> {
    sync_source_with_options(
        runtime,
        store,
        source_id,
        local_root,
        &SourceSyncOptions::default(),
        None,
    )
}

pub fn sync_source_with_options(
    runtime: &SectiondRuntime,
    store: &ProviderStore,
    source_id: &str,
    local_root: &Path,
    options: &SourceSyncOptions,
    lifecycle: Option<SyncLifecycleObserver>,
) -> Result<SourceSyncResult> {
    let operator = runtime.router().get_operator(source_id)?;
    let rt = tokio::runtime::Runtime::new()?;
    fs::create_dir_all(local_root)?;

    let existing = store
        .list_path_sync_states(source_id)?
        .into_iter()
        .map(|record| (record.path.clone(), record))
        .collect::<HashMap<_, _>>();
    let local_cache = store
        .list_local_scan_cache(source_id)?
        .into_iter()
        .map(|record| (record.path.clone(), record))
        .collect::<HashMap<String, LocalScanCacheRecord>>();
    let remote_manifest = store
        .list_remote_manifest(source_id)?
        .into_iter()
        .map(|record| (record.path.clone(), record))
        .collect::<HashMap<String, RemoteManifestRecord>>();

    let inventory_manifest_path = runtime
        .config()
        .sources
        .get(source_id)
        .and_then(|source| source.options.get("section.sync_inventory_manifest"))
        .cloned()
        .filter(|path| !path.trim().is_empty());
    let collector = DefaultSnapshotCollector::new(&rt, inventory_manifest_path);
    let local_snapshot = collector.collect_local(local_root, &local_cache)?;
    let remote_snapshot = collector.collect_remote(operator, &remote_manifest)?;
    let collector::LocalSnapshot {
        entries: local,
        cache_records: refreshed_local_cache,
        stats: local_scan,
    } = local_snapshot;
    let collector::RemoteSnapshot {
        entries: remote,
        manifest_records: refreshed_remote_manifest,
        stats: remote_scan,
    } = remote_snapshot;
    let inputs = collector.build_inputs(existing, local, remote);
    let mut refreshed_local_cache = refreshed_local_cache
        .into_iter()
        .map(|record| (record.path.clone(), record))
        .collect::<HashMap<String, LocalScanCacheRecord>>();
    let mut refreshed_remote_manifest = refreshed_remote_manifest
        .into_iter()
        .map(|record| (record.path.clone(), record))
        .collect::<HashMap<String, RemoteManifestRecord>>();

    let coordinator = DefaultSyncCoordinator;
    let SyncPlan { paths } = coordinator.plan(source_id, inputs)?;

    let transport = OpenDalTransport::new(
        &rt,
        operator.clone(),
        local_root.to_path_buf(),
        options.transport_config(),
    );

    let mut events_emitted = 0;

    for path_plan in &paths {
        if !path_plan.ops.is_empty() {
            if let Some(observer) = &lifecycle {
                observer(SyncLifecycleEvent {
                    source_id: source_id.to_string(),
                    path: path_plan.path.clone(),
                    stage: SyncLifecycleStage::Queued,
                    bytes_complete: None,
                    bytes_total: None,
                });
            }

            if options.emit_syncing_events {
                store.append_sync_event(
                    source_id,
                    &path_plan.path,
                    "state_changed",
                    "syncing",
                    now_ms(),
                )?;
                events_emitted += 1;
            }
        }
    }

    let executed = execute_path_plans(
        source_id,
        transport,
        paths,
        options.path_concurrency.max(1),
        lifecycle,
    )?;
    let totals = apply_executed_path_plans(
        &rt,
        operator,
        store,
        source_id,
        local_root,
        executed,
        &mut refreshed_local_cache,
        &mut refreshed_remote_manifest,
    )?;
    events_emitted += totals.events_emitted;

    Ok(SourceSyncResult {
        source_id: source_id.to_string(),
        local_root: local_root.to_path_buf(),
        pulled: totals.pulled,
        pushed: totals.pushed,
        conflicts: totals.conflicts,
        events_emitted,
        local_scan,
        remote_scan,
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
    let (local_matches_base, local_matches_current_remote) = compare_match_flags(
        &local_version,
        &base_remote_version,
        &current_remote_version,
        stored.as_ref(),
    );

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
        local_matches_base,
        local_matches_current_remote,
        stale,
    })
}

fn execute_path_plans(
    source_id: &str,
    transport: OpenDalTransport,
    paths: Vec<PathSyncPlan>,
    path_concurrency: usize,
    lifecycle: Option<SyncLifecycleObserver>,
) -> Result<Vec<(PathSyncPlan, Result<TransportOutcome>)>> {
    let queue = Arc::new(Mutex::new(
        paths
            .into_iter()
            .enumerate()
            .collect::<VecDeque<(usize, PathSyncPlan)>>(),
    ));
    let results = Arc::new(Mutex::new(Vec::<(
        usize,
        PathSyncPlan,
        Result<TransportOutcome>,
    )>::new()));
    let worker_count = path_concurrency.max(1).min(
        queue
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .len()
            .max(1),
    );

    thread::scope(|scope| -> Result<()> {
        let mut handles = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let queue = Arc::clone(&queue);
            let results = Arc::clone(&results);
            let transport = transport.clone();
            let lifecycle = lifecycle.clone();
            let source_id = source_id.to_string();

            handles.push(scope.spawn(move || loop {
                let next = {
                    let mut queue = queue.lock().unwrap_or_else(|err| err.into_inner());
                    queue.pop_front()
                };
                let Some((index, path_plan)) = next else {
                    break;
                };

                let outcome = if path_plan.ops.is_empty() {
                    Ok(TransportOutcome::default())
                } else {
                    emit_lifecycle_event(
                        lifecycle.as_ref(),
                        &source_id,
                        &path_plan.path,
                        SyncLifecycleStage::Running,
                        None,
                        None,
                    );
                    let progress = lifecycle
                        .as_ref()
                        .map(|observer| progress_observer(observer, &source_id, &path_plan.path));
                    let outcome = transport.execute(&path_plan, progress);
                    emit_lifecycle_event(
                        lifecycle.as_ref(),
                        &source_id,
                        &path_plan.path,
                        if outcome.is_ok() {
                            SyncLifecycleStage::Completed
                        } else {
                            SyncLifecycleStage::Failed
                        },
                        None,
                        None,
                    );
                    outcome
                };

                results
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .push((index, path_plan, outcome));
            }));
        }

        for handle in handles {
            handle
                .join()
                .map_err(|panic| anyhow!("sync worker panicked: {}", panic_message(panic)))?;
        }
        Ok(())
    })?;

    let mut results = {
        let mut guard = results.lock().unwrap_or_else(|err| err.into_inner());
        std::mem::take(&mut *guard)
    };
    results.sort_by_key(|(index, _, _)| *index);
    Ok(results
        .into_iter()
        .map(|(_, path_plan, outcome)| (path_plan, outcome))
        .collect())
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

fn emit_lifecycle_event(
    lifecycle: Option<&SyncLifecycleObserver>,
    source_id: &str,
    path: &str,
    stage: SyncLifecycleStage,
    bytes_complete: Option<u64>,
    bytes_total: Option<u64>,
) {
    if let Some(observer) = lifecycle {
        observer(SyncLifecycleEvent {
            source_id: source_id.to_string(),
            path: path.to_string(),
            stage,
            bytes_complete,
            bytes_total,
        });
    }
}

fn progress_observer(
    observer: &SyncLifecycleObserver,
    source_id: &str,
    path: &str,
) -> TransportProgressObserver {
    let observer = Arc::clone(observer);
    let source_id = source_id.to_string();
    let path = path.to_string();
    Arc::new(move |progress: TransportProgress| {
        observer(SyncLifecycleEvent {
            source_id: source_id.clone(),
            path: path.clone(),
            stage: SyncLifecycleStage::Progress,
            bytes_complete: Some(progress.bytes_complete),
            bytes_total: progress.bytes_total,
        });
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

#[allow(clippy::too_many_arguments)]
fn apply_executed_path_plans(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
    store: &ProviderStore,
    source_id: &str,
    local_root: &Path,
    executed: Vec<(PathSyncPlan, Result<TransportOutcome>)>,
    local_cache: &mut HashMap<String, LocalScanCacheRecord>,
    remote_manifest: &mut HashMap<String, RemoteManifestRecord>,
) -> Result<AppliedSyncTotals> {
    let mut totals = AppliedSyncTotals::default();
    let mut transport_failures = Vec::new();
    let mut successful = Vec::new();

    for (path_plan, outcome) in executed {
        totals.pulled += path_plan.pulled;
        totals.pushed += path_plan.pushed;
        totals.conflicts += path_plan.conflicts;

        match outcome {
            Ok(outcome) => successful.push((path_plan, outcome)),
            Err(err) => {
                store.append_sync_event(
                    source_id,
                    &path_plan.path,
                    "state_changed",
                    "error",
                    now_ms(),
                )?;
                totals.events_emitted += 1;
                transport_failures.push(format!("{}: {err}", path_plan.path));
            }
        }
    }

    if !transport_failures.is_empty() {
        return Err(anyhow!(
            "sync transport failed for {} path(s): {}",
            transport_failures.len(),
            transport_failures.join("; ")
        ));
    }

    let mut materialized = Vec::with_capacity(successful.len());
    for (path_plan, outcome) in successful {
        refresh_cached_snapshots(
            rt,
            operator,
            local_root,
            &path_plan,
            &outcome,
            local_cache,
            remote_manifest,
        )?;
        materialized.push(materialize_path_plan(source_id, &path_plan, outcome)?);
    }

    for update in materialized {
        persist_materialized_path_plan(store, update, &mut totals.events_emitted)?;
    }
    replace_scan_snapshots(store, source_id, local_cache, remote_manifest)?;

    Ok(totals)
}

fn materialize_path_plan(
    source_id: &str,
    path_plan: &PathSyncPlan,
    outcome: TransportOutcome,
) -> Result<MaterializedPathPlan> {
    Ok(MaterializedPathPlan {
        source_id: source_id.to_string(),
        path: path_plan.path.clone(),
        record: materialize_record(source_id, &path_plan.path, &path_plan.record_spec, outcome)?,
        events: path_plan.events.clone(),
    })
}

fn persist_materialized_path_plan(
    store: &ProviderStore,
    update: MaterializedPathPlan,
    events_emitted: &mut usize,
) -> Result<()> {
    match update.record {
        Some(record) => store.upsert_path_sync_state(&record)?,
        None => store.remove_path_sync_state(&update.source_id, &update.path)?,
    }

    for event in &update.events {
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

fn replace_scan_snapshots(
    store: &ProviderStore,
    source_id: &str,
    local_cache: &HashMap<String, LocalScanCacheRecord>,
    remote_manifest: &HashMap<String, RemoteManifestRecord>,
) -> Result<()> {
    let mut local_cache = local_cache.values().cloned().collect::<Vec<_>>();
    local_cache.sort_by(|left, right| left.path.cmp(&right.path));
    store.replace_local_scan_cache(source_id, &local_cache)?;

    let mut remote_manifest = remote_manifest.values().cloned().collect::<Vec<_>>();
    remote_manifest.sort_by(|left, right| left.path.cmp(&right.path));
    store.replace_remote_manifest(source_id, &remote_manifest)?;
    Ok(())
}

fn refresh_cached_snapshots(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
    local_root: &Path,
    path_plan: &PathSyncPlan,
    outcome: &TransportOutcome,
    local_cache: &mut HashMap<String, LocalScanCacheRecord>,
    remote_manifest: &mut HashMap<String, RemoteManifestRecord>,
) -> Result<()> {
    for op in &path_plan.ops {
        match op {
            coordinator::PlannedOp::CreateLocalDir { path } => {
                upsert_local_cache_record(local_root, path, None, local_cache)?;
            }
            coordinator::PlannedOp::CreateRemoteDir { path } => {
                upsert_remote_manifest_dir_record(rt, operator, path, remote_manifest)?;
            }
            coordinator::PlannedOp::PullFile { path } => {
                upsert_local_cache_record(
                    local_root,
                    path,
                    outcome.local_version.clone(),
                    local_cache,
                )?;
            }
            coordinator::PlannedOp::PushFile { path } => {
                let planned_version = match &path_plan.record_spec {
                    PlannedRecordSpec::ReadyFromPushedLocal { local_version, .. } => {
                        local_version.clone()
                    }
                    _ => None,
                };
                upsert_remote_manifest_file_record(
                    rt,
                    operator,
                    path,
                    outcome.remote_version.clone().or(planned_version),
                    remote_manifest,
                )?;
            }
            coordinator::PlannedOp::DeleteLocal { path, .. } => {
                local_cache.remove(path);
            }
            coordinator::PlannedOp::DeleteRemote { path, .. } => {
                remote_manifest.remove(path);
            }
        }
    }

    Ok(())
}

fn upsert_local_cache_record(
    local_root: &Path,
    source_path: &str,
    version: Option<String>,
    cache: &mut HashMap<String, LocalScanCacheRecord>,
) -> Result<()> {
    let local_path = local_root.join(source_path);
    let metadata = match fs::metadata(&local_path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            cache.remove(source_path);
            return Ok(());
        }
        Err(err) => return Err(err.into()),
    };

    if metadata.is_dir() {
        cache.insert(
            source_path.to_string(),
            LocalScanCacheRecord {
                path: source_path.to_string(),
                entry_kind: "dir".to_string(),
                version: None,
                size: None,
                mtime_ms: local_mtime_ms(&metadata),
            },
        );
        return Ok(());
    }

    if !metadata.is_file() {
        bail!("unsupported local entry type for {}", local_path.display());
    }

    let version = match version {
        Some(version) => Some(version),
        None => Some(hash_bytes(&fs::read(&local_path)?)),
    };

    cache.insert(
        source_path.to_string(),
        LocalScanCacheRecord {
            path: source_path.to_string(),
            entry_kind: "file".to_string(),
            version,
            size: Some(metadata.len()),
            mtime_ms: local_mtime_ms(&metadata),
        },
    );
    Ok(())
}

fn upsert_remote_manifest_dir_record(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
    source_path: &str,
    manifest: &mut HashMap<String, RemoteManifestRecord>,
) -> Result<()> {
    let meta = stat_remote_metadata(rt, operator, source_path, EntryKind::Dir)?;
    manifest.insert(
        source_path.to_string(),
        RemoteManifestRecord {
            path: source_path.to_string(),
            entry_kind: "dir".to_string(),
            version: None,
            size: None,
            mtime_ms: meta.as_ref().and_then(remote_mtime_ms),
        },
    );
    Ok(())
}

fn upsert_remote_manifest_file_record(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
    source_path: &str,
    version: Option<String>,
    manifest: &mut HashMap<String, RemoteManifestRecord>,
) -> Result<()> {
    let meta = stat_remote_metadata(rt, operator, source_path, EntryKind::File)?;
    let version = meta.as_ref().and_then(remote_file_token).or(version);
    let size = meta.as_ref().and_then(|meta| {
        let content_length = meta.content_length();
        if content_length > 0 || version.is_some() {
            Some(content_length)
        } else {
            None
        }
    });
    let mtime_ms = meta.as_ref().and_then(remote_mtime_ms);

    manifest.insert(
        source_path.to_string(),
        RemoteManifestRecord {
            path: source_path.to_string(),
            entry_kind: "file".to_string(),
            version,
            size,
            mtime_ms,
        },
    );
    Ok(())
}

fn stat_remote_metadata(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
    source_path: &str,
    kind: EntryKind,
) -> Result<Option<Metadata>> {
    let candidates = match kind {
        EntryKind::Dir => vec![normalized_dir(source_path), source_path.to_string()],
        EntryKind::File => vec![source_path.to_string()],
    };

    for candidate in candidates {
        if candidate.is_empty() {
            continue;
        }

        match rt.block_on(operator.stat(&candidate)) {
            Ok(meta) => return Ok(Some(meta)),
            Err(err) if err.kind() == opendal::ErrorKind::NotFound => continue,
            Err(err) => return Err(err.into()),
        }
    }

    Ok(None)
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
        PlannedRecordSpec::ReadyFromPushedLocal {
            kind,
            local_version,
        } => Ok(Some(ready_record(
            source_id,
            source_path,
            *kind,
            local_version.clone(),
            outcome.remote_version.or_else(|| local_version.clone()),
            true,
        ))),
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

#[allow(clippy::too_many_arguments)]
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

pub(crate) fn metadata_entry_kind(meta: &Metadata) -> Result<EntryKind> {
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
    if let Some(version) = remote_file_token(meta) {
        return Ok(version);
    }

    let data = rt.block_on(operator.read(source_path))?;
    Ok(hash_bytes(data.to_bytes().as_ref()))
}

pub(crate) fn remote_file_token(meta: &Metadata) -> Option<String> {
    meta.version()
        .filter(|version| !version.is_empty())
        .map(|version| version.to_string())
        .or_else(|| {
            meta.etag()
                .filter(|etag| !etag.is_empty())
                .map(|etag| etag.to_string())
        })
}

fn compare_match_flags(
    local_version: &Option<String>,
    base_remote_version: &Option<String>,
    current_remote_version: &Option<String>,
    stored: Option<&PathSyncStateRecord>,
) -> (bool, bool) {
    match stored {
        Some(record) => {
            let local_matches_base =
                local_version.is_some() && *local_version == record.last_local_version;
            let local_matches_current_remote =
                local_matches_base && *current_remote_version == record.current_remote_version;
            (local_matches_base, local_matches_current_remote)
        }
        None => {
            let local_matches_base =
                local_version.is_some() && *local_version == *base_remote_version;
            let local_matches_current_remote =
                local_version.is_some() && *local_version == *current_remote_version;
            (local_matches_base, local_matches_current_remote)
        }
    }
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
            let meta = rt.block_on(operator.stat(source_path))?;
            Ok(remote_file_token(&meta).or_else(|| local.version.clone()))
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

pub(crate) fn normalize_source_path(path: &str) -> String {
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

pub(crate) fn hash_bytes(bytes: &[u8]) -> String {
    let digest = digest(&SHA256, bytes);
    digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) fn local_mtime_ms(metadata: &fs::Metadata) -> Option<i64> {
    metadata.modified().ok().and_then(system_time_to_ms)
}

pub(crate) fn remote_mtime_ms(meta: &Metadata) -> Option<i64> {
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

#[cfg(test)]
mod tests {
    use super::{
        apply_executed_path_plans, compare_match_flags, event_for, ready_record, EntryKind,
    };
    use crate::sync::coordinator::{PathSyncPlan, PlannedOp, PlannedRecordSpec};
    use crate::sync::transport::TransportOutcome;
    use anyhow::anyhow;
    use opendal::services;
    use opendal::Operator;
    use section_provider::{PathSyncStateRecord, ProviderStore};
    use std::collections::HashMap;

    fn fs_operator(root: &std::path::Path) -> Operator {
        let builder = services::Fs::default().root(root.to_str().expect("utf8 path"));
        Operator::new(builder).expect("operator").finish()
    }

    fn ready_record_with_versions() -> PathSyncStateRecord {
        PathSyncStateRecord {
            source_name: "local".to_string(),
            path: "notes.txt".to_string(),
            entry_kind: "file".to_string(),
            public_state: "ready".to_string(),
            local_present: true,
            dirty_local: false,
            dirty_remote: false,
            pinned: false,
            stale: false,
            last_local_version: Some("sha256:local".to_string()),
            base_remote_version: Some("\"etag-v1\"".to_string()),
            current_remote_version: Some("\"etag-v1\"".to_string()),
        }
    }

    #[test]
    fn compare_flags_use_stored_sync_state_for_remote_token_backends() {
        let stored = ready_record_with_versions();
        let (local_matches_base, local_matches_current_remote) = compare_match_flags(
            &Some("sha256:local".to_string()),
            &stored.base_remote_version,
            &Some("\"etag-v1\"".to_string()),
            Some(&stored),
        );

        assert!(local_matches_base);
        assert!(local_matches_current_remote);
    }

    #[test]
    fn apply_executed_path_plans_skips_success_persistence_after_transport_error() {
        let data_dir = tempfile::tempdir().expect("data dir");
        let local_root = tempfile::tempdir().expect("local root");
        let remote_root = tempfile::tempdir().expect("remote root");
        let store = ProviderStore::open(data_dir.path()).expect("open store");
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let operator = fs_operator(remote_root.path());

        let successful_plan = PathSyncPlan {
            path: "a.txt".to_string(),
            ops: Vec::new(),
            record_spec: PlannedRecordSpec::Static(ready_record(
                "local",
                "a.txt",
                EntryKind::File,
                Some("local-a".to_string()),
                Some("remote-a".to_string()),
                true,
            )),
            events: vec![event_for("local", "a.txt", "synced_to_remote", "ready")],
            pulled: 0,
            pushed: 1,
            conflicts: 0,
        };
        let failed_plan = PathSyncPlan {
            path: "b.txt".to_string(),
            ops: vec![PlannedOp::PushFile {
                path: "b.txt".to_string(),
            }],
            record_spec: PlannedRecordSpec::Remove,
            events: vec![event_for("local", "b.txt", "synced_to_remote", "ready")],
            pulled: 0,
            pushed: 1,
            conflicts: 0,
        };

        let err = apply_executed_path_plans(
            &rt,
            &operator,
            &store,
            "local",
            local_root.path(),
            vec![
                (successful_plan, Ok(TransportOutcome::default())),
                (failed_plan, Err(anyhow!("boom"))),
            ],
            &mut HashMap::new(),
            &mut HashMap::new(),
        )
        .expect_err("transport error should abort commit");

        assert!(err.to_string().contains("b.txt: boom"));
        assert!(store
            .list_path_sync_states("local")
            .expect("list states")
            .is_empty());
        assert!(store
            .list_local_scan_cache("local")
            .expect("list local cache")
            .is_empty());
        assert!(store
            .list_remote_manifest("local")
            .expect("list remote manifest")
            .is_empty());

        let events = store
            .list_sync_events_after("local", 0)
            .expect("list sync events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].path, "b.txt");
        assert_eq!(events[0].kind, "state_changed");
        assert_eq!(events[0].state, "error");
    }
}
