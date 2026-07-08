use crate::crypto;
use anyhow::Context;
use rusqlite::{Connection, OptionalExtension};
use section_core::config::{CacheConfig, SourceConfig};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentIdentityRecord {
    pub agent_id: String,
    pub installation_id: String,
    pub name: String,
    #[serde(default, skip_serializing)]
    pub auth_token: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedFilesystemRecord {
    pub fs_id: String,
    pub name: String,
    pub owner_agent_id: String,
    pub source_profile_id: String,
    pub source_name: String,
    pub accepted_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialBindingCacheRecord {
    pub credential_binding_id: String,
    pub fs_id: String,
    pub agent_id: String,
    pub installation_id: String,
    pub source_profile_id: String,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentFsMountRecord {
    pub source_name: String,
    pub fs_id: String,
    pub source_profile_id: String,
    pub agent_id: String,
    pub installation_id: String,
    pub local_root: PathBuf,
    pub base_commit_id: Option<String>,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLocalRootBinding {
    pub source_name: String,
    pub local_root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathSyncStateRecord {
    pub source_name: String,
    pub path: String,
    pub entry_kind: String,
    pub public_state: String,
    pub local_present: bool,
    pub dirty_local: bool,
    pub dirty_remote: bool,
    pub pinned: bool,
    pub stale: bool,
    pub last_local_version: Option<String>,
    pub base_remote_version: Option<String>,
    pub current_remote_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncEventRecord {
    pub id: i64,
    pub source_name: String,
    pub path: String,
    pub kind: String,
    pub state: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalScanCacheRecord {
    pub path: String,
    pub entry_kind: String,
    pub version: Option<String>,
    pub size: Option<u64>,
    pub mtime_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteManifestRecord {
    pub path: String,
    pub entry_kind: String,
    pub version: Option<String>,
    pub size: Option<u64>,
    pub mtime_ms: Option<i64>,
}

/// Persistent store for sources and their credentials.
pub struct ProviderStore {
    conn: Connection,
    /// AES-256-GCM key loaded from `{data_dir}/section.key`.
    encryption_key: Vec<u8>,
}

impl ProviderStore {
    pub fn open(data_dir: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(data_dir)?;

        let key_path = data_dir.join("section.key");
        let encryption_key = crypto::load_or_generate_key(&key_path).with_context(|| {
            format!("failed to load encryption key from {}", key_path.display())
        })?;

        let db_path = data_dir.join("section.db");
        let conn = Connection::open(&db_path)
            .with_context(|| format!("failed to open database at {}", db_path.display()))?;

        let store = Self {
            conn,
            encryption_key,
        };
        store.init_tables()?;
        store.migrate_encrypt_existing()?;
        Ok(store)
    }

    fn init_tables(&self) -> anyhow::Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sources (
                name TEXT PRIMARY KEY,
                provider TEXT NOT NULL,
                options_json TEXT NOT NULL DEFAULT '{}',
                metadata_ttl_secs INTEGER NOT NULL DEFAULT 60,
                content_ttl_secs INTEGER NOT NULL DEFAULT 300,
                encrypted INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS source_local_roots (
                source_name TEXT PRIMARY KEY,
                local_root TEXT NOT NULL UNIQUE
            );
            CREATE TABLE IF NOT EXISTS path_sync_state (
                source_name TEXT NOT NULL,
                path TEXT NOT NULL,
                entry_kind TEXT NOT NULL DEFAULT 'file',
                public_state TEXT NOT NULL DEFAULT 'ready',
                local_present INTEGER NOT NULL DEFAULT 0,
                dirty_local INTEGER NOT NULL DEFAULT 0,
                dirty_remote INTEGER NOT NULL DEFAULT 0,
                pinned INTEGER NOT NULL DEFAULT 0,
                stale INTEGER NOT NULL DEFAULT 0,
                last_local_version TEXT,
                base_remote_version TEXT,
                current_remote_version TEXT,
                PRIMARY KEY (source_name, path)
            );
            CREATE TABLE IF NOT EXISTS sync_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source_name TEXT NOT NULL,
                path TEXT NOT NULL,
                kind TEXT NOT NULL,
                state TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS local_scan_cache (
                source_name TEXT NOT NULL,
                path TEXT NOT NULL,
                entry_kind TEXT NOT NULL DEFAULT 'file',
                version TEXT,
                size_bytes INTEGER,
                mtime_ms INTEGER,
                PRIMARY KEY (source_name, path)
            );
            CREATE TABLE IF NOT EXISTS remote_manifest (
                source_name TEXT NOT NULL,
                path TEXT NOT NULL,
                entry_kind TEXT NOT NULL DEFAULT 'file',
                version TEXT,
                size_bytes INTEGER,
                mtime_ms INTEGER,
                PRIMARY KEY (source_name, path)
            );
            CREATE TABLE IF NOT EXISTS agent_identity (
                scope TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                installation_id TEXT NOT NULL DEFAULT '',
                name TEXT NOT NULL,
                auth_token TEXT NOT NULL DEFAULT '',
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS agentfs_accepted_filesystems (
                fs_id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                owner_agent_id TEXT NOT NULL,
                source_profile_id TEXT NOT NULL,
                source_name TEXT NOT NULL,
                accepted_at_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS agentfs_credential_bindings (
                credential_binding_id TEXT PRIMARY KEY,
                fs_id TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                installation_id TEXT NOT NULL,
                source_profile_id TEXT NOT NULL,
                issued_at_ms INTEGER NOT NULL,
                expires_at_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS agentfs_mounts (
                source_name TEXT PRIMARY KEY,
                fs_id TEXT NOT NULL,
                source_profile_id TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                installation_id TEXT NOT NULL,
                local_root TEXT NOT NULL UNIQUE,
                base_commit_id TEXT,
                updated_at_ms INTEGER NOT NULL
            );",
        )?;

        // Add `encrypted` column to existing tables that lack it.
        self.add_column_if_missing("sources", "encrypted", "INTEGER NOT NULL DEFAULT 0")?;
        self.add_column_if_missing(
            "path_sync_state",
            "entry_kind",
            "TEXT NOT NULL DEFAULT 'file'",
        )?;
        self.add_column_if_missing("path_sync_state", "last_local_version", "TEXT")?;
        self.add_column_if_missing(
            "agent_identity",
            "installation_id",
            "TEXT NOT NULL DEFAULT ''",
        )?;
        self.add_column_if_missing("agent_identity", "auth_token", "TEXT NOT NULL DEFAULT ''")?;
        self.ensure_existing_installation_ids()?;
        self.ensure_existing_auth_tokens()?;

        Ok(())
    }

    pub fn get_agent_identity(&self) -> anyhow::Result<Option<AgentIdentityRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT agent_id, installation_id, name, auth_token, created_at_ms, updated_at_ms
             FROM agent_identity
             WHERE scope = 'default'",
        )?;
        let row = stmt.query_row([], |row| {
            Ok(AgentIdentityRecord {
                agent_id: row.get(0)?,
                installation_id: row.get(1)?,
                name: row.get(2)?,
                auth_token: row.get(3)?,
                created_at_ms: row.get(4)?,
                updated_at_ms: row.get(5)?,
            })
        });

        match row {
            Ok(record) => Ok(Some(record)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    pub fn register_agent_identity(&self, name: &str) -> anyhow::Result<AgentIdentityRecord> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            anyhow::bail!("agent name must not be empty");
        }

        let now = now_ms();
        if let Some(mut existing) = self.get_agent_identity()? {
            existing.name = trimmed.to_string();
            if existing.auth_token.is_empty() {
                existing.auth_token = format!("auth_{}", random_hex(32)?);
            }
            existing.updated_at_ms = now;
            self.conn.execute(
                "UPDATE agent_identity
                 SET name = ?1, auth_token = ?2, updated_at_ms = ?3
                 WHERE scope = 'default'",
                rusqlite::params![existing.name, existing.auth_token, existing.updated_at_ms],
            )?;
            return Ok(existing);
        }

        let record = AgentIdentityRecord {
            agent_id: format!("agt_{}", random_hex(16)?),
            installation_id: format!("ins_{}", random_hex(16)?),
            name: trimmed.to_string(),
            auth_token: format!("auth_{}", random_hex(32)?),
            created_at_ms: now,
            updated_at_ms: now,
        };

        self.conn.execute(
            "INSERT INTO agent_identity (scope, agent_id, installation_id, name, auth_token, created_at_ms, updated_at_ms)
             VALUES ('default', ?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                record.agent_id,
                record.installation_id,
                record.name,
                record.auth_token,
                record.created_at_ms,
                record.updated_at_ms,
            ],
        )?;

        Ok(record)
    }

    pub fn cache_agent_identity(&self, record: &AgentIdentityRecord) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO agent_identity (scope, agent_id, installation_id, name, auth_token, created_at_ms, updated_at_ms)
             VALUES ('default', ?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(scope) DO UPDATE SET
                agent_id = excluded.agent_id,
                installation_id = excluded.installation_id,
                name = excluded.name,
                auth_token = excluded.auth_token,
                created_at_ms = excluded.created_at_ms,
                updated_at_ms = excluded.updated_at_ms",
            rusqlite::params![
                record.agent_id,
                record.installation_id,
                record.name,
                record.auth_token,
                record.created_at_ms,
                record.updated_at_ms,
            ],
        )?;
        Ok(())
    }

    pub fn cache_accepted_filesystem(
        &self,
        record: &AcceptedFilesystemRecord,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO agentfs_accepted_filesystems (
                fs_id, name, owner_agent_id, source_profile_id, source_name, accepted_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(fs_id) DO UPDATE SET
                name = excluded.name,
                owner_agent_id = excluded.owner_agent_id,
                source_profile_id = excluded.source_profile_id,
                source_name = excluded.source_name,
                accepted_at_ms = excluded.accepted_at_ms",
            rusqlite::params![
                record.fs_id,
                record.name,
                record.owner_agent_id,
                record.source_profile_id,
                record.source_name,
                record.accepted_at_ms,
            ],
        )?;
        Ok(())
    }

    pub fn cache_credential_binding(
        &self,
        record: &CredentialBindingCacheRecord,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO agentfs_credential_bindings (
                credential_binding_id,
                fs_id,
                agent_id,
                installation_id,
                source_profile_id,
                issued_at_ms,
                expires_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(credential_binding_id) DO UPDATE SET
                fs_id = excluded.fs_id,
                agent_id = excluded.agent_id,
                installation_id = excluded.installation_id,
                source_profile_id = excluded.source_profile_id,
                issued_at_ms = excluded.issued_at_ms,
                expires_at_ms = excluded.expires_at_ms",
            rusqlite::params![
                record.credential_binding_id,
                record.fs_id,
                record.agent_id,
                record.installation_id,
                record.source_profile_id,
                record.issued_at_ms,
                record.expires_at_ms,
            ],
        )?;
        Ok(())
    }

    pub fn upsert_agentfs_mount(&self, record: &AgentFsMountRecord) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO agentfs_mounts (
                source_name,
                fs_id,
                source_profile_id,
                agent_id,
                installation_id,
                local_root,
                base_commit_id,
                updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(source_name) DO UPDATE SET
                fs_id = excluded.fs_id,
                source_profile_id = excluded.source_profile_id,
                agent_id = excluded.agent_id,
                installation_id = excluded.installation_id,
                local_root = excluded.local_root,
                base_commit_id = excluded.base_commit_id,
                updated_at_ms = excluded.updated_at_ms",
            rusqlite::params![
                record.source_name,
                record.fs_id,
                record.source_profile_id,
                record.agent_id,
                record.installation_id,
                record.local_root.to_string_lossy().as_ref(),
                record.base_commit_id,
                record.updated_at_ms,
            ],
        )?;
        Ok(())
    }

    pub fn get_agentfs_mount(
        &self,
        source_name: &str,
    ) -> anyhow::Result<Option<AgentFsMountRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT
                source_name,
                fs_id,
                source_profile_id,
                agent_id,
                installation_id,
                local_root,
                base_commit_id,
                updated_at_ms
             FROM agentfs_mounts
             WHERE source_name = ?1",
        )?;
        let row = stmt.query_row([source_name], read_agentfs_mount_row);

        match row {
            Ok(record) => Ok(Some(record)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    pub fn remove_agentfs_mount(&self, source_name: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "DELETE FROM agentfs_mounts WHERE source_name = ?1",
            [source_name],
        )?;
        Ok(())
    }

    pub fn remove_agentfs_filesystem_cache(
        &self,
        fs_id: &str,
        source_name: &str,
    ) -> anyhow::Result<()> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> anyhow::Result<()> {
            self.conn
                .execute("DELETE FROM sources WHERE name = ?1", [source_name])?;
            self.conn.execute(
                "DELETE FROM source_local_roots WHERE source_name = ?1",
                [source_name],
            )?;
            self.conn.execute(
                "DELETE FROM path_sync_state WHERE source_name = ?1",
                [source_name],
            )?;
            self.conn.execute(
                "DELETE FROM local_scan_cache WHERE source_name = ?1",
                [source_name],
            )?;
            self.conn.execute(
                "DELETE FROM remote_manifest WHERE source_name = ?1",
                [source_name],
            )?;
            self.conn.execute(
                "DELETE FROM sync_events WHERE source_name = ?1",
                [source_name],
            )?;
            self.conn.execute(
                "DELETE FROM agentfs_mounts WHERE source_name = ?1 OR fs_id = ?2",
                rusqlite::params![source_name, fs_id],
            )?;
            self.conn.execute(
                "DELETE FROM agentfs_accepted_filesystems WHERE source_name = ?1 OR fs_id = ?2",
                rusqlite::params![source_name, fs_id],
            )?;
            self.conn.execute(
                "DELETE FROM agentfs_credential_bindings WHERE fs_id = ?1",
                [fs_id],
            )?;
            Ok(())
        })();

        match result {
            Ok(()) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(())
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }

    pub fn list_agentfs_mounts(&self) -> anyhow::Result<Vec<AgentFsMountRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT
                source_name,
                fs_id,
                source_profile_id,
                agent_id,
                installation_id,
                local_root,
                base_commit_id,
                updated_at_ms
             FROM agentfs_mounts
             ORDER BY source_name",
        )?;
        let rows = stmt.query_map([], read_agentfs_mount_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn is_agentfs_source(&self, source_name: &str) -> anyhow::Result<bool> {
        let accepted: Option<String> = self
            .conn
            .query_row(
                "SELECT fs_id FROM agentfs_accepted_filesystems WHERE source_name = ?1",
                [source_name],
                |row| row.get(0),
            )
            .optional()?;
        if accepted.is_some() {
            return Ok(true);
        }
        let mounted: Option<String> = self
            .conn
            .query_row(
                "SELECT fs_id FROM agentfs_mounts WHERE source_name = ?1",
                [source_name],
                |row| row.get(0),
            )
            .optional()?;
        Ok(mounted.is_some())
    }

    /// Idempotently add a column to an existing table (SQLite has no IF NOT EXISTS for columns).
    fn add_column_if_missing(
        &self,
        table: &str,
        column: &str,
        column_def: &str,
    ) -> anyhow::Result<()> {
        let mut stmt = self
            .conn
            .prepare(&format!("PRAGMA table_info({})", table))?;
        let has_column = stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .filter_map(|r| r.ok())
            .any(|name| name == column);

        if !has_column {
            self.conn.execute_batch(&format!(
                "ALTER TABLE {} ADD COLUMN {} {};",
                table, column, column_def,
            ))?;
        }

        Ok(())
    }

    fn ensure_existing_installation_ids(&self) -> anyhow::Result<()> {
        let mut stmt = self.conn.prepare(
            "SELECT scope FROM agent_identity
             WHERE installation_id IS NULL OR installation_id = ''",
        )?;
        let scopes = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);

        for scope in scopes {
            self.conn.execute(
                "UPDATE agent_identity
                 SET installation_id = ?1
                 WHERE scope = ?2",
                rusqlite::params![format!("ins_{}", random_hex(16)?), scope],
            )?;
        }
        Ok(())
    }

    fn ensure_existing_auth_tokens(&self) -> anyhow::Result<()> {
        let mut stmt = self.conn.prepare(
            "SELECT scope FROM agent_identity
             WHERE auth_token IS NULL OR auth_token = ''",
        )?;
        let scopes = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);

        for scope in scopes {
            self.conn.execute(
                "UPDATE agent_identity
                 SET auth_token = ?1
                 WHERE scope = ?2",
                rusqlite::params![format!("auth_{}", random_hex(32)?), scope],
            )?;
        }
        Ok(())
    }

    /// Encrypt any existing plaintext rows (encrypted == 0) so that
    /// databases created before encryption was added are migrated transparently.
    fn migrate_encrypt_existing(&self) -> anyhow::Result<()> {
        let mut stmt = self
            .conn
            .prepare("SELECT name, options_json FROM sources WHERE encrypted = 0")?;
        let rows: Vec<(String, String)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .filter_map(|r| r.ok())
            .collect();

        for (name, plaintext) in rows {
            let ciphertext = crypto::encrypt(&self.encryption_key, plaintext.as_bytes())?;
            self.conn.execute(
                "UPDATE sources SET options_json = ?1, encrypted = 1 WHERE name = ?2",
                rusqlite::params![ciphertext, name],
            )?;
        }

        Ok(())
    }

    pub fn add_source(&self, name: &str, source: &SourceConfig) -> anyhow::Result<()> {
        let options_json = serde_json::to_string(&source.options)?;
        let encrypted_options = crypto::encrypt(&self.encryption_key, options_json.as_bytes())?;
        self.conn.execute(
            "INSERT OR REPLACE INTO sources (name, provider, options_json, metadata_ttl_secs, content_ttl_secs, encrypted)
             VALUES (?1, ?2, ?3, ?4, ?5, 1)",
            rusqlite::params![
                name,
                source.provider,
                encrypted_options,
                source.cache.metadata_ttl_secs,
                source.cache.content_ttl_secs,
            ],
        )?;
        Ok(())
    }

    pub fn remove_source(&self, name: &str) -> anyhow::Result<()> {
        self.conn
            .execute("DELETE FROM sources WHERE name = ?1", [name])?;
        self.conn.execute(
            "DELETE FROM source_local_roots WHERE source_name = ?1",
            [name],
        )?;
        self.conn
            .execute("DELETE FROM path_sync_state WHERE source_name = ?1", [name])?;
        self.conn.execute(
            "DELETE FROM local_scan_cache WHERE source_name = ?1",
            [name],
        )?;
        self.conn
            .execute("DELETE FROM remote_manifest WHERE source_name = ?1", [name])?;
        Ok(())
    }

    pub fn set_source_local_root(
        &self,
        source_name: &str,
        local_root: &Path,
    ) -> anyhow::Result<()> {
        let local_root_value = local_root.to_string_lossy();
        let existing = self.conn.query_row(
            "SELECT source_name FROM source_local_roots
             WHERE local_root = ?1 AND source_name != ?2",
            rusqlite::params![local_root_value.as_ref(), source_name],
            |row| row.get::<_, String>(0),
        );
        match existing {
            Ok(existing_source) => {
                anyhow::bail!(
                    "local root {} is already bound to source {}",
                    local_root.display(),
                    existing_source
                );
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {}
            Err(err) => return Err(err.into()),
        }

        self.conn.execute(
            "INSERT OR REPLACE INTO source_local_roots (source_name, local_root)
             VALUES (?1, ?2)",
            rusqlite::params![source_name, local_root_value.as_ref()],
        )?;
        Ok(())
    }

    pub fn clear_source_sync_state(&self, source_name: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "DELETE FROM path_sync_state WHERE source_name = ?1",
            [source_name],
        )?;
        self.conn.execute(
            "DELETE FROM local_scan_cache WHERE source_name = ?1",
            [source_name],
        )?;
        self.conn.execute(
            "DELETE FROM remote_manifest WHERE source_name = ?1",
            [source_name],
        )?;
        Ok(())
    }

    pub fn remove_source_local_root(&self, source_name: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "DELETE FROM source_local_roots WHERE source_name = ?1",
            [source_name],
        )?;
        Ok(())
    }

    pub fn get_source_local_root(&self, source_name: &str) -> anyhow::Result<Option<PathBuf>> {
        let mut stmt = self
            .conn
            .prepare("SELECT local_root FROM source_local_roots WHERE source_name = ?1")?;
        let row = stmt.query_row([source_name], |row| row.get::<_, String>(0));

        match row {
            Ok(local_root) => Ok(Some(PathBuf::from(local_root))),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    pub fn list_source_local_roots(&self) -> anyhow::Result<Vec<SourceLocalRootBinding>> {
        let mut stmt = self.conn.prepare(
            "SELECT source_name, local_root FROM source_local_roots ORDER BY source_name",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SourceLocalRootBinding {
                source_name: row.get(0)?,
                local_root: PathBuf::from(row.get::<_, String>(1)?),
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn upsert_path_sync_state(&self, record: &PathSyncStateRecord) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO path_sync_state (
                source_name,
                path,
                entry_kind,
                public_state,
                local_present,
                dirty_local,
                dirty_remote,
                pinned,
                stale,
                last_local_version,
                base_remote_version,
                current_remote_version
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(source_name, path) DO UPDATE SET
                entry_kind = excluded.entry_kind,
                public_state = excluded.public_state,
                local_present = excluded.local_present,
                dirty_local = excluded.dirty_local,
                dirty_remote = excluded.dirty_remote,
                pinned = excluded.pinned,
                stale = excluded.stale,
                last_local_version = excluded.last_local_version,
                base_remote_version = excluded.base_remote_version,
                current_remote_version = excluded.current_remote_version",
            rusqlite::params![
                record.source_name,
                record.path,
                record.entry_kind,
                record.public_state,
                i64::from(record.local_present),
                i64::from(record.dirty_local),
                i64::from(record.dirty_remote),
                i64::from(record.pinned),
                i64::from(record.stale),
                record.last_local_version,
                record.base_remote_version,
                record.current_remote_version,
            ],
        )?;
        Ok(())
    }

    pub fn get_path_sync_state(
        &self,
        source_name: &str,
        path: &str,
    ) -> anyhow::Result<Option<PathSyncStateRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT
                source_name,
                path,
                entry_kind,
                public_state,
                local_present,
                dirty_local,
                dirty_remote,
                pinned,
                stale,
                last_local_version,
                base_remote_version,
                current_remote_version
             FROM path_sync_state
             WHERE source_name = ?1 AND path = ?2",
        )?;
        let row = stmt.query_row(rusqlite::params![source_name, path], |row| {
            Ok(PathSyncStateRecord {
                source_name: row.get(0)?,
                path: row.get(1)?,
                entry_kind: row.get(2)?,
                public_state: row.get(3)?,
                local_present: row.get::<_, i64>(4)? != 0,
                dirty_local: row.get::<_, i64>(5)? != 0,
                dirty_remote: row.get::<_, i64>(6)? != 0,
                pinned: row.get::<_, i64>(7)? != 0,
                stale: row.get::<_, i64>(8)? != 0,
                last_local_version: row.get(9)?,
                base_remote_version: row.get(10)?,
                current_remote_version: row.get(11)?,
            })
        });

        match row {
            Ok(record) => Ok(Some(record)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    pub fn remove_path_sync_state(&self, source_name: &str, path: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "DELETE FROM path_sync_state WHERE source_name = ?1 AND path = ?2",
            rusqlite::params![source_name, path],
        )?;
        Ok(())
    }

    pub fn list_path_sync_states(
        &self,
        source_name: &str,
    ) -> anyhow::Result<Vec<PathSyncStateRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT
                source_name,
                path,
                entry_kind,
                public_state,
                local_present,
                dirty_local,
                dirty_remote,
                pinned,
                stale,
                last_local_version,
                base_remote_version,
                current_remote_version
             FROM path_sync_state
             WHERE source_name = ?1
             ORDER BY path",
        )?;
        let rows = stmt.query_map([source_name], |row| {
            Ok(PathSyncStateRecord {
                source_name: row.get(0)?,
                path: row.get(1)?,
                entry_kind: row.get(2)?,
                public_state: row.get(3)?,
                local_present: row.get::<_, i64>(4)? != 0,
                dirty_local: row.get::<_, i64>(5)? != 0,
                dirty_remote: row.get::<_, i64>(6)? != 0,
                pinned: row.get::<_, i64>(7)? != 0,
                stale: row.get::<_, i64>(8)? != 0,
                last_local_version: row.get(9)?,
                base_remote_version: row.get(10)?,
                current_remote_version: row.get(11)?,
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn list_local_scan_cache(
        &self,
        source_name: &str,
    ) -> anyhow::Result<Vec<LocalScanCacheRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT path, entry_kind, version, size_bytes, mtime_ms
             FROM local_scan_cache
             WHERE source_name = ?1
             ORDER BY path",
        )?;
        let rows = stmt.query_map([source_name], |row| {
            Ok(LocalScanCacheRecord {
                path: row.get(0)?,
                entry_kind: row.get(1)?,
                version: row.get(2)?,
                size: row.get::<_, Option<i64>>(3)?.map(|size| size as u64),
                mtime_ms: row.get(4)?,
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn replace_local_scan_cache(
        &self,
        source_name: &str,
        records: &[LocalScanCacheRecord],
    ) -> anyhow::Result<()> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> anyhow::Result<()> {
            self.conn.execute(
                "DELETE FROM local_scan_cache WHERE source_name = ?1",
                [source_name],
            )?;
            let mut stmt = self.conn.prepare(
                "INSERT INTO local_scan_cache (
                    source_name,
                    path,
                    entry_kind,
                    version,
                    size_bytes,
                    mtime_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;

            for record in records {
                stmt.execute(rusqlite::params![
                    source_name,
                    record.path,
                    record.entry_kind,
                    record.version,
                    record.size.map(|size| size as i64),
                    record.mtime_ms,
                ])?;
            }

            Ok(())
        })();

        match result {
            Ok(()) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(())
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }

    pub fn list_remote_manifest(
        &self,
        source_name: &str,
    ) -> anyhow::Result<Vec<RemoteManifestRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT path, entry_kind, version, size_bytes, mtime_ms
             FROM remote_manifest
             WHERE source_name = ?1
             ORDER BY path",
        )?;
        let rows = stmt.query_map([source_name], |row| {
            Ok(RemoteManifestRecord {
                path: row.get(0)?,
                entry_kind: row.get(1)?,
                version: row.get(2)?,
                size: row.get::<_, Option<i64>>(3)?.map(|size| size as u64),
                mtime_ms: row.get(4)?,
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn replace_remote_manifest(
        &self,
        source_name: &str,
        records: &[RemoteManifestRecord],
    ) -> anyhow::Result<()> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> anyhow::Result<()> {
            self.conn.execute(
                "DELETE FROM remote_manifest WHERE source_name = ?1",
                [source_name],
            )?;
            let mut stmt = self.conn.prepare(
                "INSERT INTO remote_manifest (
                    source_name,
                    path,
                    entry_kind,
                    version,
                    size_bytes,
                    mtime_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;

            for record in records {
                stmt.execute(rusqlite::params![
                    source_name,
                    record.path,
                    record.entry_kind,
                    record.version,
                    record.size.map(|size| size as i64),
                    record.mtime_ms,
                ])?;
            }

            Ok(())
        })();

        match result {
            Ok(()) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(())
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }

    pub fn append_sync_event(
        &self,
        source_name: &str,
        path: &str,
        kind: &str,
        state: &str,
        created_at_ms: i64,
    ) -> anyhow::Result<i64> {
        self.conn.execute(
            "INSERT INTO sync_events (source_name, path, kind, state, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![source_name, path, kind, state, created_at_ms],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_sync_events_after(
        &self,
        source_name: &str,
        after_id: i64,
    ) -> anyhow::Result<Vec<SyncEventRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, source_name, path, kind, state, created_at_ms
             FROM sync_events
             WHERE source_name = ?1 AND id > ?2
             ORDER BY id",
        )?;
        let rows = stmt.query_map(rusqlite::params![source_name, after_id], |row| {
            Ok(SyncEventRecord {
                id: row.get(0)?,
                source_name: row.get(1)?,
                path: row.get(2)?,
                kind: row.get(3)?,
                state: row.get(4)?,
                created_at_ms: row.get(5)?,
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn list_sources(&self) -> anyhow::Result<Vec<(String, String, HashMap<String, String>)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name, provider, options_json, encrypted FROM sources ORDER BY name")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;
        let mut sources = Vec::new();
        for row in rows {
            let (name, provider, raw_options, encrypted) = row?;
            let options_json = self.decrypt_options(&raw_options, encrypted)?;
            let options: HashMap<String, String> =
                serde_json::from_str(&options_json).unwrap_or_default();
            sources.push((name, provider, options));
        }
        Ok(sources)
    }

    /// Load all sources into a config-compatible map.
    pub fn load_all(&self) -> anyhow::Result<HashMap<String, SourceConfig>> {
        let mut result = HashMap::new();
        let mut stmt = self.conn.prepare(
            "SELECT name, provider, options_json, metadata_ttl_secs, content_ttl_secs, encrypted FROM sources",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, u64>(3)?,
                row.get::<_, u64>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })?;

        for row in rows {
            let (name, provider, raw_options, metadata_ttl, content_ttl, encrypted) = row?;
            let options_json = self.decrypt_options(&raw_options, encrypted)?;
            let options: HashMap<String, String> = serde_json::from_str(&options_json)?;
            result.insert(
                name,
                SourceConfig {
                    provider,
                    options,
                    cache: CacheConfig {
                        metadata_ttl_secs: metadata_ttl,
                        content_ttl_secs: content_ttl,
                    },
                },
            );
        }

        Ok(result)
    }

    /// Decrypt the stored options value. If `encrypted == 1`, decrypt via AES-256-GCM;
    /// otherwise treat the value as plaintext JSON (backward compatibility).
    fn decrypt_options(&self, raw: &str, encrypted: i64) -> anyhow::Result<String> {
        if encrypted == 1 {
            let plaintext_bytes = crypto::decrypt(&self.encryption_key, raw)?;
            Ok(String::from_utf8(plaintext_bytes)?)
        } else {
            Ok(raw.to_owned())
        }
    }
}

fn random_hex(byte_len: usize) -> anyhow::Result<String> {
    use ring::rand::{SecureRandom, SystemRandom};

    let rng = SystemRandom::new();
    let mut bytes = vec![0_u8; byte_len];
    rng.fill(&mut bytes)
        .map_err(|_| anyhow::anyhow!("failed to generate random bytes"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock drift")
        .as_millis() as i64
}

fn read_agentfs_mount_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentFsMountRecord> {
    Ok(AgentFsMountRecord {
        source_name: row.get(0)?,
        fs_id: row.get(1)?,
        source_profile_id: row.get(2)?,
        agent_id: row.get(3)?,
        installation_id: row.get(4)?,
        local_root: PathBuf::from(row.get::<_, String>(5)?),
        base_commit_id: row.get(6)?,
        updated_at_ms: row.get(7)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample_source(root: &str) -> SourceConfig {
        let mut options = HashMap::new();
        options.insert("root".to_string(), root.to_string());

        SourceConfig {
            provider: "fs".to_string(),
            options,
            cache: CacheConfig {
                metadata_ttl_secs: 12,
                content_ttl_secs: 34,
            },
        }
    }

    #[test]
    fn source_crud_round_trips_through_encrypted_store() {
        let temp_dir = TempDir::new().expect("temp dir");
        let store = ProviderStore::open(temp_dir.path()).expect("store");

        store
            .add_source("local", &sample_source("/tmp/local"))
            .expect("add source");

        let listed = store.list_sources().expect("list sources");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].0, "local");
        assert_eq!(listed[0].1, "fs");
        assert_eq!(
            listed[0].2.get("root").map(String::as_str),
            Some("/tmp/local")
        );

        let loaded = store.load_all().expect("load all");
        let loaded_source = loaded.get("local").expect("loaded source");
        assert_eq!(loaded_source.cache.metadata_ttl_secs, 12);
        assert_eq!(loaded_source.cache.content_ttl_secs, 34);

        store.remove_source("local").expect("remove source");
        assert!(store.list_sources().expect("list after remove").is_empty());
    }

    #[test]
    fn open_migrates_plaintext_rows_to_encrypted_rows() {
        let temp_dir = TempDir::new().expect("temp dir");
        let key_path = temp_dir.path().join("section.key");
        let _ = crypto::load_or_generate_key(&key_path).expect("key");

        let db_path = temp_dir.path().join("section.db");
        let conn = Connection::open(&db_path).expect("db");
        conn.execute_batch(
            "CREATE TABLE sources (
                name TEXT PRIMARY KEY,
                provider TEXT NOT NULL,
                options_json TEXT NOT NULL DEFAULT '{}',
                metadata_ttl_secs INTEGER NOT NULL DEFAULT 60,
                content_ttl_secs INTEGER NOT NULL DEFAULT 300,
                encrypted INTEGER NOT NULL DEFAULT 0
            );",
        )
        .expect("create legacy table");
        conn.execute(
            "INSERT INTO sources (name, provider, options_json, metadata_ttl_secs, content_ttl_secs, encrypted)
             VALUES (?1, ?2, ?3, ?4, ?5, 0)",
            rusqlite::params![
                "legacy",
                "fs",
                r#"{"root":"/tmp/legacy"}"#,
                7_u64,
                8_u64,
            ],
        )
        .expect("insert legacy row");
        drop(conn);

        let store = ProviderStore::open(temp_dir.path()).expect("store with migration");
        let loaded = store.load_all().expect("load migrated row");
        let migrated = loaded.get("legacy").expect("legacy source");
        assert_eq!(
            migrated.options.get("root").map(String::as_str),
            Some("/tmp/legacy")
        );
        assert_eq!(migrated.cache.metadata_ttl_secs, 7);
        assert_eq!(migrated.cache.content_ttl_secs, 8);

        let conn = Connection::open(&db_path).expect("db reopen");
        let (raw_options, encrypted): (String, i64) = conn
            .query_row(
                "SELECT options_json, encrypted FROM sources WHERE name = ?1",
                ["legacy"],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("migrated row");
        assert_eq!(encrypted, 1);
        assert_ne!(raw_options, r#"{"root":"/tmp/legacy"}"#);
    }

    #[test]
    fn source_local_root_bindings_round_trip() {
        let temp_dir = TempDir::new().expect("temp dir");
        let store = ProviderStore::open(temp_dir.path()).expect("store");
        let local_root = temp_dir.path().join("bound");

        store
            .set_source_local_root("local", &local_root)
            .expect("set local root");

        assert_eq!(
            store
                .get_source_local_root("local")
                .expect("get local root"),
            Some(local_root.clone())
        );

        let bindings = store
            .list_source_local_roots()
            .expect("list source local roots");
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].source_name, "local");
        assert_eq!(bindings[0].local_root, local_root);

        store
            .remove_source_local_root("local")
            .expect("remove local root");
        assert_eq!(
            store
                .get_source_local_root("local")
                .expect("get local root after remove"),
            None
        );
    }

    #[test]
    fn source_local_root_binding_rejects_cross_source_root_collision() {
        let temp_dir = TempDir::new().expect("temp dir");
        let store = ProviderStore::open(temp_dir.path()).expect("store");
        let local_root = temp_dir.path().join("bound");

        store
            .set_source_local_root("first", &local_root)
            .expect("set first root");
        let collision = store.set_source_local_root("second", &local_root);
        assert!(
            collision.is_err(),
            "second source must not replace first source binding"
        );

        let bindings = store
            .list_source_local_roots()
            .expect("list source local roots");
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].source_name, "first");
        assert_eq!(bindings[0].local_root, local_root);
    }

    #[test]
    fn agent_identity_registers_and_updates_display_name() {
        let temp_dir = TempDir::new().expect("temp dir");
        let store = ProviderStore::open(temp_dir.path()).expect("store");

        assert_eq!(
            store
                .get_agent_identity()
                .expect("identify before register"),
            None
        );

        let first = store
            .register_agent_identity("agent-a")
            .expect("register agent");
        assert!(first.agent_id.starts_with("agt_"));
        assert_eq!(first.agent_id.len(), 36);
        assert_eq!(first.name, "agent-a");

        let second = store
            .register_agent_identity("agent-a-renamed")
            .expect("update agent");
        assert_eq!(second.agent_id, first.agent_id);
        assert_eq!(second.name, "agent-a-renamed");

        let loaded = store
            .get_agent_identity()
            .expect("get identity")
            .expect("identity exists");
        assert_eq!(loaded, second);
    }

    #[test]
    fn path_sync_state_round_trips() {
        let temp_dir = TempDir::new().expect("temp dir");
        let store = ProviderStore::open(temp_dir.path()).expect("store");
        let record = PathSyncStateRecord {
            source_name: "local".to_string(),
            path: "nested/file.txt".to_string(),
            entry_kind: "file".to_string(),
            public_state: "conflict".to_string(),
            local_present: true,
            dirty_local: true,
            dirty_remote: false,
            pinned: false,
            stale: true,
            last_local_version: Some("l1".to_string()),
            base_remote_version: Some("v1".to_string()),
            current_remote_version: Some("v2".to_string()),
        };

        store
            .upsert_path_sync_state(&record)
            .expect("upsert path sync state");

        let loaded = store
            .get_path_sync_state("local", "nested/file.txt")
            .expect("get path sync state")
            .expect("path sync state should exist");
        assert_eq!(loaded, record);
    }

    #[test]
    fn sync_events_round_trip_in_order() {
        let temp_dir = TempDir::new().expect("temp dir");
        let store = ProviderStore::open(temp_dir.path()).expect("store");

        let first_id = store
            .append_sync_event("local", "a.txt", "state_changed", "syncing", 1000)
            .expect("append first event");
        let second_id = store
            .append_sync_event("local", "a.txt", "state_changed", "ready", 1001)
            .expect("append second event");

        let events = store
            .list_sync_events_after("local", first_id - 1)
            .expect("list events");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].id, first_id);
        assert_eq!(events[1].id, second_id);
        assert_eq!(events[0].path, "a.txt");
        assert_eq!(events[0].state, "syncing");
        assert_eq!(events[1].state, "ready");
    }

    #[test]
    fn local_scan_cache_round_trips() {
        let temp_dir = TempDir::new().expect("temp dir");
        let store = ProviderStore::open(temp_dir.path()).expect("store");
        let records = vec![
            LocalScanCacheRecord {
                path: "docs".to_string(),
                entry_kind: "dir".to_string(),
                version: None,
                size: None,
                mtime_ms: Some(10),
            },
            LocalScanCacheRecord {
                path: "docs/readme.txt".to_string(),
                entry_kind: "file".to_string(),
                version: Some("sha256:v1".to_string()),
                size: Some(42),
                mtime_ms: Some(11),
            },
        ];

        store
            .replace_local_scan_cache("local", &records)
            .expect("replace local scan cache");

        let loaded = store
            .list_local_scan_cache("local")
            .expect("list local scan cache");
        assert_eq!(loaded, records);
    }

    #[test]
    fn remote_manifest_round_trips() {
        let temp_dir = TempDir::new().expect("temp dir");
        let store = ProviderStore::open(temp_dir.path()).expect("store");
        let records = vec![RemoteManifestRecord {
            path: "docs/readme.txt".to_string(),
            entry_kind: "file".to_string(),
            version: Some("\"etag-v1\"".to_string()),
            size: Some(42),
            mtime_ms: Some(12),
        }];

        store
            .replace_remote_manifest("local", &records)
            .expect("replace remote manifest");

        let loaded = store
            .list_remote_manifest("local")
            .expect("list remote manifest");
        assert_eq!(loaded, records);
    }
}
