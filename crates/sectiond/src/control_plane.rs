use crate::control_service::FilesystemCreateResult;
use crate::sync::{
    compare_path as sync_compare_path, list_watch_events, resolve_path as sync_resolve_path,
    sync_source as run_source_sync, sync_source_with_options as run_source_sync_with_options,
    LocalScanStats, PathCompareSnapshot, PathResolveResult, PathResolveStrategy, RemoteScanStats,
    SourceSyncOptions, SourceSyncResult, SyncLifecycleObserver,
};
use crate::{
    agentfs, AgentFsAcceptResult, AgentFsAvailableShare, AgentFsCapability,
    AgentFsCommitPathRecord, AgentFsCommitRecord, AgentFsCommitStagingRecord, AgentFsError,
    AgentFsGrantRecord, AgentFsHeadRecord, AgentFsMaterializationState, AgentFsRecord, AgentFsRole,
    AgentFsShareResult, ControlServiceStore,
};
use crate::{SectiondRuntime, SourceOrigin, StatusSnapshot};
use anyhow::{bail, Result};
use opendal::{EntryMode, Operator, Scheme};
use ring::digest::{digest, SHA256};
use section_core::config::{CacheConfig, SourceConfig};
use section_core::SectionConfig;
use section_provider::{
    AcceptedFilesystemRecord, AgentFsMountRecord, AgentIdentityRecord,
    CredentialBindingCacheRecord, PathSyncStateRecord, ProviderStore, SyncEventRecord,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

const REFRESH_XATTR_NAME: &str = "section.refresh";
const REFRESH_XATTR_NAME_LINUX: &str = "user.section.refresh";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SourceRegistryEntry {
    pub name: String,
    pub provider: String,
    pub origin: SourceOrigin,
    pub metadata_ttl_secs: u64,
    pub content_ttl_secs: u64,
    pub local_root: Option<PathBuf>,
    pub options: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RootDiscoveryMarker {
    #[serde(default = "default_root_marker_schema_version")]
    pub schema_version: u32,
    pub source_id: String,
    pub local_root: PathBuf,
    pub control_plane_endpoint: String,
    #[serde(default)]
    pub fs_id: Option<String>,
    #[serde(default)]
    pub source_profile_id: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub installation_id: Option<String>,
    #[serde(default)]
    pub base_commit_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PathDetailSnapshot {
    pub local_present: bool,
    pub dirty_local: bool,
    pub dirty_remote: bool,
    pub pinned: bool,
    pub stale: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PathInspectSnapshot {
    pub source_id: String,
    pub local_root: PathBuf,
    pub local_path: PathBuf,
    pub source_path: String,
    pub state: String,
    pub detail: PathDetailSnapshot,
    pub base_remote_version: Option<String>,
    pub current_remote_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RefreshResult {
    pub mount_active: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AgentFsAttachResult {
    pub fs: AgentFsRecord,
    pub source: AgentFsAttachedSource,
    pub head: AgentFsHeadRecord,
    pub local_root: PathBuf,
    pub sync: SourceSyncResult,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AgentFsAttachedSource {
    pub name: String,
    pub provider: String,
    pub origin: SourceOrigin,
    pub local_root: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AgentFsStatusSnapshot {
    pub fs: AgentFsRecord,
    pub head: AgentFsHeadRecord,
    pub materialization_state: Option<AgentFsMaterializationState>,
    pub local_root: Option<PathBuf>,
    pub agent_id: Option<String>,
    pub role: Option<AgentFsRole>,
    pub capabilities: Vec<AgentFsCapability>,
    pub base_commit_id: Option<String>,
    pub dirty: bool,
    pub dirty_count: usize,
    pub stale: bool,
    pub warnings: Vec<String>,
    pub next_actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AgentFsCommitStatus {
    pub fs: AgentFsRecord,
    pub source_name: String,
    pub local_root: PathBuf,
    pub base_commit_id: Option<String>,
    pub current_head_commit_id: Option<String>,
    pub stale: bool,
    pub dirty_paths: Vec<AgentFsCommitPathRecord>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AgentFsCommitApplyResult {
    pub commit: AgentFsCommitRecord,
    pub sync: SourceSyncResult,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct AgentFsStagingManifest {
    schema_version: u32,
    fs_id: String,
    commit_id: String,
    base_commit_id: Option<String>,
    created_at_ms: i64,
    paths: Vec<AgentFsStagedPathRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct AgentFsStagedPathRecord {
    path: String,
    op: String,
    kind: String,
    hash: Option<String>,
    size: Option<u64>,
    staged_path: Option<String>,
    previous_version: Option<String>,
}

struct PreparedAgentFsCommit {
    commit_id: String,
    manifest: AgentFsStagingManifest,
    manifest_path: PathBuf,
    manifest_hash: String,
    paths: Vec<AgentFsCommitPathRecord>,
}

pub struct SectiondControlPlane {
    config: SectionConfig,
    store: ProviderStore,
    control_service: ControlServiceStore,
}

impl SectiondControlPlane {
    pub fn load(config_path: Option<&Path>) -> Result<Self> {
        let config = SectionConfig::load(config_path)?;
        config.ensure_dirs()?;
        let store = ProviderStore::open(&config.data_dir)?;
        let control_service = ControlServiceStore::open(&config)?;
        Ok(Self {
            config,
            store,
            control_service,
        })
    }

    pub fn agent_register(&self, name: &str) -> Result<AgentIdentityRecord> {
        self.agent_login(name)
    }

    pub fn agent_login(&self, name: &str) -> Result<AgentIdentityRecord> {
        let existing = self.store.get_agent_identity()?;
        let existing_installation_id = existing.as_ref().and_then(|identity| {
            (identity.name == name).then_some(identity.installation_id.as_str())
        });
        let auth_token = existing
            .as_ref()
            .and_then(|identity| (identity.name == name).then_some(identity.auth_token.as_str()));
        let identity =
            self.control_service
                .login_agent(name, existing_installation_id, auth_token)?;
        self.store.cache_agent_identity(&identity)?;
        Ok(identity)
    }

    pub fn agent_identify(&self) -> Result<Option<AgentIdentityRecord>> {
        self.store.get_agent_identity()
    }

    pub fn fs_create(&self, name: &str, source_profile_name: &str) -> Result<AgentFsRecord> {
        let agent = self.require_agent_identity()?;
        let (_source_profile, source) = self
            .control_service
            .source_profile_by_name(source_profile_name)?;
        let operator = build_operator(&source.provider, &source.options)?;
        let rt = tokio::runtime::Runtime::new()?;

        if agentfs::read_optional_json::<AgentFsRecord>(&rt, &operator, agentfs::FS_PATH)?.is_some()
        {
            bail!("source profile {source_profile_name} already contains AgentFS metadata");
        }
        ensure_backing_source_is_empty(&rt, &operator, source_profile_name)?;

        let created = self
            .control_service
            .create_filesystem(name, source_profile_name, &agent)?;
        self.ensure_local_source_from_config(&created.fs.source_name, &created.source)?;
        self.cache_accepted_filesystem(&created.fs, created.event.created_at_ms)?;

        if let Err(err) = mirror_created_filesystem(&rt, &operator, &created) {
            let _ = self.source_remove(&created.fs.source_name);
            return Err(err);
        }
        Ok(created.fs)
    }

    pub fn fs_list(&self) -> Result<Vec<AgentFsRecord>> {
        let agent = self.require_agent_identity()?;
        self.control_service
            .list_filesystems_for_agent(&agent.agent_id)
    }

    pub fn fs_grant(
        &self,
        fs_ref: &str,
        agent_id: &str,
        role: AgentFsRole,
    ) -> Result<AgentFsGrantRecord> {
        let actor = self.require_agent_identity()?;
        let rt = tokio::runtime::Runtime::new()?;
        let resolved = self.find_agentfs(&rt, fs_ref)?;
        let mutation =
            self.control_service
                .fs_grant(&resolved.fs.fs_id, &actor.agent_id, agent_id, role)?;
        for revoked in &mutation.revoked {
            agentfs::write_grant(&rt, &resolved.operator, revoked)?;
        }
        agentfs::write_grant(&rt, &resolved.operator, &mutation.grant)?;
        for event in &mutation.events {
            agentfs::write_event(&rt, &resolved.operator, event)?;
        }
        Ok(mutation.grant)
    }

    pub fn fs_revoke(&self, fs_ref: &str, agent_id: &str) -> Result<Vec<AgentFsGrantRecord>> {
        let actor = self.require_agent_identity()?;
        let rt = tokio::runtime::Runtime::new()?;
        let resolved = self.find_agentfs(&rt, fs_ref)?;
        let mutation =
            self.control_service
                .fs_revoke(&resolved.fs.fs_id, &actor.agent_id, agent_id)?;
        for revoked in &mutation.revoked {
            agentfs::write_grant(&rt, &resolved.operator, revoked)?;
        }
        for event in &mutation.events {
            agentfs::write_event(&rt, &resolved.operator, event)?;
        }
        Ok(mutation.revoked)
    }

    pub fn fs_share(&self, fs_ref: &str, agent_id: &str) -> Result<AgentFsShareResult> {
        let actor = self.require_agent_identity()?;
        let rt = tokio::runtime::Runtime::new()?;
        let resolved = self.find_agentfs(&rt, fs_ref)?;
        self.control_service
            .fs_share(&resolved.fs.fs_id, &actor.agent_id, agent_id)
    }

    pub fn fs_available(&self) -> Result<Vec<AgentFsAvailableShare>> {
        let actor = self.require_agent_identity()?;
        self.control_service.available_shares(&actor.agent_id)
    }

    pub fn fs_accept(&self, share_id: &str) -> Result<AgentFsAcceptResult> {
        let actor = self.require_agent_identity()?;
        let (accepted, source) =
            self.control_service
                .accept_share(share_id, &actor.agent_id, &actor.installation_id)?;
        self.ensure_local_source_from_config(&accepted.fs.source_name, &source)?;
        self.cache_accepted_filesystem(
            &accepted.fs,
            accepted
                .share
                .accepted_at_ms
                .unwrap_or_else(agentfs::now_ms),
        )?;
        self.cache_credential_binding_record(&accepted.credential_binding)?;
        Ok(accepted)
    }

    pub fn fs_attach(&self, fs_ref: &str, local_root: &Path) -> Result<AgentFsAttachResult> {
        let actor = self.require_agent_identity()?;
        let rt = tokio::runtime::Runtime::new()?;
        let resolved = self.find_agentfs(&rt, fs_ref)?;
        let issued = self.control_service.issue_credential(
            &resolved.fs.fs_id,
            &actor.agent_id,
            &actor.installation_id,
        )?;
        self.ensure_local_source_from_config(&resolved.fs.source_name, &issued.source)?;
        self.cache_accepted_filesystem(&resolved.fs, issued.binding.issued_at_ms)?;
        self.cache_credential_binding_record(&issued.binding)?;
        ensure_head_is_materialized(&rt, &resolved.operator, &resolved.head, &resolved.fs.fs_id)?;

        let local_root = absolutize_path(local_root)?;
        ensure_attach_root_does_not_overlap_backing_root(&resolved.source, &local_root)?;
        ensure_local_root_does_not_overlap_existing(
            &self.store,
            &resolved.source.name,
            &local_root,
        )?;
        ensure_attach_root_is_empty(&local_root)?;
        let previous_root = self.store.get_source_local_root(&resolved.source.name)?;
        self.store
            .set_source_local_root(&resolved.source.name, &local_root)?;
        write_root_marker_for_agentfs(
            &resolved.source.name,
            &local_root,
            &resolved.fs.fs_id,
            &resolved.fs.source_profile_id,
            &actor.agent_id,
            &actor.installation_id,
            &self.control_service.endpoint(),
            resolved.head.commit_id.clone(),
        )?;

        self.store.clear_source_sync_state(&resolved.source.name)?;
        let sync = match self.source_sync_internal(&resolved.source.name) {
            Ok(sync) if sync.conflicts == 0 => sync,
            Ok(sync) => {
                self.rollback_failed_attach(
                    &resolved.source.name,
                    &local_root,
                    previous_root.as_deref(),
                )?;
                anyhow::bail!(
                    "fs attach could not materialize a clean working copy: {} conflict(s)",
                    sync.conflicts
                );
            }
            Err(err) => {
                self.rollback_failed_attach(
                    &resolved.source.name,
                    &local_root,
                    previous_root.as_deref(),
                )?;
                return Err(err);
            }
        };
        self.store.upsert_agentfs_mount(&AgentFsMountRecord {
            source_name: resolved.source.name.clone(),
            fs_id: resolved.fs.fs_id.clone(),
            source_profile_id: resolved.fs.source_profile_id.clone(),
            agent_id: actor.agent_id.clone(),
            installation_id: actor.installation_id.clone(),
            local_root: local_root.clone(),
            base_commit_id: resolved.head.commit_id.clone(),
            updated_at_ms: agentfs::now_ms(),
        })?;

        if let Some(previous_root) = previous_root {
            if previous_root != local_root {
                remove_root_marker(&previous_root)?;
            }
        }

        let event = agentfs::event_record(
            &resolved.fs.fs_id,
            "fs.attached",
            &actor.agent_id,
            &resolved.fs.fs_id,
            None,
            serde_json::json!({}),
        )?;
        agentfs::write_event(&rt, &resolved.operator, &event)?;

        Ok(AgentFsAttachResult {
            fs: resolved.fs,
            source: attached_source_summary(
                &self
                    .find_source(&resolved.source.name)?
                    .ok_or_else(|| anyhow::anyhow!("source disappeared after attach"))?,
            ),
            head: resolved.head,
            local_root,
            sync,
        })
    }

    pub fn fs_status(&self, fs_ref: &str) -> Result<AgentFsStatusSnapshot> {
        let rt = tokio::runtime::Runtime::new()?;
        let resolved_ref = self.agentfs_ref_from_local_marker(fs_ref)?;
        let resolved = self.find_agentfs(&rt, &resolved_ref)?;
        let materialization_state =
            head_materialization_state(&rt, &resolved.operator, &resolved.head)?;
        let agent = self.agent_identify()?;
        let role = match agent.as_ref() {
            Some(agent) => self
                .control_service
                .active_role(&resolved.fs, &agent.agent_id)?,
            None => None,
        };
        let capabilities = role.map(AgentFsRole::capabilities).unwrap_or_default();
        let local_root = self.store.get_source_local_root(&resolved.source.name)?;
        let mount = self.store.get_agentfs_mount(&resolved.source.name)?;
        let base_commit_id = mount
            .as_ref()
            .and_then(|mount| mount.base_commit_id.clone());
        let stale = local_root.is_some() && base_commit_id != resolved.head.commit_id;
        let mut warnings = Vec::new();
        let mut next_actions = Vec::new();
        let dirty_count = match local_root.as_ref() {
            Some(local_root) => collect_dirty_paths(&rt, &resolved.operator, local_root)?.len(),
            None => 0,
        };
        let dirty = dirty_count > 0;

        if agent.is_none() {
            warnings.push("agent is not logged in".to_string());
            next_actions.push("login".to_string());
        }
        if local_root.is_none() {
            warnings.push("fs is not attached on this installation".to_string());
            next_actions.push("attach".to_string());
        }
        match materialization_state {
            Some(AgentFsMaterializationState::Pending)
            | Some(AgentFsMaterializationState::FailedToMaterialize) => {
                warnings.push("head commit is not materialized".to_string());
                next_actions.push("repair".to_string());
            }
            _ => {}
        }
        if stale {
            warnings.push("local base is behind current head".to_string());
            next_actions.push("sync".to_string());
        }
        if !capabilities.contains(&AgentFsCapability::Commit) {
            next_actions.push("request_grant".to_string());
        } else if dirty
            && !stale
            && matches!(
                materialization_state,
                None | Some(AgentFsMaterializationState::Materialized)
            )
        {
            next_actions.push("commit".to_string());
        }
        dedup_strings(&mut warnings);
        dedup_strings(&mut next_actions);

        Ok(AgentFsStatusSnapshot {
            fs: resolved.fs,
            head: resolved.head,
            materialization_state,
            local_root,
            agent_id: agent.map(|agent| agent.agent_id),
            role,
            capabilities,
            base_commit_id,
            dirty,
            dirty_count,
            stale,
            warnings,
            next_actions,
        })
    }

    pub fn fs_events(
        &self,
        fs_ref: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<crate::AgentFsEventRecord>> {
        let rt = tokio::runtime::Runtime::new()?;
        let resolved_ref = self.agentfs_ref_from_local_marker(fs_ref)?;
        let resolved = self.find_agentfs(&rt, &resolved_ref)?;
        let events = agentfs::list_events(&rt, &resolved.operator)?;
        let after_seq = parse_event_offset(after, &events)?;
        Ok(events
            .into_iter()
            .filter(|event| event.seq > after_seq)
            .take(limit)
            .collect())
    }

    pub fn commit_status(&self, input_path: &Path) -> Result<AgentFsCommitStatus> {
        let actor = self.require_agent_identity()?;
        let rt = tokio::runtime::Runtime::new()?;
        let marker = discover_root_marker(&absolutize_path(input_path)?)?;
        let fs_id = marker
            .fs_id
            .clone()
            .ok_or_else(|| AgentFsError::unknown_fs(&marker.source_id))?;
        let resolved = self.find_agentfs(&rt, &fs_id)?;
        self.control_service.authorize_capability(
            &resolved.fs.fs_id,
            &actor.agent_id,
            AgentFsCapability::Read,
        )?;
        ensure_head_is_materialized(&rt, &resolved.operator, &resolved.head, &resolved.fs.fs_id)?;

        let mount = local_mount_from_marker(&self.store, &resolved.source.name, &marker)?;
        let local_root = mount.local_root.clone();
        let dirty_paths = collect_dirty_paths(&rt, &resolved.operator, &local_root)?;
        let stale = mount.base_commit_id != resolved.head.commit_id;

        Ok(AgentFsCommitStatus {
            fs: resolved.fs,
            source_name: resolved.source.name,
            local_root,
            base_commit_id: mount.base_commit_id,
            current_head_commit_id: resolved.head.commit_id,
            stale,
            dirty_paths,
        })
    }

    pub fn commit_apply(
        &self,
        input_path: &Path,
        message: &str,
    ) -> Result<AgentFsCommitApplyResult> {
        let summary = message.trim();
        if summary.is_empty() {
            bail!("commit summary must not be empty");
        }

        let actor = self.require_agent_identity()?;
        let rt = tokio::runtime::Runtime::new()?;
        let marker = discover_root_marker(&absolutize_path(input_path)?)?;
        let fs_id = marker
            .fs_id
            .clone()
            .ok_or_else(|| AgentFsError::unknown_fs(&marker.source_id))?;
        let resolved = self.find_agentfs(&rt, &fs_id)?;
        self.control_service.authorize_capability(
            &resolved.fs.fs_id,
            &actor.agent_id,
            AgentFsCapability::Commit,
        )?;
        ensure_head_is_materialized(&rt, &resolved.operator, &resolved.head, &resolved.fs.fs_id)?;

        let mount = local_mount_from_marker(&self.store, &resolved.source.name, &marker)?;
        if mount.base_commit_id != resolved.head.commit_id {
            anyhow::bail!(AgentFsError::stale_base(
                &resolved.fs.fs_id,
                mount.base_commit_id.as_deref(),
                resolved.head.commit_id.as_deref(),
            ));
        }

        let local_root = mount.local_root.clone();
        let dirty_paths = collect_dirty_paths(&rt, &resolved.operator, &local_root)?;
        if dirty_paths.is_empty() {
            bail!(
                "empty commit: no dirty paths under {}",
                local_root.display()
            );
        }
        let commit_id = agentfs::new_commit_id()?;
        let prepared = prepare_staging_snapshot(
            &self.config.data_dir,
            &resolved.fs.fs_id,
            &commit_id,
            &mount.base_commit_id,
            &local_root,
            &dirty_paths,
        )?;

        let lock = agentfs::acquire_head_lock(
            &rt,
            &resolved.operator,
            &resolved.fs.fs_id,
            &actor.agent_id,
        )?;
        let accepted = (|| -> Result<AgentFsCommitRecord> {
            let current_head = agentfs::read_json::<AgentFsHeadRecord>(
                &rt,
                &resolved.operator,
                agentfs::HEAD_PATH,
            )?;
            let authorized_by = self.control_service.authorize_capability(
                &resolved.fs.fs_id,
                &actor.agent_id,
                AgentFsCapability::Commit,
            )?;
            ensure_head_is_materialized(
                &rt,
                &resolved.operator,
                &current_head,
                &resolved.fs.fs_id,
            )?;
            if mount.base_commit_id != current_head.commit_id {
                anyhow::bail!(AgentFsError::stale_base(
                    &resolved.fs.fs_id,
                    mount.base_commit_id.as_deref(),
                    current_head.commit_id.as_deref(),
                ));
            }

            let commit = AgentFsCommitRecord {
                schema_version: agentfs::SCHEMA_VERSION,
                commit_id: prepared.commit_id.clone(),
                fs_id: resolved.fs.fs_id.clone(),
                parent_commit_id: current_head.commit_id.clone(),
                base_commit_id: mount.base_commit_id.clone(),
                base_manifest_hash: None,
                agent_id: actor.agent_id.clone(),
                summary: summary.to_string(),
                paths: prepared.paths.clone(),
                authorized_by: Some(authorized_by.clone()),
                staging_snapshot: Some(AgentFsCommitStagingRecord {
                    manifest_path: prepared.manifest_path.display().to_string(),
                    manifest_hash: prepared.manifest_hash.clone(),
                }),
                created_at_ms: agentfs::now_ms(),
                materialization_state: AgentFsMaterializationState::Pending,
                materialized_at_ms: None,
                error: None,
            };

            agentfs::write_commit(&rt, &resolved.operator, &commit)?;
            agentfs::write_json(
                &rt,
                &resolved.operator,
                agentfs::HEAD_PATH,
                &agentfs::head_record(&resolved.fs.fs_id, Some(commit.commit_id.clone())),
            )?;
            let accepted_event = agentfs::event_record(
                &resolved.fs.fs_id,
                "commit.accepted",
                &actor.agent_id,
                &commit.commit_id,
                None,
                serde_json::json!({
                    "summary": commit.summary,
                    "paths": commit.paths,
                    "authorized_by": authorized_by,
                }),
            )?;
            agentfs::write_event(&rt, &resolved.operator, &accepted_event)?;
            Ok(commit)
        })();
        agentfs::release_head_lock(&rt, &resolved.operator, &lock)?;
        let mut commit = accepted?;

        match materialize_staged_commit(
            &rt,
            &resolved.operator,
            &self.store,
            &resolved.source.name,
            &local_root,
            prepared
                .manifest_path
                .parent()
                .ok_or_else(|| AgentFsError::missing_commit_snapshot(&commit.commit_id))?,
            &prepared.manifest,
        ) {
            Ok(sync) if sync.conflicts == 0 => {
                commit.materialization_state = AgentFsMaterializationState::Materialized;
                commit.materialized_at_ms = Some(agentfs::now_ms());
                agentfs::write_commit(&rt, &resolved.operator, &commit)?;
                let materialized_event = agentfs::event_record(
                    &resolved.fs.fs_id,
                    "commit.materialized",
                    &actor.agent_id,
                    &commit.commit_id,
                    None,
                    serde_json::json!({ "pushed": sync.pushed, "pulled": sync.pulled }),
                )?;
                agentfs::write_event(&rt, &resolved.operator, &materialized_event)?;
                write_root_marker_for_agentfs(
                    &resolved.source.name,
                    &local_root,
                    &resolved.fs.fs_id,
                    &resolved.fs.source_profile_id,
                    &actor.agent_id,
                    &actor.installation_id,
                    &self.control_service.endpoint(),
                    Some(commit.commit_id.clone()),
                )?;
                self.store.upsert_agentfs_mount(&AgentFsMountRecord {
                    source_name: resolved.source.name.clone(),
                    fs_id: resolved.fs.fs_id.clone(),
                    source_profile_id: resolved.fs.source_profile_id.clone(),
                    agent_id: actor.agent_id.clone(),
                    installation_id: actor.installation_id.clone(),
                    local_root: local_root.clone(),
                    base_commit_id: Some(commit.commit_id.clone()),
                    updated_at_ms: agentfs::now_ms(),
                })?;
                Ok(AgentFsCommitApplyResult { commit, sync })
            }
            Ok(sync) => {
                commit.materialization_state = AgentFsMaterializationState::FailedToMaterialize;
                commit.error = Some(format!(
                    "source sync reported {} conflict(s)",
                    sync.conflicts
                ));
                agentfs::write_commit(&rt, &resolved.operator, &commit)?;
                write_materialization_failed_event(
                    &rt,
                    &resolved.operator,
                    &resolved.fs.fs_id,
                    &actor.agent_id,
                    &commit,
                )?;
                anyhow::bail!(AgentFsError::materialization_failed(
                    &resolved.fs.fs_id,
                    commit.error.as_deref().unwrap_or("source sync failed"),
                ));
            }
            Err(err) => {
                commit.materialization_state = AgentFsMaterializationState::FailedToMaterialize;
                commit.error = Some(err.to_string());
                agentfs::write_commit(&rt, &resolved.operator, &commit)?;
                write_materialization_failed_event(
                    &rt,
                    &resolved.operator,
                    &resolved.fs.fs_id,
                    &actor.agent_id,
                    &commit,
                )?;
                Err(AgentFsError::materialization_failed(
                    &resolved.fs.fs_id,
                    commit.error.as_deref().unwrap_or("source sync failed"),
                )
                .into())
            }
        }
    }

    pub fn commit_repair(
        &self,
        fs_ref: &str,
        commit_id: Option<&str>,
    ) -> Result<AgentFsCommitApplyResult> {
        let actor = self.require_agent_identity()?;
        let rt = tokio::runtime::Runtime::new()?;
        let resolved_ref = self.agentfs_ref_from_local_marker(fs_ref)?;
        let resolved = self.find_agentfs(&rt, &resolved_ref)?;
        self.control_service.authorize_capability(
            &resolved.fs.fs_id,
            &actor.agent_id,
            AgentFsCapability::Commit,
        )?;

        let target_commit_id = match commit_id {
            Some(commit_id) => commit_id.to_string(),
            None => resolved
                .head
                .commit_id
                .clone()
                .ok_or_else(|| AgentFsError::unknown_fs(&resolved.fs.fs_id))?,
        };
        let mut commit: AgentFsCommitRecord = agentfs::read_json(
            &rt,
            &resolved.operator,
            &format!(
                "{}/commits/{}.json",
                agentfs::METADATA_ROOT,
                target_commit_id
            ),
        )?;
        if commit.fs_id != resolved.fs.fs_id {
            anyhow::bail!(AgentFsError::malformed_metadata(format!(
                "commit {} belongs to fs {}, not {}",
                commit.commit_id, commit.fs_id, resolved.fs.fs_id
            )));
        }
        if resolved.head.commit_id.as_deref() != Some(commit.commit_id.as_str()) {
            anyhow::bail!(AgentFsError::stale_base(
                &resolved.fs.fs_id,
                Some(&commit.commit_id),
                resolved.head.commit_id.as_deref(),
            ));
        }
        if !matches!(
            commit.materialization_state,
            AgentFsMaterializationState::Pending | AgentFsMaterializationState::FailedToMaterialize
        ) {
            anyhow::bail!(AgentFsError::new(
                "invalid_commit_state",
                format!(
                    "commit {} is {:?}, so it does not need repair",
                    commit.commit_id, commit.materialization_state
                ),
                false,
            ));
        }

        let staging = commit
            .staging_snapshot
            .clone()
            .ok_or_else(|| AgentFsError::missing_commit_snapshot(&commit.commit_id))?;
        let manifest_path = PathBuf::from(&staging.manifest_path);
        let manifest = read_staging_manifest(&manifest_path, &staging.manifest_hash)?;
        let staging_root = manifest_path
            .parent()
            .ok_or_else(|| AgentFsError::missing_commit_snapshot(&commit.commit_id))?;
        let local_root = resolved
            .source
            .local_root
            .clone()
            .unwrap_or_else(PathBuf::new);

        match materialize_staged_commit(
            &rt,
            &resolved.operator,
            &self.store,
            &resolved.source.name,
            &local_root,
            staging_root,
            &manifest,
        ) {
            Ok(sync) if sync.conflicts == 0 => {
                commit.materialization_state = AgentFsMaterializationState::Materialized;
                commit.materialized_at_ms = Some(agentfs::now_ms());
                commit.error = None;
                agentfs::write_commit(&rt, &resolved.operator, &commit)?;
                let materialized_event = agentfs::event_record(
                    &resolved.fs.fs_id,
                    "commit.materialized",
                    &actor.agent_id,
                    &commit.commit_id,
                    None,
                    serde_json::json!({ "pushed": sync.pushed, "pulled": sync.pulled, "repair": true }),
                )?;
                agentfs::write_event(&rt, &resolved.operator, &materialized_event)?;
                if !local_root.as_os_str().is_empty() {
                    write_root_marker_for_agentfs(
                        &resolved.source.name,
                        &local_root,
                        &resolved.fs.fs_id,
                        &resolved.fs.source_profile_id,
                        &actor.agent_id,
                        &actor.installation_id,
                        &self.control_service.endpoint(),
                        Some(commit.commit_id.clone()),
                    )?;
                    self.store.upsert_agentfs_mount(&AgentFsMountRecord {
                        source_name: resolved.source.name.clone(),
                        fs_id: resolved.fs.fs_id.clone(),
                        source_profile_id: resolved.fs.source_profile_id.clone(),
                        agent_id: actor.agent_id.clone(),
                        installation_id: actor.installation_id.clone(),
                        local_root: local_root.clone(),
                        base_commit_id: Some(commit.commit_id.clone()),
                        updated_at_ms: agentfs::now_ms(),
                    })?;
                }
                Ok(AgentFsCommitApplyResult { commit, sync })
            }
            Ok(sync) => {
                commit.materialization_state = AgentFsMaterializationState::FailedToMaterialize;
                commit.error = Some(format!(
                    "repair source materialization reported {} conflict(s)",
                    sync.conflicts
                ));
                agentfs::write_commit(&rt, &resolved.operator, &commit)?;
                write_materialization_failed_event(
                    &rt,
                    &resolved.operator,
                    &resolved.fs.fs_id,
                    &actor.agent_id,
                    &commit,
                )?;
                anyhow::bail!(AgentFsError::materialization_failed(
                    &resolved.fs.fs_id,
                    commit.error.as_deref().unwrap_or("repair failed"),
                ));
            }
            Err(err) => {
                commit.materialization_state = AgentFsMaterializationState::FailedToMaterialize;
                commit.error = Some(err.to_string());
                agentfs::write_commit(&rt, &resolved.operator, &commit)?;
                write_materialization_failed_event(
                    &rt,
                    &resolved.operator,
                    &resolved.fs.fs_id,
                    &actor.agent_id,
                    &commit,
                )?;
                Err(AgentFsError::materialization_failed(
                    &resolved.fs.fs_id,
                    commit.error.as_deref().unwrap_or("repair failed"),
                )
                .into())
            }
        }
    }

    pub fn source_add(
        &self,
        name: &str,
        provider: &str,
        options: HashMap<String, String>,
    ) -> Result<SourceRegistryEntry> {
        self.ensure_name_is_not_config_owned(name, "change")?;
        self.ensure_name_is_not_agentfs_owned(name, "change")?;

        let source = SourceConfig {
            provider: provider.to_string(),
            options,
            cache: CacheConfig::default(),
        };
        self.store.add_source(name, &source)?;
        self.find_source(name)?
            .ok_or_else(|| anyhow::anyhow!("source {name} was added but could not be reloaded"))
    }

    pub fn source_remove(&self, name: &str) -> Result<()> {
        self.ensure_name_is_not_config_owned(name, "remove")?;
        self.ensure_source_is_not_agentfs_owned(name, "remove")?;
        let previous_root = self.store.get_source_local_root(name)?;
        self.store.remove_source(name)?;
        self.store.remove_agentfs_mount(name)?;
        if let Some(previous_root) = previous_root {
            remove_root_marker(&previous_root)?;
        }
        Ok(())
    }

    pub fn list_sources(&self) -> Result<Vec<SourceRegistryEntry>> {
        self.list_sources_internal(false)
    }

    fn list_sources_internal(&self, include_agentfs: bool) -> Result<Vec<SourceRegistryEntry>> {
        let store_sources = self.store.load_all()?;
        let binding_map = self
            .store
            .list_source_local_roots()?
            .into_iter()
            .map(|binding| (binding.source_name, binding.local_root))
            .collect::<HashMap<_, _>>();
        let mut source_names = BTreeSet::new();
        source_names.extend(self.config.sources.keys().cloned());
        source_names.extend(store_sources.keys().cloned());

        let mut entries = Vec::with_capacity(source_names.len());
        for name in source_names {
            if !include_agentfs && self.store.is_agentfs_source(&name)? {
                continue;
            }
            let origin = match (
                self.config.sources.contains_key(&name),
                store_sources.contains_key(&name),
            ) {
                (true, true) => SourceOrigin::ConfigPreferredWithStoreFallback,
                (true, false) => SourceOrigin::ConfigFile,
                (false, true) => SourceOrigin::ProviderStore,
                (false, false) => unreachable!("source should exist in config or store"),
            };

            let effective_source = match origin {
                SourceOrigin::ConfigFile | SourceOrigin::ConfigPreferredWithStoreFallback => self
                    .config
                    .sources
                    .get(&name)
                    .expect("config-owned source should exist in config"),
                SourceOrigin::ProviderStore => store_sources
                    .get(&name)
                    .expect("store-owned source should exist in provider store"),
            };

            entries.push(SourceRegistryEntry {
                local_root: binding_map.get(&name).cloned(),
                name,
                provider: effective_source.provider.clone(),
                origin,
                metadata_ttl_secs: effective_source.cache.metadata_ttl_secs,
                content_ttl_secs: effective_source.cache.content_ttl_secs,
                options: effective_source
                    .options
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
            });
        }

        Ok(entries)
    }

    pub fn source_bind_local_root(
        &self,
        name: &str,
        local_root: &Path,
    ) -> Result<SourceRegistryEntry> {
        self.ensure_public_source_exists(name)?;

        let local_root = absolutize_path(local_root)?;
        ensure_local_root_does_not_overlap_existing(&self.store, name, &local_root)?;
        let previous_root = self.store.get_source_local_root(name)?;
        self.store.set_source_local_root(name, &local_root)?;
        if previous_root.as_ref() != Some(&local_root) {
            self.store.clear_source_sync_state(name)?;
        }
        write_root_marker(name, &local_root)?;

        if let Some(previous_root) = previous_root {
            if previous_root != local_root {
                remove_root_marker(&previous_root)?;
            }
        }

        self.find_source(name)?
            .ok_or_else(|| anyhow::anyhow!("source {name} was bound but could not be reloaded"))
    }

    pub fn source_unbind_local_root(&self, name: &str) -> Result<()> {
        self.ensure_public_source_exists(name)?;

        if let Some(previous_root) = self.store.get_source_local_root(name)? {
            self.store.remove_source_local_root(name)?;
            self.store.remove_agentfs_mount(name)?;
            remove_root_marker(&previous_root)?;
        }

        Ok(())
    }

    pub fn path_inspect(&self, input_path: &Path) -> Result<PathInspectSnapshot> {
        let local_path = absolutize_path(input_path)?;
        let marker = discover_root_marker(&local_path)?;
        self.ensure_source_is_not_agentfs_owned(
            &marker.source_id,
            "inspect through low-level path commands",
        )?;
        let authoritative_root = self
            .store
            .get_source_local_root(&marker.source_id)?
            .ok_or_else(|| {
                anyhow::anyhow!("source {} has no bound local root", marker.source_id)
            })?;

        let source_path = relative_source_path(&authoritative_root, &local_path)?;
        let state = self
            .store
            .get_path_sync_state(&marker.source_id, &source_path)?
            .unwrap_or_else(|| default_path_state(&marker.source_id, &source_path, &local_path));

        Ok(PathInspectSnapshot {
            source_id: marker.source_id,
            local_root: authoritative_root,
            local_path,
            source_path: display_source_path(&source_path),
            state: state.public_state,
            detail: PathDetailSnapshot {
                local_present: state.local_present,
                dirty_local: state.dirty_local,
                dirty_remote: state.dirty_remote,
                pinned: state.pinned,
                stale: state.stale,
            },
            base_remote_version: state.base_remote_version,
            current_remote_version: state.current_remote_version,
        })
    }

    pub fn source_sync(&self, name: &str) -> Result<SourceSyncResult> {
        self.ensure_public_source_exists(name)?;
        let local_root = self.source_local_root(name)?;
        let runtime = self.runtime()?;
        run_source_sync(&runtime, &self.store, name, &local_root)
    }

    pub fn source_sync_with_options(
        &self,
        name: &str,
        options: &SourceSyncOptions,
        lifecycle: Option<SyncLifecycleObserver>,
    ) -> Result<SourceSyncResult> {
        self.ensure_public_source_exists(name)?;
        let local_root = self.source_local_root(name)?;
        let runtime = self.runtime()?;
        run_source_sync_with_options(&runtime, &self.store, name, &local_root, options, lifecycle)
    }

    pub fn source_local_root(&self, name: &str) -> Result<PathBuf> {
        self.ensure_public_source_exists(name)?;
        self.store
            .get_source_local_root(name)?
            .ok_or_else(|| anyhow::anyhow!("source {name} has no bound local root"))
    }

    fn source_sync_internal(&self, name: &str) -> Result<SourceSyncResult> {
        self.ensure_source_exists(name)?;
        let local_root = self
            .store
            .get_source_local_root(name)?
            .ok_or_else(|| anyhow::anyhow!("source {name} has no bound local root"))?;
        let runtime = self.runtime_including_agentfs()?;
        run_source_sync(&runtime, &self.store, name, &local_root)
    }

    pub fn path_compare(&self, input_path: &Path) -> Result<PathCompareSnapshot> {
        let (source_id, local_root, local_path, source_path) =
            self.resolve_local_path(input_path)?;
        let runtime = self.runtime()?;
        sync_compare_path(
            &runtime,
            &self.store,
            &source_id,
            &local_root,
            &local_path,
            &source_path,
        )
    }

    pub fn path_resolve(
        &self,
        input_path: &Path,
        strategy: PathResolveStrategy,
    ) -> Result<PathResolveResult> {
        let (source_id, local_root, local_path, source_path) =
            self.resolve_local_path(input_path)?;
        let runtime = self.runtime()?;
        sync_resolve_path(
            &runtime,
            &self.store,
            &source_id,
            &local_root,
            &local_path,
            &source_path,
            strategy,
        )
    }

    pub fn watch_path(&self, input_path: &Path, after_id: i64) -> Result<Vec<SyncEventRecord>> {
        let (source_id, _local_root, _local_path, source_path) =
            self.resolve_local_path(input_path)?;
        list_watch_events(&self.store, &source_id, &source_path, after_id)
    }

    pub fn watch_agentfs_events(
        &self,
        fs_ref: &str,
        after_seq: i64,
        limit: usize,
    ) -> Result<Vec<crate::AgentFsEventRecord>> {
        self.fs_events(fs_ref, Some(&after_seq.to_string()), limit)
    }

    pub fn status_snapshot(&self) -> Result<StatusSnapshot> {
        let mount_active = is_mount_active(&self.config.mount_point);
        self.runtime()?
            .status_snapshot(&self.config.mount_point, mount_active)
    }

    pub fn refresh_path(&self, path: &str) -> Result<RefreshResult> {
        let mount_active = is_mount_active(&self.config.mount_point);
        let message = if mount_active {
            let mount_target = refresh_mount_target(&self.config.mount_point, path);
            let data = trigger_refresh_xattr(&mount_target)?;
            let response = String::from_utf8_lossy(&data);
            if response.trim().is_empty() || response.trim() == "ok" {
                format!("Cache refreshed for {path}")
            } else {
                format!("Cache refreshed for {path}: {}", response.trim())
            }
        } else {
            format!(
                "No active mount at {}; CLI has no persistent cache to invalidate for {path}",
                self.config.mount_point.display()
            )
        };

        Ok(RefreshResult {
            mount_active,
            message,
        })
    }

    fn ensure_local_source_from_config(&self, name: &str, source: &SourceConfig) -> Result<()> {
        self.ensure_name_is_not_config_owned(name, "change")?;
        self.store.add_source(name, source)?;
        Ok(())
    }

    fn cache_accepted_filesystem(&self, fs: &AgentFsRecord, accepted_at_ms: i64) -> Result<()> {
        self.store
            .cache_accepted_filesystem(&AcceptedFilesystemRecord {
                fs_id: fs.fs_id.clone(),
                name: fs.name.clone(),
                owner_agent_id: fs.owner_agent_id.clone(),
                source_profile_id: fs.source_profile_id.clone(),
                source_name: fs.source_name.clone(),
                accepted_at_ms,
            })?;
        Ok(())
    }

    fn cache_credential_binding_record(
        &self,
        binding: &crate::AgentFsCredentialBindingRecord,
    ) -> Result<()> {
        self.store
            .cache_credential_binding(&CredentialBindingCacheRecord {
                credential_binding_id: binding.credential_binding_id.clone(),
                fs_id: binding.fs_id.clone(),
                agent_id: binding.agent_id.clone(),
                installation_id: binding.installation_id.clone(),
                source_profile_id: binding.source_profile_id.clone(),
                issued_at_ms: binding.issued_at_ms,
                expires_at_ms: binding.expires_at_ms,
            })?;
        Ok(())
    }

    fn runtime(&self) -> Result<SectiondRuntime> {
        SectiondRuntime::from_config_and_store(&self.config, &self.store)
    }

    fn runtime_including_agentfs(&self) -> Result<SectiondRuntime> {
        SectiondRuntime::from_config_and_store_including_agentfs(&self.config, &self.store)
    }

    fn find_source(&self, name: &str) -> Result<Option<SourceRegistryEntry>> {
        Ok(self
            .list_sources_internal(true)?
            .into_iter()
            .find(|entry| entry.name == name))
    }

    fn ensure_source_exists(&self, name: &str) -> Result<()> {
        if self.find_source(name)?.is_none() {
            bail!("source {name} is not registered");
        }
        Ok(())
    }

    fn ensure_public_source_exists(&self, name: &str) -> Result<()> {
        self.ensure_source_exists(name)?;
        self.ensure_source_is_not_agentfs_owned(name, "use through low-level source commands")?;
        Ok(())
    }

    fn ensure_name_is_not_agentfs_owned(&self, name: &str, action: &str) -> Result<()> {
        if self.store.is_agentfs_source(name)? {
            bail!(
                "Source '{name}' is managed by AgentFS and cannot be changed through low-level source commands; use AgentFS commands to {action} it."
            );
        }
        Ok(())
    }

    fn ensure_source_is_not_agentfs_owned(&self, name: &str, action: &str) -> Result<()> {
        if self.store.is_agentfs_source(name)? {
            bail!(
                "Source '{name}' is managed by AgentFS and cannot be used through low-level source commands; use AgentFS commands to {action} it."
            );
        }
        Ok(())
    }

    fn resolve_local_path(&self, input_path: &Path) -> Result<(String, PathBuf, PathBuf, String)> {
        let local_path = absolutize_path(input_path)?;
        let marker = discover_root_marker(&local_path)?;
        self.ensure_source_is_not_agentfs_owned(
            &marker.source_id,
            "use through low-level path commands",
        )?;
        let local_root = self
            .store
            .get_source_local_root(&marker.source_id)?
            .ok_or_else(|| {
                anyhow::anyhow!("source {} has no bound local root", marker.source_id)
            })?;
        let source_path = relative_source_path(&local_root, &local_path)?;
        Ok((marker.source_id, local_root, local_path, source_path))
    }

    fn ensure_name_is_not_config_owned(&self, name: &str, action: &str) -> Result<()> {
        if self.config.sources.contains_key(name) {
            bail!(
                "Source '{name}' is defined in the config file and cannot be changed via the control plane. Edit the config file to {action} it."
            );
        }

        Ok(())
    }

    fn require_agent_identity(&self) -> Result<AgentIdentityRecord> {
        self.store
            .get_agent_identity()?
            .ok_or_else(|| AgentFsError::unknown_agent().into())
    }

    fn find_agentfs(&self, rt: &tokio::runtime::Runtime, fs_ref: &str) -> Result<ResolvedAgentFs> {
        let resolved = self.control_service.resolve_filesystem(fs_ref)?;
        let operator = build_operator(&resolved.source.provider, &resolved.source.options)?;
        let source = SourceRegistryEntry {
            name: resolved.fs.source_name.clone(),
            provider: resolved.source.provider.clone(),
            origin: SourceOrigin::ProviderStore,
            metadata_ttl_secs: resolved.source.cache.metadata_ttl_secs,
            content_ttl_secs: resolved.source.cache.content_ttl_secs,
            local_root: self.store.get_source_local_root(&resolved.fs.source_name)?,
            options: resolved
                .source
                .options
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        };
        let head = agentfs::read_json::<AgentFsHeadRecord>(rt, &operator, agentfs::HEAD_PATH)?;
        Ok(ResolvedAgentFs {
            source,
            fs: resolved.fs,
            head,
            operator,
        })
    }

    fn agentfs_ref_from_local_marker(&self, fs_ref: &str) -> Result<String> {
        let path = Path::new(fs_ref);
        let looks_like_path = path.is_absolute()
            || fs_ref.starts_with('.')
            || fs_ref.contains(std::path::MAIN_SEPARATOR);
        if !looks_like_path && !path.exists() {
            return Ok(fs_ref.to_string());
        }

        match discover_root_marker(&absolutize_path(path)?) {
            Ok(marker) => Ok(marker.fs_id.unwrap_or(marker.source_id)),
            Err(_) if looks_like_path => Err(AgentFsError::unknown_fs(fs_ref).into()),
            Err(_) => Ok(fs_ref.to_string()),
        }
    }

    fn rollback_failed_attach(
        &self,
        source_name: &str,
        local_root: &Path,
        previous_root: Option<&Path>,
    ) -> Result<()> {
        remove_root_marker(local_root)?;
        match previous_root {
            Some(previous_root) => {
                self.store
                    .set_source_local_root(source_name, previous_root)?;
            }
            None => {
                self.store.remove_source_local_root(source_name)?;
            }
        }
        self.store.clear_source_sync_state(source_name)?;
        Ok(())
    }
}

struct ResolvedAgentFs {
    source: SourceRegistryEntry,
    fs: AgentFsRecord,
    head: AgentFsHeadRecord,
    operator: opendal::Operator,
}

fn parse_event_offset(after: Option<&str>, events: &[crate::AgentFsEventRecord]) -> Result<i64> {
    let Some(after) = after else {
        return Ok(0);
    };
    let after = after.trim();
    if after.is_empty() {
        return Ok(0);
    }
    if let Ok(seq) = after.parse::<i64>() {
        return Ok(seq);
    }
    events
        .iter()
        .find(|event| event.event_id == after)
        .map(|event| event.seq)
        .ok_or_else(|| anyhow::anyhow!("AgentFS event offset {after} was not found"))
}

fn dedup_strings(values: &mut Vec<String>) {
    let mut seen = BTreeSet::new();
    values.retain(|value| seen.insert(value.clone()));
}

fn attached_source_summary(source: &SourceRegistryEntry) -> AgentFsAttachedSource {
    AgentFsAttachedSource {
        name: source.name.clone(),
        provider: source.provider.clone(),
        origin: source.origin,
        local_root: source.local_root.clone(),
    }
}

fn mirror_created_filesystem(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
    created: &FilesystemCreateResult,
) -> Result<()> {
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

fn build_operator(provider: &str, options: &HashMap<String, String>) -> Result<Operator> {
    let mut options = options.clone();
    options.retain(|key, _| !key.starts_with("section."));
    if provider == "webdav" {
        if let Some(endpoint) = options.get_mut("endpoint") {
            *endpoint = endpoint.trim_end_matches('/').to_string();
        }
    }

    Operator::via_iter(
        Scheme::from_str(provider)
            .map_err(|err| anyhow::anyhow!("unknown provider '{provider}': {err}"))?,
        options.into_iter(),
    )
    .map_err(|err| anyhow::anyhow!("failed to build operator for '{provider}': {err}"))
}

fn ensure_backing_source_is_empty(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
    source_name: &str,
) -> Result<()> {
    let entries = rt.block_on(async { operator.list_with("").recursive(true).await })?;
    let first_path = entries
        .into_iter()
        .map(|entry| normalize_agent_source_path(entry.path()))
        .find(|path| !path.is_empty());
    if let Some(path) = first_path {
        bail!(
            "fs create requires an empty backing source; source {source_name} already contains {path}"
        );
    }
    Ok(())
}

fn ensure_attach_root_does_not_overlap_backing_root(
    source: &SourceRegistryEntry,
    local_root: &Path,
) -> Result<()> {
    if source.provider != "fs" {
        return Ok(());
    }

    let Some(backing_root) = source.options.get("root") else {
        return Ok(());
    };
    let backing_root = canonicalize_existing_or_future_path(Path::new(backing_root))?;
    let local_root = canonicalize_existing_or_future_path(local_root)?;

    if local_root == backing_root
        || local_root.starts_with(&backing_root)
        || backing_root.starts_with(&local_root)
    {
        bail!(
            "fs attach target {} must not overlap backing source root {}",
            local_root.display(),
            backing_root.display()
        );
    }

    Ok(())
}

fn ensure_local_root_does_not_overlap_existing(
    store: &ProviderStore,
    source_name: &str,
    local_root: &Path,
) -> Result<()> {
    let local_root = canonicalize_existing_or_future_path(local_root)?;
    for binding in store.list_source_local_roots()? {
        let existing_root = canonicalize_existing_or_future_path(&binding.local_root)?;
        if binding.source_name == source_name && existing_root == local_root {
            continue;
        }
        if local_root == existing_root
            || local_root.starts_with(&existing_root)
            || existing_root.starts_with(&local_root)
        {
            bail!(
                "local root {} must not overlap existing source root {} for source {}",
                local_root.display(),
                existing_root.display(),
                binding.source_name
            );
        }
    }
    Ok(())
}

fn canonicalize_existing_or_future_path(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        return Ok(path.canonicalize()?);
    }

    let mut missing = Vec::new();
    let mut current = path.to_path_buf();
    while !current.exists() {
        let Some(name) = current.file_name().map(|name| name.to_os_string()) else {
            bail!("cannot canonicalize {}", path.display());
        };
        missing.push(name);
        if !current.pop() {
            bail!("cannot canonicalize {}", path.display());
        }
    }

    let mut canonical = current.canonicalize()?;
    for segment in missing.into_iter().rev() {
        canonical.push(segment);
    }
    Ok(canonical)
}

fn local_mount_from_marker(
    store: &ProviderStore,
    source_name: &str,
    marker: &RootDiscoveryMarker,
) -> Result<AgentFsMountRecord> {
    let mount = store
        .get_agentfs_mount(source_name)?
        .ok_or_else(|| anyhow::anyhow!("source {source_name} has no trusted AgentFS mount"))?;
    if mount.local_root != marker.local_root {
        bail!(
            "local marker root {} does not match current source binding {}",
            marker.local_root.display(),
            mount.local_root.display()
        );
    }
    if marker.fs_id.as_deref() != Some(mount.fs_id.as_str()) {
        bail!(
            "local marker fs_id does not match trusted mount fs_id {}",
            mount.fs_id
        );
    }
    if marker.source_profile_id.as_deref() != Some(mount.source_profile_id.as_str()) {
        bail!(
            "local marker source_profile_id does not match trusted mount source_profile_id {}",
            mount.source_profile_id
        );
    }
    if marker.agent_id.as_deref() != Some(mount.agent_id.as_str()) {
        bail!(
            "local marker agent_id does not match trusted mount agent_id {}",
            mount.agent_id
        );
    }
    if marker.installation_id.as_deref() != Some(mount.installation_id.as_str()) {
        bail!(
            "local marker installation_id does not match trusted mount installation_id {}",
            mount.installation_id
        );
    }
    Ok(mount)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentPathObservation {
    kind: String,
    version: Option<String>,
}

fn ensure_head_is_materialized(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
    head: &AgentFsHeadRecord,
    fs_id: &str,
) -> Result<()> {
    let Some(commit_id) = &head.commit_id else {
        return Ok(());
    };

    let commit: AgentFsCommitRecord = agentfs::read_json(
        rt,
        operator,
        &format!("{}/commits/{}.json", agentfs::METADATA_ROOT, commit_id),
    )?;
    if commit.materialization_state == AgentFsMaterializationState::Materialized {
        return Ok(());
    }

    anyhow::bail!(AgentFsError::materialization_failed(
        fs_id,
        format!(
            "head commit {commit_id} is {:?}",
            commit.materialization_state
        ),
    ));
}

fn head_materialization_state(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
    head: &AgentFsHeadRecord,
) -> Result<Option<AgentFsMaterializationState>> {
    let Some(commit_id) = &head.commit_id else {
        return Ok(None);
    };

    let commit: AgentFsCommitRecord = agentfs::read_json(
        rt,
        operator,
        &format!("{}/commits/{}.json", agentfs::METADATA_ROOT, commit_id),
    )?;
    Ok(Some(commit.materialization_state))
}

fn write_materialization_failed_event(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
    fs_id: &str,
    actor_agent_id: &str,
    commit: &AgentFsCommitRecord,
) -> Result<()> {
    let event = agentfs::event_record(
        fs_id,
        "commit.materialization_failed",
        actor_agent_id,
        &commit.commit_id,
        None,
        serde_json::json!({ "error": commit.error }),
    )?;
    agentfs::write_event(rt, operator, &event)
}

fn prepare_staging_snapshot(
    data_dir: &Path,
    fs_id: &str,
    commit_id: &str,
    base_commit_id: &Option<String>,
    local_root: &Path,
    dirty_paths: &[AgentFsCommitPathRecord],
) -> Result<PreparedAgentFsCommit> {
    let staging_root = data_dir
        .join("agentfs")
        .join("staging")
        .join(fs_id)
        .join(commit_id);
    let files_root = staging_root.join("files");
    fs::create_dir_all(&files_root)?;

    let mut manifest_paths = Vec::with_capacity(dirty_paths.len());
    let mut commit_paths = Vec::with_capacity(dirty_paths.len());

    for (index, dirty) in dirty_paths.iter().enumerate() {
        let local_path = local_root.join(&dirty.path);
        let mut staged_path = None;
        let mut hash = None;
        let mut size = None;

        if dirty.op != "delete" {
            match dirty.kind.as_str() {
                "file" => {
                    let bytes = fs::read(&local_path).map_err(|err| {
                        anyhow::anyhow!(
                            "failed to stage file {} for commit {commit_id}: {err}",
                            local_path.display()
                        )
                    })?;
                    let relative_staged_path = format!("files/{index}");
                    fs::write(staging_root.join(&relative_staged_path), &bytes)?;
                    size = Some(bytes.len() as u64);
                    hash = Some(hash_bytes(&bytes));
                    staged_path = Some(relative_staged_path);
                }
                "dir" => {
                    if !local_path.is_dir() {
                        bail!(
                            "failed to stage directory {} for commit {commit_id}: path is not a directory",
                            local_path.display()
                        );
                    }
                }
                other => bail!("unsupported AgentFS staged path kind {other}"),
            }
        }

        manifest_paths.push(AgentFsStagedPathRecord {
            path: dirty.path.clone(),
            op: dirty.op.clone(),
            kind: dirty.kind.clone(),
            hash: hash.clone(),
            size,
            staged_path,
            previous_version: dirty.previous_version.clone(),
        });
        commit_paths.push(AgentFsCommitPathRecord {
            path: dirty.path.clone(),
            kind: dirty.kind.clone(),
            op: dirty.op.clone(),
            local_version: hash,
            previous_version: dirty.previous_version.clone(),
        });
    }

    let manifest = AgentFsStagingManifest {
        schema_version: agentfs::SCHEMA_VERSION,
        fs_id: fs_id.to_string(),
        commit_id: commit_id.to_string(),
        base_commit_id: base_commit_id.clone(),
        created_at_ms: agentfs::now_ms(),
        paths: manifest_paths,
    };
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    let manifest_hash = hash_bytes(&manifest_bytes);
    let manifest_path = staging_root.join("manifest.json");
    fs::write(&manifest_path, manifest_bytes)?;

    Ok(PreparedAgentFsCommit {
        commit_id: commit_id.to_string(),
        manifest,
        manifest_path,
        manifest_hash,
        paths: commit_paths,
    })
}

fn read_staging_manifest(
    manifest_path: &Path,
    expected_manifest_hash: &str,
) -> Result<AgentFsStagingManifest> {
    let bytes = fs::read(manifest_path).map_err(|_| {
        AgentFsError::missing_commit_snapshot(
            manifest_path
                .parent()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                .unwrap_or("<unknown>"),
        )
    })?;
    let actual_hash = hash_bytes(&bytes);
    if actual_hash != expected_manifest_hash {
        anyhow::bail!(AgentFsError::malformed_metadata(format!(
            "staging manifest {} hash mismatch",
            manifest_path.display()
        )));
    }
    serde_json::from_slice(&bytes).map_err(|err| {
        AgentFsError::malformed_metadata(format!(
            "failed to parse staging manifest {}: {err}",
            manifest_path.display()
        ))
        .into()
    })
}

fn materialize_staged_commit(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
    store: &ProviderStore,
    source_name: &str,
    local_root: &Path,
    staging_root: &Path,
    manifest: &AgentFsStagingManifest,
) -> Result<SourceSyncResult> {
    let mut pushed = 0;
    let mut events_emitted = 0;

    let mut deletes = manifest
        .paths
        .iter()
        .filter(|path| path.op == "delete")
        .collect::<Vec<_>>();
    deletes.sort_by(|left, right| {
        right
            .path
            .matches('/')
            .count()
            .cmp(&left.path.matches('/').count())
            .then_with(|| right.path.cmp(&left.path))
    });
    for path in deletes {
        delete_remote_staged_path(rt, operator, path)?;
        store.remove_path_sync_state(source_name, &path.path)?;
        store.append_sync_event(
            source_name,
            &path.path,
            "synced_to_remote",
            "ready",
            agentfs::now_ms(),
        )?;
        pushed += 1;
        events_emitted += 1;
    }

    let mut creates = manifest
        .paths
        .iter()
        .filter(|path| path.op != "delete")
        .collect::<Vec<_>>();
    creates.sort_by(|left, right| {
        staged_kind_rank(&left.kind)
            .cmp(&staged_kind_rank(&right.kind))
            .then_with(|| left.path.cmp(&right.path))
    });
    for path in creates {
        materialize_staged_path(rt, operator, staging_root, path)?;
        store.upsert_path_sync_state(&PathSyncStateRecord {
            source_name: source_name.to_string(),
            path: path.path.clone(),
            entry_kind: path.kind.clone(),
            public_state: "ready".to_string(),
            local_present: true,
            dirty_local: false,
            dirty_remote: false,
            pinned: false,
            stale: false,
            last_local_version: path.hash.clone(),
            base_remote_version: path.hash.clone(),
            current_remote_version: path.hash.clone(),
        })?;
        store.append_sync_event(
            source_name,
            &path.path,
            "synced_to_remote",
            "ready",
            agentfs::now_ms(),
        )?;
        pushed += 1;
        events_emitted += 1;
    }

    Ok(SourceSyncResult {
        source_id: source_name.to_string(),
        local_root: local_root.to_path_buf(),
        pulled: 0,
        pushed,
        conflicts: 0,
        events_emitted,
        local_scan: LocalScanStats::default(),
        remote_scan: RemoteScanStats::default(),
    })
}

fn staged_kind_rank(kind: &str) -> u8 {
    match kind {
        "dir" => 0,
        _ => 1,
    }
}

fn materialize_staged_path(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
    staging_root: &Path,
    path: &AgentFsStagedPathRecord,
) -> Result<()> {
    match path.kind.as_str() {
        "dir" => ensure_remote_dir_for_agentfs(rt, operator, &path.path),
        "file" => {
            let staged_path = path
                .staged_path
                .as_deref()
                .ok_or_else(|| AgentFsError::missing_commit_snapshot(&path.path))?;
            let bytes = fs::read(staging_root.join(staged_path))
                .map_err(|_| AgentFsError::missing_commit_snapshot(&path.path))?;
            if path.hash.as_deref() != Some(hash_bytes(&bytes).as_str()) {
                anyhow::bail!(AgentFsError::malformed_metadata(format!(
                    "staged file {} hash mismatch",
                    path.path
                )));
            }
            ensure_remote_parent_dirs_for_agentfs(rt, operator, &path.path)?;
            rt.block_on(operator.write(&path.path, bytes))?;
            Ok(())
        }
        other => bail!("unsupported AgentFS staged path kind {other}"),
    }
}

fn delete_remote_staged_path(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
    path: &AgentFsStagedPathRecord,
) -> Result<()> {
    let result = match path.kind.as_str() {
        "dir" => rt.block_on(operator.remove_all(&normalized_dir_for_agentfs(&path.path))),
        "file" => rt.block_on(operator.delete(&path.path)),
        other => bail!("unsupported AgentFS staged path kind {other}"),
    };
    match result {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == opendal::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

fn ensure_remote_dir_for_agentfs(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
    source_path: &str,
) -> Result<()> {
    ensure_remote_parent_dirs_for_agentfs(rt, operator, source_path)?;
    rt.block_on(operator.create_dir(&normalized_dir_for_agentfs(source_path)))?;
    Ok(())
}

fn ensure_remote_parent_dirs_for_agentfs(
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
            rt.block_on(operator.create_dir(&normalized_dir_for_agentfs(&current)))?;
        }
    }
    Ok(())
}

fn normalized_dir_for_agentfs(path: &str) -> String {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}/")
    }
}

fn collect_dirty_paths(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
    local_root: &Path,
) -> Result<Vec<AgentFsCommitPathRecord>> {
    let local = collect_local_agent_paths(local_root)?;
    let remote = collect_remote_agent_paths(rt, operator)?;
    let mut paths = BTreeSet::new();
    paths.extend(local.keys().cloned());
    paths.extend(remote.keys().cloned());

    let mut dirty = Vec::new();
    for path in paths {
        agentfs::validate_source_relative_path(&path)?;
        let local_entry = local.get(&path);
        let remote_entry = remote.get(&path);
        if local_entry == remote_entry {
            continue;
        }

        let op = match (local_entry, remote_entry) {
            (Some(_), None) => "create",
            (Some(_), Some(_)) => "update",
            (None, Some(_)) => "delete",
            (None, None) => continue,
        };
        let kind = local_entry
            .or(remote_entry)
            .map(|entry| entry.kind.clone())
            .unwrap_or_else(|| "file".to_string());
        dirty.push(AgentFsCommitPathRecord {
            path,
            kind,
            op: op.to_string(),
            local_version: local_entry.and_then(|entry| entry.version.clone()),
            previous_version: remote_entry.and_then(|entry| entry.version.clone()),
        });
    }

    Ok(dirty)
}

fn collect_local_agent_paths(root: &Path) -> Result<BTreeMap<String, AgentPathObservation>> {
    let mut entries = BTreeMap::new();

    fn walk(
        current: &Path,
        root: &Path,
        entries: &mut BTreeMap<String, AgentPathObservation>,
    ) -> Result<()> {
        if !current.exists() {
            return Ok(());
        }

        let mut dir_entries = fs::read_dir(current)?.collect::<std::result::Result<Vec<_>, _>>()?;
        dir_entries.sort_by_key(|entry| entry.file_name());

        for entry in dir_entries {
            let path = entry.path();
            if current == root && entry.file_name() == ".section" {
                continue;
            }

            let relative = normalize_local_relative_path(root, &path)?;
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                bail!(
                    "AgentFS commit does not support symlink paths: {}",
                    path.display()
                );
            }
            if metadata.is_dir() {
                entries.insert(
                    relative.clone(),
                    AgentPathObservation {
                        kind: "dir".to_string(),
                        version: None,
                    },
                );
                walk(&path, root, entries)?;
            } else if metadata.is_file() {
                entries.insert(
                    relative,
                    AgentPathObservation {
                        kind: "file".to_string(),
                        version: Some(hash_file(&path)?),
                    },
                );
            } else {
                bail!("unsupported local entry type for {}", path.display());
            }
        }

        Ok(())
    }

    walk(root, root, &mut entries)?;
    Ok(entries)
}

fn collect_remote_agent_paths(
    rt: &tokio::runtime::Runtime,
    operator: &Operator,
) -> Result<BTreeMap<String, AgentPathObservation>> {
    let mut listed = rt.block_on(async { operator.list_with("").recursive(true).await })?;
    listed.sort_by_key(|entry| entry.path().to_string());
    let mut entries = BTreeMap::new();

    for entry in listed {
        let path = normalize_agent_source_path(entry.path());
        if path.is_empty() || path == ".section" || path.starts_with(".section/") {
            continue;
        }

        match entry.metadata().mode() {
            EntryMode::DIR => {
                entries.insert(
                    path,
                    AgentPathObservation {
                        kind: "dir".to_string(),
                        version: None,
                    },
                );
            }
            EntryMode::FILE => {
                let data = rt.block_on(operator.read(&path))?;
                entries.insert(
                    path,
                    AgentPathObservation {
                        kind: "file".to_string(),
                        version: Some(hash_bytes(data.to_bytes().as_ref())),
                    },
                );
            }
            other => bail!("unsupported remote entry mode {other:?} for {path}"),
        }
    }

    Ok(entries)
}

fn normalize_local_relative_path(root: &Path, path: &Path) -> Result<String> {
    let relative = path.strip_prefix(root)?;
    Ok(relative
        .iter()
        .map(|segment| segment.to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

fn normalize_agent_source_path(path: &str) -> String {
    path.trim_matches('/').to_string()
}

fn hash_file(path: &Path) -> Result<String> {
    Ok(hash_bytes(&fs::read(path)?))
}

fn hash_bytes(bytes: &[u8]) -> String {
    let digest = digest(&SHA256, bytes);
    format!(
        "sha256:{}",
        digest
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

fn default_root_marker_schema_version() -> u32 {
    1
}

fn absolutize_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn ensure_attach_root_is_empty(local_root: &Path) -> Result<()> {
    if !local_root.exists() {
        return Ok(());
    }
    if !local_root.is_dir() {
        bail!(
            "fs attach target {} exists but is not a directory",
            local_root.display()
        );
    }

    let mut user_entries = Vec::new();
    for entry in std::fs::read_dir(local_root)? {
        let entry = entry?;
        if entry.file_name() == ".section" {
            continue;
        }
        user_entries.push(entry.path());
    }

    if !user_entries.is_empty() {
        bail!(
            "fs attach target {} must be empty so attach cannot publish local drafts",
            local_root.display()
        );
    }

    Ok(())
}

fn write_root_marker(source_id: &str, local_root: &Path) -> Result<()> {
    let marker = RootDiscoveryMarker {
        schema_version: 1,
        source_id: source_id.to_string(),
        local_root: local_root.to_path_buf(),
        control_plane_endpoint: "sectiond://local".to_string(),
        fs_id: None,
        source_profile_id: None,
        agent_id: None,
        installation_id: None,
        base_commit_id: None,
    };
    write_root_marker_record(local_root, &marker)
}

fn write_root_marker_for_agentfs(
    source_id: &str,
    local_root: &Path,
    fs_id: &str,
    source_profile_id: &str,
    agent_id: &str,
    installation_id: &str,
    control_plane_endpoint: &str,
    base_commit_id: Option<String>,
) -> Result<()> {
    let marker = RootDiscoveryMarker {
        schema_version: 1,
        source_id: source_id.to_string(),
        local_root: local_root.to_path_buf(),
        control_plane_endpoint: control_plane_endpoint.to_string(),
        fs_id: Some(fs_id.to_string()),
        source_profile_id: Some(source_profile_id.to_string()),
        agent_id: Some(agent_id.to_string()),
        installation_id: Some(installation_id.to_string()),
        base_commit_id,
    };
    write_root_marker_record(local_root, &marker)
}

fn write_root_marker_record(local_root: &Path, marker: &RootDiscoveryMarker) -> Result<()> {
    let marker_dir = local_root.join(".section");
    std::fs::create_dir_all(&marker_dir)?;
    std::fs::write(
        marker_dir.join("root.json"),
        serde_json::to_vec_pretty(&marker)?,
    )?;
    Ok(())
}

fn remove_root_marker(local_root: &Path) -> Result<()> {
    let marker_dir = local_root.join(".section");
    let marker_path = marker_dir.join("root.json");
    if marker_path.exists() {
        std::fs::remove_file(&marker_path)?;
    }
    if marker_dir.exists() {
        let mut entries = std::fs::read_dir(&marker_dir)?;
        if entries.next().is_none() {
            std::fs::remove_dir(&marker_dir)?;
        }
    }
    Ok(())
}

fn discover_root_marker(path: &Path) -> Result<RootDiscoveryMarker> {
    let mut current = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| path.to_path_buf())
    };

    loop {
        let marker_path = current.join(".section").join("root.json");
        if marker_path.exists() {
            let marker = std::fs::read(&marker_path)?;
            return Ok(serde_json::from_slice(&marker)?);
        }

        if !current.pop() {
            bail!(
                "no Section local-root marker found above {}",
                path.display()
            );
        }
    }
}

fn relative_source_path(local_root: &Path, local_path: &Path) -> Result<String> {
    let relative = local_path.strip_prefix(local_root).map_err(|_| {
        anyhow::anyhow!(
            "{} is outside the bound local root {}",
            local_path.display(),
            local_root.display()
        )
    })?;

    let raw = relative
        .iter()
        .map(|segment| segment.to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");

    Ok(raw)
}

fn display_source_path(path: &str) -> String {
    if path.is_empty() {
        "/".to_string()
    } else {
        path.to_string()
    }
}

fn default_path_state(source_id: &str, path: &str, local_path: &Path) -> PathSyncStateRecord {
    PathSyncStateRecord {
        source_name: source_id.to_string(),
        path: path.to_string(),
        entry_kind: if local_path.is_dir() {
            "dir".to_string()
        } else {
            "file".to_string()
        },
        public_state: "ready".to_string(),
        local_present: local_path.exists(),
        dirty_local: false,
        dirty_remote: false,
        pinned: false,
        stale: false,
        last_local_version: None,
        base_remote_version: None,
        current_remote_version: None,
    }
}

fn refresh_attr_names() -> &'static [&'static str] {
    #[cfg(target_os = "linux")]
    {
        &[REFRESH_XATTR_NAME_LINUX, REFRESH_XATTR_NAME]
    }

    #[cfg(not(target_os = "linux"))]
    {
        &[REFRESH_XATTR_NAME, REFRESH_XATTR_NAME_LINUX]
    }
}

fn refresh_mount_target(mount_point: &Path, path: &str) -> PathBuf {
    let mut target = mount_point.to_path_buf();
    for segment in path.split('/') {
        if !segment.is_empty() {
            target.push(segment);
        }
    }
    target
}

fn trigger_refresh_xattr(path: &Path) -> Result<Vec<u8>> {
    let mut last_error = None;

    for attr_name in refresh_attr_names() {
        match xattr::get(path, attr_name) {
            Ok(Some(data)) => return Ok(data),
            Ok(None) => {
                last_error = Some(anyhow::anyhow!(
                    "refresh xattr {attr_name} returned no data for {}",
                    path.display()
                ));
            }
            Err(err) => {
                last_error = Some(anyhow::anyhow!(
                    "failed to read refresh xattr {attr_name} on {}: {err}",
                    path.display()
                ));
            }
        }
    }

    Err(last_error
        .unwrap_or_else(|| anyhow::anyhow!("refresh xattr not available for {}", path.display())))
}

fn is_mount_active(mount_point: &Path) -> bool {
    check_proc_mounts(mount_point) || check_mount_command(mount_point)
}

fn check_proc_mounts(mount_point: &Path) -> bool {
    let mount_str = mount_point.to_string_lossy();

    if let Ok(mounts) = std::fs::read_to_string("/proc/mounts") {
        for line in mounts.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 && parts[1] == mount_str.as_ref() {
                return true;
            }
        }
    }

    false
}

fn check_mount_command(mount_point: &Path) -> bool {
    let output = match Command::new("mount").output() {
        Ok(output) if output.status.success() => output,
        _ => return false,
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    mount_output_contains_target(&stdout, mount_point)
}

fn mount_output_contains_target(output: &str, mount_point: &Path) -> bool {
    let mount_str = mount_point.to_string_lossy();

    output.lines().any(|line| {
        let Some((_, rest)) = line.split_once(" on ") else {
            return false;
        };

        rest == mount_str.as_ref()
            || rest.starts_with(&format!("{mount_str} "))
            || rest.starts_with(&format!("{mount_str} ("))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_config(data_dir: &Path, path: &Path, source_name: &str) {
        let config = format!(
            "data_dir = {:?}\nmount_point = \"/tmp/section-mount\"\n\n[sources.{source_name}]\nprovider = \"fs\"\n\n[sources.{source_name}.options]\nroot = \"/tmp/from-config\"\n",
            data_dir.to_string_lossy()
        );
        fs::write(path, config).expect("write config");
    }

    #[test]
    fn list_sources_returns_merged_effective_registry() {
        let temp_dir = TempDir::new().expect("temp dir");
        let config_path = temp_dir.path().join("section.toml");
        write_config(temp_dir.path(), &config_path, "config-only");

        let control = SectiondControlPlane::load(Some(&config_path)).expect("control plane");
        control
            .source_add(
                "store-only",
                "fs",
                HashMap::from([("root".to_string(), "/tmp/from-store".to_string())]),
            )
            .expect("add source");

        let sources = control.list_sources().expect("list sources");
        assert_eq!(sources.len(), 2);

        let config_source = sources
            .iter()
            .find(|entry| entry.name == "config-only")
            .expect("config source");
        assert_eq!(config_source.origin, SourceOrigin::ConfigFile);
        assert_eq!(
            config_source.options.get("root").map(String::as_str),
            Some("/tmp/from-config")
        );

        let store_source = sources
            .iter()
            .find(|entry| entry.name == "store-only")
            .expect("store source");
        assert_eq!(store_source.origin, SourceOrigin::ProviderStore);
        assert_eq!(
            store_source.options.get("root").map(String::as_str),
            Some("/tmp/from-store")
        );
    }

    #[test]
    fn source_bind_local_root_writes_marker_and_updates_registry_entry() {
        let temp_dir = TempDir::new().expect("temp dir");
        let config_path = temp_dir.path().join("section.toml");
        write_config(temp_dir.path(), &config_path, "config-only");

        let control = SectiondControlPlane::load(Some(&config_path)).expect("control plane");
        control
            .source_add(
                "store-only",
                "fs",
                HashMap::from([("root".to_string(), "/tmp/from-store".to_string())]),
            )
            .expect("add source");

        let local_root = temp_dir.path().join("bound-root");
        let entry = control
            .source_bind_local_root("store-only", &local_root)
            .expect("bind local root");

        assert_eq!(entry.local_root, Some(local_root.clone()));

        let marker =
            std::fs::read(local_root.join(".section").join("root.json")).expect("read root marker");
        let marker: RootDiscoveryMarker =
            serde_json::from_slice(&marker).expect("parse root marker");
        assert_eq!(marker.source_id, "store-only");
        assert_eq!(marker.local_root, local_root);
        assert_eq!(marker.control_plane_endpoint, "sectiond://local");
    }

    #[test]
    fn path_inspect_resolves_local_path_and_detail_state() {
        let temp_dir = TempDir::new().expect("temp dir");
        let config_path = temp_dir.path().join("section.toml");
        write_config(temp_dir.path(), &config_path, "config-only");

        let control = SectiondControlPlane::load(Some(&config_path)).expect("control plane");
        control
            .source_add(
                "store-only",
                "fs",
                HashMap::from([("root".to_string(), "/tmp/from-store".to_string())]),
            )
            .expect("add source");

        let local_root = temp_dir.path().join("bound-root");
        control
            .source_bind_local_root("store-only", &local_root)
            .expect("bind local root");

        let nested = local_root.join("notes").join("todo.txt");
        std::fs::create_dir_all(nested.parent().expect("parent")).expect("create parent");
        std::fs::write(&nested, "hello").expect("write local file");
        control
            .store
            .upsert_path_sync_state(&PathSyncStateRecord {
                source_name: "store-only".to_string(),
                path: "notes/todo.txt".to_string(),
                entry_kind: "file".to_string(),
                public_state: "conflict".to_string(),
                local_present: true,
                dirty_local: true,
                dirty_remote: true,
                pinned: false,
                stale: true,
                last_local_version: Some("l1".to_string()),
                base_remote_version: Some("v1".to_string()),
                current_remote_version: Some("v2".to_string()),
            })
            .expect("seed path state");

        let inspect = control.path_inspect(&nested).expect("path inspect");
        assert_eq!(inspect.source_id, "store-only");
        assert_eq!(inspect.local_root, local_root);
        assert_eq!(inspect.local_path, nested);
        assert_eq!(inspect.source_path, "notes/todo.txt");
        assert_eq!(inspect.state, "conflict");
        assert!(inspect.detail.local_present);
        assert!(inspect.detail.dirty_local);
        assert!(inspect.detail.dirty_remote);
        assert!(inspect.detail.stale);
        assert_eq!(inspect.base_remote_version.as_deref(), Some("v1"));
        assert_eq!(inspect.current_remote_version.as_deref(), Some("v2"));
    }

    #[test]
    fn source_remove_cleans_up_root_marker() {
        let temp_dir = TempDir::new().expect("temp dir");
        let config_path = temp_dir.path().join("section.toml");
        write_config(temp_dir.path(), &config_path, "config-only");

        let control = SectiondControlPlane::load(Some(&config_path)).expect("control plane");
        control
            .source_add(
                "store-only",
                "fs",
                HashMap::from([("root".to_string(), "/tmp/from-store".to_string())]),
            )
            .expect("add source");

        let local_root = temp_dir.path().join("bound-root");
        control
            .source_bind_local_root("store-only", &local_root)
            .expect("bind local root");

        control
            .source_remove("store-only")
            .expect("remove store-owned source");

        assert!(
            !local_root.join(".section").join("root.json").exists(),
            "root marker should be removed"
        );
    }

    #[test]
    fn source_add_rejects_config_owned_name() {
        let temp_dir = TempDir::new().expect("temp dir");
        let config_path = temp_dir.path().join("section.toml");
        write_config(temp_dir.path(), &config_path, "shared");

        let control = SectiondControlPlane::load(Some(&config_path)).expect("control plane");
        let err = control
            .source_add(
                "shared",
                "fs",
                HashMap::from([("root".to_string(), "/tmp/override".to_string())]),
            )
            .expect_err("config-owned source should fail");

        assert!(err
            .to_string()
            .contains("defined in the config file and cannot be changed via the control plane"));
    }

    #[test]
    fn source_remove_rejects_config_owned_name() {
        let temp_dir = TempDir::new().expect("temp dir");
        let config_path = temp_dir.path().join("section.toml");
        write_config(temp_dir.path(), &config_path, "shared");

        let control = SectiondControlPlane::load(Some(&config_path)).expect("control plane");
        let err = control
            .source_remove("shared")
            .expect_err("config-owned source should fail");

        assert!(err
            .to_string()
            .contains("defined in the config file and cannot be changed via the control plane"));
    }
}
