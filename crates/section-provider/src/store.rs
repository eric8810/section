use crate::crypto;
use anyhow::Context;
use rusqlite::Connection;
use section_core::config::{CacheConfig, SourceConfig};
use std::collections::HashMap;
use std::path::Path;

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
            );",
        )?;

        // Add `encrypted` column to existing tables that lack it.
        self.add_column_if_missing("sources", "encrypted", "INTEGER NOT NULL DEFAULT 0")?;

        Ok(())
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
        Ok(())
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
}
