use std::path::{Path, PathBuf};

use crate::settings::PlaybackSettings;

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaybackProgress {
    pub media_id: String,
    pub position_seconds: f64,
    pub duration_seconds: Option<f64>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredAddon {
    pub id: String,
    pub name: String,
    pub version: String,
    pub manifest_url: String,
    pub enabled: bool,
    pub manifest_json: String,
    pub installed_at: i64,
}

const CURRENT_SCHEMA_VERSION: i64 = 1;

pub struct LocalDatabase {
    connection: Connection,
}

impl LocalDatabase {
    /// Opens SENZO's per-user database under the operating system's local app-data directory.
    ///
    /// On Windows this resolves to %LOCALAPPDATA%\SENZO\senzo.db.
    pub fn open_default() -> Result<Self> {
        let app_data = dirs::data_local_dir()
            .context("the operating system did not provide a local app-data directory")?;
        let app_directory = app_data.join("SENZO");
        std::fs::create_dir_all(&app_directory)
            .with_context(|| format!("failed to create {}", app_directory.display()))?;
        Self::open(app_directory.join("senzo.db"))
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let mut connection = Connection::open(path)
            .with_context(|| format!("failed to open local database at {}", path.display()))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        let schema_version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .context("failed to read the local database schema version")?;
        if schema_version > CURRENT_SCHEMA_VERSION {
            bail!(
                "the local database schema version {schema_version} is newer than this SENZO build supports ({CURRENT_SCHEMA_VERSION})"
            );
        }

        if schema_version < 1 {
            let transaction = connection
                .transaction()
                .context("failed to begin the local database schema migration")?;
            transaction.execute_batch(
                "
                CREATE TABLE IF NOT EXISTS settings (
                    key TEXT PRIMARY KEY NOT NULL,
                    value TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS playback_progress (
                    media_id TEXT PRIMARY KEY NOT NULL,
                    position_seconds REAL NOT NULL,
                    duration_seconds REAL,
                    updated_at INTEGER NOT NULL
                );

                CREATE TABLE IF NOT EXISTS watch_history (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    media_id TEXT NOT NULL,
                    title TEXT NOT NULL,
                    media_type TEXT NOT NULL,
                    watched_at INTEGER NOT NULL
                );

                CREATE INDEX IF NOT EXISTS idx_watch_history_watched_at
                    ON watch_history(watched_at DESC);

                CREATE TABLE IF NOT EXISTS addons (
                    id TEXT PRIMARY KEY NOT NULL,
                    name TEXT NOT NULL,
                    version TEXT NOT NULL,
                    manifest_url TEXT NOT NULL,
                    enabled INTEGER NOT NULL DEFAULT 1,
                    manifest_json TEXT NOT NULL,
                    installed_at INTEGER NOT NULL
                );

                PRAGMA user_version = 1;
                ",
            )?;
            transaction
                .commit()
                .context("failed to commit the local database schema migration")?;
        }

        Ok(Self { connection })
    }

    pub fn path_for_default_database() -> Result<PathBuf> {
        let app_data = dirs::data_local_dir()
            .context("the operating system did not provide a local app-data directory")?;
        Ok(app_data.join("SENZO").join("senzo.db"))
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.connection.execute(
            "INSERT INTO settings(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .connection
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?)
    }

    /// Loads typed playback preferences. Missing or invalid legacy values use safe defaults.
    pub fn playback_settings(&self) -> Result<PlaybackSettings> {
        let mut settings = match self.get_setting("playback.preferences")? {
            Some(value) => serde_json::from_str::<PlaybackSettings>(&value).unwrap_or_default(),
            None => PlaybackSettings::default(),
        };
        settings.normalize();
        Ok(settings)
    }

    /// Persists normalized playback preferences as one atomic settings value.
    pub fn save_playback_settings(&self, settings: &PlaybackSettings) -> Result<()> {
        let mut settings = settings.clone();
        settings.normalize();
        let value =
            serde_json::to_string(&settings).context("failed to serialize playback preferences")?;
        self.set_setting("playback.preferences", &value)
    }

    pub fn save_playback_progress(
        &self,
        media_id: &str,
        position_seconds: f64,
        duration_seconds: Option<f64>,
        updated_at: i64,
    ) -> Result<()> {
        self.connection.execute(
            "INSERT INTO playback_progress
                (media_id, position_seconds, duration_seconds, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(media_id) DO UPDATE SET
                position_seconds = excluded.position_seconds,
                duration_seconds = excluded.duration_seconds,
                updated_at = excluded.updated_at",
            params![
                media_id,
                position_seconds.max(0.0),
                duration_seconds,
                updated_at
            ],
        )?;
        Ok(())
    }

    pub fn playback_progress(&self, media_id: &str) -> Result<Option<PlaybackProgress>> {
        Ok(self
            .connection
            .query_row(
                "SELECT media_id, position_seconds, duration_seconds, updated_at
                 FROM playback_progress WHERE media_id = ?1",
                [media_id],
                |row| {
                    Ok(PlaybackProgress {
                        media_id: row.get(0)?,
                        position_seconds: row.get(1)?,
                        duration_seconds: row.get(2)?,
                        updated_at: row.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn record_watch_history(
        &self,
        media_id: &str,
        title: &str,
        media_type: &str,
        watched_at: i64,
    ) -> Result<()> {
        self.connection.execute(
            "INSERT INTO watch_history(media_id, title, media_type, watched_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![media_id, title, media_type, watched_at],
        )?;
        Ok(())
    }

    pub fn save_addon(&self, addon: &StoredAddon) -> Result<()> {
        self.connection.execute(
            "INSERT INTO addons
                (id, name, version, manifest_url, enabled, manifest_json, installed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                version = excluded.version,
                manifest_url = excluded.manifest_url,
                enabled = excluded.enabled,
                manifest_json = excluded.manifest_json",
            params![
                addon.id,
                addon.name,
                addon.version,
                addon.manifest_url,
                addon.enabled,
                addon.manifest_json,
                addon.installed_at
            ],
        )?;
        Ok(())
    }

    pub fn installed_addons(&self) -> Result<Vec<StoredAddon>> {
        let mut statement = self.connection.prepare(
            "SELECT id, name, version, manifest_url, enabled, manifest_json, installed_at
             FROM addons ORDER BY name COLLATE NOCASE",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(StoredAddon {
                id: row.get(0)?,
                name: row.get(1)?,
                version: row.get(2)?,
                manifest_url: row.get(3)?,
                enabled: row.get(4)?,
                manifest_json: row.get(5)?,
                installed_at: row.get(6)?,
            })
        })?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("failed to read installed add-ons")
    }

    pub fn set_addon_enabled(&self, id: &str, enabled: bool) -> Result<bool> {
        let changed = self.connection.execute(
            "UPDATE addons SET enabled = ?2 WHERE id = ?1",
            params![id, enabled],
        )?;
        Ok(changed > 0)
    }

    pub fn remove_addon(&self, id: &str) -> Result<bool> {
        let removed = self
            .connection
            .execute("DELETE FROM addons WHERE id = ?1", [id])?;
        Ok(removed > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_and_settings_survive_database_reopen() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("senzo.db");

        {
            let database = LocalDatabase::open(&path).expect("open database");
            database
                .save_playback_progress("tmdb:movie:550", 123.5, Some(7200.0), 100)
                .expect("save progress");
            database
                .set_setting("metadata.language", "en-US")
                .expect("save setting");
        }

        let database = LocalDatabase::open(&path).expect("reopen database");
        let progress = database
            .playback_progress("tmdb:movie:550")
            .expect("read progress")
            .expect("progress exists");

        assert_eq!(progress.position_seconds, 123.5);
        assert_eq!(progress.duration_seconds, Some(7200.0));
        assert_eq!(
            database
                .get_setting("metadata.language")
                .expect("read setting")
                .as_deref(),
            Some("en-US")
        );
    }

    #[test]
    fn addon_registry_starts_empty_and_persists_entries() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let database =
            LocalDatabase::open(directory.path().join("senzo.db")).expect("open database");

        assert!(
            database
                .installed_addons()
                .expect("list add-ons")
                .is_empty()
        );

        database
            .save_addon(&StoredAddon {
                id: "org.example.addon".into(),
                name: "Example".into(),
                version: "1.0.0".into(),
                manifest_url: "https://example.org/manifest.json".into(),
                enabled: true,
                manifest_json: "{}".into(),
                installed_at: 1,
            })
            .expect("save add-on");

        assert_eq!(database.installed_addons().expect("list add-ons").len(), 1);
    }
    #[test]
    fn schema_migration_is_versioned_and_idempotent() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("senzo.db");

        for _ in 0..2 {
            let database = LocalDatabase::open(&path).expect("open migrated database");
            let version: i64 = database
                .connection
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .expect("read schema version");
            assert_eq!(version, CURRENT_SCHEMA_VERSION);

            let settings_table_exists: bool = database
                .connection
                .query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM sqlite_master
                        WHERE type = 'table' AND name = 'settings'
                    )",
                    [],
                    |row| row.get(0),
                )
                .expect("check settings table");
            assert!(settings_table_exists);
        }
    }

    #[test]
    fn refuses_a_database_from_a_newer_schema_version() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("senzo.db");
        let connection = Connection::open(&path).expect("create database");
        connection
            .pragma_update(None, "user_version", CURRENT_SCHEMA_VERSION + 1)
            .expect("set future schema version");
        drop(connection);

        let result = LocalDatabase::open(&path);
        assert!(result.is_err(), "future schema must be rejected");
        let error = result.err().expect("the error was checked above");
        assert!(
            error
                .to_string()
                .contains("newer than this SENZO build supports")
        );
    }
}
