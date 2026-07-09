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
pub const HEAD_LOCK_DIR: &str = ".section/agentfs/locks/head/";
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
    Propose,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentFsRole {
    Owner,
    Reader,
    Writer,
    Manager,
    Contributor,
}

impl AgentFsRole {
    pub fn capabilities(self) -> Vec<AgentFsCapability> {
        match self {
            Self::Owner => vec![
                AgentFsCapability::Read,
                AgentFsCapability::Commit,
                AgentFsCapability::Manage,
                AgentFsCapability::Propose,
            ],
            Self::Reader => vec![AgentFsCapability::Read],
            Self::Writer => vec![AgentFsCapability::Read, AgentFsCapability::Commit],
            Self::Manager => vec![AgentFsCapability::Read, AgentFsCapability::Manage],
            Self::Contributor => vec![AgentFsCapability::Read, AgentFsCapability::Propose],
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_scopes: Option<Vec<String>>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentFsHookRecord {
    pub schema_version: u32,
    pub hook_id: String,
    pub fs_id: String,
    pub name: String,
    pub event: String,
    pub command: Vec<String>,
    pub created_by_agent_id: String,
    pub created_at_ms: i64,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path_scopes: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        matched_path_scopes: Option<Vec<String>>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentFsProposalRecord {
    pub schema_version: u32,
    pub proposal_id: String,
    pub fs_id: String,
    pub commit_id: String,
    pub base_commit_id: Option<String>,
    pub agent_id: String,
    pub summary: String,
    pub paths: Vec<AgentFsCommitPathRecord>,
    pub authorized_by: Option<AgentFsAuthorization>,
    pub staging_snapshot: AgentFsCommitStagingRecord,
    pub status: String,
    pub created_at_ms: i64,
    pub decided_at_ms: Option<i64>,
    pub decided_by_agent_id: Option<String>,
    pub error: Option<String>,
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
    pub code: String,
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
                code: code.to_string(),
                message: message.into(),
                retryable,
                details: json!({}),
            },
        }
    }

    pub fn from_payload(payload: AgentFsErrorPayload) -> Self {
        Self { payload }
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

    pub fn ambiguous_fs_ref(reference: &str, matched_field: &str, matches: &[String]) -> Self {
        Self::new(
            "ambiguous_fs_ref",
            format!(
                "AgentFS reference {reference} matched multiple filesystems by {matched_field}"
            ),
            false,
        )
        .with_details(json!({
            "reference": reference,
            "matched_field": matched_field,
            "matches": matches,
        }))
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
            format!("metadata write conflict for fs {fs_id}"),
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

    pub fn non_utf8_path(path: &std::path::Path) -> Self {
        Self::new(
            "non_utf8_path",
            format!(
                "AgentFS commit only supports UTF-8 source-relative paths: {}",
                path.display()
            ),
            false,
        )
        .with_details(json!({
            "local_path": path.display().to_string(),
        }))
    }

    pub fn agent_rules_invalid(message: impl Into<String>) -> Self {
        Self::new("agent_rules_invalid", message, false)
    }

    pub fn path_scope_denied(fs_id: &str, paths: &[String], scopes: &[String]) -> Self {
        Self::new(
            "path_scope_denied",
            format!("agent grant does not allow committing one or more paths on fs {fs_id}"),
            false,
        )
        .with_details(json!({
            "fs_id": fs_id,
            "paths": paths,
            "path_scopes": scopes,
        }))
    }

    pub fn remote_drift(
        path: &str,
        expected_kind: Option<&str>,
        expected_version: Option<&str>,
        actual_kind: Option<&str>,
        actual_version: Option<&str>,
    ) -> Self {
        Self::new(
            "remote_drift",
            format!(
                "backing source path {path} changed outside AgentFS; sync or repair before committing"
            ),
            true,
        )
        .with_details(json!({
            "path": path,
            "expected_kind": expected_kind,
            "expected_version": expected_version,
            "actual_kind": actual_kind,
            "actual_version": actual_version,
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

pub fn new_proposal_id() -> Result<String> {
    Ok(format!("prop_{}", random_hex(16)?))
}

pub fn new_hook_id() -> Result<String> {
    Ok(format!("hook_{}", random_hex(16)?))
}

pub fn new_hook_run_id() -> Result<String> {
    Ok(format!("hookrun_{}", random_hex(16)?))
}

pub fn new_event_id(created_at_ms: i64) -> Result<String> {
    Ok(format!("evt_{created_at_ms:013}_{}", random_hex(8)?))
}

pub fn new_lock_token() -> Result<String> {
    Ok(format!("lck_{}", random_hex(16)?))
}

pub fn is_reserved_metadata_path(path: &str) -> bool {
    let path = path.trim_matches('/');
    path == ".section" || path.starts_with(".section/")
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
            format!("path {path} is reserved Section metadata"),
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

pub fn validate_path_scope(scope: &str) -> Result<()> {
    let scope = scope.trim();
    if scope.is_empty() {
        anyhow::bail!(AgentFsError::new(
            "invalid_path_scope",
            "path scope must not be empty",
            false,
        ));
    }
    if scope.starts_with('/') {
        anyhow::bail!(AgentFsError::new(
            "invalid_path_scope",
            format!("path scope {scope} must be FS-root-relative"),
            false,
        ));
    }
    if is_reserved_metadata_path(scope) {
        anyhow::bail!(AgentFsError::new(
            "invalid_path_scope",
            format!("path scope {scope} must not target Section metadata"),
            false,
        ));
    }
    for segment in scope.split('/') {
        if segment.is_empty() || segment == ".." {
            anyhow::bail!(AgentFsError::new(
                "invalid_path_scope",
                format!("path scope {scope} is not valid"),
                false,
            ));
        }
    }
    Ok(())
}

pub fn path_matches_any_scope(path: &str, scopes: &[String]) -> bool {
    scopes.iter().any(|scope| path_matches_scope(path, scope))
}

pub fn path_matches_scope(path: &str, scope: &str) -> bool {
    let path_segments = path.split('/').collect::<Vec<_>>();
    let scope_segments = scope.split('/').collect::<Vec<_>>();
    path_segments_match(&path_segments, &scope_segments)
}

fn path_segments_match(path: &[&str], scope: &[&str]) -> bool {
    match scope.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => {
            path_segments_match(path, rest)
                || (!path.is_empty() && path_segments_match(&path[1..], scope))
        }
        Some((segment_scope, rest)) => {
            if let Some((path_segment, path_rest)) = path.split_first() {
                segment_matches(path_segment, segment_scope) && path_segments_match(path_rest, rest)
            } else {
                false
            }
        }
    }
}

fn segment_matches(value: &str, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if !pattern.contains('*') {
        return value == pattern;
    }

    let mut remainder = value;
    let mut first = true;
    for part in pattern.split('*') {
        if part.is_empty() {
            continue;
        }
        if first && !pattern.starts_with('*') {
            let Some(stripped) = remainder.strip_prefix(part) else {
                return false;
            };
            remainder = stripped;
        } else {
            let Some(index) = remainder.find(part) else {
                return false;
            };
            remainder = &remainder[index + part.len()..];
        }
        first = false;
    }
    pattern.ends_with('*') || remainder.is_empty()
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

pub trait AgentFsMetadataRecord {
    fn validate_metadata(&self, path: &str) -> Result<()>;
}

impl AgentFsMetadataRecord for AgentFsRecord {
    fn validate_metadata(&self, path: &str) -> Result<()> {
        validate_schema(path, self.schema_version)?;
        validate_id_field(path, "fs_id", &self.fs_id, "fs_", 32)?;
        validate_non_empty(path, "name", &self.name)?;
        validate_id_field(path, "owner_agent_id", &self.owner_agent_id, "agt_", 32)?;
        validate_id_field(
            path,
            "source_profile_id",
            &self.source_profile_id,
            "srcp_",
            32,
        )?;
        validate_non_empty(path, "source_name", &self.source_name)?;
        validate_timestamp(path, "created_at_ms", self.created_at_ms)
    }
}

impl AgentFsMetadataRecord for AgentFsSourceProfileRecord {
    fn validate_metadata(&self, path: &str) -> Result<()> {
        validate_schema(path, self.schema_version)?;
        validate_id_field(
            path,
            "source_profile_id",
            &self.source_profile_id,
            "srcp_",
            32,
        )?;
        validate_non_empty(path, "name", &self.name)?;
        validate_non_empty(path, "provider", &self.provider)?;
        validate_timestamp(path, "created_at_ms", self.created_at_ms)?;
        validate_timestamp(path, "updated_at_ms", self.updated_at_ms)
    }
}

impl AgentFsMetadataRecord for AgentFsGrantRecord {
    fn validate_metadata(&self, path: &str) -> Result<()> {
        validate_schema(path, self.schema_version)?;
        validate_id_field(path, "grant_id", &self.grant_id, "grt_", 32)?;
        validate_id_field(path, "fs_id", &self.fs_id, "fs_", 32)?;
        validate_id_field(path, "agent_id", &self.agent_id, "agt_", 32)?;
        validate_id_field(path, "granted_by", &self.granted_by, "agt_", 32)?;
        validate_timestamp(path, "created_at_ms", self.created_at_ms)?;
        if self.capabilities != self.role.capabilities() {
            anyhow::bail!(malformed_record(path, "grant capabilities must match role"));
        }
        if let Some(revoked_at_ms) = self.revoked_at_ms {
            validate_timestamp(path, "revoked_at_ms", revoked_at_ms)?;
        }
        if let Some(revoked_by) = &self.revoked_by {
            validate_id_field(path, "revoked_by", revoked_by, "agt_", 32)?;
        }
        Ok(())
    }
}

impl AgentFsMetadataRecord for AgentFsShareRecord {
    fn validate_metadata(&self, path: &str) -> Result<()> {
        validate_schema(path, self.schema_version)?;
        validate_id_field(path, "share_id", &self.share_id, "shr_", 32)?;
        validate_id_field(path, "fs_id", &self.fs_id, "fs_", 32)?;
        validate_id_field(path, "target_agent_id", &self.target_agent_id, "agt_", 32)?;
        validate_id_field(path, "grant_id", &self.grant_id, "grt_", 32)?;
        validate_id_field(
            path,
            "source_profile_id",
            &self.source_profile_id,
            "srcp_",
            32,
        )?;
        validate_id_field(path, "created_by", &self.created_by, "agt_", 32)?;
        validate_timestamp(path, "created_at_ms", self.created_at_ms)?;
        if let Some(expires_at_ms) = self.expires_at_ms {
            validate_timestamp(path, "expires_at_ms", expires_at_ms)?;
        }
        if let Some(accepted_at_ms) = self.accepted_at_ms {
            validate_timestamp(path, "accepted_at_ms", accepted_at_ms)?;
        }
        if let Some(revoked_at_ms) = self.revoked_at_ms {
            validate_timestamp(path, "revoked_at_ms", revoked_at_ms)?;
        }
        Ok(())
    }
}

impl AgentFsMetadataRecord for AgentFsCredentialBindingRecord {
    fn validate_metadata(&self, path: &str) -> Result<()> {
        validate_schema(path, self.schema_version)?;
        validate_id_field(
            path,
            "credential_binding_id",
            &self.credential_binding_id,
            "cred_",
            32,
        )?;
        validate_id_field(path, "fs_id", &self.fs_id, "fs_", 32)?;
        validate_id_field(path, "agent_id", &self.agent_id, "agt_", 32)?;
        validate_id_field(path, "installation_id", &self.installation_id, "ins_", 32)?;
        validate_id_field(
            path,
            "source_profile_id",
            &self.source_profile_id,
            "srcp_",
            32,
        )?;
        validate_timestamp(path, "issued_at_ms", self.issued_at_ms)?;
        validate_timestamp(path, "expires_at_ms", self.expires_at_ms)
    }
}

impl AgentFsMetadataRecord for AgentFsHookRecord {
    fn validate_metadata(&self, path: &str) -> Result<()> {
        validate_schema(path, self.schema_version)?;
        validate_id_field(path, "hook_id", &self.hook_id, "hook_", 32)?;
        validate_id_field(path, "fs_id", &self.fs_id, "fs_", 32)?;
        validate_non_empty(path, "name", &self.name)?;
        if self.event != "commit.materialized" {
            anyhow::bail!(malformed_record(
                path,
                "hook event must be commit.materialized"
            ));
        }
        if self.command.is_empty() {
            anyhow::bail!(malformed_record(path, "hook command must not be empty"));
        }
        for item in &self.command {
            validate_non_empty(path, "command", item)?;
        }
        validate_id_field(
            path,
            "created_by_agent_id",
            &self.created_by_agent_id,
            "agt_",
            32,
        )?;
        validate_timestamp(path, "created_at_ms", self.created_at_ms)
    }
}

impl AgentFsMetadataRecord for AgentFsProposalRecord {
    fn validate_metadata(&self, path: &str) -> Result<()> {
        validate_schema(path, self.schema_version)?;
        validate_id_field(path, "proposal_id", &self.proposal_id, "prop_", 32)?;
        validate_id_field(path, "fs_id", &self.fs_id, "fs_", 32)?;
        validate_id_field(path, "commit_id", &self.commit_id, "cmt_", 32)?;
        if let Some(base_commit_id) = &self.base_commit_id {
            validate_id_field(path, "base_commit_id", base_commit_id, "cmt_", 32)?;
        }
        validate_id_field(path, "agent_id", &self.agent_id, "agt_", 32)?;
        validate_non_empty(path, "summary", self.summary.trim())?;
        if self.paths.is_empty() {
            anyhow::bail!(malformed_record(path, "proposal paths must not be empty"));
        }
        for commit_path in &self.paths {
            commit_path.validate_metadata(path)?;
        }
        if !matches!(self.status.as_str(), "proposed" | "accepted" | "rejected") {
            anyhow::bail!(malformed_record(path, "proposal status is invalid"));
        }
        validate_timestamp(path, "created_at_ms", self.created_at_ms)?;
        if let Some(decided_at_ms) = self.decided_at_ms {
            validate_timestamp(path, "decided_at_ms", decided_at_ms)?;
        }
        if let Some(decided_by_agent_id) = &self.decided_by_agent_id {
            validate_id_field(path, "decided_by_agent_id", decided_by_agent_id, "agt_", 32)?;
        }
        Ok(())
    }
}

impl AgentFsMetadataRecord for AgentFsHeadRecord {
    fn validate_metadata(&self, path: &str) -> Result<()> {
        validate_schema(path, self.schema_version)?;
        validate_id_field(path, "fs_id", &self.fs_id, "fs_", 32)?;
        if let Some(commit_id) = &self.commit_id {
            validate_id_field(path, "commit_id", commit_id, "cmt_", 32)?;
        }
        validate_timestamp(path, "updated_at_ms", self.updated_at_ms)
    }
}

impl AgentFsMetadataRecord for AgentFsHeadLockRecord {
    fn validate_metadata(&self, path: &str) -> Result<()> {
        validate_schema(path, self.schema_version)?;
        validate_id_field(path, "fs_id", &self.fs_id, "fs_", 32)?;
        validate_id_field(path, "lock_token", &self.lock_token, "lck_", 32)?;
        validate_id_field(path, "owner_agent_id", &self.owner_agent_id, "agt_", 32)?;
        validate_timestamp(path, "created_at_ms", self.created_at_ms)?;
        validate_timestamp(path, "expires_at_ms", self.expires_at_ms)
    }
}

impl AgentFsMetadataRecord for AgentFsCommitRecord {
    fn validate_metadata(&self, path: &str) -> Result<()> {
        validate_schema(path, self.schema_version)?;
        validate_id_field(path, "commit_id", &self.commit_id, "cmt_", 32)?;
        validate_id_field(path, "fs_id", &self.fs_id, "fs_", 32)?;
        if let Some(parent_commit_id) = &self.parent_commit_id {
            validate_id_field(path, "parent_commit_id", parent_commit_id, "cmt_", 32)?;
        }
        if let Some(base_commit_id) = &self.base_commit_id {
            validate_id_field(path, "base_commit_id", base_commit_id, "cmt_", 32)?;
        }
        validate_id_field(path, "agent_id", &self.agent_id, "agt_", 32)?;
        validate_non_empty(path, "summary", self.summary.trim())?;
        if self.paths.is_empty() {
            anyhow::bail!(malformed_record(path, "commit paths must not be empty"));
        }
        for commit_path in &self.paths {
            commit_path.validate_metadata(path)?;
        }
        let authorized_by = self
            .authorized_by
            .as_ref()
            .ok_or_else(|| malformed_record(path, "commit must record authorized_by"))?;
        validate_authorization(path, authorized_by)?;
        let staging = self
            .staging_snapshot
            .as_ref()
            .ok_or_else(|| malformed_record(path, "commit must record staging_snapshot"))?;
        validate_non_empty(
            path,
            "staging_snapshot.manifest_path",
            &staging.manifest_path,
        )?;
        validate_non_empty(
            path,
            "staging_snapshot.manifest_hash",
            &staging.manifest_hash,
        )?;
        validate_timestamp(path, "created_at_ms", self.created_at_ms)?;
        match self.materialization_state {
            AgentFsMaterializationState::Materialized => {
                if self.materialized_at_ms.is_none() {
                    anyhow::bail!(malformed_record(
                        path,
                        "materialized commit must record materialized_at_ms"
                    ));
                }
            }
            AgentFsMaterializationState::FailedToMaterialize => {
                if self.error.as_deref().unwrap_or_default().trim().is_empty() {
                    anyhow::bail!(malformed_record(path, "failed commit must record error"));
                }
            }
            AgentFsMaterializationState::Pending => {}
        }
        Ok(())
    }
}

impl AgentFsMetadataRecord for AgentFsCommitPathRecord {
    fn validate_metadata(&self, path: &str) -> Result<()> {
        validate_agentfs_path_field(path, "paths.path", &self.path)?;
        match self.kind.as_str() {
            "file" | "dir" => {}
            _ => anyhow::bail!(malformed_record(
                path,
                "commit path kind must be file or dir"
            )),
        }
        match self.op.as_str() {
            "create" | "update" | "delete" => {}
            _ => anyhow::bail!(malformed_record(
                path,
                "commit path op must be create, update, or delete"
            )),
        }
        Ok(())
    }
}

impl AgentFsMetadataRecord for AgentFsEventRecord {
    fn validate_metadata(&self, path: &str) -> Result<()> {
        validate_schema(path, self.schema_version)?;
        validate_event_id(path, &self.event_id)?;
        if self.seq <= 0 {
            anyhow::bail!(malformed_record(path, "event seq must be positive"));
        }
        validate_id_field(path, "fs_id", &self.fs_id, "fs_", 32)?;
        validate_non_empty(path, "kind", &self.kind)?;
        validate_id_field(path, "actor_agent_id", &self.actor_agent_id, "agt_", 32)?;
        validate_non_empty(path, "subject_id", &self.subject_id)?;
        if let Some(event_path) = &self.path {
            validate_agentfs_path_field(path, "path", event_path)?;
        }
        validate_timestamp(path, "created_at_ms", self.created_at_ms)
    }
}

fn validate_schema(path: &str, schema_version: u32) -> Result<()> {
    if schema_version != SCHEMA_VERSION {
        anyhow::bail!(malformed_record(
            path,
            format!("schema_version {schema_version} is not {SCHEMA_VERSION}")
        ));
    }
    Ok(())
}

fn validate_id_field(
    path: &str,
    field: &str,
    value: &str,
    prefix: &str,
    hex_len: usize,
) -> Result<()> {
    let valid = value.strip_prefix(prefix).is_some_and(|hex| {
        hex.len() == hex_len
            && hex
                .chars()
                .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase())
    });
    if !valid {
        anyhow::bail!(malformed_record(
            path,
            format!("{field} must match {prefix}[0-9a-f]{{{hex_len}}}")
        ));
    }
    Ok(())
}

fn validate_event_id(path: &str, event_id: &str) -> Result<()> {
    let Some(rest) = event_id.strip_prefix("evt_") else {
        anyhow::bail!(malformed_record(path, "event_id must start with evt_"));
    };
    let Some((millis, random)) = rest.split_once('_') else {
        anyhow::bail!(malformed_record(
            path,
            "event_id must include timestamp and random suffix"
        ));
    };
    let valid = millis.len() == 13
        && millis.chars().all(|ch| ch.is_ascii_digit())
        && random.len() == 16
        && random
            .chars()
            .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase());
    if !valid {
        anyhow::bail!(malformed_record(
            path,
            "event_id must match evt_<13 digit ms>_<16 lowercase hex>"
        ));
    }
    Ok(())
}

fn validate_non_empty(path: &str, field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        anyhow::bail!(malformed_record(path, format!("{field} must not be empty")));
    }
    Ok(())
}

fn validate_timestamp(path: &str, field: &str, value: i64) -> Result<()> {
    if value <= 0 {
        anyhow::bail!(malformed_record(path, format!("{field} must be positive")));
    }
    Ok(())
}

fn validate_agentfs_path_field(path: &str, field: &str, value: &str) -> Result<()> {
    validate_source_relative_path(value).map_err(|err| {
        malformed_record(path, format!("{field} is not a valid AgentFS path: {err}")).into()
    })
}

fn validate_authorization(path: &str, authorization: &AgentFsAuthorization) -> Result<()> {
    match authorization {
        AgentFsAuthorization::Owner { agent_id } => {
            validate_id_field(path, "authorized_by.agent_id", agent_id, "agt_", 32)
        }
        AgentFsAuthorization::Grant {
            grant_id,
            role,
            capabilities,
            ..
        } => {
            validate_id_field(path, "authorized_by.grant_id", grant_id, "grt_", 32)?;
            if !capabilities.contains(&AgentFsCapability::Commit) {
                anyhow::bail!(malformed_record(
                    path,
                    "authorized grant must include commit capability"
                ));
            }
            if !role.has_capability(AgentFsCapability::Commit) {
                anyhow::bail!(malformed_record(
                    path,
                    "authorized grant role must include commit capability"
                ));
            }
            Ok(())
        }
    }
}

fn malformed_record(path: &str, message: impl Into<String>) -> AgentFsError {
    AgentFsError::malformed_metadata(format!(
        "invalid AgentFS metadata {path}: {}",
        message.into()
    ))
}

pub fn owner_grant(fs: &AgentFsRecord, owner: &AgentIdentityRecord) -> Result<AgentFsGrantRecord> {
    grant_record(
        fs,
        &owner.agent_id,
        AgentFsRole::Owner,
        &owner.agent_id,
        None,
    )
}

pub fn grant_record(
    fs: &AgentFsRecord,
    agent_id: &str,
    role: AgentFsRole,
    granted_by: &str,
    path_scopes: Option<Vec<String>>,
) -> Result<AgentFsGrantRecord> {
    if let Some(scopes) = &path_scopes {
        if scopes.is_empty() {
            anyhow::bail!(AgentFsError::new(
                "invalid_path_scope",
                "path-scoped grant must include at least one scope",
                false,
            ));
        }
        for scope in scopes {
            validate_path_scope(scope)?;
        }
    }
    Ok(AgentFsGrantRecord {
        schema_version: SCHEMA_VERSION,
        grant_id: new_grant_id()?,
        fs_id: fs.fs_id.clone(),
        agent_id: agent_id.to_string(),
        role,
        capabilities: role.capabilities(),
        path_scopes,
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

pub fn hook_record(
    fs_id: &str,
    name: &str,
    command: Vec<String>,
    created_by_agent_id: &str,
) -> Result<AgentFsHookRecord> {
    let hook = AgentFsHookRecord {
        schema_version: SCHEMA_VERSION,
        hook_id: new_hook_id()?,
        fs_id: fs_id.to_string(),
        name: name.trim().to_string(),
        event: "commit.materialized".to_string(),
        command,
        created_by_agent_id: created_by_agent_id.to_string(),
        created_at_ms: now_ms(),
    };
    hook.validate_metadata("hook")?;
    Ok(hook)
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

fn write_json_if_not_exists<T: Serialize>(
    rt: &tokio::runtime::Runtime,
    op: &Operator,
    path: &str,
    value: &T,
    fs_id: &str,
) -> Result<()> {
    ensure_remote_parent_dirs(rt, op, path)?;
    let bytes = serde_json::to_vec_pretty(value)?;
    match rt.block_on(async { op.write_with(path, bytes).if_not_exists(true).await }) {
        Ok(_) => Ok(()),
        Err(err) if is_create_conflict(&err) => {
            anyhow::bail!(AgentFsError::metadata_write_conflict(fs_id))
        }
        Err(err) => Err(err).with_context(|| format!("failed to write AgentFS metadata {path}")),
    }
}

fn is_create_conflict(err: &opendal::Error) -> bool {
    matches!(
        err.kind(),
        opendal::ErrorKind::AlreadyExists | opendal::ErrorKind::ConditionNotMatch
    )
}

pub fn read_json<T: DeserializeOwned + AgentFsMetadataRecord>(
    rt: &tokio::runtime::Runtime,
    op: &Operator,
    path: &str,
) -> Result<T> {
    let data = rt
        .block_on(op.read(path))
        .with_context(|| format!("failed to read AgentFS metadata {path}"))?;
    let record: T = serde_json::from_slice(data.to_bytes().as_ref()).map_err(|err| {
        anyhow::Error::from(AgentFsError::malformed_metadata(format!(
            "failed to parse {path}: {err}"
        )))
    })?;
    record.validate_metadata(path)?;
    Ok(record)
}

pub fn read_optional_json<T: DeserializeOwned + AgentFsMetadataRecord>(
    rt: &tokio::runtime::Runtime,
    op: &Operator,
    path: &str,
) -> Result<Option<T>> {
    match rt.block_on(op.read(path)) {
        Ok(data) => {
            let record: T = serde_json::from_slice(data.to_bytes().as_ref()).map_err(|err| {
                anyhow::Error::from(AgentFsError::malformed_metadata(format!(
                    "failed to parse {path}: {err}"
                )))
            })?;
            record.validate_metadata(path)?;
            Ok(Some(record))
        }
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
) -> Result<AgentFsEventRecord> {
    let mut event = event.clone();
    if event.seq <= 0 {
        event.seq = next_event_seq(rt, op)?;
    }
    let path = format!("{METADATA_ROOT}/events/{}.json", event.event_id);
    write_json_if_not_exists(rt, op, &path, &event, &event.fs_id)?;
    Ok(event)
}

pub fn ensure_event_log_ready(
    rt: &tokio::runtime::Runtime,
    op: &Operator,
    fs_id: &str,
) -> Result<()> {
    let probe_path = format!("{METADATA_ROOT}/events/.ready");
    ensure_remote_parent_dirs(rt, op, &probe_path)
        .with_context(|| format!("failed to prepare AgentFS event log for fs {fs_id}"))?;
    rt.block_on(op.write(&probe_path, b"ready".to_vec()))
        .with_context(|| format!("failed to write AgentFS event log probe for fs {fs_id}"))?;
    let _ = rt.block_on(op.delete(&probe_path));
    Ok(())
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
    let lock_path = head_lock_record_path(&lock.lock_token);
    write_json_if_not_exists(rt, op, &lock_path, &lock, fs_id)?;

    let active_locks = list_active_head_locks(rt, op, fs_id, now)?;
    let acquired = active_locks
        .first()
        .is_some_and(|active| active.lock_token == lock.lock_token);
    if acquired {
        Ok(lock)
    } else {
        delete_head_lock_record(rt, op, &lock.lock_token)?;
        anyhow::bail!(AgentFsError::metadata_write_conflict(fs_id));
    }
}

pub fn release_head_lock(
    rt: &tokio::runtime::Runtime,
    op: &Operator,
    lock: &AgentFsHeadLockRecord,
) -> Result<()> {
    delete_head_lock_record(rt, op, &lock.lock_token)
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

fn head_lock_record_path(lock_token: &str) -> String {
    format!("{HEAD_LOCK_DIR}{lock_token}.json")
}

fn list_active_head_locks(
    rt: &tokio::runtime::Runtime,
    op: &Operator,
    fs_id: &str,
    now: i64,
) -> Result<Vec<AgentFsHeadLockRecord>> {
    let mut locks: Vec<AgentFsHeadLockRecord> =
        list_json_records::<AgentFsHeadLockRecord>(rt, op, HEAD_LOCK_DIR)?
            .into_iter()
            .filter(|lock| lock.fs_id == fs_id && lock.expires_at_ms > now)
            .collect();
    locks.sort_by(|left, right| {
        left.created_at_ms
            .cmp(&right.created_at_ms)
            .then_with(|| left.lock_token.cmp(&right.lock_token))
    });
    Ok(locks)
}

fn delete_head_lock_record(
    rt: &tokio::runtime::Runtime,
    op: &Operator,
    lock_token: &str,
) -> Result<()> {
    let path = head_lock_record_path(lock_token);
    match rt.block_on(op.delete(&path)) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == opendal::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("failed to delete AgentFS metadata {path}")),
    }
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

fn list_json_records<T: DeserializeOwned + AgentFsMetadataRecord>(
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
    use opendal::services;
    use std::fs;
    use std::path::Path;

    fn fs_operator(root: &Path) -> Operator {
        let builder = services::Fs::default().root(root.to_str().expect("utf8 root"));
        Operator::new(builder).expect("operator").finish()
    }

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
        assert!(validate_source_relative_path(".section/root.json").is_err());
        assert!(validate_source_relative_path(".section/user-note.txt").is_err());
        assert!(validate_source_relative_path(".section/agentfs/fs.json").is_err());
    }

    #[test]
    fn metadata_records_validate_schema_and_required_fields() {
        let now = now_ms();
        let fs_id = new_fs_id().expect("fs id");
        let agent_id = new_agent_id().expect("agent id");
        let source_profile_id = new_source_profile_id().expect("source profile id");
        let grant_id = new_grant_id().expect("grant id");
        let commit_id = new_commit_id().expect("commit id");
        let event_id = new_event_id(now).expect("event id");

        let fs = AgentFsRecord {
            schema_version: SCHEMA_VERSION,
            fs_id: fs_id.clone(),
            name: "project".to_string(),
            owner_agent_id: agent_id.clone(),
            source_profile_id: source_profile_id.clone(),
            source_name: fs_id.clone(),
            created_at_ms: now,
        };
        fs.validate_metadata(FS_PATH).expect("fs metadata");

        let source_profile = AgentFsSourceProfileRecord {
            schema_version: SCHEMA_VERSION,
            source_profile_id: source_profile_id.clone(),
            name: "test-profile".to_string(),
            provider: "fs".to_string(),
            created_at_ms: now,
            updated_at_ms: now,
        };
        source_profile
            .validate_metadata("source_profiles/test-profile.json")
            .expect("source profile metadata");

        let grant = AgentFsGrantRecord {
            schema_version: SCHEMA_VERSION,
            grant_id: grant_id.clone(),
            fs_id: fs_id.clone(),
            agent_id: agent_id.clone(),
            role: AgentFsRole::Writer,
            capabilities: AgentFsRole::Writer.capabilities(),
            path_scopes: None,
            granted_by: agent_id.clone(),
            created_at_ms: now,
            revoked_at_ms: None,
            revoked_by: None,
        };
        grant
            .validate_metadata("grants/grant.json")
            .expect("grant metadata");

        let share = AgentFsShareRecord {
            schema_version: SCHEMA_VERSION,
            share_id: new_share_id().expect("share id"),
            fs_id: fs_id.clone(),
            target_agent_id: agent_id.clone(),
            grant_id: grant_id.clone(),
            role: AgentFsRole::Writer,
            source_profile_id: source_profile_id.clone(),
            created_by: agent_id.clone(),
            created_at_ms: now,
            expires_at_ms: None,
            accepted_at_ms: None,
            revoked_at_ms: None,
        };
        share
            .validate_metadata("shares/share.json")
            .expect("share metadata");

        let credential = AgentFsCredentialBindingRecord {
            schema_version: SCHEMA_VERSION,
            credential_binding_id: new_credential_binding_id().expect("credential id"),
            fs_id: fs_id.clone(),
            agent_id: agent_id.clone(),
            installation_id: new_installation_id().expect("installation id"),
            source_profile_id: source_profile_id.clone(),
            issued_at_ms: now,
            expires_at_ms: now + 1,
        };
        credential
            .validate_metadata("credentials/credential.json")
            .expect("credential metadata");

        let head = AgentFsHeadRecord {
            schema_version: SCHEMA_VERSION,
            fs_id: fs_id.clone(),
            commit_id: Some(commit_id.clone()),
            updated_at_ms: now,
        };
        head.validate_metadata(HEAD_PATH).expect("head metadata");

        let lock = AgentFsHeadLockRecord {
            schema_version: SCHEMA_VERSION,
            fs_id: fs_id.clone(),
            lock_token: new_lock_token().expect("lock token"),
            owner_agent_id: agent_id.clone(),
            created_at_ms: now,
            expires_at_ms: now + 1,
        };
        lock.validate_metadata("locks/head/lock.json")
            .expect("lock metadata");

        let commit = AgentFsCommitRecord {
            schema_version: SCHEMA_VERSION,
            commit_id: commit_id.clone(),
            fs_id: fs_id.clone(),
            parent_commit_id: None,
            base_commit_id: None,
            base_manifest_hash: None,
            agent_id: agent_id.clone(),
            summary: "Update docs".to_string(),
            paths: vec![AgentFsCommitPathRecord {
                path: "docs/readme.md".to_string(),
                kind: "file".to_string(),
                op: "create".to_string(),
                local_version: Some("sha256:test".to_string()),
                previous_version: None,
            }],
            authorized_by: Some(AgentFsAuthorization::Grant {
                grant_id,
                role: AgentFsRole::Writer,
                capabilities: AgentFsRole::Writer.capabilities(),
                path_scopes: None,
                matched_path_scopes: None,
            }),
            staging_snapshot: Some(AgentFsCommitStagingRecord {
                manifest_path: "agentfs/staging/manifest.json".to_string(),
                manifest_hash: "sha256:manifest".to_string(),
            }),
            created_at_ms: now,
            materialization_state: AgentFsMaterializationState::Pending,
            materialized_at_ms: None,
            error: None,
        };
        commit
            .validate_metadata("commits/commit.json")
            .expect("commit metadata");

        let event = AgentFsEventRecord {
            schema_version: SCHEMA_VERSION,
            event_id,
            seq: 1,
            fs_id,
            kind: "commit.accepted".to_string(),
            actor_agent_id: agent_id,
            subject_id: commit_id,
            path: None,
            created_at_ms: now,
            data: serde_json::json!({}),
        };
        event
            .validate_metadata("events/event.json")
            .expect("event metadata");

        let mut bad_schema = fs.clone();
        bad_schema.schema_version = SCHEMA_VERSION + 1;
        assert!(bad_schema.validate_metadata(FS_PATH).is_err());
        assert!(serde_json::from_value::<AgentFsRecord>(serde_json::json!({
            "schema_version": SCHEMA_VERSION
        }))
        .is_err());
    }

    #[test]
    fn event_write_does_not_overwrite_existing_event_id() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let op = fs_operator(temp_dir.path());
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let fs_id = new_fs_id().expect("fs id");
        let agent_id = new_agent_id().expect("agent id");
        let event = event_record(
            &fs_id,
            "fs.created",
            &agent_id,
            &fs_id,
            None,
            serde_json::json!({ "name": "project" }),
        )
        .expect("event");

        write_event(&rt, &op, &event).expect("write event");
        let event_path = temp_dir
            .path()
            .join(METADATA_ROOT)
            .join("events")
            .join(format!("{}.json", event.event_id));
        let original = fs::read_to_string(&event_path).expect("read event");

        let err = write_event(&rt, &op, &event).expect_err("duplicate event should fail");
        assert!(
            err.to_string().contains("metadata_write_conflict"),
            "unexpected error: {err}"
        );
        assert_eq!(
            fs::read_to_string(&event_path).expect("read unchanged event"),
            original
        );
    }

    #[test]
    fn head_lock_conflicts_until_released() {
        let temp_dir = tempfile::TempDir::new().expect("temp dir");
        let op = fs_operator(temp_dir.path());
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let fs_id = new_fs_id().expect("fs id");
        let agent_a = new_agent_id().expect("agent a");
        let agent_b = new_agent_id().expect("agent b");

        let lock = acquire_head_lock(&rt, &op, &fs_id, &agent_a).expect("first lock");
        let err =
            acquire_head_lock(&rt, &op, &fs_id, &agent_b).expect_err("second lock should fail");
        assert!(
            err.to_string().contains("metadata_write_conflict"),
            "unexpected error: {err}"
        );

        release_head_lock(&rt, &op, &lock).expect("release lock");
        let next = acquire_head_lock(&rt, &op, &fs_id, &agent_b).expect("lock after release");
        assert_ne!(lock.lock_token, next.lock_token);
    }
}
