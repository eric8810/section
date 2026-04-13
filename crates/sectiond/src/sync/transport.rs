use super::{
    delete_local_entry, delete_remote_entry, ensure_remote_dir, ensure_remote_parent_dirs,
    hash_bytes, remote_file_token,
};
use crate::sync::coordinator::{PathSyncPlan, PlannedOp};
use anyhow::Result;
use opendal::layers::{ConcurrentLimitLayer, RetryLayer, TracingLayer};
use opendal::Operator;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;
use tracing::{debug, info, warn};

pub(crate) trait Transport {
    fn execute(&self, plan: &PathSyncPlan) -> Result<TransportOutcome>;
}

#[derive(Debug, Clone)]
pub(crate) struct TransportConfig {
    pub(crate) concurrency: usize,
    pub(crate) http_concurrency: usize,
    pub(crate) max_retries: usize,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            concurrency: 8,
            http_concurrency: 8,
            max_retries: 3,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct TransportOutcome {
    pub(crate) local_version: Option<String>,
    pub(crate) remote_version: Option<String>,
}

pub(crate) struct OpenDalTransport<'a> {
    rt: &'a tokio::runtime::Runtime,
    operator: Operator,
    local_root: PathBuf,
    config: TransportConfig,
}

fn log_retry(err: &opendal::Error, dur: std::time::Duration) {
    warn!(
        error = %err,
        backoff_ms = dur.as_millis() as u64,
        "retrying sync transport operation"
    );
}

impl<'a> OpenDalTransport<'a> {
    pub(crate) fn new(
        rt: &'a tokio::runtime::Runtime,
        operator: Operator,
        local_root: PathBuf,
        config: TransportConfig,
    ) -> Self {
        let operator = operator
            .layer(TracingLayer)
            .layer(
                RetryLayer::new()
                    .with_max_times(config.max_retries)
                    .with_notify(log_retry),
            )
            // sync_source still executes one PathSyncPlan at a time.
            // The layer is wired here now so transport remains the only layer
            // mount point; Issue #002 can make path execution parallel later.
            .layer(
                ConcurrentLimitLayer::new(config.concurrency)
                    .with_http_concurrent_limit(config.http_concurrency),
            );

        info!(
            local_root = %local_root.display(),
            concurrency = config.concurrency,
            http_concurrency = config.http_concurrency,
            max_retries = config.max_retries,
            "configured sync transport"
        );

        Self {
            rt,
            operator,
            local_root,
            config,
        }
    }
}

impl Transport for OpenDalTransport<'_> {
    fn execute(&self, plan: &PathSyncPlan) -> Result<TransportOutcome> {
        let started = Instant::now();
        debug!(
            path = %plan.path,
            ops = plan.ops.len(),
            concurrency = self.config.concurrency,
            "executing sync transport plan"
        );

        let mut outcome = TransportOutcome::default();

        for op in &plan.ops {
            match op {
                PlannedOp::CreateLocalDir { path } => {
                    fs::create_dir_all(self.local_root.join(path))?;
                }
                PlannedOp::CreateRemoteDir { path } => {
                    ensure_remote_dir(self.rt, &self.operator, path)?;
                }
                PlannedOp::PullFile { path } => {
                    let local_path = self.local_root.join(path);
                    if let Some(parent) = local_path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    let data = self.rt.block_on(self.operator.read(path))?;
                    fs::write(&local_path, data.to_bytes().as_ref())?;
                    outcome.local_version = Some(hash_bytes(data.to_bytes().as_ref()));
                }
                PlannedOp::PushFile { path } => {
                    let local_path = self.local_root.join(path);
                    let data = fs::read(&local_path)?;
                    ensure_remote_parent_dirs(self.rt, &self.operator, path)?;
                    self.rt.block_on(self.operator.write(path, data))?;
                    let meta = self.rt.block_on(self.operator.stat(path))?;
                    outcome.remote_version = remote_file_token(&meta);
                }
                PlannedOp::DeleteLocal { path, kind } => {
                    delete_local_entry(&self.local_root, path, *kind)?;
                }
                PlannedOp::DeleteRemote { path, kind } => {
                    delete_remote_entry(self.rt, &self.operator, path, *kind)?;
                }
            }
        }

        info!(
            path = %plan.path,
            ops = plan.ops.len(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "sync transport plan completed"
        );

        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::{OpenDalTransport, Transport, TransportConfig};
    use crate::sync::coordinator::{PathSyncPlan, PlannedOp, PlannedRecordSpec};
    use crate::sync::EntryKind;
    use opendal::services;
    use opendal::Operator;
    use std::fs;

    fn fs_operator(root: &std::path::Path) -> Operator {
        let builder = services::Fs::default().root(root.to_str().expect("utf8 path"));
        Operator::new(builder).expect("operator").finish()
    }

    #[test]
    fn transport_pushes_local_file_to_remote_fs() {
        let local = tempfile::tempdir().expect("local tempdir");
        let remote = tempfile::tempdir().expect("remote tempdir");
        fs::write(local.path().join("notes.txt"), "hello").expect("write local");
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let transport = OpenDalTransport::new(
            &rt,
            fs_operator(remote.path()),
            local.path().to_path_buf(),
            TransportConfig::default(),
        );

        let plan = PathSyncPlan {
            path: "notes.txt".to_string(),
            ops: vec![PlannedOp::PushFile {
                path: "notes.txt".to_string(),
            }],
            record_spec: PlannedRecordSpec::Remove,
            events: Vec::new(),
            pulled: 0,
            pushed: 1,
            conflicts: 0,
        };

        transport.execute(&plan).expect("execute");
        assert_eq!(
            fs::read_to_string(remote.path().join("notes.txt")).expect("read remote"),
            "hello"
        );
    }

    #[test]
    fn transport_pulls_remote_file_to_local_fs() {
        let local = tempfile::tempdir().expect("local tempdir");
        let remote = tempfile::tempdir().expect("remote tempdir");
        fs::write(remote.path().join("notes.txt"), "hello").expect("write remote");
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let transport = OpenDalTransport::new(
            &rt,
            fs_operator(remote.path()),
            local.path().to_path_buf(),
            TransportConfig::default(),
        );

        let plan = PathSyncPlan {
            path: "notes.txt".to_string(),
            ops: vec![PlannedOp::PullFile {
                path: "notes.txt".to_string(),
            }],
            record_spec: PlannedRecordSpec::Remove,
            events: Vec::new(),
            pulled: 1,
            pushed: 0,
            conflicts: 0,
        };

        let outcome = transport.execute(&plan).expect("execute");
        assert_eq!(
            fs::read_to_string(local.path().join("notes.txt")).expect("read local"),
            "hello"
        );
        assert!(outcome.local_version.is_some());
    }

    #[test]
    fn transport_deletes_local_file() {
        let local = tempfile::tempdir().expect("local tempdir");
        let remote = tempfile::tempdir().expect("remote tempdir");
        fs::write(local.path().join("notes.txt"), "hello").expect("write local");
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let transport = OpenDalTransport::new(
            &rt,
            fs_operator(remote.path()),
            local.path().to_path_buf(),
            TransportConfig::default(),
        );

        let plan = PathSyncPlan {
            path: "notes.txt".to_string(),
            ops: vec![PlannedOp::DeleteLocal {
                path: "notes.txt".to_string(),
                kind: EntryKind::File,
            }],
            record_spec: PlannedRecordSpec::Remove,
            events: Vec::new(),
            pulled: 1,
            pushed: 0,
            conflicts: 0,
        };

        transport.execute(&plan).expect("execute");
        assert!(!local.path().join("notes.txt").exists());
    }
}
