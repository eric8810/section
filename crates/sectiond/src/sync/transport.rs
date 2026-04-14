use super::{delete_local_entry, normalized_dir, remote_file_token};
use crate::sync::coordinator::{PathSyncPlan, PlannedOp};
use anyhow::Result;
use bytes::Bytes;
use futures::TryStreamExt;
use opendal::layers::{ConcurrentLimitLayer, RetryLayer, TracingLayer};
use opendal::Operator;
use ring::digest::{Context, SHA256};
use std::fs;
use std::io::{BufReader, Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::runtime::Handle;
use tracing::{debug, info, warn};

pub(crate) trait Transport {
    fn execute(
        &self,
        plan: &PathSyncPlan,
        progress: Option<TransportProgressObserver>,
    ) -> Result<TransportOutcome>;
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

#[derive(Debug, Clone)]
pub(crate) struct TransportProgress {
    pub(crate) bytes_complete: u64,
    pub(crate) bytes_total: Option<u64>,
}

pub(crate) type TransportProgressObserver = Arc<dyn Fn(TransportProgress) + Send + Sync + 'static>;

#[derive(Clone)]
pub(crate) struct OpenDalTransport {
    handle: Handle,
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

impl OpenDalTransport {
    pub(crate) fn new(
        rt: &tokio::runtime::Runtime,
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
            // Path plans are now executed concurrently, so keep transport as the
            // single mount point for OpenDAL concurrency and retry policy.
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
            handle: rt.handle().clone(),
            operator,
            local_root,
            config,
        }
    }
}

impl Transport for OpenDalTransport {
    fn execute(
        &self,
        plan: &PathSyncPlan,
        progress: Option<TransportProgressObserver>,
    ) -> Result<TransportOutcome> {
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
                    ensure_remote_dir(&self.handle, &self.operator, path)?;
                }
                PlannedOp::PullFile { path } => {
                    let local_path = self.local_root.join(path);
                    if let Some(parent) = local_path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    let bytes_total = self
                        .handle
                        .block_on(self.operator.stat(path))
                        .ok()
                        .map(|meta| meta.content_length());
                    let reader = self.handle.block_on(self.operator.reader(path))?;
                    let mut stream = self.handle.block_on(reader.into_bytes_stream(..))?;
                    let mut file = fs::File::create(&local_path)?;
                    let mut hasher = Context::new(&SHA256);
                    let mut bytes_complete = 0_u64;
                    let mut last_emitted = 0_u64;

                    while let Some(chunk) = self.handle.block_on(stream.try_next())? {
                        file.write_all(chunk.as_ref())?;
                        hasher.update(chunk.as_ref());
                        bytes_complete += chunk.len() as u64;
                        emit_progress(
                            progress.as_ref(),
                            &mut last_emitted,
                            bytes_complete,
                            bytes_total,
                            false,
                        );
                    }

                    emit_progress(
                        progress.as_ref(),
                        &mut last_emitted,
                        bytes_complete,
                        bytes_total,
                        true,
                    );
                    outcome.local_version = Some(digest_to_hex(hasher.finish().as_ref()));
                }
                PlannedOp::PushFile { path } => {
                    let local_path = self.local_root.join(path);
                    let metadata = fs::metadata(&local_path)?;
                    let bytes_total = Some(metadata.len());
                    let mut reader = BufReader::new(fs::File::open(&local_path)?);
                    ensure_remote_parent_dirs(&self.handle, &self.operator, path)?;
                    let mut writer = self.handle.block_on(self.operator.writer(path))?;
                    let mut bytes_complete = 0_u64;
                    let mut last_emitted = 0_u64;
                    let mut buffer = vec![0_u8; 1024 * 1024];

                    loop {
                        let read = reader.read(&mut buffer)?;
                        if read == 0 {
                            break;
                        }

                        self.handle
                            .block_on(writer.write(Bytes::copy_from_slice(&buffer[..read])))?;
                        bytes_complete += read as u64;
                        emit_progress(
                            progress.as_ref(),
                            &mut last_emitted,
                            bytes_complete,
                            bytes_total,
                            false,
                        );
                    }

                    let meta = self.handle.block_on(writer.close())?;
                    emit_progress(
                        progress.as_ref(),
                        &mut last_emitted,
                        bytes_complete,
                        bytes_total,
                        true,
                    );
                    let mut remote_version = remote_file_token(&meta);
                    if remote_version.is_none() {
                        let stat_meta = self.handle.block_on(self.operator.stat(path))?;
                        remote_version = remote_file_token(&stat_meta);
                    }
                    outcome.remote_version = remote_version;
                }
                PlannedOp::DeleteLocal { path, kind } => {
                    delete_local_entry(&self.local_root, path, *kind)?;
                }
                PlannedOp::DeleteRemote { path, kind } => {
                    delete_remote_entry(&self.handle, &self.operator, path, *kind)?;
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

fn emit_progress(
    observer: Option<&TransportProgressObserver>,
    last_emitted: &mut u64,
    bytes_complete: u64,
    bytes_total: Option<u64>,
    force: bool,
) {
    if !force && bytes_complete.saturating_sub(*last_emitted) < 1024 * 1024 {
        return;
    }
    *last_emitted = bytes_complete;

    if let Some(observer) = observer {
        observer(TransportProgress {
            bytes_complete,
            bytes_total,
        });
    }
}

fn digest_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn ensure_remote_parent_dirs(
    handle: &Handle,
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
            handle.block_on(operator.create_dir(&normalized_dir(&current)))?;
        }
    }
    Ok(())
}

fn ensure_remote_dir(handle: &Handle, operator: &Operator, source_path: &str) -> Result<()> {
    ensure_remote_parent_dirs(handle, operator, source_path)?;
    handle.block_on(operator.create_dir(&normalized_dir(source_path)))?;
    Ok(())
}

fn delete_remote_entry(
    handle: &Handle,
    operator: &Operator,
    source_path: &str,
    kind: crate::sync::EntryKind,
) -> Result<()> {
    match kind {
        crate::sync::EntryKind::Dir => {
            handle.block_on(operator.remove_all(&normalized_dir(source_path)))?;
        }
        crate::sync::EntryKind::File => {
            handle.block_on(operator.delete(source_path))?;
        }
    }
    Ok(())
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

        transport.execute(&plan, None).expect("execute");
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

        let outcome = transport.execute(&plan, None).expect("execute");
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

        transport.execute(&plan, None).expect("execute");
        assert!(!local.path().join("notes.txt").exists());
    }
}
