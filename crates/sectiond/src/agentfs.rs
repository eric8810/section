use anyhow::{Context, Result};
use opendal::Operator;
use ring::rand::{SecureRandom, SystemRandom};
use section_provider::AgentIdentityRecord;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::json;
use std::error::Error;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

pub const SCHEMA_VERSION: u32 = 1;
pub const METADATA_ROOT: &str = ".section/agentfs";
pub const FS_PATH: &str = ".section/agentfs/fs.json";
pub const HEAD_PATH: &str = ".section/agentfs/heads/current.json";
pub const HEAD_LOCK_PATH: &str = ".section/agentfs/locks/head.json";
const HEAD_LOCK_TTL_MS: i64 = 30_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentFsRecord {
    pub schema_version: u32,
    pub fs_id: String,
    pub name: String,
    pub owner_agent_id: String,
    pub source_profile_id: String,
    pub source_name: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentFsSourceProfileRecord {
    pub schema_version: u32,
    pub source_profile_id: String,
    pub name: String,
    pub provider: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentFsCapability {
    Read,
    Commit,
    Manage,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentFsRole {
    Owner,
    Reader,
    Writer,
    Manager,
}

impl AgentFsRole {
    pub fn capabilities(self) -> Vec<AgentFsCapability> {
        match self {
            Self::Owner => vec![
                AgentFsCapability::Read,
                AgentFsCapability::Commit,
                AgentFsCapability::Manage,
            ],
            Self::Reader => vec![AgentFsCapability::Read],
            Self::Writer => vec![AgentFsCapability::Read, AgentFsCapability::Commit],
            Self::Manager => vec![AgentFsCapability::Read, AgentFsCapability::Manage],
        }
    }

    pub fn has_capability(self, capability: AgentFsCapability) -> bool {
        self.capabilities().contains(&capability)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentFsGrantRecord {
    pub schema_version: u32,
    pub grant_id: String,
    pub fs_id: String,
    pub agent_id: String,
    pub role: AgentFsRole,
    pub capabilities: Vec<AgentFsCapability>,
    pub granted_by: String,
    pub created_at_ms: i64,
    pub revoked_at_ms: Option<i64>,
    pub revoked_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentFsShareRecord {
    pub schema_version: u32,
    pub share_id: String,
    pub fs_id: String,
    pub target_agent_id: String,
    pub grant_id: String,
    pub role: AgentFsRole,
    pub source_profile_id: String,
    pub created_by: String,
    pub created_at_ms: i64,
    pub expires_at_ms: Option<i64>,
    pub accepted_at_ms: Option<i64>,
    pub revoked_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentFsCredentialBindingRecord {
    pub schema_version: u32,
    pub credential_binding_id: String,
    pub fs_id: String,
    pub agent_id: String,
    pub installation_id: String,
    pub source_profile_id: String,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
}

impl AgentFsGrantRecord {
    pub fn is_active(&self) -> bool {
        self.revoked_at_ms.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentFsHeadRecord {
    pub schema_version: u32,
    pub fs_id: String,
    pub commit_id: Option<String>,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentFsHeadLockRecord {
    pub schema_version: u32,
    pub fs_id: String,
    pub lock_token: String,
    pub owner_agent_id: String,
    pub created_at_ms: i64,
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentFsCommitPathRecord {
    pub path: String,
    pub kind: String,
    pub op: String,
    pub local_version: Option<String>,
    pub previous_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentFsCommitStagingRecord {
    pub manifest_path: String,
    pub manifest_hash: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentFsMaterializationState {
    Pending,
    Materialized,
    FailedToMaterialize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentFsAuthorization {
    Owner {
        agent_id: String,
    },
    Grant {
        grant_id: String,
        role: AgentFsRole,
        capabilities: Vec<AgentFsCapability>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentFsCommitRecord {
    pub schema_version: u32,
    pub commit_id: String,
    pub fs_id: String,
    pub parent_commit_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_commit_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_manifest_hash: Option<String>,
    pub agent_id: String,
    pub summary: String,
    pub paths: Vec<AgentFsCommitPathRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorized_by: Option<AgentFsAuthorization>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub staging_snapshot: Option<AgentFsCommitStagingRecord>,
    pub created_at_ms: i64,
    pub materialization_state: AgentFsMaterializationState,
    pub materialized_at_ms: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentFsEventRecord {
    pub schema_version: u32,
    pub event_id: String,
    #[serde(default)]
    pub seq: i64,
    pub fs_id: String,
    pub kind: String,
    pub actor_agent_id: String,
    pub subject_id: String,
    pub path: Option<String>,
    pub created_at_ms: i64,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentFsErrorPayload {
    pub code: &'static str,
    pub message: String,
    pub retryable: bool,
    pub details: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct AgentFsError {
    payload: AgentFsErrorPayload,
}

impl AgentFsError {
    pub fn new(code: &'static str, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            payload: AgentFsErrorPayload {
                code,
                message: message.into(),
                retryable,
                details: json!({}),
            },
        }
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.payload.details = details;
        self
    }

    pub fn unknown_agent() -> Self {
        Self::new(
            "unknown_agent",
            "agent identity is missing; run `section agent login <name>` first",
            false,
        )
    }

    pub fn unknown_fs(reference: &str) -> Self {
        Self::new(
            "unknown_fs",
            format!("AgentFS {reference} was not found in registered sources"),
            false,
        )
        .with_details(json!({ "reference": reference }))
    }

    pub fn grant_denied(message: impl Into<String>) -> Self {
        Self::new("grant_denied", message, false)
    }

    pub fn malformed_metadata(message: impl Into<String>) -> Self {
        Self::new("malformed_shared_metadata", message, false)
    }

    pub fn stale_base(fs_id: &str, base: Option<&str>, head: Option<&str>) -> Self {
        Self::new(
            "stale_base",
            format!(
                "local base {} differs from current head {} for fs {fs_id}",
                base.unwrap_or("<empty>"),
                head.unwrap_or("<empty>")
            ),
            true,
        )
        .with_details(json!({
            "fs_id": fs_id,
            "base_commit_id": base,
            "head_commit_id": head,
        }))
    }

    pub fn materialization_failed(fs_id: &str, message: impl Into<String>) -> Self {
        let message = message.into();
        Self::new(
            "materialization_failed",
            format!("fs {fs_id} is not ready: {message}"),
            true,
        )
        .with_details(json!({ "fs_id": fs_id }))
    }

    pub fn metadata_write_conflict(fs_id: &str) -> Self {
        Self::new(
            "metadata_write_conflict",
            format!("metadata head lock is held for fs {fs_id}"),
            true,
        )
        .with_details(json!({ "fs_id": fs_id }))
    }

    pub fn missing_commit_snapshot(commit_id: &str) -> Self {
        Self::new(
            "missing_commit_snapshot",
            format!("staging snapshot for commit {commit_id} is missing"),
            false,
        )
        .with_details(json!({ "commit_id": commit_id }))
    }

    pub fn path_type_conflict(path: &str, local_kind: &str, remote_kind: &str) -> Self {
        Self::new(
            "path_type_conflict",
            format!(
                "path {path} changed type from remote {remote_kind} to local {local_kind}; replace it with separate delete and create commits"
            ),
            false,
        )
        .with_details(json!({
            "path": path,
            "local_kind": local_kind,
            "remote_kind": remote_kind,
        }))
    }

    pub fn payload(&self) -> &AgentFsErrorPayload {
        &self.payload
    }
}

impl fmt::Display for AgentFsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.payload.code, self.payload.message)
    }
}

impl Error for AgentFsError {}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock drift")
        .as_millis() as i64
}

pub fn new_fs_id() -> Result<String> {
    Ok(format!("fs_{}", random_hex(16)?))
}

pub fn new_agent_id() -> Result<String> {
    Ok(format!("agt_{}", random_hex(16)?))
}

pub fn new_installation_id() -> Result<String> {
    Ok(format!("ins_{}", random_hex(16)?))
}

pub fn new_source_profile_id() -> Result<String> {
    Ok(format!("srcp_{}", random_hex(16)?))
}

pub fn new_auth_token() -> Result<String> {
    Ok(format!("auth_{}", random_hex(32)?))
}

pub fn new_grant_id() -> Result<String> {
    Ok(format!("grt_{}", random_hex(16)?))
}

pub fn new_share_id() -> Result<String> {
    Ok(format!("shr_{}", random_hex(16)?))
}

pub fn new_credential_binding_id() -> Result<String> {
    Ok(format!("cred_{}", random_hex(16)?))
}

pub fn new_commit_id() -> Result<String> {
    Ok(format!("cmt_{}", random_hex(16)?))
}

pub fn new_event_id(created_at_ms: i64) -> Result<String> {
    Ok(format!("evt_{created_at_ms:013}_{}", random_hex(8)?))
}

pub fn new_lock_token() -> Result<String> {
    Ok(format!("lck_{}", random_hex(16)?))
}

pub fn is_reserved_metadata_path(path: &str) -> bool {
    let path = path.trim_matches('/');
    path == METADATA_ROOT || path.starts_with(&format!("{METADATA_ROOT}/"))
}

pub fn validate_source_relative_path(path: &str) -> Result<()> {
    if path.starts_with('/') {
        anyhow::bail!(AgentFsError::new(
            "reserved_metadata_path",
            format!("path {path} must be source-root-relative"),
            false,
        ));
    }
    if is_reserved_metadata_path(path) {
        anyhow::bail!(AgentFsError::new(
            "reserved_metadata_path",
            format!("path {path} is reserved AgentFS metadata"),
            false,
        ));
    }
    for segment in path.split('/') {
        if segment == ".." || (segment.is_empty() && !path.is_empty()) {
            anyhow::bail!(AgentFsError::new(
                "reserved_metadata_path",
                format!("path {path} is not a valid source-root-relative path"),
                false,
            ));
        }
    }
    Ok(())
}

pub fn validate_agent_id(agent_id: &str) -> Result<()> {
    let valid = agent_id.strip_prefix("agt_").is_some_and(|hex| {
        hex.len() == 32
            && hex
                .chars()
                .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase())
    });
    if !valid {
        anyhow::bail!(AgentFsError::new(
            "unknown_agent",
            format!("agent id {agent_id} must match agt_[0-9a-f]{{32}}"),
            false,
        ));
    }
    Ok(())
}

pub fn owner_grant(fs: &AgentFsRecord, owner: &AgentIdentityRecord) -> Result<AgentFsGrantRecord> {
    grant_record(fs, &owner.agent_id, AgentFsRole::Owner, &owner.agent_id)
}

pub fn grant_record(
    fs: &AgentFsRecord,
    agent_id: &str,
    role: AgentFsRole,
    granted_by: &str,
) -> Result<AgentFsGrantRecord> {
    Ok(AgentFsGrantRecord {
        schema_version: SCHEMA_VERSION,
        grant_id: new_grant_id()?,
        fs_id: fs.fs_id.clone(),
        agent_id: agent_id.to_string(),
        role,
        capabilities: role.capabilities(),
        granted_by: granted_by.to_string(),
        created_at_ms: now_ms(),
        revoked_at_ms: None,
        revoked_by: None,
    })
}

pub fn head_record(fs_id: &str, commit_id: Option<String>) -> AgentFsHeadRecord {
    AgentFsHeadRecord {
        schema_version: SCHEMA_VERSION,
        fs_id: fs_id.to_string(),
        commit_id,
        updated_at_ms: now_ms(),
    }
}

pub fn event_record(
    fs_id: &str,
    kind: &str,
    actor_agent_id: &str,
    subject_id: &str,
    path: Option<String>,
    data: serde_json::Value,
) -> Result<AgentFsEventRecord> {
    let created_at_ms = now_ms();
    Ok(AgentFsEventRecord {
        schema_version: SCHEMA_VERSION,
        event_id: new_event_id(created_at_ms)?,
        seq: 0,
        fs_id: fs_id.to_string(),
        kind: kind.to_string(),
        actor_agent_id: actor_agent_id.to_string(),
        subject_id: subject_id.to_string(),
        path,
        created_at_ms,
        data,
    })
}

pub fn has_capability(
    fs: &AgentFsRecord,
    grants: &[AgentFsGrantRecord],
    agent_id: &str,
    capability: AgentFsCapability,
) -> bool {
    if fs.owner_agent_id == agent_id && AgentFsRole::Owner.has_capability(capability) {
        return true;
    }

    grants.iter().any(|grant| {
        grant.fs_id == fs.fs_id
            && grant.agent_id == agent_id
            && grant.is_active()
            && grant.capabilities.contains(&capability)
    })
}

pub fn ensure_capability(
    fs: &AgentFsRecord,
    grants: &[AgentFsGrantRecord],
    agent_id: &str,
    capability: AgentFsCapability,
) -> Result<()> {
    if has_capability(fs, grants, agent_id, capability) {
        return Ok(());
    }

    anyhow::bail!(AgentFsError::grant_denied(format!(
        "agent {agent_id} does not have {capability:?} access to fs {}",
        fs.fs_id
    )));
}

pub fn write_json<T: Serialize>(
    rt: &tokio::runtime::Runtime,
    op: &Operator,
    path: &str,
    value: &T,
) -> Result<()> {
    ensure_remote_parent_dirs(rt, op, path)?;
    let bytes = serde_json::to_vec_pretty(value)?;
    rt.block_on(op.write(path, bytes))
        .with_context(|| format!("failed to write AgentFS metadata {path}"))?;
    Ok(())
}

pub fn read_json<T: DeserializeOwned>(
    rt: &tokio::runtime::Runtime,
    op: &Operator,
    path: &str,
) -> Result<T> {
    let data = rt
        .block_on(op.read(path))
        .with_context(|| format!("failed to read AgentFS metadata {path}"))?;
    serde_json::from_slice(data.to_bytes().as_ref()).map_err(|err| {
        AgentFsError::malformed_metadata(format!("failed to parse {path}: {err}")).into()
    })
}

pub fn read_optional_json<T: DeserializeOwned>(
    rt: &tokio::runtime::Runtime,
    op: &Operator,
    path: &str,
) -> Result<Option<T>> {
    match rt.block_on(op.read(path)) {
        Ok(data) => serde_json::from_slice(data.to_bytes().as_ref())
            .map(Some)
            .map_err(|err| {
                AgentFsError::malformed_metadata(format!("failed to parse {path}: {err}")).into()
            }),
        Err(err) if err.kind() == opendal::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| format!("failed to read AgentFS metadata {path}")),
    }
}

pub fn write_grant(
    rt: &tokio::runtime::Runtime,
    op: &Operator,
    grant: &AgentFsGrantRecord,
) -> Result<()> {
    write_json(
        rt,
        op,
        &format!("{METADATA_ROOT}/grants/{}.json", grant.grant_id),
        grant,
    )
}

pub fn write_event(
    rt: &tokio::runtime::Runtime,
    op: &Operator,
    event: &AgentFsEventRecord,
) -> Result<()> {
    let mut event = event.clone();
    if event.seq <= 0 {
        event.seq = next_event_seq(rt, op)?;
    }
    let path = format!("{METADATA_ROOT}/events/{}.json", event.event_id);
    match rt.block_on(op.stat(&path)) {
        Ok(_) => anyhow::bail!(AgentFsError::metadata_write_conflict(&event.fs_id)),
        Err(err) if err.kind() == opendal::ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }
    write_json(rt, op, &path, &event)
}

pub fn write_commit(
    rt: &tokio::runtime::Runtime,
    op: &Operator,
    commit: &AgentFsCommitRecord,
) -> Result<()> {
    write_json(
        rt,
        op,
        &format!("{METADATA_ROOT}/commits/{}.json", commit.commit_id),
        commit,
    )
}

pub fn acquire_head_lock(
    rt: &tokio::runtime::Runtime,
    op: &Operator,
    fs_id: &str,
    owner_agent_id: &str,
) -> Result<AgentFsHeadLockRecord> {
    let now = now_ms();
    if let Some(existing) = read_optional_json::<AgentFsHeadLockRecord>(rt, op, HEAD_LOCK_PATH)? {
        if existing.fs_id == fs_id && existing.expires_at_ms > now {
            anyhow::bail!(AgentFsError::metadata_write_conflict(fs_id));
        }
    }

    let lock = AgentFsHeadLockRecord {
        schema_version: SCHEMA_VERSION,
        fs_id: fs_id.to_string(),
        lock_token: new_lock_token()?,
        owner_agent_id: owner_agent_id.to_string(),
        created_at_ms: now,
        expires_at_ms: now + HEAD_LOCK_TTL_MS,
    };
    write_json(rt, op, HEAD_LOCK_PATH, &lock)?;
    Ok(lock)
}

pub fn release_head_lock(
    rt: &tokio::runtime::Runtime,
    op: &Operator,
    lock: &AgentFsHeadLockRecord,
) -> Result<()> {
    let existing = read_optional_json::<AgentFsHeadLockRecord>(rt, op, HEAD_LOCK_PATH)?;
    if existing
        .as_ref()
        .is_some_and(|existing| existing.lock_token == lock.lock_token)
    {
        match rt.block_on(op.delete(HEAD_LOCK_PATH)) {
            Ok(()) => {}
            Err(err) if err.kind() == opendal::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
    }
    Ok(())
}

pub fn list_grants(rt: &tokio::runtime::Runtime, op: &Operator) -> Result<Vec<AgentFsGrantRecord>> {
    list_json_records(rt, op, &format!("{METADATA_ROOT}/grants/"))
}

pub fn list_events(rt: &tokio::runtime::Runtime, op: &Operator) -> Result<Vec<AgentFsEventRecord>> {
    let mut events: Vec<AgentFsEventRecord> =
        list_json_records(rt, op, &format!("{METADATA_ROOT}/events/"))?;
    events.sort_by(|left, right| {
        left.seq
            .cmp(&right.seq)
            .then_with(|| left.event_id.cmp(&right.event_id))
    });
    Ok(events)
}

fn next_event_seq(rt: &tokio::runtime::Runtime, op: &Operator) -> Result<i64> {
    let max_seq = list_events(rt, op)?
        .into_iter()
        .map(|event| event.seq)
        .max()
        .unwrap_or(0);
    Ok(max_seq + 1)
}

pub fn initialize_fs_metadata(
    rt: &tokio::runtime::Runtime,
    op: &Operator,
    fs: &AgentFsRecord,
    owner: &AgentIdentityRecord,
) -> Result<AgentFsGrantRecord> {
    write_json(rt, op, FS_PATH, fs)?;
    write_json(rt, op, HEAD_PATH, &head_record(&fs.fs_id, None))?;

    let grant = owner_grant(fs, owner)?;
    write_grant(rt, op, &grant)?;

    let event = event_record(
        &fs.fs_id,
        "fs.created",
        &owner.agent_id,
        &fs.fs_id,
        None,
        json!({
            "name": fs.name,
            "source_name": fs.source_name,
            "source_profile_id": fs.source_profile_id,
        }),
    )?;
    write_event(rt, op, &event)?;

    Ok(grant)
}

fn list_json_records<T: DeserializeOwned>(
    rt: &tokio::runtime::Runtime,
    op: &Operator,
    prefix: &str,
) -> Result<Vec<T>> {
    let entries = match rt.block_on(op.list(prefix)) {
        Ok(entries) => entries,
        Err(err) if err.kind() == opendal::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err.into()),
    };

    let mut records = Vec::new();
    for entry in entries {
        let path = entry.path();
        if !path.ends_with(".json") {
            continue;
        }
        records.push(read_json(rt, op, path)?);
    }
    Ok(records)
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

fn normalized_dir(path: &str) -> String {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}/")
    }
}

fn random_hex(byte_len: usize) -> Result<String> {
    let rng = SystemRandom::new();
    let mut bytes = vec![0_u8; byte_len];
    rng.fill(&mut bytes)
        .map_err(|_| anyhow::anyhow!("failed to generate random bytes"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_match_contract_prefixes_and_lengths() {
        assert_eq!(new_fs_id().expect("fs id").len(), 35);
        assert!(new_fs_id().expect("fs id").starts_with("fs_"));
        assert_eq!(new_commit_id().expect("commit id").len(), 36);
        assert!(new_commit_id().expect("commit id").starts_with("cmt_"));
        assert_eq!(new_grant_id().expect("grant id").len(), 36);
        assert!(new_grant_id().expect("grant id").starts_with("grt_"));

        let event_id = new_event_id(1780000000000).expect("event id");
        assert_eq!(event_id.len(), 34);
        assert!(event_id.starts_with("evt_1780000000000_"));
    }

    #[test]
    fn roles_map_to_contract_capabilities() {
        assert!(AgentFsRole::Owner.has_capability(AgentFsCapability::Read));
        assert!(AgentFsRole::Owner.has_capability(AgentFsCapability::Commit));
        assert!(AgentFsRole::Owner.has_capability(AgentFsCapability::Manage));
        assert!(AgentFsRole::Reader.has_capability(AgentFsCapability::Read));
        assert!(!AgentFsRole::Reader.has_capability(AgentFsCapability::Commit));
        assert!(AgentFsRole::Writer.has_capability(AgentFsCapability::Commit));
        assert!(!AgentFsRole::Writer.has_capability(AgentFsCapability::Manage));
        assert!(AgentFsRole::Manager.has_capability(AgentFsCapability::Manage));
        assert!(!AgentFsRole::Manager.has_capability(AgentFsCapability::Commit));
    }

    #[test]
    fn path_validation_rejects_reserved_metadata() {
        validate_source_relative_path("").expect("root");
        validate_source_relative_path("docs/readme.md").expect("normal path");
        assert!(validate_source_relative_path("/docs/readme.md").is_err());
        assert!(validate_source_relative_path("docs//readme.md").is_err());
        assert!(validate_source_relative_path("docs/../secret").is_err());
        assert!(validate_source_relative_path(".section/agentfs/fs.json").is_err());
    }
}
