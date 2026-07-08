use crate::agentfs;
use crate::{
    AgentFsAuthorization, AgentFsCapability, AgentFsCredentialBindingRecord, AgentFsError,
    AgentFsErrorPayload, AgentFsEventRecord, AgentFsGrantRecord, AgentFsRecord, AgentFsRole,
    AgentFsShareRecord, AgentFsSourceProfileRecord,
};
use anyhow::{Context, Result};
use opendal::{Operator, Scheme};
use ring::digest::{digest, SHA256};
use rusqlite::{params, Connection, OptionalExtension};
use section_core::config::{ControlServiceConfig, SourceConfig};
use section_core::SectionConfig;
use section_provider::AgentIdentityRecord;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

const CREDENTIAL_TTL_MS: i64 = 60 * 60 * 1000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentFsAvailableShare {
    pub share: AgentFsShareRecord,
    pub fs: AgentFsRecord,
    pub source_profile: AgentFsSourceProfileRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentFsShareResult {
    pub share: AgentFsShareRecord,
    pub fs: AgentFsRecord,
    pub source_profile: AgentFsSourceProfileRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentFsAcceptResult {
    pub share: AgentFsShareRecord,
    pub fs: AgentFsRecord,
    pub source_profile: AgentFsSourceProfileRecord,
    pub credential_binding: AgentFsCredentialBindingRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssuedAgentFsCredential {
    pub binding: AgentFsCredentialBindingRecord,
    pub source_profile: AgentFsSourceProfileRecord,
    pub source: SourceConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilesystemCreateResult {
    pub fs: AgentFsRecord,
    pub owner_grant: AgentFsGrantRecord,
    pub event: AgentFsEventRecord,
    pub source_profile: AgentFsSourceProfileRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GrantMutationResult {
    pub grant: AgentFsGrantRecord,
    pub revoked: Vec<AgentFsGrantRecord>,
    pub events: Vec<AgentFsEventRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RevokeMutationResult {
    pub revoked: Vec<AgentFsGrantRecord>,
    pub events: Vec<AgentFsEventRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedServiceFilesystem {
    pub fs: AgentFsRecord,
    pub source_profile: AgentFsSourceProfileRecord,
    pub grants: Vec<AgentFsGrantRecord>,
}

struct ResolvedServiceFilesystemWithSource {
    fs: AgentFsRecord,
    source_profile: AgentFsSourceProfileRecord,
    source: SourceConfig,
    grants: Vec<AgentFsGrantRecord>,
}

pub struct ControlServiceStore {
    conn: Connection,
    path: PathBuf,
    enforce_source_profile_create_policy: bool,
}

pub enum ControlService {
    Local(ControlServiceStore),
    Remote(HttpControlServiceClient),
}

#[derive(Debug, Clone)]
pub struct HttpControlServiceClient {
    endpoint: String,
    client: reqwest::blocking::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentIdentityWire {
    pub agent_id: String,
    pub installation_id: String,
    pub name: String,
    pub auth_token: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl From<AgentIdentityRecord> for AgentIdentityWire {
    fn from(record: AgentIdentityRecord) -> Self {
        Self {
            agent_id: record.agent_id,
            installation_id: record.installation_id,
            name: record.name,
            auth_token: record.auth_token,
            created_at_ms: record.created_at_ms,
            updated_at_ms: record.updated_at_ms,
        }
    }
}

impl From<AgentIdentityWire> for AgentIdentityRecord {
    fn from(record: AgentIdentityWire) -> Self {
        Self {
            agent_id: record.agent_id,
            installation_id: record.installation_id,
            name: record.name,
            auth_token: record.auth_token,
            created_at_ms: record.created_at_ms,
            updated_at_ms: record.updated_at_ms,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum ControlServiceRequest {
    LoginAgent {
        name: String,
        existing_installation_id: Option<String>,
        auth_token: Option<String>,
    },
    CreateFilesystem {
        name: String,
        source_profile_name: String,
        owner: AgentIdentityWire,
    },
    RollbackCreatedFilesystem {
        fs_id: String,
        actor: AgentIdentityWire,
    },
    ListFilesystemsForAgent {
        actor: AgentIdentityWire,
    },
    ResolveFilesystem {
        fs_ref: String,
        actor: AgentIdentityWire,
    },
    ListEvents {
        fs_id: String,
        actor: AgentIdentityWire,
    },
    ActiveRole {
        fs: AgentFsRecord,
        actor: AgentIdentityWire,
    },
    AuthorizeCapability {
        fs_id: String,
        actor: AgentIdentityWire,
        capability: AgentFsCapability,
    },
    FsGrant {
        fs_id: String,
        actor: AgentIdentityWire,
        target_agent_id: String,
        role: AgentFsRole,
    },
    FsRevoke {
        fs_id: String,
        actor: AgentIdentityWire,
        target_agent_id: String,
    },
    FsShare {
        fs_id: String,
        actor: AgentIdentityWire,
        target_agent_id: String,
    },
    AvailableShares {
        actor: AgentIdentityWire,
    },
    AcceptShare {
        share_id: String,
        actor: AgentIdentityWire,
    },
    IssueCredential {
        fs_id: String,
        actor: AgentIdentityWire,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum ControlServiceResponse {
    AgentIdentity(AgentIdentityWire),
    FilesystemCreate(FilesystemCreateResult),
    Unit,
    Filesystems(Vec<AgentFsRecord>),
    ResolvedFilesystem(ResolvedServiceFilesystem),
    Events(Vec<AgentFsEventRecord>),
    ActiveRole(Option<AgentFsRole>),
    Authorization(AgentFsAuthorization),
    GrantMutation(GrantMutationResult),
    RevokeMutation(RevokeMutationResult),
    Share(AgentFsShareResult),
    AvailableShares(Vec<AgentFsAvailableShare>),
    AcceptShare {
        accepted: AgentFsAcceptResult,
        source: SourceConfig,
    },
    IssuedCredential(IssuedAgentFsCredential),
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ControlServiceRpcResult {
    Ok { response: ControlServiceResponse },
    Err { error: AgentFsErrorPayload },
}

impl ControlService {
    pub fn open(config: &SectionConfig) -> Result<Self> {
        match config.control_service.endpoint.as_deref() {
            Some(endpoint) if !endpoint.trim().is_empty() => {
                Ok(Self::Remote(HttpControlServiceClient::new(endpoint)))
            }
            _ => Ok(Self::Local(ControlServiceStore::open(config)?)),
        }
    }

    pub fn endpoint(&self) -> String {
        match self {
            Self::Local(store) => store.endpoint(),
            Self::Remote(client) => client.endpoint().to_string(),
        }
    }

    pub fn login_agent(
        &self,
        name: &str,
        existing_installation_id: Option<&str>,
        auth_token: Option<&str>,
    ) -> Result<AgentIdentityRecord> {
        match self {
            Self::Local(store) => store.login_agent(name, existing_installation_id, auth_token),
            Self::Remote(client) => client.login_agent(name, existing_installation_id, auth_token),
        }
    }

    pub fn create_filesystem(
        &self,
        name: &str,
        source_profile_name: &str,
        owner: &AgentIdentityRecord,
    ) -> Result<FilesystemCreateResult> {
        match self {
            Self::Local(store) => store.create_filesystem(name, source_profile_name, owner),
            Self::Remote(client) => client.create_filesystem(name, source_profile_name, owner),
        }
    }

    pub fn rollback_created_filesystem(
        &self,
        fs_id: &str,
        actor: &AgentIdentityRecord,
    ) -> Result<()> {
        match self {
            Self::Local(store) => store.rollback_created_filesystem(fs_id),
            Self::Remote(client) => client.rollback_created_filesystem(fs_id, actor),
        }
    }

    pub fn list_filesystems_for_agent(
        &self,
        actor: &AgentIdentityRecord,
    ) -> Result<Vec<AgentFsRecord>> {
        match self {
            Self::Local(store) => store.list_filesystems_for_agent(&actor.agent_id),
            Self::Remote(client) => client.list_filesystems_for_agent(actor),
        }
    }

    pub fn resolve_filesystem(
        &self,
        fs_ref: &str,
        actor: &AgentIdentityRecord,
    ) -> Result<ResolvedServiceFilesystem> {
        match self {
            Self::Local(store) => store.resolve_filesystem(fs_ref),
            Self::Remote(client) => client.resolve_filesystem(fs_ref, actor),
        }
    }

    pub fn list_events(
        &self,
        fs_id: &str,
        actor: &AgentIdentityRecord,
    ) -> Result<Vec<AgentFsEventRecord>> {
        match self {
            Self::Local(store) => store.list_events(fs_id),
            Self::Remote(client) => client.list_events(fs_id, actor),
        }
    }

    pub fn active_role(
        &self,
        fs: &AgentFsRecord,
        actor: &AgentIdentityRecord,
    ) -> Result<Option<AgentFsRole>> {
        match self {
            Self::Local(store) => store.active_role(fs, &actor.agent_id),
            Self::Remote(client) => client.active_role(fs, actor),
        }
    }

    pub fn authorize_capability(
        &self,
        fs_id: &str,
        actor: &AgentIdentityRecord,
        capability: AgentFsCapability,
    ) -> Result<AgentFsAuthorization> {
        match self {
            Self::Local(store) => store.authorize_capability(fs_id, &actor.agent_id, capability),
            Self::Remote(client) => client.authorize_capability(fs_id, actor, capability),
        }
    }

    pub fn fs_grant(
        &self,
        fs_id: &str,
        actor: &AgentIdentityRecord,
        target_agent_id: &str,
        role: AgentFsRole,
    ) -> Result<GrantMutationResult> {
        match self {
            Self::Local(store) => store.fs_grant(fs_id, &actor.agent_id, target_agent_id, role),
            Self::Remote(client) => client.fs_grant(fs_id, actor, target_agent_id, role),
        }
    }

    pub fn fs_revoke(
        &self,
        fs_id: &str,
        actor: &AgentIdentityRecord,
        target_agent_id: &str,
    ) -> Result<RevokeMutationResult> {
        match self {
            Self::Local(store) => store.fs_revoke(fs_id, &actor.agent_id, target_agent_id),
            Self::Remote(client) => client.fs_revoke(fs_id, actor, target_agent_id),
        }
    }

    pub fn fs_share(
        &self,
        fs_id: &str,
        actor: &AgentIdentityRecord,
        target_agent_id: &str,
    ) -> Result<AgentFsShareResult> {
        match self {
            Self::Local(store) => store.fs_share(fs_id, &actor.agent_id, target_agent_id),
            Self::Remote(client) => client.fs_share(fs_id, actor, target_agent_id),
        }
    }

    pub fn available_shares(
        &self,
        actor: &AgentIdentityRecord,
    ) -> Result<Vec<AgentFsAvailableShare>> {
        match self {
            Self::Local(store) => store.available_shares(&actor.agent_id),
            Self::Remote(client) => client.available_shares(actor),
        }
    }

    pub fn accept_share(
        &self,
        share_id: &str,
        actor: &AgentIdentityRecord,
    ) -> Result<(AgentFsAcceptResult, SourceConfig)> {
        match self {
            Self::Local(store) => {
                store.accept_share(share_id, &actor.agent_id, &actor.installation_id)
            }
            Self::Remote(client) => client.accept_share(share_id, actor),
        }
    }

    pub fn issue_credential(
        &self,
        fs_id: &str,
        actor: &AgentIdentityRecord,
    ) -> Result<IssuedAgentFsCredential> {
        match self {
            Self::Local(store) => {
                store.issue_credential(fs_id, &actor.agent_id, &actor.installation_id)
            }
            Self::Remote(client) => client.issue_credential(fs_id, actor),
        }
    }
}

impl HttpControlServiceClient {
    pub fn new(endpoint: &str) -> Self {
        Self {
            endpoint: endpoint.trim_end_matches('/').to_string(),
            client: reqwest::blocking::Client::new(),
        }
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    fn rpc(&self, request: ControlServiceRequest) -> Result<ControlServiceResponse> {
        let response = self
            .client
            .post(format!("{}/v1/rpc", self.endpoint))
            .json(&request)
            .send()
            .with_context(|| {
                format!("failed to reach Section Control Service {}", self.endpoint)
            })?;
        let status = response.status();
        let rpc: ControlServiceRpcResult = response.json().with_context(|| {
            format!(
                "Section Control Service {} returned non-JSON response with status {status}",
                self.endpoint
            )
        })?;
        match rpc {
            ControlServiceRpcResult::Ok { response } if status.is_success() => Ok(response),
            ControlServiceRpcResult::Ok { .. } => anyhow::bail!(
                "Section Control Service {} returned unexpected status {status}",
                self.endpoint
            ),
            ControlServiceRpcResult::Err { error } => {
                anyhow::bail!(AgentFsError::from_payload(error))
            }
        }
    }

    pub fn login_agent(
        &self,
        name: &str,
        existing_installation_id: Option<&str>,
        auth_token: Option<&str>,
    ) -> Result<AgentIdentityRecord> {
        match self.rpc(ControlServiceRequest::LoginAgent {
            name: name.to_string(),
            existing_installation_id: existing_installation_id.map(str::to_string),
            auth_token: auth_token.map(str::to_string),
        })? {
            ControlServiceResponse::AgentIdentity(identity) => Ok(identity.into()),
            other => unexpected_rpc_response(other),
        }
    }

    pub fn create_filesystem(
        &self,
        name: &str,
        source_profile_name: &str,
        owner: &AgentIdentityRecord,
    ) -> Result<FilesystemCreateResult> {
        match self.rpc(ControlServiceRequest::CreateFilesystem {
            name: name.to_string(),
            source_profile_name: source_profile_name.to_string(),
            owner: owner.clone().into(),
        })? {
            ControlServiceResponse::FilesystemCreate(result) => Ok(result),
            other => unexpected_rpc_response(other),
        }
    }

    pub fn rollback_created_filesystem(
        &self,
        fs_id: &str,
        actor: &AgentIdentityRecord,
    ) -> Result<()> {
        match self.rpc(ControlServiceRequest::RollbackCreatedFilesystem {
            fs_id: fs_id.to_string(),
            actor: actor.clone().into(),
        })? {
            ControlServiceResponse::Unit => Ok(()),
            other => unexpected_rpc_response(other),
        }
    }

    pub fn list_filesystems_for_agent(
        &self,
        actor: &AgentIdentityRecord,
    ) -> Result<Vec<AgentFsRecord>> {
        match self.rpc(ControlServiceRequest::ListFilesystemsForAgent {
            actor: actor.clone().into(),
        })? {
            ControlServiceResponse::Filesystems(filesystems) => Ok(filesystems),
            other => unexpected_rpc_response(other),
        }
    }

    pub fn resolve_filesystem(
        &self,
        fs_ref: &str,
        actor: &AgentIdentityRecord,
    ) -> Result<ResolvedServiceFilesystem> {
        match self.rpc(ControlServiceRequest::ResolveFilesystem {
            fs_ref: fs_ref.to_string(),
            actor: actor.clone().into(),
        })? {
            ControlServiceResponse::ResolvedFilesystem(resolved) => Ok(resolved),
            other => unexpected_rpc_response(other),
        }
    }

    pub fn list_events(
        &self,
        fs_id: &str,
        actor: &AgentIdentityRecord,
    ) -> Result<Vec<AgentFsEventRecord>> {
        match self.rpc(ControlServiceRequest::ListEvents {
            fs_id: fs_id.to_string(),
            actor: actor.clone().into(),
        })? {
            ControlServiceResponse::Events(events) => Ok(events),
            other => unexpected_rpc_response(other),
        }
    }

    pub fn active_role(
        &self,
        fs: &AgentFsRecord,
        actor: &AgentIdentityRecord,
    ) -> Result<Option<AgentFsRole>> {
        match self.rpc(ControlServiceRequest::ActiveRole {
            fs: fs.clone(),
            actor: actor.clone().into(),
        })? {
            ControlServiceResponse::ActiveRole(role) => Ok(role),
            other => unexpected_rpc_response(other),
        }
    }

    pub fn authorize_capability(
        &self,
        fs_id: &str,
        actor: &AgentIdentityRecord,
        capability: AgentFsCapability,
    ) -> Result<AgentFsAuthorization> {
        match self.rpc(ControlServiceRequest::AuthorizeCapability {
            fs_id: fs_id.to_string(),
            actor: actor.clone().into(),
            capability,
        })? {
            ControlServiceResponse::Authorization(authorization) => Ok(authorization),
            other => unexpected_rpc_response(other),
        }
    }

    pub fn fs_grant(
        &self,
        fs_id: &str,
        actor: &AgentIdentityRecord,
        target_agent_id: &str,
        role: AgentFsRole,
    ) -> Result<GrantMutationResult> {
        match self.rpc(ControlServiceRequest::FsGrant {
            fs_id: fs_id.to_string(),
            actor: actor.clone().into(),
            target_agent_id: target_agent_id.to_string(),
            role,
        })? {
            ControlServiceResponse::GrantMutation(result) => Ok(result),
            other => unexpected_rpc_response(other),
        }
    }

    pub fn fs_revoke(
        &self,
        fs_id: &str,
        actor: &AgentIdentityRecord,
        target_agent_id: &str,
    ) -> Result<RevokeMutationResult> {
        match self.rpc(ControlServiceRequest::FsRevoke {
            fs_id: fs_id.to_string(),
            actor: actor.clone().into(),
            target_agent_id: target_agent_id.to_string(),
        })? {
            ControlServiceResponse::RevokeMutation(result) => Ok(result),
            other => unexpected_rpc_response(other),
        }
    }

    pub fn fs_share(
        &self,
        fs_id: &str,
        actor: &AgentIdentityRecord,
        target_agent_id: &str,
    ) -> Result<AgentFsShareResult> {
        match self.rpc(ControlServiceRequest::FsShare {
            fs_id: fs_id.to_string(),
            actor: actor.clone().into(),
            target_agent_id: target_agent_id.to_string(),
        })? {
            ControlServiceResponse::Share(result) => Ok(result),
            other => unexpected_rpc_response(other),
        }
    }

    pub fn available_shares(
        &self,
        actor: &AgentIdentityRecord,
    ) -> Result<Vec<AgentFsAvailableShare>> {
        match self.rpc(ControlServiceRequest::AvailableShares {
            actor: actor.clone().into(),
        })? {
            ControlServiceResponse::AvailableShares(shares) => Ok(shares),
            other => unexpected_rpc_response(other),
        }
    }

    pub fn accept_share(
        &self,
        share_id: &str,
        actor: &AgentIdentityRecord,
    ) -> Result<(AgentFsAcceptResult, SourceConfig)> {
        match self.rpc(ControlServiceRequest::AcceptShare {
            share_id: share_id.to_string(),
            actor: actor.clone().into(),
        })? {
            ControlServiceResponse::AcceptShare { accepted, source } => Ok((accepted, source)),
            other => unexpected_rpc_response(other),
        }
    }

    pub fn issue_credential(
        &self,
        fs_id: &str,
        actor: &AgentIdentityRecord,
    ) -> Result<IssuedAgentFsCredential> {
        match self.rpc(ControlServiceRequest::IssueCredential {
            fs_id: fs_id.to_string(),
            actor: actor.clone().into(),
        })? {
            ControlServiceResponse::IssuedCredential(result) => Ok(result),
            other => unexpected_rpc_response(other),
        }
    }
}

fn unexpected_rpc_response<T>(response: ControlServiceResponse) -> Result<T> {
    anyhow::bail!("unexpected Section Control Service response: {response:?}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceProfileSeedMode {
    InsertOnly,
    UpdateExisting,
}

impl ControlServiceStore {
    pub fn open(config: &SectionConfig) -> Result<Self> {
        Self::open_with_source_profile_seed_mode(config, SourceProfileSeedMode::InsertOnly, false)
    }

    pub fn open_authoritative(config: &SectionConfig) -> Result<Self> {
        Self::open_with_source_profile_seed_mode(
            config,
            SourceProfileSeedMode::UpdateExisting,
            true,
        )
    }

    fn open_with_source_profile_seed_mode(
        config: &SectionConfig,
        seed_mode: SourceProfileSeedMode,
        enforce_source_profile_create_policy: bool,
    ) -> Result<Self> {
        let path = control_service_path(config);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let conn = Connection::open(&path).with_context(|| {
            format!("failed to open control service database {}", path.display())
        })?;
        let store = Self {
            conn,
            path,
            enforce_source_profile_create_policy,
        };
        store.init_tables()?;
        store.seed_source_profiles(&config.control_service, seed_mode)?;
        Ok(store)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn endpoint(&self) -> String {
        format!("section-control-service:file:{}", self.path.display())
    }

    fn init_tables(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS agents (
                agent_id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                auth_token_hash TEXT NOT NULL DEFAULT '',
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS installations (
                installation_id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS source_profiles (
                source_profile_id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                provider TEXT NOT NULL,
                options_json TEXT NOT NULL DEFAULT '{}',
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS filesystems (
                fs_id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                owner_agent_id TEXT NOT NULL,
                source_profile_id TEXT NOT NULL,
                source_name TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS grants (
                grant_id TEXT PRIMARY KEY,
                fs_id TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                role TEXT NOT NULL,
                capabilities_json TEXT NOT NULL,
                granted_by TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL,
                revoked_at_ms INTEGER,
                revoked_by TEXT
            );
            CREATE TABLE IF NOT EXISTS shares (
                share_id TEXT PRIMARY KEY,
                fs_id TEXT NOT NULL,
                target_agent_id TEXT NOT NULL,
                grant_id TEXT NOT NULL,
                role TEXT NOT NULL,
                source_profile_id TEXT NOT NULL,
                created_by TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL,
                expires_at_ms INTEGER,
                accepted_at_ms INTEGER,
                revoked_at_ms INTEGER
            );
            CREATE TABLE IF NOT EXISTS credential_bindings (
                credential_binding_id TEXT PRIMARY KEY,
                fs_id TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                installation_id TEXT NOT NULL,
                source_profile_id TEXT NOT NULL,
                issued_at_ms INTEGER NOT NULL,
                expires_at_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS events (
                event_id TEXT PRIMARY KEY,
                seq INTEGER NOT NULL DEFAULT 0,
                fs_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                actor_agent_id TEXT NOT NULL,
                subject_id TEXT NOT NULL,
                path TEXT,
                data_json TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL
            );",
        )?;
        add_column_if_missing(
            &self.conn,
            "agents",
            "auth_token_hash",
            "TEXT NOT NULL DEFAULT ''",
        )?;
        add_column_if_missing(&self.conn, "events", "seq", "INTEGER NOT NULL DEFAULT 0")?;
        self.ensure_existing_event_seqs()?;
        Ok(())
    }

    fn seed_source_profiles(
        &self,
        config: &ControlServiceConfig,
        mode: SourceProfileSeedMode,
    ) -> Result<()> {
        for (name, source) in &config.source_profiles {
            let now = agentfs::now_ms();
            let options_json = serde_json::to_string(&source.options)?;
            let existing: Option<(String, String, String)> = self
                .conn
                .query_row(
                    "SELECT source_profile_id, provider, options_json
                     FROM source_profiles
                     WHERE name = ?1",
                    [name],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            match existing {
                Some((source_profile_id, provider, existing_options_json)) => {
                    if mode == SourceProfileSeedMode::UpdateExisting
                        && (provider != source.provider || existing_options_json != options_json)
                    {
                        self.conn.execute(
                            "UPDATE source_profiles
                             SET provider = ?1, options_json = ?2, updated_at_ms = ?3
                             WHERE source_profile_id = ?4",
                            params![source.provider, options_json, now, source_profile_id],
                        )?;
                    }
                }
                None => {
                    self.conn.execute(
                        "INSERT INTO source_profiles (
                            source_profile_id, name, provider, options_json, created_at_ms, updated_at_ms
                         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        params![
                            agentfs::new_source_profile_id()?,
                            name,
                            source.provider,
                            options_json,
                            now,
                            now,
                        ],
                    )?;
                }
            }
        }
        Ok(())
    }

    pub fn login_agent(
        &self,
        name: &str,
        existing_installation_id: Option<&str>,
        auth_token: Option<&str>,
    ) -> Result<AgentIdentityRecord> {
        let name = name.trim();
        if name.is_empty() {
            anyhow::bail!("agent name must not be empty");
        }

        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<AgentIdentityRecord> {
            let now = agentfs::now_ms();
            let mut agent = self.find_agent_by_name(name)?;
            if let Some(existing) = agent.as_mut() {
                let Some(auth_token) = auth_token else {
                    anyhow::bail!(AgentFsError::new(
                        "unknown_agent",
                        format!(
                            "agent name {name} already exists, but this installation has no auth token"
                        ),
                        false,
                    ));
                };
                if !verify_auth_token(auth_token, &existing.auth_token) {
                    anyhow::bail!(AgentFsError::new(
                        "unknown_agent",
                        format!(
                            "agent name {name} already exists, but the local auth token is invalid"
                        ),
                        false,
                    ));
                }
                existing.auth_token = auth_token.to_string();
                existing.updated_at_ms = now;
                self.conn.execute(
                    "UPDATE agents SET updated_at_ms = ?1 WHERE agent_id = ?2",
                    params![now, existing.agent_id],
                )?;
            }

            let agent = match agent {
                Some(agent) => agent,
                None => {
                    let auth_token = agentfs::new_auth_token()?;
                    let agent = AgentIdentityRecord {
                        agent_id: agentfs::new_agent_id()?,
                        installation_id: String::new(),
                        name: name.to_string(),
                        auth_token,
                        created_at_ms: now,
                        updated_at_ms: now,
                    };
                    self.conn.execute(
                        "INSERT INTO agents (agent_id, name, auth_token_hash, created_at_ms, updated_at_ms)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![
                            agent.agent_id,
                            agent.name,
                            hash_auth_token(&agent.auth_token),
                            agent.created_at_ms,
                            agent.updated_at_ms
                        ],
                    )?;
                    agent
                }
            };

            let installation_id = match existing_installation_id {
                Some(installation_id)
                    if self.installation_belongs_to_agent(installation_id, &agent.agent_id)? =>
                {
                    self.conn.execute(
                        "UPDATE installations SET updated_at_ms = ?1 WHERE installation_id = ?2",
                        params![now, installation_id],
                    )?;
                    installation_id.to_string()
                }
                _ => {
                    let installation_id = agentfs::new_installation_id()?;
                    self.conn.execute(
                        "INSERT INTO installations (
                            installation_id, agent_id, created_at_ms, updated_at_ms
                         ) VALUES (?1, ?2, ?3, ?4)",
                        params![installation_id, agent.agent_id, now, now],
                    )?;
                    installation_id
                }
            };

            Ok(AgentIdentityRecord {
                installation_id,
                ..agent
            })
        })();
        finish_transaction(&self.conn, result)
    }

    pub fn authenticate_agent(&self, agent: &AgentIdentityRecord) -> Result<()> {
        if agent.auth_token.trim().is_empty() {
            anyhow::bail!(AgentFsError::new(
                "unknown_agent",
                format!("agent {} did not provide an auth token", agent.agent_id),
                false,
            ));
        }
        let stored_hash: Option<String> = self
            .conn
            .query_row(
                "SELECT auth_token_hash FROM agents WHERE agent_id = ?1",
                [agent.agent_id.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        let Some(stored_hash) = stored_hash else {
            anyhow::bail!(AgentFsError::new(
                "unknown_agent",
                format!(
                    "agent {} is not known to the control service",
                    agent.agent_id
                ),
                false,
            ));
        };
        if !verify_auth_token(&agent.auth_token, &stored_hash) {
            anyhow::bail!(AgentFsError::new(
                "unknown_agent",
                format!("agent {} auth token is invalid", agent.agent_id),
                false,
            ));
        }
        if !self.installation_belongs_to_agent(&agent.installation_id, &agent.agent_id)? {
            anyhow::bail!(AgentFsError::new(
                "unknown_agent",
                format!(
                    "installation {} does not belong to agent {}",
                    agent.installation_id, agent.agent_id
                ),
                false,
            ));
        }
        Ok(())
    }

    fn ensure_source_profile_create_allowed(
        &self,
        source_profile: &AgentFsSourceProfileRecord,
        source: &SourceConfig,
        owner: &AgentIdentityRecord,
    ) -> Result<()> {
        if !self.enforce_source_profile_create_policy {
            return Ok(());
        }

        let allowed = source
            .options
            .get("section.agentfs.create_agents")
            .map(|raw| {
                raw.split(',')
                    .map(str::trim)
                    .any(|value| value == "*" || value == owner.agent_id || value == owner.name)
            })
            .unwrap_or(false);

        if allowed {
            return Ok(());
        }

        anyhow::bail!(AgentFsError::grant_denied(format!(
            "agent {} is not allowed to create AgentFS filesystems from source profile {}",
            owner.agent_id, source_profile.name
        )))
    }

    pub fn create_filesystem(
        &self,
        name: &str,
        source_profile_name: &str,
        owner: &AgentIdentityRecord,
    ) -> Result<FilesystemCreateResult> {
        let name = name.trim();
        if name.is_empty() {
            anyhow::bail!("filesystem name must not be empty");
        }
        let (source_profile, source) = self.source_profile_by_name(source_profile_name)?;
        self.ensure_source_profile_create_allowed(&source_profile, &source, owner)?;
        let operator = build_operator(&source)?;
        let rt = tokio::runtime::Runtime::new()?;
        ensure_backing_source_is_empty(&rt, &operator, source_profile_name)?;

        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<FilesystemCreateResult> {
            let duplicate: Option<String> = self
                .conn
                .query_row(
                    "SELECT fs_id FROM filesystems WHERE name = ?1",
                    [name],
                    |row| row.get(0),
                )
                .optional()?;
            if duplicate.is_some() {
                anyhow::bail!("AgentFS {name} already exists");
            }

            let now = agentfs::now_ms();
            let fs_id = agentfs::new_fs_id()?;
            let fs = AgentFsRecord {
                schema_version: agentfs::SCHEMA_VERSION,
                fs_id: fs_id.clone(),
                name: name.to_string(),
                owner_agent_id: owner.agent_id.clone(),
                source_profile_id: source_profile.source_profile_id.clone(),
                source_name: fs_id,
                created_at_ms: now,
            };
            self.conn.execute(
                "INSERT INTO filesystems (
                    fs_id, name, owner_agent_id, source_profile_id, source_name, created_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    fs.fs_id,
                    fs.name,
                    fs.owner_agent_id,
                    fs.source_profile_id,
                    fs.source_name,
                    fs.created_at_ms,
                ],
            )?;

            let owner_grant =
                agentfs::grant_record(&fs, &owner.agent_id, AgentFsRole::Owner, &owner.agent_id)?;
            self.insert_grant(&owner_grant)?;

            let event = agentfs::event_record(
                &fs.fs_id,
                "fs.created",
                &owner.agent_id,
                &fs.fs_id,
                None,
                serde_json::json!({
                    "name": fs.name,
                    "source_name": fs.source_name,
                    "source_profile_id": fs.source_profile_id,
                }),
            )?;
            let event = self.insert_event(&event)?;

            let created = FilesystemCreateResult {
                fs,
                owner_grant,
                event,
                source_profile: source_profile.clone(),
            };
            if let Err(err) = mirror_created_filesystem(&rt, &operator, &created) {
                let _ = cleanup_created_filesystem_metadata(&rt, &operator);
                return Err(err);
            }
            Ok(created)
        })();
        finish_transaction(&self.conn, result)
    }

    pub fn rollback_created_filesystem(&self, fs_id: &str) -> Result<()> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<()> {
            self.conn
                .execute("DELETE FROM credential_bindings WHERE fs_id = ?1", [fs_id])?;
            self.conn
                .execute("DELETE FROM shares WHERE fs_id = ?1", [fs_id])?;
            self.conn
                .execute("DELETE FROM grants WHERE fs_id = ?1", [fs_id])?;
            self.conn
                .execute("DELETE FROM events WHERE fs_id = ?1", [fs_id])?;
            self.conn
                .execute("DELETE FROM filesystems WHERE fs_id = ?1", [fs_id])?;
            Ok(())
        })();
        finish_transaction(&self.conn, result)
    }

    pub fn list_filesystems_for_agent(&self, agent_id: &str) -> Result<Vec<AgentFsRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT fs_id, name, owner_agent_id, source_profile_id, source_name, created_at_ms
             FROM filesystems
             ORDER BY name",
        )?;
        let filesystems = stmt
            .query_map([], read_fs_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut visible = Vec::new();
        for fs in filesystems {
            if self
                .authorize_capability_for_fs(&fs, agent_id, AgentFsCapability::Read)
                .is_ok()
            {
                visible.push(fs);
            }
        }
        Ok(visible)
    }

    pub fn resolve_filesystem(&self, fs_ref: &str) -> Result<ResolvedServiceFilesystem> {
        let resolved = self.resolve_filesystem_with_source(fs_ref)?;
        Ok(ResolvedServiceFilesystem {
            fs: resolved.fs,
            source_profile: resolved.source_profile,
            grants: resolved.grants,
        })
    }

    fn resolve_filesystem_with_source(
        &self,
        fs_ref: &str,
    ) -> Result<ResolvedServiceFilesystemWithSource> {
        let fs = self
            .find_filesystem(fs_ref)?
            .ok_or_else(|| AgentFsError::unknown_fs(fs_ref))?;
        let (source_profile, source) = self.source_profile_by_id(&fs.source_profile_id)?;
        let grants = self.list_grants(&fs.fs_id)?;
        Ok(ResolvedServiceFilesystemWithSource {
            fs,
            source_profile,
            source,
            grants,
        })
    }

    pub fn list_grants(&self, fs_id: &str) -> Result<Vec<AgentFsGrantRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT grant_id, fs_id, agent_id, role, capabilities_json, granted_by,
                    created_at_ms, revoked_at_ms, revoked_by
             FROM grants
             WHERE fs_id = ?1
             ORDER BY created_at_ms, grant_id",
        )?;
        let rows = stmt.query_map([fs_id], read_grant_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn list_events(&self, fs_id: &str) -> Result<Vec<AgentFsEventRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT event_id, seq, fs_id, kind, actor_agent_id, subject_id,
                    path, data_json, created_at_ms
             FROM events
             WHERE fs_id = ?1
             ORDER BY seq, created_at_ms, event_id",
        )?;
        let rows = stmt.query_map([fs_id], read_event_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn active_role(&self, fs: &AgentFsRecord, agent_id: &str) -> Result<Option<AgentFsRole>> {
        if fs.owner_agent_id == agent_id {
            return Ok(Some(AgentFsRole::Owner));
        }
        Ok(self
            .list_grants(&fs.fs_id)?
            .into_iter()
            .find(|grant| grant.agent_id == agent_id && grant.is_active())
            .map(|grant| grant.role))
    }

    pub fn authorize_capability(
        &self,
        fs_id: &str,
        agent_id: &str,
        capability: AgentFsCapability,
    ) -> Result<AgentFsAuthorization> {
        let fs = self
            .find_filesystem(fs_id)?
            .ok_or_else(|| AgentFsError::unknown_fs(fs_id))?;
        self.authorize_capability_for_fs(&fs, agent_id, capability)
    }

    pub fn fs_grant(
        &self,
        fs_id: &str,
        actor_agent_id: &str,
        target_agent_id: &str,
        role: AgentFsRole,
    ) -> Result<GrantMutationResult> {
        agentfs::validate_agent_id(target_agent_id)?;
        if role == AgentFsRole::Owner {
            anyhow::bail!(AgentFsError::grant_denied(
                "owner grants are not supported in the MVP"
            ));
        }
        if !self.agent_exists(target_agent_id)? {
            anyhow::bail!(AgentFsError::new(
                "unknown_agent",
                format!("agent {target_agent_id} is not known to the control service"),
                false,
            ));
        }

        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<GrantMutationResult> {
            let fs = self
                .find_filesystem(fs_id)?
                .ok_or_else(|| AgentFsError::unknown_fs(fs_id))?;
            self.authorize_capability_for_fs(&fs, actor_agent_id, AgentFsCapability::Manage)?;

            if actor_agent_id == target_agent_id
                && role.has_capability(AgentFsCapability::Commit)
                && self
                    .authorize_capability_for_fs(&fs, actor_agent_id, AgentFsCapability::Commit)
                    .is_err()
            {
                anyhow::bail!(AgentFsError::grant_denied(
                    "manager cannot grant commit access to itself"
                ));
            }
            if target_agent_id == fs.owner_agent_id {
                anyhow::bail!(AgentFsError::grant_denied(
                    "owner grant cannot be replaced in the MVP"
                ));
            }

            let mut events = Vec::new();
            let now = agentfs::now_ms();
            let mut revoked = Vec::new();
            for mut existing in self
                .list_grants(&fs.fs_id)?
                .into_iter()
                .filter(|grant| grant.agent_id == target_agent_id && grant.is_active())
            {
                if existing.role == AgentFsRole::Owner {
                    anyhow::bail!(AgentFsError::grant_denied(
                        "owner grant cannot be replaced in the MVP"
                    ));
                }
                existing.revoked_at_ms = Some(now);
                existing.revoked_by = Some(actor_agent_id.to_string());
                self.update_grant(&existing)?;
                let event = agentfs::event_record(
                    &fs.fs_id,
                    "grant.revoked",
                    actor_agent_id,
                    &existing.grant_id,
                    None,
                    serde_json::json!({
                        "agent_id": existing.agent_id,
                        "reason": "replaced",
                    }),
                )?;
                let event = self.insert_event(&event)?;
                events.push(event);
                revoked.push(existing);
            }

            let grant = agentfs::grant_record(&fs, target_agent_id, role, actor_agent_id)?;
            self.insert_grant(&grant)?;
            let event = agentfs::event_record(
                &fs.fs_id,
                "grant.created",
                actor_agent_id,
                &grant.grant_id,
                None,
                serde_json::json!({
                    "agent_id": grant.agent_id,
                    "role": grant.role,
                }),
            )?;
            let event = self.insert_event(&event)?;
            events.push(event);

            Ok(GrantMutationResult {
                grant,
                revoked,
                events,
            })
        })();
        finish_transaction(&self.conn, result)
    }

    pub fn fs_revoke(
        &self,
        fs_id: &str,
        actor_agent_id: &str,
        target_agent_id: &str,
    ) -> Result<RevokeMutationResult> {
        agentfs::validate_agent_id(target_agent_id)?;
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<RevokeMutationResult> {
            let fs = self
                .find_filesystem(fs_id)?
                .ok_or_else(|| AgentFsError::unknown_fs(fs_id))?;
            self.authorize_capability_for_fs(&fs, actor_agent_id, AgentFsCapability::Manage)?;
            if target_agent_id == fs.owner_agent_id {
                anyhow::bail!(AgentFsError::grant_denied(
                    "owner grant cannot be revoked in the MVP"
                ));
            }

            let now = agentfs::now_ms();
            let mut revoked = Vec::new();
            let mut events = Vec::new();
            for mut grant in self
                .list_grants(&fs.fs_id)?
                .into_iter()
                .filter(|grant| grant.agent_id == target_agent_id && grant.is_active())
            {
                if grant.role == AgentFsRole::Owner {
                    anyhow::bail!(AgentFsError::grant_denied(
                        "owner grant cannot be revoked in the MVP"
                    ));
                }
                grant.revoked_at_ms = Some(now);
                grant.revoked_by = Some(actor_agent_id.to_string());
                self.update_grant(&grant)?;
                let event = agentfs::event_record(
                    &fs.fs_id,
                    "grant.revoked",
                    actor_agent_id,
                    &grant.grant_id,
                    None,
                    serde_json::json!({ "agent_id": grant.agent_id }),
                )?;
                let event = self.insert_event(&event)?;
                events.push(event);
                revoked.push(grant);
            }

            if revoked.is_empty() {
                anyhow::bail!(AgentFsError::grant_denied(format!(
                    "agent {target_agent_id} has no active grant on fs {}",
                    fs.fs_id
                )));
            }

            Ok(RevokeMutationResult { revoked, events })
        })();
        finish_transaction(&self.conn, result)
    }

    pub fn fs_share(
        &self,
        fs_id: &str,
        actor_agent_id: &str,
        target_agent_id: &str,
    ) -> Result<AgentFsShareResult> {
        if !self.agent_exists(target_agent_id)? {
            anyhow::bail!(AgentFsError::new(
                "unknown_agent",
                format!("agent {target_agent_id} is not known to the control service"),
                false,
            ));
        }

        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<AgentFsShareResult> {
            let resolved = self.resolve_filesystem(fs_id)?;
            self.authorize_capability_for_fs(
                &resolved.fs,
                actor_agent_id,
                AgentFsCapability::Manage,
            )?;
            let grant = self
                .list_grants(&resolved.fs.fs_id)?
                .into_iter()
                .find(|grant| {
                    grant.agent_id == target_agent_id
                        && grant.is_active()
                        && grant.capabilities.contains(&AgentFsCapability::Read)
                })
                .ok_or_else(|| {
                    AgentFsError::grant_denied(format!(
                        "agent {target_agent_id} does not have an active read grant on fs {}",
                        resolved.fs.fs_id
                    ))
                })?;
            let share = AgentFsShareRecord {
                schema_version: agentfs::SCHEMA_VERSION,
                share_id: agentfs::new_share_id()?,
                fs_id: resolved.fs.fs_id.clone(),
                target_agent_id: target_agent_id.to_string(),
                grant_id: grant.grant_id,
                role: grant.role,
                source_profile_id: resolved.fs.source_profile_id.clone(),
                created_by: actor_agent_id.to_string(),
                created_at_ms: agentfs::now_ms(),
                expires_at_ms: None,
                accepted_at_ms: None,
                revoked_at_ms: None,
            };
            self.insert_share(&share)?;
            Ok(AgentFsShareResult {
                share,
                fs: resolved.fs,
                source_profile: resolved.source_profile,
            })
        })();
        finish_transaction(&self.conn, result)
    }

    pub fn available_shares(&self, agent_id: &str) -> Result<Vec<AgentFsAvailableShare>> {
        let now = agentfs::now_ms();
        let mut stmt = self.conn.prepare(
            "SELECT share_id, fs_id, target_agent_id, grant_id, role, source_profile_id,
                    created_by, created_at_ms, expires_at_ms, accepted_at_ms, revoked_at_ms
             FROM shares
             WHERE target_agent_id = ?1
               AND accepted_at_ms IS NULL
               AND revoked_at_ms IS NULL
             ORDER BY created_at_ms, share_id",
        )?;
        let shares = stmt
            .query_map([agent_id], read_share_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let mut available = Vec::new();
        for share in shares {
            if share.expires_at_ms.is_some_and(|expires| expires <= now) {
                continue;
            }
            let resolved = self.resolve_filesystem(&share.fs_id)?;
            if self
                .authorize_capability_for_fs(&resolved.fs, agent_id, AgentFsCapability::Read)
                .is_err()
            {
                continue;
            }
            available.push(AgentFsAvailableShare {
                share,
                fs: resolved.fs,
                source_profile: resolved.source_profile,
            });
        }
        Ok(available)
    }

    pub fn accept_share(
        &self,
        share_id: &str,
        agent_id: &str,
        installation_id: &str,
    ) -> Result<(AgentFsAcceptResult, SourceConfig)> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<(AgentFsAcceptResult, SourceConfig)> {
            let mut share = self.find_share(share_id)?.ok_or_else(|| {
                AgentFsError::new(
                    "unknown_fs",
                    format!("share {share_id} was not found"),
                    false,
                )
            })?;
            if share.target_agent_id != agent_id {
                anyhow::bail!(AgentFsError::grant_denied(format!(
                    "share {share_id} is not assigned to agent {agent_id}"
                )));
            }
            if share.revoked_at_ms.is_some() {
                anyhow::bail!(AgentFsError::grant_denied(format!(
                    "share {share_id} has been revoked"
                )));
            }
            if share
                .expires_at_ms
                .is_some_and(|expires_at_ms| expires_at_ms <= agentfs::now_ms())
            {
                anyhow::bail!(AgentFsError::grant_denied(format!(
                    "share {share_id} has expired"
                )));
            }

            let resolved = self.resolve_filesystem_with_source(&share.fs_id)?;
            self.authorize_capability_for_fs(&resolved.fs, agent_id, AgentFsCapability::Read)?;
            let grant = self
                .list_grants(&resolved.fs.fs_id)?
                .into_iter()
                .find(|grant| grant.grant_id == share.grant_id && grant.is_active())
                .ok_or_else(|| {
                    AgentFsError::grant_denied(format!(
                        "share {share_id} no longer has an active backing grant"
                    ))
                })?;
            if !grant.capabilities.contains(&AgentFsCapability::Read) {
                anyhow::bail!(AgentFsError::grant_denied(format!(
                    "share {share_id} no longer grants read access"
                )));
            }

            if share.accepted_at_ms.is_none() {
                share.accepted_at_ms = Some(agentfs::now_ms());
                self.conn.execute(
                    "UPDATE shares SET accepted_at_ms = ?1 WHERE share_id = ?2",
                    params![share.accepted_at_ms, share.share_id],
                )?;
            }
            let issued =
                self.issue_credential_for_resolved(&resolved, agent_id, installation_id)?;
            Ok((
                AgentFsAcceptResult {
                    share,
                    fs: resolved.fs,
                    source_profile: issued.source_profile,
                    credential_binding: issued.binding,
                },
                issued.source,
            ))
        })();
        finish_transaction(&self.conn, result)
    }

    pub fn issue_credential(
        &self,
        fs_id: &str,
        agent_id: &str,
        installation_id: &str,
    ) -> Result<IssuedAgentFsCredential> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<IssuedAgentFsCredential> {
            let resolved = self.resolve_filesystem_with_source(fs_id)?;
            self.authorize_capability_for_fs(&resolved.fs, agent_id, AgentFsCapability::Read)?;
            self.issue_credential_for_resolved(&resolved, agent_id, installation_id)
        })();
        finish_transaction(&self.conn, result)
    }

    fn issue_credential_for_resolved(
        &self,
        resolved: &ResolvedServiceFilesystemWithSource,
        agent_id: &str,
        installation_id: &str,
    ) -> Result<IssuedAgentFsCredential> {
        let now = agentfs::now_ms();
        let binding = AgentFsCredentialBindingRecord {
            schema_version: agentfs::SCHEMA_VERSION,
            credential_binding_id: agentfs::new_credential_binding_id()?,
            fs_id: resolved.fs.fs_id.clone(),
            agent_id: agent_id.to_string(),
            installation_id: installation_id.to_string(),
            source_profile_id: resolved.fs.source_profile_id.clone(),
            issued_at_ms: now,
            expires_at_ms: now + CREDENTIAL_TTL_MS,
        };
        self.conn.execute(
            "INSERT INTO credential_bindings (
                credential_binding_id, fs_id, agent_id, installation_id, source_profile_id,
                issued_at_ms, expires_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                binding.credential_binding_id,
                binding.fs_id,
                binding.agent_id,
                binding.installation_id,
                binding.source_profile_id,
                binding.issued_at_ms,
                binding.expires_at_ms,
            ],
        )?;
        Ok(IssuedAgentFsCredential {
            binding,
            source_profile: resolved.source_profile.clone(),
            source: resolved.source.clone(),
        })
    }

    fn authorize_capability_for_fs(
        &self,
        fs: &AgentFsRecord,
        agent_id: &str,
        capability: AgentFsCapability,
    ) -> Result<AgentFsAuthorization> {
        if fs.owner_agent_id == agent_id && AgentFsRole::Owner.has_capability(capability) {
            return Ok(AgentFsAuthorization::Owner {
                agent_id: agent_id.to_string(),
            });
        }
        for grant in self.list_grants(&fs.fs_id)? {
            if grant.agent_id == agent_id
                && grant.is_active()
                && grant.capabilities.contains(&capability)
            {
                return Ok(AgentFsAuthorization::Grant {
                    grant_id: grant.grant_id,
                    role: grant.role,
                    capabilities: grant.capabilities,
                });
            }
        }
        anyhow::bail!(AgentFsError::grant_denied(format!(
            "agent {agent_id} does not have {capability:?} access to fs {}",
            fs.fs_id
        )));
    }

    fn find_agent_by_name(&self, name: &str) -> Result<Option<AgentIdentityRecord>> {
        self.conn
            .query_row(
                "SELECT agent_id, name, auth_token_hash, created_at_ms, updated_at_ms
                 FROM agents
                 WHERE name = ?1",
                [name],
                |row| {
                    Ok(AgentIdentityRecord {
                        agent_id: row.get(0)?,
                        installation_id: String::new(),
                        name: row.get(1)?,
                        auth_token: row.get(2)?,
                        created_at_ms: row.get(3)?,
                        updated_at_ms: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    fn agent_exists(&self, agent_id: &str) -> Result<bool> {
        let exists: Option<String> = self
            .conn
            .query_row(
                "SELECT agent_id FROM agents WHERE agent_id = ?1",
                [agent_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(exists.is_some())
    }

    fn installation_belongs_to_agent(&self, installation_id: &str, agent_id: &str) -> Result<bool> {
        let found: Option<String> = self
            .conn
            .query_row(
                "SELECT installation_id FROM installations
                 WHERE installation_id = ?1 AND agent_id = ?2",
                params![installation_id, agent_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(found.is_some())
    }

    fn find_filesystem(&self, fs_ref: &str) -> Result<Option<AgentFsRecord>> {
        if let Some(fs) = self
            .conn
            .query_row(
                "SELECT fs_id, name, owner_agent_id, source_profile_id, source_name, created_at_ms
                 FROM filesystems
                 WHERE fs_id = ?1",
                [fs_ref],
                read_fs_row,
            )
            .optional()?
        {
            return Ok(Some(fs));
        }

        let by_source_name = self.find_filesystems_by("source_name", fs_ref)?;
        match by_source_name.as_slice() {
            [] => {}
            [fs] => return Ok(Some(fs.clone())),
            matches => {
                let ids = matches
                    .iter()
                    .map(|fs| fs.fs_id.clone())
                    .collect::<Vec<_>>();
                anyhow::bail!(AgentFsError::ambiguous_fs_ref(fs_ref, "source_name", &ids));
            }
        }

        let by_name = self.find_filesystems_by("name", fs_ref)?;
        match by_name.as_slice() {
            [] => Ok(None),
            [fs] => Ok(Some(fs.clone())),
            matches => {
                let ids = matches
                    .iter()
                    .map(|fs| fs.fs_id.clone())
                    .collect::<Vec<_>>();
                anyhow::bail!(AgentFsError::ambiguous_fs_ref(fs_ref, "name", &ids));
            }
        }
    }

    fn find_filesystems_by(&self, field: &str, value: &str) -> Result<Vec<AgentFsRecord>> {
        let sql = match field {
            "name" => {
                "SELECT fs_id, name, owner_agent_id, source_profile_id, source_name, created_at_ms
                 FROM filesystems
                 WHERE name = ?1
                 ORDER BY fs_id"
            }
            "source_name" => {
                "SELECT fs_id, name, owner_agent_id, source_profile_id, source_name, created_at_ms
                 FROM filesystems
                 WHERE source_name = ?1
                 ORDER BY fs_id"
            }
            _ => unreachable!("unsupported AgentFS lookup field"),
        };
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map([value], read_fs_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn source_profile_by_name(
        &self,
        name: &str,
    ) -> Result<(AgentFsSourceProfileRecord, SourceConfig)> {
        self.conn
            .query_row(
                "SELECT source_profile_id, name, provider, options_json, created_at_ms, updated_at_ms
                 FROM source_profiles
                 WHERE name = ?1",
                [name],
                read_source_profile_row,
            )
            .optional()?
            .ok_or_else(|| anyhow::anyhow!("source profile {name} was not found"))
    }

    fn source_profile_by_id(
        &self,
        source_profile_id: &str,
    ) -> Result<(AgentFsSourceProfileRecord, SourceConfig)> {
        self.conn
            .query_row(
                "SELECT source_profile_id, name, provider, options_json, created_at_ms, updated_at_ms
                 FROM source_profiles
                 WHERE source_profile_id = ?1",
                [source_profile_id],
                read_source_profile_row,
            )
            .optional()?
            .ok_or_else(|| anyhow::anyhow!("source profile {source_profile_id} was not found"))
    }

    fn find_share(&self, share_id: &str) -> Result<Option<AgentFsShareRecord>> {
        self.conn
            .query_row(
                "SELECT share_id, fs_id, target_agent_id, grant_id, role, source_profile_id,
                        created_by, created_at_ms, expires_at_ms, accepted_at_ms, revoked_at_ms
                 FROM shares
                 WHERE share_id = ?1",
                [share_id],
                read_share_row,
            )
            .optional()
            .map_err(Into::into)
    }

    fn insert_grant(&self, grant: &AgentFsGrantRecord) -> Result<()> {
        self.conn.execute(
            "INSERT INTO grants (
                grant_id, fs_id, agent_id, role, capabilities_json, granted_by,
                created_at_ms, revoked_at_ms, revoked_by
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                grant.grant_id,
                grant.fs_id,
                grant.agent_id,
                serde_json::to_string(&grant.role)?,
                serde_json::to_string(&grant.capabilities)?,
                grant.granted_by,
                grant.created_at_ms,
                grant.revoked_at_ms,
                grant.revoked_by,
            ],
        )?;
        Ok(())
    }

    fn update_grant(&self, grant: &AgentFsGrantRecord) -> Result<()> {
        self.conn.execute(
            "UPDATE grants
             SET role = ?1,
                 capabilities_json = ?2,
                 revoked_at_ms = ?3,
                 revoked_by = ?4
             WHERE grant_id = ?5",
            params![
                serde_json::to_string(&grant.role)?,
                serde_json::to_string(&grant.capabilities)?,
                grant.revoked_at_ms,
                grant.revoked_by,
                grant.grant_id,
            ],
        )?;
        Ok(())
    }

    fn insert_share(&self, share: &AgentFsShareRecord) -> Result<()> {
        self.conn.execute(
            "INSERT INTO shares (
                share_id, fs_id, target_agent_id, grant_id, role, source_profile_id,
                created_by, created_at_ms, expires_at_ms, accepted_at_ms, revoked_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                share.share_id,
                share.fs_id,
                share.target_agent_id,
                share.grant_id,
                serde_json::to_string(&share.role)?,
                share.source_profile_id,
                share.created_by,
                share.created_at_ms,
                share.expires_at_ms,
                share.accepted_at_ms,
                share.revoked_at_ms,
            ],
        )?;
        Ok(())
    }

    fn insert_event(&self, event: &AgentFsEventRecord) -> Result<AgentFsEventRecord> {
        let mut event = event.clone();
        event.seq = if event.seq > 0 {
            event.seq
        } else {
            self.next_event_seq(&event.fs_id)?
        };
        self.conn.execute(
            "INSERT INTO events (
                event_id, seq, fs_id, kind, actor_agent_id, subject_id, path, data_json, created_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                event.event_id,
                event.seq,
                event.fs_id,
                event.kind,
                event.actor_agent_id,
                event.subject_id,
                event.path,
                serde_json::to_string(&event.data)?,
                event.created_at_ms,
            ],
        )?;
        Ok(event)
    }

    fn next_event_seq(&self, fs_id: &str) -> Result<i64> {
        let seq: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM events WHERE fs_id = ?1",
            [fs_id],
            |row| row.get(0),
        )?;
        Ok(seq)
    }

    fn ensure_existing_event_seqs(&self) -> Result<()> {
        let mut stmt = self.conn.prepare(
            "SELECT fs_id, event_id
             FROM events
             WHERE seq <= 0
             ORDER BY fs_id, created_at_ms, event_id",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(stmt);

        let mut next_by_fs = HashMap::<String, i64>::new();
        for (fs_id, event_id) in rows {
            let next = match next_by_fs.get(&fs_id) {
                Some(next) => *next,
                None => self.next_event_seq(&fs_id)?,
            };
            self.conn.execute(
                "UPDATE events SET seq = ?1 WHERE event_id = ?2",
                params![next, event_id],
            )?;
            next_by_fs.insert(fs_id, next + 1);
        }
        Ok(())
    }
}

fn control_service_path(config: &SectionConfig) -> PathBuf {
    config
        .control_service
        .path
        .clone()
        .unwrap_or_else(|| config.data_dir.join("control-service.db"))
}

fn finish_transaction<T>(conn: &Connection, result: Result<T>) -> Result<T> {
    match result {
        Ok(value) => {
            conn.execute_batch("COMMIT")?;
            Ok(value)
        }
        Err(err) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(err)
        }
    }
}

fn build_operator(source: &SourceConfig) -> Result<Operator> {
    let mut options = source.options.clone();
    options.retain(|key, _| !key.starts_with("section."));
    if source.provider == "webdav" {
        if let Some(endpoint) = options.get_mut("endpoint") {
            *endpoint = endpoint.trim_end_matches('/').to_string();
        }
    }

    Operator::via_iter(
        Scheme::from_str(&source.provider)
            .map_err(|err| anyhow::anyhow!("unknown provider '{}': {err}", source.provider))?,
        options,
    )
    .map_err(|err| anyhow::anyhow!("failed to build operator for '{}': {err}", source.provider))
}

fn ensure_backing_source_is_empty(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
    source_profile_name: &str,
) -> Result<()> {
    let entries = rt.block_on(async { operator.list_with("").recursive(true).await })?;
    let first_path = entries
        .into_iter()
        .map(|entry| entry.path().trim_matches('/').to_string())
        .find(|path| !path.is_empty());
    if let Some(path) = first_path {
        anyhow::bail!(
            "fs create requires an empty backing source; source profile {source_profile_name} already contains {path}"
        );
    }
    Ok(())
}

fn mirror_created_filesystem(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
    created: &FilesystemCreateResult,
) -> Result<()> {
    agentfs::ensure_event_log_ready(rt, operator, &created.fs.fs_id)?;
    agentfs::write_json(rt, operator, agentfs::FS_PATH, &created.fs)?;
    agentfs::write_json(
        rt,
        operator,
        agentfs::HEAD_PATH,
        &agentfs::head_record(&created.fs.fs_id, None),
    )?;
    agentfs::write_grant(rt, operator, &created.owner_grant)?;
    agentfs::write_event(rt, operator, &created.event)?;
    Ok(())
}

fn cleanup_created_filesystem_metadata(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
) -> Result<()> {
    for path in [agentfs::METADATA_ROOT, ".section"] {
        let path = normalized_metadata_dir(path);
        match rt.block_on(operator.remove_all(&path)) {
            Ok(()) => {}
            Err(err) if err.kind() == opendal::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
    }
    Ok(())
}

fn normalized_metadata_dir(path: &str) -> String {
    let path = path.trim_matches('/');
    if path.is_empty() {
        String::new()
    } else {
        format!("{path}/")
    }
}

fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    column_def: &str,
) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({})", table))?;
    let has_column = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(|row| row.ok())
        .any(|name| name == column);
    if !has_column {
        conn.execute_batch(&format!(
            "ALTER TABLE {} ADD COLUMN {} {};",
            table, column, column_def,
        ))?;
    }
    Ok(())
}

fn hash_auth_token(token: &str) -> String {
    let digest = digest(&SHA256, token.as_bytes());
    format!(
        "sha256:{}",
        digest
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

fn verify_auth_token(token: &str, expected_hash: &str) -> bool {
    !token.is_empty() && hash_auth_token(token) == expected_hash
}

fn read_fs_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentFsRecord> {
    Ok(AgentFsRecord {
        schema_version: agentfs::SCHEMA_VERSION,
        fs_id: row.get(0)?,
        name: row.get(1)?,
        owner_agent_id: row.get(2)?,
        source_profile_id: row.get(3)?,
        source_name: row.get(4)?,
        created_at_ms: row.get(5)?,
    })
}

fn read_grant_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentFsGrantRecord> {
    let role_json: String = row.get(3)?;
    let capabilities_json: String = row.get(4)?;
    let role = serde_json::from_str(&role_json).map_err(json_to_sql_error)?;
    let capabilities = serde_json::from_str(&capabilities_json).map_err(json_to_sql_error)?;
    Ok(AgentFsGrantRecord {
        schema_version: agentfs::SCHEMA_VERSION,
        grant_id: row.get(0)?,
        fs_id: row.get(1)?,
        agent_id: row.get(2)?,
        role,
        capabilities,
        granted_by: row.get(5)?,
        created_at_ms: row.get(6)?,
        revoked_at_ms: row.get(7)?,
        revoked_by: row.get(8)?,
    })
}

fn read_share_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentFsShareRecord> {
    let role_json: String = row.get(4)?;
    let role = serde_json::from_str(&role_json).map_err(json_to_sql_error)?;
    Ok(AgentFsShareRecord {
        schema_version: agentfs::SCHEMA_VERSION,
        share_id: row.get(0)?,
        fs_id: row.get(1)?,
        target_agent_id: row.get(2)?,
        grant_id: row.get(3)?,
        role,
        source_profile_id: row.get(5)?,
        created_by: row.get(6)?,
        created_at_ms: row.get(7)?,
        expires_at_ms: row.get(8)?,
        accepted_at_ms: row.get(9)?,
        revoked_at_ms: row.get(10)?,
    })
}

fn read_event_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentFsEventRecord> {
    let data_json: String = row.get(7)?;
    let data = serde_json::from_str(&data_json).map_err(json_to_sql_error)?;
    Ok(AgentFsEventRecord {
        schema_version: agentfs::SCHEMA_VERSION,
        event_id: row.get(0)?,
        seq: row.get(1)?,
        fs_id: row.get(2)?,
        kind: row.get(3)?,
        actor_agent_id: row.get(4)?,
        subject_id: row.get(5)?,
        path: row.get(6)?,
        data,
        created_at_ms: row.get(8)?,
    })
}

fn read_source_profile_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<(AgentFsSourceProfileRecord, SourceConfig)> {
    let options_json: String = row.get(3)?;
    let options: HashMap<String, String> =
        serde_json::from_str(&options_json).map_err(json_to_sql_error)?;
    let provider: String = row.get(2)?;
    let record = AgentFsSourceProfileRecord {
        schema_version: agentfs::SCHEMA_VERSION,
        source_profile_id: row.get(0)?,
        name: row.get(1)?,
        provider: provider.clone(),
        created_at_ms: row.get(4)?,
        updated_at_ms: row.get(5)?,
    };
    Ok((
        record,
        SourceConfig {
            provider,
            options,
            cache: Default::default(),
        },
    ))
}

fn json_to_sql_error(err: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(err))
}

#[cfg(test)]
mod tests {
    use super::*;
    use opendal::services;
    use tempfile::TempDir;

    fn config_for_profile(root: &std::path::Path, db_path: &std::path::Path) -> SectionConfig {
        let mut config = SectionConfig {
            data_dir: root.join("data"),
            ..Default::default()
        };
        config.control_service.path = Some(db_path.to_path_buf());
        config.control_service.source_profiles.insert(
            "default".to_string(),
            SourceConfig {
                provider: "fs".to_string(),
                options: HashMap::from([(
                    "root".to_string(),
                    root.join("remote").display().to_string(),
                )]),
                cache: Default::default(),
            },
        );
        config
    }

    fn allow_profile_create(config: &mut SectionConfig, value: &str) {
        config
            .control_service
            .source_profiles
            .get_mut("default")
            .expect("source profile")
            .options
            .insert(
                "section.agentfs.create_agents".to_string(),
                value.to_string(),
            );
    }

    #[test]
    fn local_harness_open_keeps_existing_source_profile() {
        let temp_dir = TempDir::new().expect("temp dir");
        let db_path = temp_dir.path().join("control.sqlite");
        let first_root = temp_dir.path().join("first");
        let second_root = temp_dir.path().join("second");

        ControlServiceStore::open(&config_for_profile(&first_root, &db_path)).expect("first open");
        let store =
            ControlServiceStore::open(&config_for_profile(&second_root, &db_path)).expect("open");
        let (_profile, source) = store
            .source_profile_by_name("default")
            .expect("source profile");
        let expected_root = first_root.join("remote").display().to_string();

        assert_eq!(source.options.get("root"), Some(&expected_root));
    }

    #[test]
    fn authoritative_open_updates_existing_source_profile() {
        let temp_dir = TempDir::new().expect("temp dir");
        let db_path = temp_dir.path().join("control.sqlite");
        let first_root = temp_dir.path().join("first");
        let second_root = temp_dir.path().join("second");

        let first_store =
            ControlServiceStore::open_authoritative(&config_for_profile(&first_root, &db_path))
                .expect("first open");
        let (first_profile, _) = first_store
            .source_profile_by_name("default")
            .expect("source profile");
        drop(first_store);

        let second_store =
            ControlServiceStore::open_authoritative(&config_for_profile(&second_root, &db_path))
                .expect("second open");
        let (second_profile, source) = second_store
            .source_profile_by_name("default")
            .expect("source profile");
        let expected_root = second_root.join("remote").display().to_string();

        assert_eq!(
            second_profile.source_profile_id,
            first_profile.source_profile_id
        );
        assert_eq!(source.options.get("root"), Some(&expected_root));
    }

    #[test]
    fn authoritative_create_requires_explicit_source_profile_policy() {
        let temp_dir = TempDir::new().expect("temp dir");
        let db_path = temp_dir.path().join("control.sqlite");
        let root = temp_dir.path().join("server");
        std::fs::create_dir_all(root.join("remote")).expect("create remote");
        let config = config_for_profile(&root, &db_path);
        let store = ControlServiceStore::open_authoritative(&config).expect("open");
        let owner = store.login_agent("owner", None, None).expect("login");

        let err = store
            .create_filesystem("project", "default", &owner)
            .expect_err("create without source profile policy should fail");

        assert!(
            err.to_string().contains("grant_denied"),
            "unexpected error: {err}"
        );
        assert!(
            !root.join("remote").join(agentfs::FS_PATH).exists(),
            "denied create must not initialize AgentFS metadata"
        );
    }

    #[test]
    fn authoritative_create_policy_allows_named_agent_and_resolve_hides_source_options() {
        let temp_dir = TempDir::new().expect("temp dir");
        let db_path = temp_dir.path().join("control.sqlite");
        let root = temp_dir.path().join("server");
        std::fs::create_dir_all(root.join("remote")).expect("create remote");
        let mut config = config_for_profile(&root, &db_path);
        allow_profile_create(&mut config, "owner");
        let store = ControlServiceStore::open_authoritative(&config).expect("open");
        let owner = store.login_agent("owner", None, None).expect("login");

        let created = store
            .create_filesystem("project", "default", &owner)
            .expect("create");
        let resolved = store
            .resolve_filesystem(&created.fs.fs_id)
            .expect("resolve filesystem");
        let resolved_json =
            serde_json::to_string(&ControlServiceResponse::ResolvedFilesystem(resolved))
                .expect("resolved json");

        assert!(
            root.join("remote").join(agentfs::FS_PATH).exists(),
            "service create should initialize AgentFS metadata"
        );
        assert!(
            !resolved_json.contains(root.join("remote").to_str().expect("utf8 remote root")),
            "resolve response must not expose source options: {resolved_json}"
        );
    }

    #[test]
    fn failed_create_metadata_mirror_cleanup_removes_partial_remote_records() {
        let temp_dir = TempDir::new().expect("temp dir");
        let remote_root = temp_dir.path().join("remote");
        std::fs::create_dir_all(&remote_root).expect("create remote root");
        let builder = services::Fs::default().root(remote_root.to_str().expect("utf8 remote root"));
        let operator = Operator::new(builder).expect("operator").finish();
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let now = agentfs::now_ms();
        let owner = AgentIdentityRecord {
            agent_id: agentfs::new_agent_id().expect("agent id"),
            installation_id: agentfs::new_installation_id().expect("installation id"),
            name: "owner".to_string(),
            auth_token: "auth_test".to_string(),
            created_at_ms: now,
            updated_at_ms: now,
        };
        let fs_record = AgentFsRecord {
            schema_version: agentfs::SCHEMA_VERSION,
            fs_id: agentfs::new_fs_id().expect("fs id"),
            name: "project".to_string(),
            owner_agent_id: owner.agent_id.clone(),
            source_profile_id: agentfs::new_source_profile_id().expect("source profile id"),
            source_name: "project-source".to_string(),
            created_at_ms: now,
        };
        let owner_grant = agentfs::owner_grant(&fs_record, &owner).expect("owner grant");
        let event = agentfs::event_record(
            &fs_record.fs_id,
            "fs.created",
            &owner.agent_id,
            &fs_record.fs_id,
            None,
            serde_json::json!({ "name": fs_record.name }),
        )
        .expect("event");
        let source_profile = crate::AgentFsSourceProfileRecord {
            schema_version: agentfs::SCHEMA_VERSION,
            source_profile_id: fs_record.source_profile_id.clone(),
            name: "test-profile".to_string(),
            provider: "fs".to_string(),
            created_at_ms: now,
            updated_at_ms: now,
        };
        let created = FilesystemCreateResult {
            fs: fs_record,
            owner_grant,
            event,
            source_profile,
        };

        let event_path = remote_root
            .join(agentfs::METADATA_ROOT)
            .join("events")
            .join(format!("{}.json", created.event.event_id));
        std::fs::create_dir_all(event_path.parent().expect("event parent")).expect("event parent");
        let mut existing_event = created.event.clone();
        existing_event.seq = 1;
        std::fs::write(
            &event_path,
            serde_json::to_vec_pretty(&existing_event).expect("event json"),
        )
        .expect("write conflicting event");

        let err = mirror_created_filesystem(&rt, &operator, &created)
            .expect_err("event conflict should fail mirror");
        assert!(
            err.to_string().contains("metadata_write_conflict"),
            "unexpected mirror error: {err}"
        );
        assert!(
            remote_root.join(agentfs::FS_PATH).exists(),
            "mirror failure should happen after fs.json is written"
        );

        cleanup_created_filesystem_metadata(&rt, &operator).expect("cleanup metadata");
        assert!(
            !remote_root.join(".section").exists(),
            "partial AgentFS metadata tree should be removed"
        );
    }
}
