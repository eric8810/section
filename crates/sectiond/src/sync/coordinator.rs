use super::{
    absent_conflict_record, conflict_record, event_for, ready_record, EntryKind, ObservedEntry,
    PathSyncInput, PendingSyncEvent,
};
use anyhow::Result;
use section_provider::PathSyncStateRecord;

pub(crate) trait SyncCoordinator {
    fn plan(&self, source_id: &str, inputs: Vec<PathSyncInput>) -> Result<SyncPlan>;
}

#[derive(Debug, Clone)]
pub(crate) enum PlannedOp {
    CreateLocalDir { path: String },
    CreateRemoteDir { path: String },
    PullFile { path: String },
    PushFile { path: String },
    DeleteLocal { path: String, kind: EntryKind },
    DeleteRemote { path: String, kind: EntryKind },
}

#[derive(Debug, Clone)]
pub(crate) enum PlannedRecordSpec {
    Remove,
    Static(PathSyncStateRecord),
    ReadyFromPushedLocal {
        kind: EntryKind,
        local_version: Option<String>,
    },
    ReadyFromPulledRemote {
        kind: EntryKind,
        remote_version: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct PathSyncPlan {
    pub(crate) path: String,
    pub(crate) ops: Vec<PlannedOp>,
    pub(crate) record_spec: PlannedRecordSpec,
    pub(crate) events: Vec<PendingSyncEvent>,
    pub(crate) pulled: usize,
    pub(crate) pushed: usize,
    pub(crate) conflicts: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct SyncPlan {
    pub(crate) paths: Vec<PathSyncPlan>,
}

#[derive(Debug, Default)]
pub(crate) struct DefaultSyncCoordinator;

impl SyncCoordinator for DefaultSyncCoordinator {
    fn plan(&self, source_id: &str, inputs: Vec<PathSyncInput>) -> Result<SyncPlan> {
        let paths = inputs
            .into_iter()
            .map(|input| plan_path(source_id, input))
            .collect::<Result<Vec<_>>>()?;
        Ok(SyncPlan { paths })
    }
}

fn plan_path(source_id: &str, input: PathSyncInput) -> Result<PathSyncPlan> {
    let PathSyncInput {
        path,
        previous,
        local,
        remote,
    } = input;

    if local.is_none() && remote.is_none() {
        return Ok(PathSyncPlan {
            path,
            ops: Vec::new(),
            record_spec: PlannedRecordSpec::Remove,
            events: Vec::new(),
            pulled: 0,
            pushed: 0,
            conflicts: 0,
        });
    }

    if let (Some(local), Some(remote)) = (&local, &remote) {
        if local.kind != remote.kind {
            let record = conflict_record(source_id, &path, local, remote, previous.as_ref());
            return Ok(PathSyncPlan {
                path: path.clone(),
                ops: Vec::new(),
                record_spec: PlannedRecordSpec::Static(record),
                events: vec![event_for(source_id, &path, "conflict_detected", "conflict")],
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
                Ok(PathSyncPlan {
                    path: path.clone(),
                    ops: Vec::new(),
                    record_spec: PlannedRecordSpec::Static(ready_record(
                        source_id,
                        &path,
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
                Ok(PathSyncPlan {
                    path: path.clone(),
                    ops: push_ops(&path, &local),
                    record_spec: push_record_spec(source_id, &path, &local),
                    events: vec![event_for(source_id, &path, "synced_to_remote", "ready")],
                    pulled: 0,
                    pushed: 1,
                    conflicts: 0,
                })
            } else if !local_changed && remote_changed {
                Ok(PathSyncPlan {
                    path: path.clone(),
                    ops: pull_ops(&path, &remote),
                    record_spec: pull_record_spec(source_id, &path, &remote),
                    events: vec![event_for(source_id, &path, "synced_from_remote", "ready")],
                    pulled: 1,
                    pushed: 0,
                    conflicts: 0,
                })
            } else {
                let record = conflict_record(source_id, &path, &local, &remote, previous.as_ref());
                Ok(PathSyncPlan {
                    path: path.clone(),
                    ops: Vec::new(),
                    record_spec: PlannedRecordSpec::Static(record),
                    events: vec![event_for(source_id, &path, "conflict_detected", "conflict")],
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
                        &path,
                        remote.kind,
                        false,
                        remote.version.clone(),
                        previous.as_ref(),
                    );
                    Ok(PathSyncPlan {
                        path: path.clone(),
                        ops: Vec::new(),
                        record_spec: PlannedRecordSpec::Static(record),
                        events: vec![event_for(source_id, &path, "conflict_detected", "conflict")],
                        pulled: 0,
                        pushed: 0,
                        conflicts: 1,
                    })
                } else {
                    Ok(PathSyncPlan {
                        path: path.clone(),
                        ops: vec![PlannedOp::DeleteRemote {
                            path: path.clone(),
                            kind: remote.kind,
                        }],
                        record_spec: PlannedRecordSpec::Remove,
                        events: vec![event_for(source_id, &path, "synced_to_remote", "ready")],
                        pulled: 0,
                        pushed: 1,
                        conflicts: 0,
                    })
                }
            } else {
                Ok(PathSyncPlan {
                    path: path.clone(),
                    ops: pull_ops(&path, &remote),
                    record_spec: pull_record_spec(source_id, &path, &remote),
                    events: vec![event_for(source_id, &path, "synced_from_remote", "ready")],
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
                        &path,
                        local.kind,
                        true,
                        None,
                        previous.as_ref(),
                    );
                    Ok(PathSyncPlan {
                        path: path.clone(),
                        ops: Vec::new(),
                        record_spec: PlannedRecordSpec::Static(record),
                        events: vec![event_for(source_id, &path, "conflict_detected", "conflict")],
                        pulled: 0,
                        pushed: 0,
                        conflicts: 1,
                    })
                } else {
                    Ok(PathSyncPlan {
                        path: path.clone(),
                        ops: vec![PlannedOp::DeleteLocal {
                            path: path.clone(),
                            kind: local.kind,
                        }],
                        record_spec: PlannedRecordSpec::Remove,
                        events: vec![event_for(source_id, &path, "synced_from_remote", "ready")],
                        pulled: 1,
                        pushed: 0,
                        conflicts: 0,
                    })
                }
            } else {
                Ok(PathSyncPlan {
                    path: path.clone(),
                    ops: push_ops(&path, &local),
                    record_spec: push_record_spec(source_id, &path, &local),
                    events: vec![event_for(source_id, &path, "synced_to_remote", "ready")],
                    pulled: 0,
                    pushed: 1,
                    conflicts: 0,
                })
            }
        }
        (None, None) => unreachable!("handled above"),
    }
}

fn pull_ops(path: &str, remote: &ObservedEntry) -> Vec<PlannedOp> {
    match remote.kind {
        EntryKind::Dir => vec![PlannedOp::CreateLocalDir {
            path: path.to_string(),
        }],
        EntryKind::File => vec![PlannedOp::PullFile {
            path: path.to_string(),
        }],
    }
}

fn push_ops(path: &str, local: &ObservedEntry) -> Vec<PlannedOp> {
    match local.kind {
        EntryKind::Dir => vec![PlannedOp::CreateRemoteDir {
            path: path.to_string(),
        }],
        EntryKind::File => vec![PlannedOp::PushFile {
            path: path.to_string(),
        }],
    }
}

fn pull_record_spec(source_id: &str, path: &str, remote: &ObservedEntry) -> PlannedRecordSpec {
    match remote.kind {
        EntryKind::Dir => PlannedRecordSpec::Static(ready_record(
            source_id,
            path,
            EntryKind::Dir,
            None,
            None,
            true,
        )),
        EntryKind::File => PlannedRecordSpec::ReadyFromPulledRemote {
            kind: EntryKind::File,
            remote_version: remote.version.clone(),
        },
    }
}

fn push_record_spec(source_id: &str, path: &str, local: &ObservedEntry) -> PlannedRecordSpec {
    match local.kind {
        EntryKind::Dir => PlannedRecordSpec::Static(ready_record(
            source_id,
            path,
            EntryKind::Dir,
            None,
            None,
            true,
        )),
        EntryKind::File => PlannedRecordSpec::ReadyFromPushedLocal {
            kind: EntryKind::File,
            local_version: local.version.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{DefaultSyncCoordinator, PlannedOp, PlannedRecordSpec, SyncCoordinator};
    use crate::sync::{EntryKind, ObservedEntry, PathSyncInput};
    use section_provider::PathSyncStateRecord;

    fn previous_record(path: &str) -> PathSyncStateRecord {
        PathSyncStateRecord {
            source_name: "local".to_string(),
            path: path.to_string(),
            entry_kind: "file".to_string(),
            public_state: "ready".to_string(),
            local_present: true,
            dirty_local: false,
            dirty_remote: false,
            pinned: false,
            stale: false,
            last_local_version: Some("base-local".to_string()),
            base_remote_version: Some("base-remote".to_string()),
            current_remote_version: Some("base-remote".to_string()),
        }
    }

    fn file(version: &str) -> ObservedEntry {
        ObservedEntry {
            kind: EntryKind::File,
            version: Some(version.to_string()),
            size: Some(version.len() as u64),
            mtime_ms: Some(1),
        }
    }

    fn dir() -> ObservedEntry {
        ObservedEntry {
            kind: EntryKind::Dir,
            version: None,
            size: None,
            mtime_ms: Some(1),
        }
    }

    #[test]
    fn coordinator_marks_dual_file_modification_as_conflict() {
        let coordinator = DefaultSyncCoordinator;
        let plan = coordinator
            .plan(
                "local",
                vec![PathSyncInput {
                    path: "readme.txt".to_string(),
                    previous: Some(previous_record("readme.txt")),
                    local: Some(file("local-v2")),
                    remote: Some(file("remote-v2")),
                }],
            )
            .expect("plan");

        assert_eq!(plan.paths[0].conflicts, 1);
        assert!(matches!(
            plan.paths[0].record_spec,
            PlannedRecordSpec::Static(_)
        ));
    }

    #[test]
    fn coordinator_marks_delete_modify_as_conflict() {
        let coordinator = DefaultSyncCoordinator;
        let plan = coordinator
            .plan(
                "local",
                vec![PathSyncInput {
                    path: "readme.txt".to_string(),
                    previous: Some(previous_record("readme.txt")),
                    local: None,
                    remote: Some(file("remote-v2")),
                }],
            )
            .expect("plan");

        assert_eq!(plan.paths[0].conflicts, 1);
    }

    #[test]
    fn coordinator_marks_type_conflict() {
        let coordinator = DefaultSyncCoordinator;
        let plan = coordinator
            .plan(
                "local",
                vec![PathSyncInput {
                    path: "docs".to_string(),
                    previous: None,
                    local: Some(file("file-version")),
                    remote: Some(dir()),
                }],
            )
            .expect("plan");

        assert_eq!(plan.paths[0].conflicts, 1);
    }

    #[test]
    fn coordinator_pushes_first_local_file_sync() {
        let coordinator = DefaultSyncCoordinator;
        let plan = coordinator
            .plan(
                "local",
                vec![PathSyncInput {
                    path: "notes.txt".to_string(),
                    previous: None,
                    local: Some(file("local-v1")),
                    remote: None,
                }],
            )
            .expect("plan");

        assert_eq!(plan.paths[0].pushed, 1);
        assert!(matches!(plan.paths[0].ops[0], PlannedOp::PushFile { .. }));
    }
}
