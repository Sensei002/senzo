//! SQLite persistence with ordered, transactional schema migrations.

use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
    time::Duration,
};

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};
use senzo_addons::{AddonRecord, AddonStore, AddonStoreError};
use senzo_domain::{
    MediaKind, PlaybackProgress, PlaybackProgressRepository, Profile, ProfileRepository,
    RepositoryError, SettingKey, SettingsRepository, WatchedItem,
};
use thiserror::Error;

pub const CURRENT_SCHEMA_VERSION: i64 = 4;

const MIGRATIONS: &[(i64, &str)] = &[
    (
        1,
        "CREATE TABLE app_settings (
        key TEXT PRIMARY KEY NOT NULL,
        value TEXT NOT NULL,
        updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
    );",
    ),
    (
        2,
        "CREATE TABLE profiles (
        id TEXT PRIMARY KEY NOT NULL,
        name TEXT NOT NULL,
        created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
    );
    INSERT OR IGNORE INTO profiles (id, name) VALUES ('default', 'Default');",
    ),
    (
        3,
        "CREATE TABLE installed_addons (
            id TEXT NOT NULL,
            profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
            secret_key TEXT NOT NULL UNIQUE,
            manifest_json TEXT,
            allow_insecure_http INTEGER NOT NULL DEFAULT 0,
            enabled INTEGER NOT NULL DEFAULT 1,
            sort_order INTEGER NOT NULL DEFAULT 0,
            last_checked_at TEXT,
            last_error TEXT,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            PRIMARY KEY(profile_id, id)
        );
        CREATE INDEX installed_addons_profile_order_idx
            ON installed_addons(profile_id, sort_order, created_at);",
    ),
    (
        4,
        "CREATE TABLE playback_progress (
            profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
            media_kind TEXT NOT NULL CHECK (media_kind IN ('movie', 'series')),
            media_id TEXT NOT NULL,
            season_number INTEGER NOT NULL DEFAULT -1,
            episode_number INTEGER NOT NULL DEFAULT -1,
            title TEXT NOT NULL,
            episode_title TEXT,
            poster_path TEXT,
            backdrop_path TEXT,
            still_path TEXT,
            position_ms INTEGER NOT NULL CHECK (position_ms >= 0),
            duration_ms INTEGER NOT NULL CHECK (duration_ms >= 0),
            is_completed INTEGER NOT NULL DEFAULT 0,
            updated_at_ms INTEGER NOT NULL,
            PRIMARY KEY (profile_id, media_kind, media_id, season_number, episode_number),
            CHECK (
                (media_kind = 'movie' AND season_number = -1 AND episode_number = -1)
                OR
                (media_kind = 'series' AND season_number >= 0 AND episode_number >= 1)
            )
        );
        CREATE INDEX playback_progress_profile_updated_idx
            ON playback_progress(profile_id, updated_at_ms DESC);
        CREATE TABLE watched_items (
            profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
            media_kind TEXT NOT NULL CHECK (media_kind IN ('movie', 'series')),
            media_id TEXT NOT NULL,
            season_number INTEGER NOT NULL DEFAULT -1,
            episode_number INTEGER NOT NULL DEFAULT -1,
            marked_at_ms INTEGER NOT NULL,
            PRIMARY KEY (profile_id, media_kind, media_id, season_number, episode_number),
            CHECK (
                (media_kind = 'movie' AND season_number = -1 AND episode_number = -1)
                OR
                (media_kind = 'series' AND season_number >= 0 AND episode_number >= 1)
            )
        );
        CREATE INDEX watched_items_profile_media_idx
            ON watched_items(profile_id, media_kind, media_id);",
    ),
];

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("database directory could not be created: {0}")]
    Io(#[from] std::io::Error),
    #[error("database schema version {found} is newer than supported version {supported}")]
    UnsupportedSchema { found: i64, supported: i64 },
    #[error("database lock is poisoned")]
    LockPoisoned,
}

pub struct SqliteDatabase {
    connection: Mutex<Connection>,
}

impl SqliteDatabase {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }
        Self::initialize(Connection::open(path)?)
    }

    pub fn in_memory() -> Result<Self, StorageError> {
        Self::initialize(Connection::open_in_memory()?)
    }

    fn initialize(mut connection: Connection) -> Result<Self, StorageError> {
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;",
        )?;
        run_migrations(&mut connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn schema_version(&self) -> Result<i64, StorageError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| StorageError::LockPoisoned)?;
        read_schema_version(&connection).map_err(StorageError::Sqlite)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, RepositoryError> {
        self.connection
            .lock()
            .map_err(|_| RepositoryError::Unavailable("database lock is poisoned".to_owned()))
    }
}

fn run_migrations(connection: &mut Connection) -> Result<(), StorageError> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY NOT NULL,
            applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );",
    )?;
    let current = read_schema_version(&transaction)?;
    if current > CURRENT_SCHEMA_VERSION {
        return Err(StorageError::UnsupportedSchema {
            found: current,
            supported: CURRENT_SCHEMA_VERSION,
        });
    }
    apply_migrations(&transaction, current)?;
    transaction.commit()?;
    Ok(())
}

fn apply_migrations(transaction: &Transaction<'_>, current: i64) -> Result<(), rusqlite::Error> {
    for (version, sql) in MIGRATIONS {
        if *version > current {
            transaction.execute_batch(sql)?;
            transaction.execute(
                "INSERT INTO schema_migrations (version) VALUES (?1)",
                [version],
            )?;
        }
    }
    Ok(())
}

fn read_schema_version(connection: &Connection) -> Result<i64, rusqlite::Error> {
    connection.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |row| row.get(0),
    )
}

impl AddonStore for SqliteDatabase {
    fn list(&self, profile_id: &str) -> Result<Vec<AddonRecord>, AddonStoreError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| AddonStoreError::Unavailable("database lock is poisoned".to_owned()))?;
        let mut statement = connection
            .prepare(
                "SELECT id, secret_key, manifest_json, allow_insecure_http, enabled,
                    sort_order, last_checked_at, last_error
             FROM installed_addons
             WHERE profile_id = ?1
             ORDER BY sort_order ASC, created_at ASC, id ASC",
            )
            .map_err(|error| AddonStoreError::Unavailable(error.to_string()))?;
        let records = statement
            .query_map([profile_id], |row| {
                Ok(AddonRecord {
                    id: row.get(0)?,
                    secret_key: row.get(1)?,
                    manifest_json: row.get(2)?,
                    allow_insecure_http: row.get::<_, i64>(3)? != 0,
                    enabled: row.get::<_, i64>(4)? != 0,
                    sort_order: row.get(5)?,
                    last_checked_at: row.get(6)?,
                    last_error: row.get(7)?,
                })
            })
            .map_err(|error| AddonStoreError::Unavailable(error.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| AddonStoreError::Unavailable(error.to_string()))?;
        Ok(records)
    }

    fn insert(&self, profile_id: &str, record: &AddonRecord) -> Result<(), AddonStoreError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| AddonStoreError::Unavailable("database lock is poisoned".to_owned()))?;
        connection
            .execute(
                "INSERT INTO installed_addons
             (id, profile_id, secret_key, manifest_json, allow_insecure_http, enabled,
              sort_order, last_checked_at, last_error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                (
                    &record.id,
                    profile_id,
                    &record.secret_key,
                    &record.manifest_json,
                    i64::from(record.allow_insecure_http),
                    i64::from(record.enabled),
                    record.sort_order,
                    &record.last_checked_at,
                    &record.last_error,
                ),
            )
            .map_err(|error| AddonStoreError::Unavailable(error.to_string()))?;
        Ok(())
    }

    fn update(&self, profile_id: &str, record: &AddonRecord) -> Result<(), AddonStoreError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| AddonStoreError::Unavailable("database lock is poisoned".to_owned()))?;
        let changed = connection
            .execute(
                "UPDATE installed_addons
             SET manifest_json = ?1, allow_insecure_http = ?2, enabled = ?3,
                 sort_order = ?4, last_checked_at = ?5, last_error = ?6,
                 updated_at = CURRENT_TIMESTAMP
             WHERE profile_id = ?7 AND id = ?8 AND secret_key = ?9",
                (
                    &record.manifest_json,
                    i64::from(record.allow_insecure_http),
                    i64::from(record.enabled),
                    record.sort_order,
                    &record.last_checked_at,
                    &record.last_error,
                    profile_id,
                    &record.id,
                    &record.secret_key,
                ),
            )
            .map_err(|error| AddonStoreError::Unavailable(error.to_string()))?;
        if changed == 0 {
            return Err(AddonStoreError::NotFound);
        }
        Ok(())
    }

    fn remove(&self, profile_id: &str, id: &str) -> Result<(), AddonStoreError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| AddonStoreError::Unavailable("database lock is poisoned".to_owned()))?;
        let changed = connection
            .execute(
                "DELETE FROM installed_addons WHERE profile_id = ?1 AND id = ?2",
                (profile_id, id),
            )
            .map_err(|error| AddonStoreError::Unavailable(error.to_string()))?;
        if changed == 0 {
            return Err(AddonStoreError::NotFound);
        }
        Ok(())
    }
}

impl SettingsRepository for SqliteDatabase {
    fn get(&self, key: &SettingKey) -> Result<Option<String>, RepositoryError> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT value FROM app_settings WHERE key = ?1",
                [key.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))
    }

    fn set(&self, key: &SettingKey, value: &str) -> Result<(), RepositoryError> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO app_settings (key, value, updated_at)
             VALUES (?1, ?2, CURRENT_TIMESTAMP)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP",
                (key.as_str(), value),
            )
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        Ok(())
    }
}

impl ProfileRepository for SqliteDatabase {
    fn list_profiles(&self) -> Result<Vec<Profile>, RepositoryError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare("SELECT id, name FROM profiles ORDER BY created_at, id")
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        let profiles = statement
            .query_map([], |row| {
                Ok(Profile {
                    id: row.get(0)?,
                    name: row.get(1)?,
                })
            })
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        Ok(profiles)
    }

    fn create_profile(&self, name: &str) -> Result<Profile, RepositoryError> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
            return Err(RepositoryError::InvalidProfileName);
        }
        let profile = Profile {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_owned(),
        };
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO profiles (id, name) VALUES (?1, ?2)",
                (&profile.id, &profile.name),
            )
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        Ok(profile)
    }
}

impl PlaybackProgressRepository for SqliteDatabase {
    fn get_playback_progress(
        &self,
        profile_id: &str,
        media_id: &str,
        media_kind: MediaKind,
        season_number: Option<u32>,
        episode_number: Option<u32>,
    ) -> Result<Option<PlaybackProgress>, RepositoryError> {
        validate_watch_identity(
            profile_id,
            media_id,
            media_kind,
            season_number,
            episode_number,
        )?;
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT profile_id, media_id, media_kind, title, season_number, episode_number,
                        episode_title, poster_path, backdrop_path, still_path, position_ms,
                        duration_ms, is_completed, updated_at_ms
                 FROM playback_progress
                 WHERE profile_id = ?1 AND media_id = ?2 AND media_kind = ?3
                   AND season_number = ?4 AND episode_number = ?5",
                (
                    profile_id,
                    media_id,
                    media_kind_storage_value(media_kind),
                    stored_episode_number(season_number),
                    stored_episode_number(episode_number),
                ),
                playback_progress_from_row,
            )
            .optional()
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))
    }

    fn save_playback_progress(&self, progress: &PlaybackProgress) -> Result<(), RepositoryError> {
        validate_playback_progress(progress)?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| RepositoryError::Unavailable("database lock is poisoned".to_owned()))?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        transaction
            .execute(
                "INSERT INTO playback_progress (
                    profile_id, media_kind, media_id, season_number, episode_number, title,
                    episode_title, poster_path, backdrop_path, still_path, position_ms,
                    duration_ms, is_completed, updated_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                 ON CONFLICT(profile_id, media_kind, media_id, season_number, episode_number)
                 DO UPDATE SET
                    title = excluded.title,
                    episode_title = excluded.episode_title,
                    poster_path = excluded.poster_path,
                    backdrop_path = excluded.backdrop_path,
                    still_path = excluded.still_path,
                    position_ms = excluded.position_ms,
                    duration_ms = excluded.duration_ms,
                    is_completed = excluded.is_completed,
                    updated_at_ms = excluded.updated_at_ms
                 WHERE excluded.updated_at_ms >= playback_progress.updated_at_ms",
                (
                    &progress.profile_id,
                    media_kind_storage_value(progress.media_kind),
                    &progress.media_id,
                    stored_episode_number(progress.season_number),
                    stored_episode_number(progress.episode_number),
                    &progress.title,
                    &progress.episode_title,
                    &progress.poster_path,
                    &progress.backdrop_path,
                    &progress.still_path,
                    i64::try_from(progress.position_ms)
                        .map_err(|_| RepositoryError::InvalidPlaybackProgress)?,
                    i64::try_from(progress.duration_ms)
                        .map_err(|_| RepositoryError::InvalidPlaybackProgress)?,
                    i64::from(progress.is_completed),
                    progress.updated_at_ms,
                ),
            )
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        if progress.is_completed {
            transaction
                .execute(
                    "INSERT INTO watched_items
                        (profile_id, media_kind, media_id, season_number, episode_number, marked_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(profile_id, media_kind, media_id, season_number, episode_number)
                     DO UPDATE SET marked_at_ms = MAX(watched_items.marked_at_ms, excluded.marked_at_ms)",
                    (
                        &progress.profile_id,
                        media_kind_storage_value(progress.media_kind),
                        &progress.media_id,
                        stored_episode_number(progress.season_number),
                        stored_episode_number(progress.episode_number),
                        progress.updated_at_ms,
                    ),
                )
                .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        }
        transaction
            .commit()
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))
    }

    fn list_continue_watching(
        &self,
        profile_id: &str,
        limit: usize,
    ) -> Result<Vec<PlaybackProgress>, RepositoryError> {
        validate_profile_id(profile_id)?;
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "WITH ranked AS (
                    SELECT profile_id, media_id, media_kind, title, season_number, episode_number,
                           episode_title, poster_path, backdrop_path, still_path, position_ms,
                           duration_ms, is_completed, updated_at_ms,
                           ROW_NUMBER() OVER (
                               PARTITION BY media_kind, media_id
                               ORDER BY updated_at_ms DESC, season_number DESC, episode_number DESC
                           ) AS recency_rank
                    FROM playback_progress
                    WHERE profile_id = ?1
                 )
                 SELECT profile_id, media_id, media_kind, title, season_number, episode_number,
                        episode_title, poster_path, backdrop_path, still_path, position_ms,
                        duration_ms, is_completed, updated_at_ms
                 FROM ranked
                 WHERE recency_rank = 1 AND is_completed = 0 AND position_ms >= 1000
                   AND (duration_ms = 0 OR duration_ms >= 121000)
                   AND (duration_ms = 0 OR CAST(position_ms AS REAL) < CAST(duration_ms AS REAL) * 0.9)
                 ORDER BY updated_at_ms DESC, media_id ASC
                 LIMIT ?2",
            )
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        let rows = statement
            .query_map(
                (profile_id, limit.clamp(1, 100) as i64),
                playback_progress_from_row,
            )
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))
    }

    fn list_recent_completed_series(
        &self,
        profile_id: &str,
        limit: usize,
    ) -> Result<Vec<PlaybackProgress>, RepositoryError> {
        validate_profile_id(profile_id)?;
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "WITH completed AS (
                    SELECT w.profile_id, w.media_id, w.media_kind,
                           COALESCE(p.title, w.media_id) AS title,
                           w.season_number, w.episode_number, p.episode_title,
                           p.poster_path, p.backdrop_path, p.still_path,
                           COALESCE(p.position_ms, 0) AS position_ms,
                           COALESCE(p.duration_ms, 0) AS duration_ms,
                           1 AS is_completed,
                           MAX(w.marked_at_ms, COALESCE(p.updated_at_ms, 0)) AS updated_at_ms
                    FROM watched_items w
                    LEFT JOIN playback_progress p
                      ON p.profile_id = w.profile_id
                     AND p.media_kind = w.media_kind
                     AND p.media_id = w.media_id
                     AND p.season_number = w.season_number
                     AND p.episode_number = w.episode_number
                    WHERE w.profile_id = ?1 AND w.media_kind = 'series'
                    UNION ALL
                    SELECT p.profile_id, p.media_id, p.media_kind, p.title,
                           p.season_number, p.episode_number, p.episode_title,
                           p.poster_path, p.backdrop_path, p.still_path,
                           p.position_ms, p.duration_ms, p.is_completed, p.updated_at_ms
                    FROM playback_progress p
                    WHERE p.profile_id = ?1 AND p.media_kind = 'series'
                      AND p.is_completed = 1
                      AND NOT EXISTS (
                          SELECT 1 FROM watched_items w
                          WHERE w.profile_id = p.profile_id
                            AND w.media_kind = p.media_kind
                            AND w.media_id = p.media_id
                            AND w.season_number = p.season_number
                            AND w.episode_number = p.episode_number
                      )
                 ),
                 eligible AS (
                    SELECT c.*
                    FROM completed c
                    WHERE NOT EXISTS (
                        SELECT 1 FROM playback_progress active
                        WHERE active.profile_id = c.profile_id
                          AND active.media_kind = 'series'
                          AND active.media_id = c.media_id
                          AND active.is_completed = 0
                          AND active.position_ms >= 1000
                          AND active.updated_at_ms > c.updated_at_ms
                    )
                 ),
                 ranked AS (
                    SELECT profile_id, media_id, media_kind, title, season_number, episode_number,
                           episode_title, poster_path, backdrop_path, still_path, position_ms,
                           duration_ms, is_completed, updated_at_ms,
                           ROW_NUMBER() OVER (
                               PARTITION BY media_id
                               ORDER BY updated_at_ms DESC, season_number DESC, episode_number DESC
                           ) AS recency_rank
                    FROM eligible
                 )
                 SELECT profile_id, media_id, media_kind, title, season_number, episode_number,
                        episode_title, poster_path, backdrop_path, still_path, position_ms,
                        duration_ms, is_completed, updated_at_ms
                 FROM ranked
                 WHERE recency_rank = 1
                 ORDER BY updated_at_ms DESC, media_id ASC
                 LIMIT ?2",
            )
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        let rows = statement
            .query_map(
                (profile_id, limit.clamp(1, 100) as i64),
                playback_progress_from_row,
            )
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))
    }

    fn is_watched(&self, item: &WatchedItem) -> Result<bool, RepositoryError> {
        validate_watch_identity(
            &item.profile_id,
            &item.media_id,
            item.media_kind,
            item.season_number,
            item.episode_number,
        )?;
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM watched_items
                    WHERE profile_id = ?1 AND media_kind = ?2 AND media_id = ?3
                      AND season_number = ?4 AND episode_number = ?5
                )",
                (
                    &item.profile_id,
                    media_kind_storage_value(item.media_kind),
                    &item.media_id,
                    stored_episode_number(item.season_number),
                    stored_episode_number(item.episode_number),
                ),
                |row| row.get(0),
            )
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))
    }

    fn set_watched(&self, item: &WatchedItem, watched: bool) -> Result<(), RepositoryError> {
        validate_watch_identity(
            &item.profile_id,
            &item.media_id,
            item.media_kind,
            item.season_number,
            item.episode_number,
        )?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| RepositoryError::Unavailable("database lock is poisoned".to_owned()))?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        if watched {
            transaction
                .execute(
                    "INSERT INTO watched_items
                        (profile_id, media_kind, media_id, season_number, episode_number, marked_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(profile_id, media_kind, media_id, season_number, episode_number)
                     DO UPDATE SET marked_at_ms = MAX(watched_items.marked_at_ms, excluded.marked_at_ms)",
                    (
                        &item.profile_id,
                        media_kind_storage_value(item.media_kind),
                        &item.media_id,
                        stored_episode_number(item.season_number),
                        stored_episode_number(item.episode_number),
                        item.marked_at_ms,
                    ),
                )
                .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
            transaction
                .execute(
                    "UPDATE playback_progress SET is_completed = 1,
                        updated_at_ms = MAX(updated_at_ms, ?6)
                     WHERE profile_id = ?1 AND media_kind = ?2 AND media_id = ?3
                       AND season_number = ?4 AND episode_number = ?5",
                    (
                        &item.profile_id,
                        media_kind_storage_value(item.media_kind),
                        &item.media_id,
                        stored_episode_number(item.season_number),
                        stored_episode_number(item.episode_number),
                        item.marked_at_ms,
                    ),
                )
                .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        } else {
            transaction
                .execute(
                    "DELETE FROM watched_items
                     WHERE profile_id = ?1 AND media_kind = ?2 AND media_id = ?3
                       AND season_number = ?4 AND episode_number = ?5",
                    (
                        &item.profile_id,
                        media_kind_storage_value(item.media_kind),
                        &item.media_id,
                        stored_episode_number(item.season_number),
                        stored_episode_number(item.episode_number),
                    ),
                )
                .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
            transaction
                .execute(
                    "UPDATE playback_progress SET is_completed = 0,
                        updated_at_ms = MAX(updated_at_ms, ?6)
                     WHERE profile_id = ?1 AND media_kind = ?2 AND media_id = ?3
                       AND season_number = ?4 AND episode_number = ?5",
                    (
                        &item.profile_id,
                        media_kind_storage_value(item.media_kind),
                        &item.media_id,
                        stored_episode_number(item.season_number),
                        stored_episode_number(item.episode_number),
                        item.marked_at_ms,
                    ),
                )
                .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        }
        transaction
            .commit()
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))
    }
}

fn validate_profile_id(profile_id: &str) -> Result<(), RepositoryError> {
    if profile_id.trim().is_empty()
        || profile_id.len() > 128
        || profile_id.chars().any(char::is_control)
    {
        return Err(RepositoryError::InvalidPlaybackProgress);
    }
    Ok(())
}

fn validate_watch_identity(
    profile_id: &str,
    media_id: &str,
    media_kind: MediaKind,
    season_number: Option<u32>,
    episode_number: Option<u32>,
) -> Result<(), RepositoryError> {
    validate_profile_id(profile_id)?;
    if media_id.trim().is_empty()
        || media_id.len() > 128
        || media_id.chars().any(char::is_control)
        || match media_kind {
            MediaKind::Movie => season_number.is_some() || episode_number.is_some(),
            MediaKind::Series => {
                season_number.is_none()
                    || episode_number.is_none()
                    || season_number.is_some_and(|season| season > 500)
                    || episode_number.is_some_and(|episode| episode == 0 || episode > 2_000)
            }
            MediaKind::Episode => true,
        }
    {
        return Err(RepositoryError::InvalidPlaybackProgress);
    }
    Ok(())
}

fn validate_playback_progress(progress: &PlaybackProgress) -> Result<(), RepositoryError> {
    validate_watch_identity(
        &progress.profile_id,
        &progress.media_id,
        progress.media_kind,
        progress.season_number,
        progress.episode_number,
    )?;
    if progress.title.trim().is_empty()
        || progress.title.chars().count() > 512
        || progress.title.chars().any(char::is_control)
        || progress
            .episode_title
            .as_ref()
            .is_some_and(|title| title.chars().count() > 512 || title.chars().any(char::is_control))
        || progress.updated_at_ms < 0
        || i64::try_from(progress.position_ms).is_err()
        || i64::try_from(progress.duration_ms).is_err()
        || !valid_artwork_path(progress.poster_path.as_deref())
        || !valid_artwork_path(progress.backdrop_path.as_deref())
        || !valid_artwork_path(progress.still_path.as_deref())
    {
        return Err(RepositoryError::InvalidPlaybackProgress);
    }
    Ok(())
}

fn valid_artwork_path(path: Option<&str>) -> bool {
    let Some(path) = path else { return true };
    let Some(relative) = path.strip_prefix('/') else {
        return false;
    };
    !relative.is_empty()
        && path.len() <= 512
        && !path.chars().any(char::is_control)
        && relative.split('/').all(|segment| {
            !segment.is_empty()
                && segment != "."
                && segment != ".."
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        })
}

fn media_kind_storage_value(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Movie => "movie",
        MediaKind::Series => "series",
        MediaKind::Episode => "episode",
    }
}

fn stored_episode_number(value: Option<u32>) -> i64 {
    value.map_or(-1, i64::from)
}

fn playback_progress_from_row(
    row: &rusqlite::Row<'_>,
) -> Result<PlaybackProgress, rusqlite::Error> {
    let kind: String = row.get(2)?;
    let media_kind = match kind.as_str() {
        "movie" => MediaKind::Movie,
        "series" => MediaKind::Series,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    let season: i64 = row.get(4)?;
    let episode: i64 = row.get(5)?;
    let position: i64 = row.get(10)?;
    let duration: i64 = row.get(11)?;
    Ok(PlaybackProgress {
        profile_id: row.get(0)?,
        media_id: row.get(1)?,
        media_kind,
        title: row.get(3)?,
        season_number: u32::try_from(season).ok(),
        episode_number: u32::try_from(episode).ok(),
        episode_title: row.get(6)?,
        poster_path: row.get(7)?,
        backdrop_path: row.get(8)?,
        still_path: row.get(9)?,
        position_ms: u64::try_from(position).unwrap_or_default(),
        duration_ms: u64::try_from(duration).unwrap_or_default(),
        is_completed: row.get::<_, i64>(12)? != 0,
        updated_at_ms: row.get(13)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use senzo_domain::{ProfileRepository, SettingKey, SettingsRepository};

    #[test]
    fn playback_progress_is_profile_scoped_and_survives_repository_reads() {
        let database = SqliteDatabase::in_memory().unwrap();
        let movie = PlaybackProgress {
            profile_id: "default".to_owned(),
            media_id: "550".to_owned(),
            media_kind: MediaKind::Movie,
            title: "Fight Club".to_owned(),
            season_number: None,
            episode_number: None,
            episode_title: None,
            poster_path: Some("/poster.jpg".to_owned()),
            backdrop_path: None,
            still_path: None,
            position_ms: 42_000,
            duration_ms: 7_200_000,
            is_completed: false,
            updated_at_ms: 100,
        };
        database.save_playback_progress(&movie).unwrap();
        assert_eq!(
            database
                .get_playback_progress("default", "550", MediaKind::Movie, None, None)
                .unwrap(),
            Some(movie.clone())
        );
        let guest = database.create_profile("Guest").unwrap();
        assert!(
            database
                .get_playback_progress(&guest.id, "550", MediaKind::Movie, None, None)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            database.list_continue_watching("default", 20).unwrap(),
            vec![movie]
        );
    }

    #[test]
    fn continue_watching_returns_only_the_latest_incomplete_episode_per_series() {
        let database = SqliteDatabase::in_memory().unwrap();
        let make = |season, episode, position, completed, updated| PlaybackProgress {
            profile_id: "default".to_owned(),
            media_id: "1399".to_owned(),
            media_kind: MediaKind::Series,
            title: "Game of Thrones".to_owned(),
            season_number: Some(season),
            episode_number: Some(episode),
            episode_title: Some(format!("Episode {episode}")),
            poster_path: None,
            backdrop_path: None,
            still_path: None,
            position_ms: position,
            duration_ms: 2_700_000,
            is_completed: completed,
            updated_at_ms: updated,
        };
        database
            .save_playback_progress(&make(1, 1, 2_000_000, false, 100))
            .unwrap();
        database
            .save_playback_progress(&make(1, 2, 900_000, false, 200))
            .unwrap();
        let entries = database.list_continue_watching("default", 20).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            (entries[0].season_number, entries[0].episode_number),
            (Some(1), Some(2))
        );

        database
            .save_playback_progress(&make(1, 2, 2_500_000, true, 300))
            .unwrap();
        assert!(
            database
                .list_continue_watching("default", 20)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn watched_markers_and_progress_completion_are_transactional() {
        let database = SqliteDatabase::in_memory().unwrap();
        let progress = PlaybackProgress {
            profile_id: "default".to_owned(),
            media_id: "1399".to_owned(),
            media_kind: MediaKind::Series,
            title: "Game of Thrones".to_owned(),
            season_number: Some(1),
            episode_number: Some(1),
            episode_title: None,
            poster_path: None,
            backdrop_path: None,
            still_path: None,
            position_ms: 2_000_000,
            duration_ms: 2_700_000,
            is_completed: false,
            updated_at_ms: 100,
        };
        database.save_playback_progress(&progress).unwrap();
        let watched = WatchedItem {
            profile_id: "default".to_owned(),
            media_id: "1399".to_owned(),
            media_kind: MediaKind::Series,
            season_number: Some(1),
            episode_number: Some(1),
            marked_at_ms: 200,
        };
        database.set_watched(&watched, true).unwrap();
        assert!(database.is_watched(&watched).unwrap());
        assert!(
            database
                .get_playback_progress("default", "1399", MediaKind::Series, Some(1), Some(1))
                .unwrap()
                .unwrap()
                .is_completed
        );
        database.set_watched(&watched, false).unwrap();
        assert!(!database.is_watched(&watched).unwrap());
        assert!(
            !database
                .get_playback_progress("default", "1399", MediaKind::Series, Some(1), Some(1))
                .unwrap()
                .unwrap()
                .is_completed
        );
    }

    #[test]
    fn recent_completed_series_are_deduplicated_and_newer_progress_suppresses_up_next() {
        let database = SqliteDatabase::in_memory().unwrap();
        let make =
            |media_id: &str, season, episode, completed, position, updated| PlaybackProgress {
                profile_id: "default".to_owned(),
                media_id: media_id.to_owned(),
                media_kind: MediaKind::Series,
                title: format!("Series {media_id}"),
                season_number: Some(season),
                episode_number: Some(episode),
                episode_title: None,
                poster_path: None,
                backdrop_path: None,
                still_path: None,
                position_ms: position,
                duration_ms: 2_700_000,
                is_completed: completed,
                updated_at_ms: updated,
            };
        database
            .save_playback_progress(&make("100", 1, 1, true, 2_500_000, 100))
            .unwrap();
        database
            .save_playback_progress(&make("100", 1, 2, true, 2_500_000, 200))
            .unwrap();
        database
            .save_playback_progress(&make("200", 2, 5, true, 2_500_000, 300))
            .unwrap();
        database
            .save_playback_progress(&make("200", 2, 6, false, 50_000, 400))
            .unwrap();
        let entries = database
            .list_recent_completed_series("default", 20)
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].media_id, "100");
        assert_eq!(entries[0].episode_number, Some(2));
    }

    #[test]
    fn phase11_continue_watching_hides_progress_at_the_completion_threshold() {
        let database = SqliteDatabase::in_memory().unwrap();
        let progress = PlaybackProgress {
            profile_id: "default".to_owned(),
            media_id: "1399".to_owned(),
            media_kind: MediaKind::Series,
            title: "Game of Thrones".to_owned(),
            season_number: Some(1),
            episode_number: Some(1),
            episode_title: Some("Winter Is Coming".to_owned()),
            poster_path: None,
            backdrop_path: None,
            still_path: None,
            position_ms: 2_430_000,
            duration_ms: 2_700_000,
            is_completed: false,
            updated_at_ms: 100,
        };
        database.save_playback_progress(&progress).unwrap();
        assert!(database.list_continue_watching("default", 20).unwrap().is_empty());
    }

    #[test]
    fn phase11_manually_watched_episode_without_progress_advances_up_next() {
        let database = SqliteDatabase::in_memory().unwrap();
        let watched = WatchedItem {
            profile_id: "default".to_owned(),
            media_id: "1399".to_owned(),
            media_kind: MediaKind::Series,
            season_number: Some(1),
            episode_number: Some(4),
            marked_at_ms: 200,
        };
        database.set_watched(&watched, true).unwrap();
        let completed = database.list_recent_completed_series("default", 20).unwrap();
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].media_id, "1399");
        assert_eq!((completed[0].season_number, completed[0].episode_number), (Some(1), Some(4)));
        assert!(completed[0].is_completed);
        assert_eq!(completed[0].updated_at_ms, 200);

        let later_progress = PlaybackProgress {
            profile_id: "default".to_owned(),
            media_id: "1399".to_owned(),
            media_kind: MediaKind::Series,
            title: "Game of Thrones".to_owned(),
            season_number: Some(1),
            episode_number: Some(5),
            episode_title: Some("The Wolf and the Lion".to_owned()),
            poster_path: None,
            backdrop_path: None,
            still_path: None,
            position_ms: 50_000,
            duration_ms: 2_700_000,
            is_completed: false,
            updated_at_ms: 300,
        };
        database.save_playback_progress(&later_progress).unwrap();
        assert!(database.list_recent_completed_series("default", 20).unwrap().is_empty());
    }

    #[test]
    fn phase11_progress_and_watched_state_survive_database_reopen() {
        let path = std::env::temp_dir().join(format!("senzo-phase11-{}.sqlite", uuid::Uuid::new_v4()));
        let progress = PlaybackProgress {
            profile_id: "default".to_owned(),
            media_id: "1399".to_owned(),
            media_kind: MediaKind::Series,
            title: "Game of Thrones".to_owned(),
            season_number: Some(2),
            episode_number: Some(3),
            episode_title: Some("What Is Dead May Never Die".to_owned()),
            poster_path: Some("/poster.jpg".to_owned()),
            backdrop_path: None,
            still_path: Some("/still.jpg".to_owned()),
            position_ms: 640_000,
            duration_ms: 2_700_000,
            is_completed: false,
            updated_at_ms: 500,
        };
        {
            let database = SqliteDatabase::open(&path).unwrap();
            database.save_playback_progress(&progress).unwrap();
            let watched = WatchedItem {
                profile_id: "default".to_owned(),
                media_id: "1399".to_owned(),
                media_kind: MediaKind::Series,
                season_number: Some(2),
                episode_number: Some(2),
                marked_at_ms: 400,
            };
            database.set_watched(&watched, true).unwrap();
        }
        {
            let database = SqliteDatabase::open(&path).unwrap();
            assert_eq!(
                database.get_playback_progress("default", "1399", MediaKind::Series, Some(2), Some(3)).unwrap(),
                Some(progress)
            );
            assert_eq!(
                database.list_recent_completed_series("default", 20).unwrap()
                    .first().and_then(|entry| entry.episode_number),
                Some(2)
            );
        }
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[test]
    fn playback_progress_rejects_unsafe_artwork_and_invalid_episode_identity() {
        let database = SqliteDatabase::in_memory().unwrap();
        let mut progress = PlaybackProgress {
            profile_id: "default".to_owned(),
            media_id: "1399".to_owned(),
            media_kind: MediaKind::Series,
            title: "Game of Thrones".to_owned(),
            season_number: Some(1),
            episode_number: Some(1),
            episode_title: None,
            poster_path: Some("/../secret.jpg".to_owned()),
            backdrop_path: None,
            still_path: None,
            position_ms: 2_000_000,
            duration_ms: 2_700_000,
            is_completed: false,
            updated_at_ms: 100,
        };
        assert!(matches!(
            database.save_playback_progress(&progress),
            Err(RepositoryError::InvalidPlaybackProgress)
        ));
        progress.poster_path = None;
        progress.episode_number = None;
        assert!(matches!(
            database.save_playback_progress(&progress),
            Err(RepositoryError::InvalidPlaybackProgress)
        ));
    }

    #[test]
    fn ordered_migrations_create_current_schema() {
        let database = SqliteDatabase::in_memory().unwrap();
        assert_eq!(database.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
        assert_eq!(database.list_profiles().unwrap().len(), 1);
    }

    #[test]
    fn addon_records_are_persisted_in_profile_scoped_order() {
        let database = SqliteDatabase::in_memory().unwrap();
        let first = AddonRecord {
            id: "addon-a".to_owned(),
            secret_key: "senzo.addon.default.a".to_owned(),
            manifest_json: Some(r#"{"id":"provider.a"}"#.to_owned()),
            allow_insecure_http: false,
            enabled: true,
            sort_order: 1,
            last_checked_at: Some("100".to_owned()),
            last_error: None,
        };
        let second = AddonRecord {
            id: "addon-b".to_owned(),
            secret_key: "senzo.addon.default.b".to_owned(),
            manifest_json: None,
            allow_insecure_http: true,
            enabled: false,
            sort_order: 0,
            last_checked_at: None,
            last_error: Some("temporary failure".to_owned()),
        };
        database.insert("default", &first).unwrap();
        database.insert("default", &second).unwrap();

        let guest = AddonRecord {
            id: "addon-a".to_owned(),
            secret_key: "senzo.addon.guest.a".to_owned(),
            manifest_json: None,
            allow_insecure_http: false,
            enabled: false,
            sort_order: 0,
            last_checked_at: None,
            last_error: None,
        };
        let guest_profile = database.create_profile("Guest").unwrap();
        database.insert(&guest_profile.id, &guest).unwrap();

        let records = database.list("default").unwrap();
        assert_eq!(records, vec![second.clone(), first.clone()]);
        assert_eq!(database.list(&guest_profile.id).unwrap(), vec![guest]);

        let mut updated = first.clone();
        updated.enabled = false;
        updated.sort_order = 3;
        updated.last_error = Some("refresh failed".to_owned());
        database.update("default", &updated).unwrap();
        assert_eq!(database.list("default").unwrap()[1], updated);
        assert!(matches!(
            database.update(&guest_profile.id, &first),
            Err(AddonStoreError::NotFound)
        ));

        database.remove("default", "addon-a").unwrap();
        assert!(matches!(
            database.remove("default", "addon-a"),
            Err(AddonStoreError::NotFound)
        ));
        assert_eq!(database.list(&guest_profile.id).unwrap().len(), 1);
    }

    #[test]
    fn settings_repository_round_trips_values() {
        let database = SqliteDatabase::in_memory().unwrap();
        let key = SettingKey::new("appearance.language").unwrap();
        assert_eq!(database.get(&key).unwrap(), None);
        database.set(&key, "en-US").unwrap();
        assert_eq!(database.get(&key).unwrap().as_deref(), Some("en-US"));
    }

    #[test]
    fn profile_repository_validates_and_persists_profiles() {
        let database = SqliteDatabase::in_memory().unwrap();
        let profile = database.create_profile("  Guest  ").unwrap();
        assert_eq!(profile.name, "Guest");
        assert_eq!(database.list_profiles().unwrap().len(), 2);
        assert!(matches!(
            database.create_profile(" "),
            Err(RepositoryError::InvalidProfileName)
        ));
    }
}
