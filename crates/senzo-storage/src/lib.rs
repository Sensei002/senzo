//! SQLite persistence with ordered, transactional schema migrations.

use std::{
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use pbkdf2::pbkdf2_hmac;
use ring::rand::{SecureRandom, SystemRandom};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};
use senzo_addons::{AddonRecord, AddonStore, AddonStoreError};
use senzo_domain::{
    LibraryCollection, LibraryItem, LibraryRepository, MediaKind, PlaybackProgress,
    PlaybackProgressRepository, Profile, ProfileRepository, ProfileScope,
    ProfileSecurityRepository, ProfileSettingsRepository, RepositoryError, SettingKey,
    SettingsRepository, SyncMutation, SyncOutboxRepository, WatchedItem,
};
use serde::Serialize;
use sha2::Sha256;
use subtle::ConstantTimeEq;
use thiserror::Error;

pub const CURRENT_SCHEMA_VERSION: i64 = 5;

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
    (
        5,
        "CREATE TABLE profile_settings (
            profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
            key TEXT NOT NULL,
            value TEXT NOT NULL,
            updated_at_ms INTEGER NOT NULL,
            PRIMARY KEY(profile_id, key)
        );
        CREATE TABLE profile_security (
            profile_id TEXT PRIMARY KEY NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
            pin_salt BLOB NOT NULL,
            pin_hash BLOB NOT NULL,
            pin_iterations INTEGER NOT NULL,
            failed_attempts INTEGER NOT NULL DEFAULT 0,
            locked_until_ms INTEGER NOT NULL DEFAULT 0,
            updated_at_ms INTEGER NOT NULL
        );
        CREATE TABLE library_items (
            profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
            media_kind TEXT NOT NULL CHECK (media_kind IN ('movie', 'series')),
            media_id TEXT NOT NULL,
            title TEXT NOT NULL,
            overview TEXT,
            release_year INTEGER,
            poster_path TEXT,
            backdrop_path TEXT,
            added_at_ms INTEGER NOT NULL,
            updated_at_ms INTEGER NOT NULL,
            is_favorite INTEGER NOT NULL DEFAULT 0,
            is_deleted INTEGER NOT NULL DEFAULT 0,
            deleted_at_ms INTEGER,
            PRIMARY KEY(profile_id, media_kind, media_id)
        );
        CREATE INDEX library_items_profile_added_idx
            ON library_items(profile_id, is_deleted, is_favorite DESC, added_at_ms DESC);
        CREATE TABLE library_collections (
            id TEXT PRIMARY KEY NOT NULL,
            profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
            title TEXT NOT NULL,
            description TEXT,
            sort_order INTEGER NOT NULL,
            created_at_ms INTEGER NOT NULL,
            updated_at_ms INTEGER NOT NULL,
            is_deleted INTEGER NOT NULL DEFAULT 0,
            deleted_at_ms INTEGER,
            UNIQUE(profile_id, id)
        );
        CREATE INDEX library_collections_profile_order_idx
            ON library_collections(profile_id, is_deleted, sort_order, created_at_ms);
        CREATE TABLE library_collection_items (
            profile_id TEXT NOT NULL,
            collection_id TEXT NOT NULL,
            media_kind TEXT NOT NULL CHECK (media_kind IN ('movie', 'series')),
            media_id TEXT NOT NULL,
            sort_order INTEGER NOT NULL,
            added_at_ms INTEGER NOT NULL,
            updated_at_ms INTEGER NOT NULL,
            is_deleted INTEGER NOT NULL DEFAULT 0,
            deleted_at_ms INTEGER,
            PRIMARY KEY(profile_id, collection_id, media_kind, media_id),
            FOREIGN KEY(profile_id, collection_id)
                REFERENCES library_collections(profile_id, id) ON DELETE CASCADE,
            FOREIGN KEY(profile_id, media_kind, media_id)
                REFERENCES library_items(profile_id, media_kind, media_id) ON DELETE CASCADE
        );
        CREATE INDEX library_collection_items_order_idx
            ON library_collection_items(profile_id, collection_id, is_deleted, sort_order, added_at_ms);
        CREATE TABLE sync_outbox (
            id TEXT PRIMARY KEY NOT NULL,
            profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
            entity_type TEXT NOT NULL,
            entity_id TEXT NOT NULL,
            operation TEXT NOT NULL CHECK (operation IN ('upsert', 'delete')),
            payload_json TEXT NOT NULL,
            changed_at_ms INTEGER NOT NULL,
            attempt_count INTEGER NOT NULL DEFAULT 0,
            next_attempt_at_ms INTEGER NOT NULL DEFAULT 0,
            last_error TEXT,
            UNIQUE(profile_id, entity_type, entity_id)
        );
        CREATE INDEX sync_outbox_due_idx
            ON sync_outbox(profile_id, next_attempt_at_ms, changed_at_ms);
        CREATE TABLE sync_checkpoints (
            profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
            provider TEXT NOT NULL,
            cursor TEXT NOT NULL,
            updated_at_ms INTEGER NOT NULL,
            PRIMARY KEY(profile_id, provider)
        );
        INSERT OR IGNORE INTO profile_settings(profile_id,key,value,updated_at_ms)
            SELECT 'default',key,value,
                CAST(COALESCE(strftime('%s',updated_at), '0') AS INTEGER) * 1000
            FROM app_settings
            WHERE key <> 'profiles.active';",
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

/// A settings facade that routes ordinary settings through the current profile
/// while keeping the active-profile pointer installation-wide.
pub struct ProfileScopedSettingsRepository {
    database: Arc<SqliteDatabase>,
    profile_scope: ProfileScope,
}

impl ProfileScopedSettingsRepository {
    pub fn new(database: Arc<SqliteDatabase>, profile_scope: ProfileScope) -> Self {
        Self {
            database,
            profile_scope,
        }
    }
}

impl SettingsRepository for ProfileScopedSettingsRepository {
    fn get(&self, key: &SettingKey) -> Result<Option<String>, RepositoryError> {
        if key.as_str() == "profiles.active" {
            self.database.get(key)
        } else {
            self.database
                .get_profile_setting(&self.profile_scope.current(), key)
        }
    }

    fn set(&self, key: &SettingKey, value: &str) -> Result<(), RepositoryError> {
        if key.as_str() == "profiles.active" {
            self.database.set(key, value)
        } else {
            self.database
                .set_profile_setting(&self.profile_scope.current(), key, value)
        }
    }
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

    fn update_profile(&self, profile_id: &str, name: &str) -> Result<Profile, RepositoryError> {
        let name = validate_profile_name(name)?;
        if profile_id.trim().is_empty()
            || profile_id.len() > 128
            || profile_id.chars().any(char::is_control)
        {
            return Err(RepositoryError::InvalidLibraryEntry);
        }
        let connection = self.lock()?;
        let changed = connection
            .execute(
                "UPDATE profiles SET name = ?2 WHERE id = ?1",
                (profile_id, &name),
            )
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        if changed == 0 {
            return Err(RepositoryError::NotFound);
        }
        Ok(Profile {
            id: profile_id.to_owned(),
            name,
        })
    }

    fn delete_profile(&self, profile_id: &str) -> Result<(), RepositoryError> {
        if profile_id == "default" || profile_id.trim().is_empty() {
            return Err(RepositoryError::InvalidLibraryEntry);
        }
        let connection = self.lock()?;
        let changed = connection
            .execute("DELETE FROM profiles WHERE id = ?1", [profile_id])
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        if changed == 0 {
            return Err(RepositoryError::NotFound);
        }
        Ok(())
    }
}

fn validate_profile_name(name: &str) -> Result<String, RepositoryError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
        return Err(RepositoryError::InvalidProfileName);
    }
    Ok(name.to_owned())
}

impl ProfileSettingsRepository for SqliteDatabase {
    fn get_profile_setting(
        &self,
        profile_id: &str,
        key: &SettingKey,
    ) -> Result<Option<String>, RepositoryError> {
        validate_profile_id(profile_id)?;
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT value FROM profile_settings WHERE profile_id = ?1 AND key = ?2",
                (profile_id, key.as_str()),
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))
    }

    fn set_profile_setting(
        &self,
        profile_id: &str,
        key: &SettingKey,
        value: &str,
    ) -> Result<(), RepositoryError> {
        validate_profile_id(profile_id)?;
        if value.len() > 1_048_576 {
            return Err(RepositoryError::InvalidLibraryEntry);
        }
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        let changed_at = now_epoch_millis();
        transaction.execute(
            "INSERT INTO profile_settings(profile_id,key,value,updated_at_ms)
             VALUES(?1,?2,?3,?4)
             ON CONFLICT(profile_id,key) DO UPDATE SET value=excluded.value,updated_at_ms=excluded.updated_at_ms
             WHERE excluded.updated_at_ms >= profile_settings.updated_at_ms",
            (profile_id, key.as_str(), value, changed_at),
        ).map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        let payload = serde_json::json!({"key":key.as_str(),"value":value});
        enqueue_sync_mutation(
            &transaction,
            profile_id,
            "profile_setting",
            key.as_str(),
            "upsert",
            &payload,
            changed_at,
        )?;
        transaction
            .commit()
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        Ok(())
    }
}

const PROFILE_PIN_ITERATIONS: u32 = 310_000;
const PROFILE_PIN_SALT_BYTES: usize = 16;
const PROFILE_PIN_HASH_BYTES: usize = 32;
const PROFILE_PIN_MAX_ATTEMPTS: i64 = 5;
const PROFILE_PIN_LOCK_MS: i64 = 5 * 60 * 1000;

impl ProfileSecurityRepository for SqliteDatabase {
    fn has_profile_pin(&self, profile_id: &str) -> Result<bool, RepositoryError> {
        validate_profile_id(profile_id)?;
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM profile_security WHERE profile_id = ?1)",
                [profile_id],
                |row| row.get::<_, i64>(0),
            )
            .map(|value| value != 0)
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))
    }

    fn set_profile_pin(&self, profile_id: &str, pin: &str) -> Result<(), RepositoryError> {
        validate_profile_pin(profile_id, pin)?;
        let random = SystemRandom::new();
        let mut salt = [0u8; PROFILE_PIN_SALT_BYTES];
        random.fill(&mut salt).map_err(|_| {
            RepositoryError::Unavailable("secure random generator failed".to_owned())
        })?;
        let mut hash = [0u8; PROFILE_PIN_HASH_BYTES];
        pbkdf2_hmac::<Sha256>(pin.as_bytes(), &salt, PROFILE_PIN_ITERATIONS, &mut hash);
        let connection = self.lock()?;
        connection.execute(
            "INSERT INTO profile_security(profile_id,pin_salt,pin_hash,pin_iterations,failed_attempts,locked_until_ms,updated_at_ms)
             VALUES(?1,?2,?3,?4,0,0,?5)
             ON CONFLICT(profile_id) DO UPDATE SET pin_salt=excluded.pin_salt,pin_hash=excluded.pin_hash,
                pin_iterations=excluded.pin_iterations,failed_attempts=0,locked_until_ms=0,updated_at_ms=excluded.updated_at_ms",
            (profile_id, salt.as_slice(), hash.as_slice(), i64::from(PROFILE_PIN_ITERATIONS), now_epoch_millis()),
        ).map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        Ok(())
    }

    fn verify_profile_pin(
        &self,
        profile_id: &str,
        pin: &str,
        now_ms: i64,
    ) -> Result<bool, RepositoryError> {
        validate_profile_id(profile_id)?;
        if pin.len() > 12 || !pin.bytes().all(|byte| byte.is_ascii_digit()) {
            return Ok(false);
        }
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        let row = transaction
            .query_row(
                "SELECT pin_salt,pin_hash,pin_iterations,failed_attempts,locked_until_ms
             FROM profile_security WHERE profile_id = ?1",
                [profile_id],
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        let Some((salt, stored_hash, iterations, attempts, locked_until)) = row else {
            transaction
                .commit()
                .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
            return Ok(false);
        };
        if locked_until > now_ms {
            return Err(RepositoryError::ProfilePinLocked);
        }
        if !(1..=1_000_000).contains(&iterations)
            || stored_hash.len() != PROFILE_PIN_HASH_BYTES
            || salt.len() != PROFILE_PIN_SALT_BYTES
        {
            return Err(RepositoryError::Unavailable(
                "stored profile PIN material is invalid".to_owned(),
            ));
        }
        let mut candidate = vec![0u8; stored_hash.len()];
        pbkdf2_hmac::<Sha256>(pin.as_bytes(), &salt, iterations as u32, &mut candidate);
        let verified = bool::from(candidate.ct_eq(&stored_hash));
        if verified {
            transaction.execute(
                "UPDATE profile_security SET failed_attempts=0,locked_until_ms=0 WHERE profile_id=?1",
                [profile_id],
            ).map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        } else {
            let next_attempts = attempts.saturating_add(1);
            let lock_until = if next_attempts >= PROFILE_PIN_MAX_ATTEMPTS {
                now_ms.saturating_add(PROFILE_PIN_LOCK_MS)
            } else {
                0
            };
            transaction.execute(
                "UPDATE profile_security SET failed_attempts=?2,locked_until_ms=?3 WHERE profile_id=?1",
                (profile_id, next_attempts, lock_until),
            ).map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        }
        transaction
            .commit()
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        Ok(verified)
    }

    fn clear_profile_pin(&self, profile_id: &str) -> Result<(), RepositoryError> {
        validate_profile_id(profile_id)?;
        let connection = self.lock()?;
        connection
            .execute(
                "DELETE FROM profile_security WHERE profile_id=?1",
                [profile_id],
            )
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        Ok(())
    }
}

fn validate_profile_pin(profile_id: &str, pin: &str) -> Result<(), RepositoryError> {
    validate_profile_id(profile_id)?;
    if !(4..=12).contains(&pin.len()) || !pin.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(RepositoryError::InvalidProfilePin);
    }
    Ok(())
}

fn now_epoch_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

impl LibraryRepository for SqliteDatabase {
    fn list_library_items(&self, profile_id: &str) -> Result<Vec<LibraryItem>, RepositoryError> {
        validate_profile_id(profile_id)?;
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT profile_id,media_id,media_kind,title,overview,release_year,poster_path,backdrop_path,
                    added_at_ms,updated_at_ms,is_favorite
             FROM library_items WHERE profile_id=?1 AND is_deleted=0
             ORDER BY is_favorite DESC,added_at_ms DESC,title COLLATE NOCASE,media_id",
        ).map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        statement
            .query_map([profile_id], library_item_from_row)
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))
    }

    fn get_library_item(
        &self,
        profile_id: &str,
        media_id: &str,
        media_kind: MediaKind,
    ) -> Result<Option<LibraryItem>, RepositoryError> {
        validate_library_identity(profile_id, media_id, media_kind)?;
        let connection = self.lock()?;
        connection.query_row(
            "SELECT profile_id,media_id,media_kind,title,overview,release_year,poster_path,backdrop_path,
                    added_at_ms,updated_at_ms,is_favorite
             FROM library_items WHERE profile_id=?1 AND media_kind=?2 AND media_id=?3 AND is_deleted=0",
            (profile_id, library_media_kind(media_kind), media_id), library_item_from_row,
        ).optional().map_err(|error| RepositoryError::Unavailable(error.to_string()))
    }

    fn save_library_item(&self, item: &LibraryItem) -> Result<(), RepositoryError> {
        validate_library_item(item)?;
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        transaction.execute(
            "INSERT INTO library_items(profile_id,media_kind,media_id,title,overview,release_year,poster_path,
                    backdrop_path,added_at_ms,updated_at_ms,is_favorite,is_deleted,deleted_at_ms)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,0,NULL)
             ON CONFLICT(profile_id,media_kind,media_id) DO UPDATE SET
                title=excluded.title,overview=excluded.overview,release_year=excluded.release_year,
                poster_path=excluded.poster_path,backdrop_path=excluded.backdrop_path,
                added_at_ms=MIN(library_items.added_at_ms,excluded.added_at_ms),
                updated_at_ms=MAX(library_items.updated_at_ms,excluded.updated_at_ms),
                is_favorite=excluded.is_favorite,is_deleted=0,deleted_at_ms=NULL",
            (&item.profile_id, library_media_kind(item.media_kind), &item.media_id, &item.title,
             &item.overview, item.release_year.map(i64::from), &item.poster_path, &item.backdrop_path,
             item.added_at_ms, item.updated_at_ms, if item.is_favorite { 1_i64 } else { 0_i64 }),
        ).map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        enqueue_sync_mutation(
            &transaction,
            &item.profile_id,
            "library_item",
            &sync_media_key(&item.media_id, item.media_kind),
            "upsert",
            item,
            item.updated_at_ms,
        )?;
        transaction
            .commit()
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        Ok(())
    }

    fn remove_library_item(
        &self,
        profile_id: &str,
        media_id: &str,
        media_kind: MediaKind,
        removed_at_ms: i64,
    ) -> Result<(), RepositoryError> {
        validate_library_identity(profile_id, media_id, media_kind)?;
        if removed_at_ms < 0 {
            return Err(RepositoryError::InvalidLibraryEntry);
        }
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        let changed = transaction.execute(
            "UPDATE library_items SET is_deleted=1,deleted_at_ms=?4,updated_at_ms=MAX(updated_at_ms,?4)
             WHERE profile_id=?1 AND media_kind=?2 AND media_id=?3 AND is_deleted=0",
            (profile_id, library_media_kind(media_kind), media_id, removed_at_ms),
        ).map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        if changed > 0 {
            let payload = serde_json::json!({"media_id":media_id,"media_kind":library_media_kind(media_kind),"deleted":true,"deleted_at_ms":removed_at_ms});
            enqueue_sync_mutation(
                &transaction,
                profile_id,
                "library_item",
                &sync_media_key(media_id, media_kind),
                "delete",
                &payload,
                removed_at_ms,
            )?;
        }
        transaction
            .commit()
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        Ok(())
    }

    fn list_library_collections(
        &self,
        profile_id: &str,
    ) -> Result<Vec<LibraryCollection>, RepositoryError> {
        validate_profile_id(profile_id)?;
        let connection = self.lock()?;
        let mut statement=connection.prepare(
            "SELECT id,profile_id,title,description,sort_order,created_at_ms,updated_at_ms
             FROM library_collections WHERE profile_id=?1 AND is_deleted=0 ORDER BY sort_order,created_at_ms,id"
        ).map_err(|error|RepositoryError::Unavailable(error.to_string()))?;
        statement
            .query_map([profile_id], library_collection_from_row)
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))
    }

    fn create_library_collection(
        &self,
        profile_id: &str,
        title: &str,
        description: Option<&str>,
        created_at_ms: i64,
    ) -> Result<LibraryCollection, RepositoryError> {
        validate_collection_fields(profile_id, title, description, created_at_ms)?;
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        let sort_order:i64=transaction.query_row(
            "SELECT COALESCE(MAX(sort_order)+1,0) FROM library_collections WHERE profile_id=?1 AND is_deleted=0",
            [profile_id],|row|row.get(0)
        ).map_err(|error|RepositoryError::Unavailable(error.to_string()))?;
        let collection = LibraryCollection {
            id: uuid::Uuid::new_v4().to_string(),
            profile_id: profile_id.to_owned(),
            title: title.trim().to_owned(),
            description: description
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_owned),
            sort_order,
            created_at_ms,
            updated_at_ms: created_at_ms,
        };
        transaction.execute(
            "INSERT INTO library_collections(id,profile_id,title,description,sort_order,created_at_ms,updated_at_ms,is_deleted,deleted_at_ms)
             VALUES(?1,?2,?3,?4,?5,?6,?7,0,NULL)",
            (&collection.id,&collection.profile_id,&collection.title,&collection.description,collection.sort_order,collection.created_at_ms,collection.updated_at_ms)
        ).map_err(|error|RepositoryError::Unavailable(error.to_string()))?;
        enqueue_sync_mutation(
            &transaction,
            profile_id,
            "library_collection",
            &collection.id,
            "upsert",
            &collection,
            created_at_ms,
        )?;
        transaction
            .commit()
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        Ok(collection)
    }

    fn update_library_collection(
        &self,
        profile_id: &str,
        collection_id: &str,
        title: &str,
        description: Option<&str>,
        updated_at_ms: i64,
    ) -> Result<LibraryCollection, RepositoryError> {
        validate_collection_fields(profile_id, title, description, updated_at_ms)?;
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        let changed=transaction.execute(
            "UPDATE library_collections SET title=?3,description=?4,updated_at_ms=MAX(updated_at_ms,?5)
             WHERE profile_id=?1 AND id=?2 AND is_deleted=0",
            (profile_id,collection_id,title.trim(),description.map(str::trim).filter(|v|!v.is_empty()),updated_at_ms)
        ).map_err(|error|RepositoryError::Unavailable(error.to_string()))?;
        if changed == 0 {
            return Err(RepositoryError::NotFound);
        }
        let collection = transaction
            .query_row(
                "SELECT id,profile_id,title,description,sort_order,created_at_ms,updated_at_ms
             FROM library_collections WHERE profile_id=?1 AND id=?2 AND is_deleted=0",
                (profile_id, collection_id),
                library_collection_from_row,
            )
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        enqueue_sync_mutation(
            &transaction,
            profile_id,
            "library_collection",
            collection_id,
            "upsert",
            &collection,
            updated_at_ms,
        )?;
        transaction
            .commit()
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        Ok(collection)
    }

    fn delete_library_collection(
        &self,
        profile_id: &str,
        collection_id: &str,
        deleted_at_ms: i64,
    ) -> Result<(), RepositoryError> {
        validate_profile_id(profile_id)?;
        if collection_id.trim().is_empty() || collection_id.len() > 128 || deleted_at_ms < 0 {
            return Err(RepositoryError::InvalidLibraryEntry);
        }
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        let changed=transaction.execute(
            "UPDATE library_collections SET is_deleted=1,deleted_at_ms=?3,updated_at_ms=MAX(updated_at_ms,?3)
             WHERE profile_id=?1 AND id=?2 AND is_deleted=0",
            (profile_id,collection_id,deleted_at_ms)
        ).map_err(|error|RepositoryError::Unavailable(error.to_string()))?;
        if changed == 0 {
            return Err(RepositoryError::NotFound);
        }
        let payload = serde_json::json!({"collection_id":collection_id,"deleted":true,"deleted_at_ms":deleted_at_ms});
        enqueue_sync_mutation(
            &transaction,
            profile_id,
            "library_collection",
            collection_id,
            "delete",
            &payload,
            deleted_at_ms,
        )?;
        transaction
            .commit()
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        Ok(())
    }

    fn list_collection_items(
        &self,
        profile_id: &str,
        collection_id: &str,
    ) -> Result<Vec<LibraryItem>, RepositoryError> {
        validate_profile_id(profile_id)?;
        let connection = self.lock()?;
        let mut statement=connection.prepare(
            "SELECT l.profile_id,l.media_id,l.media_kind,l.title,l.overview,l.release_year,l.poster_path,l.backdrop_path,
                    l.added_at_ms,l.updated_at_ms,l.is_favorite
             FROM library_collection_items c
             JOIN library_items l ON l.profile_id=c.profile_id AND l.media_kind=c.media_kind AND l.media_id=c.media_id
             JOIN library_collections g ON g.profile_id=c.profile_id AND g.id=c.collection_id
             WHERE c.profile_id=?1 AND c.collection_id=?2 AND c.is_deleted=0 AND l.is_deleted=0 AND g.is_deleted=0
             ORDER BY c.sort_order,c.added_at_ms,c.media_id"
        ).map_err(|error|RepositoryError::Unavailable(error.to_string()))?;
        statement
            .query_map((profile_id, collection_id), library_item_from_row)
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))
    }

    fn set_collection_item(
        &self,
        profile_id: &str,
        collection_id: &str,
        media_id: &str,
        media_kind: MediaKind,
        included: bool,
        changed_at_ms: i64,
    ) -> Result<(), RepositoryError> {
        validate_library_identity(profile_id, media_id, media_kind)?;
        if collection_id.trim().is_empty() || collection_id.len() > 128 || changed_at_ms < 0 {
            return Err(RepositoryError::InvalidLibraryEntry);
        }
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        if included {
            let exists:i64=transaction.query_row(
                "SELECT COUNT(*) FROM library_items WHERE profile_id=?1 AND media_kind=?2 AND media_id=?3 AND is_deleted=0",
                (profile_id,library_media_kind(media_kind),media_id),|row|row.get(0)
            ).map_err(|error|RepositoryError::Unavailable(error.to_string()))?;
            if exists == 0 {
                return Err(RepositoryError::NotFound);
            }
            let sort_order: i64 = transaction
                .query_row(
                    "SELECT COALESCE(MAX(sort_order)+1,0) FROM library_collection_items
                 WHERE profile_id=?1 AND collection_id=?2 AND is_deleted=0",
                    (profile_id, collection_id),
                    |row| row.get(0),
                )
                .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
            transaction.execute(
                "INSERT INTO library_collection_items(profile_id,collection_id,media_kind,media_id,sort_order,added_at_ms,updated_at_ms,is_deleted,deleted_at_ms)
                 VALUES(?1,?2,?3,?4,?5,?6,?6,0,NULL)
                 ON CONFLICT(profile_id,collection_id,media_kind,media_id) DO UPDATE SET
                    sort_order=excluded.sort_order,updated_at_ms=excluded.updated_at_ms,is_deleted=0,deleted_at_ms=NULL",
                (profile_id,collection_id,library_media_kind(media_kind),media_id,sort_order,changed_at_ms)
            ).map_err(|error|RepositoryError::Unavailable(error.to_string()))?;
        } else {
            let changed=transaction.execute(
                "UPDATE library_collection_items SET is_deleted=1,deleted_at_ms=?5,updated_at_ms=MAX(updated_at_ms,?5)
                 WHERE profile_id=?1 AND collection_id=?2 AND media_kind=?3 AND media_id=?4 AND is_deleted=0",
                (profile_id,collection_id,library_media_kind(media_kind),media_id,changed_at_ms)
            ).map_err(|error|RepositoryError::Unavailable(error.to_string()))?;
            if changed == 0 {
                transaction
                    .commit()
                    .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
                return Ok(());
            }
        }
        let payload = serde_json::json!({"collection_id":collection_id,"media_id":media_id,"media_kind":library_media_kind(media_kind),"included":included,"changed_at_ms":changed_at_ms});
        enqueue_sync_mutation(
            &transaction,
            profile_id,
            "collection_item",
            &format!("{collection_id}:{}", sync_media_key(media_id, media_kind)),
            if included { "upsert" } else { "delete" },
            &payload,
            changed_at_ms,
        )?;
        transaction
            .commit()
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        Ok(())
    }

    fn reorder_collection_items(
        &self,
        profile_id: &str,
        collection_id: &str,
        ordered: &[(String, MediaKind)],
        changed_at_ms: i64,
    ) -> Result<(), RepositoryError> {
        validate_profile_id(profile_id)?;
        if collection_id.trim().is_empty()
            || collection_id.len() > 128
            || ordered.len() > 500
            || changed_at_ms < 0
        {
            return Err(RepositoryError::InvalidLibraryEntry);
        }
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        for (index, (media_id, media_kind)) in ordered.iter().enumerate() {
            validate_library_identity(profile_id, media_id, *media_kind)?;
            let changed=transaction.execute(
                "UPDATE library_collection_items SET sort_order=?5,updated_at_ms=MAX(updated_at_ms,?6)
                 WHERE profile_id=?1 AND collection_id=?2 AND media_kind=?3 AND media_id=?4 AND is_deleted=0",
                (profile_id,collection_id,library_media_kind(*media_kind),media_id,index as i64,changed_at_ms)
            ).map_err(|error|RepositoryError::Unavailable(error.to_string()))?;
            if changed == 0 {
                return Err(RepositoryError::NotFound);
            }
        }
        let payload = serde_json::json!({"collection_id":collection_id,"items":ordered.iter().map(|(id,kind)|serde_json::json!({"media_id":id,"media_kind":library_media_kind(*kind)})).collect::<Vec<_>>(),"changed_at_ms":changed_at_ms});
        enqueue_sync_mutation(
            &transaction,
            profile_id,
            "collection_order",
            collection_id,
            "upsert",
            &payload,
            changed_at_ms,
        )?;
        transaction
            .commit()
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        Ok(())
    }
}

fn library_media_kind(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Movie => "movie",
        MediaKind::Series => "series",
        MediaKind::Episode => "episode",
    }
}
fn sync_media_key(media_id: &str, kind: MediaKind) -> String {
    format!("{}:{}", library_media_kind(kind), media_id)
}
fn validate_library_identity(
    profile_id: &str,
    media_id: &str,
    kind: MediaKind,
) -> Result<(), RepositoryError> {
    validate_profile_id(profile_id)?;
    if media_id.trim().is_empty()
        || media_id.len() > 128
        || media_id.chars().any(char::is_control)
        || !matches!(kind, MediaKind::Movie | MediaKind::Series)
    {
        return Err(RepositoryError::InvalidLibraryEntry);
    }
    Ok(())
}
fn validate_library_item(item: &LibraryItem) -> Result<(), RepositoryError> {
    validate_library_identity(&item.profile_id, &item.media_id, item.media_kind)?;
    if item.title.trim().is_empty()
        || item.title.chars().count() > 512
        || item.title.chars().any(char::is_control)
        || item
            .overview
            .as_ref()
            .is_some_and(|v| v.len() > 20_000 || v.chars().any(char::is_control))
        || item
            .release_year
            .is_some_and(|y| !(1800..=2200).contains(&y))
        || item.added_at_ms < 0
        || item.updated_at_ms < 0
        || !valid_artwork_path(item.poster_path.as_deref())
        || !valid_artwork_path(item.backdrop_path.as_deref())
    {
        return Err(RepositoryError::InvalidLibraryEntry);
    }
    Ok(())
}
fn validate_collection_fields(
    profile_id: &str,
    title: &str,
    description: Option<&str>,
    timestamp: i64,
) -> Result<(), RepositoryError> {
    validate_profile_id(profile_id)?;
    if title.trim().is_empty()
        || title.chars().count() > 80
        || title.chars().any(char::is_control)
        || description.is_some_and(|v| v.chars().count() > 1000 || v.chars().any(char::is_control))
        || timestamp < 0
    {
        return Err(RepositoryError::InvalidLibraryEntry);
    }
    Ok(())
}
fn library_item_from_row(row: &rusqlite::Row<'_>) -> Result<LibraryItem, rusqlite::Error> {
    let kind: String = row.get(2)?;
    let media_kind = match kind.as_str() {
        "movie" => MediaKind::Movie,
        "series" => MediaKind::Series,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    let year: Option<i64> = row.get(5)?;
    Ok(LibraryItem {
        profile_id: row.get(0)?,
        media_id: row.get(1)?,
        media_kind,
        title: row.get(3)?,
        overview: row.get(4)?,
        release_year: year.and_then(|v| u16::try_from(v).ok()),
        poster_path: row.get(6)?,
        backdrop_path: row.get(7)?,
        added_at_ms: row.get(8)?,
        updated_at_ms: row.get(9)?,
        is_favorite: row.get::<_, i64>(10)? != 0,
    })
}
fn library_collection_from_row(
    row: &rusqlite::Row<'_>,
) -> Result<LibraryCollection, rusqlite::Error> {
    Ok(LibraryCollection {
        id: row.get(0)?,
        profile_id: row.get(1)?,
        title: row.get(2)?,
        description: row.get(3)?,
        sort_order: row.get(4)?,
        created_at_ms: row.get(5)?,
        updated_at_ms: row.get(6)?,
    })
}
fn enqueue_sync_mutation<T: Serialize>(
    transaction: &Transaction<'_>,
    profile_id: &str,
    entity_type: &str,
    entity_id: &str,
    operation: &str,
    payload: &T,
    changed_at_ms: i64,
) -> Result<(), RepositoryError> {
    let payload_json = serde_json::to_string(payload)
        .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
    transaction.execute(
        "INSERT INTO sync_outbox(id,profile_id,entity_type,entity_id,operation,payload_json,changed_at_ms,attempt_count,next_attempt_at_ms,last_error)
         VALUES(?1,?2,?3,?4,?5,?6,?7,0,0,NULL)
         ON CONFLICT(profile_id,entity_type,entity_id) DO UPDATE SET
            id=excluded.id,operation=excluded.operation,payload_json=excluded.payload_json,
            changed_at_ms=MAX(sync_outbox.changed_at_ms,excluded.changed_at_ms),attempt_count=0,
            next_attempt_at_ms=0,last_error=NULL",
        (uuid::Uuid::new_v4().to_string(),profile_id,entity_type,entity_id,operation,payload_json,changed_at_ms)
    ).map_err(|error|RepositoryError::Unavailable(error.to_string()))?;
    Ok(())
}

impl SyncOutboxRepository for SqliteDatabase {
    fn apply_remote_sync_page(
        &self,
        profile_id: &str,
        provider: &str,
        next_cursor: &str,
        mutations: &[senzo_domain::RemoteSyncMutation],
        updated_at_ms: i64,
    ) -> Result<(), RepositoryError> {
        validate_profile_id(profile_id)?;
        let cursor = next_cursor
            .parse::<i64>()
            .map_err(|_| RepositoryError::InvalidLibraryEntry)?;
        if provider.trim().is_empty()
            || provider.len() > 100
            || cursor < 0
            || updated_at_ms < 0
            || mutations.len() > 500
        {
            return Err(RepositoryError::InvalidLibraryEntry);
        }
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        let previous_cursor = transaction
            .query_row(
                "SELECT cursor FROM sync_checkpoints WHERE profile_id=?1 AND provider=?2",
                (profile_id, provider),
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(0);

        for mutation in mutations {
            if mutation.cursor <= previous_cursor
                || mutation.cursor > cursor
                || mutation.changed_at_ms < 0
                || !matches!(mutation.operation.as_str(), "upsert" | "delete")
            {
                return Err(RepositoryError::InvalidLibraryEntry);
            }
            let payload: serde_json::Value = serde_json::from_str(&mutation.payload_json)
                .map_err(|_| RepositoryError::InvalidLibraryEntry)?;
            match mutation.entity_type.as_str() {
                "library_item" => {
                    if mutation.operation == "upsert" {
                        let mut item: LibraryItem = serde_json::from_value(payload)
                            .map_err(|_| RepositoryError::InvalidLibraryEntry)?;
                        item.profile_id = profile_id.to_owned();
                        item.updated_at_ms = item.updated_at_ms.max(mutation.changed_at_ms);
                        validate_library_item(&item)?;
                        transaction.execute(
                            "INSERT INTO library_items(profile_id,media_kind,media_id,title,overview,release_year,poster_path,backdrop_path,added_at_ms,updated_at_ms,is_favorite,is_deleted,deleted_at_ms)
                             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,0,NULL)
                             ON CONFLICT(profile_id,media_kind,media_id) DO UPDATE SET
                                title=excluded.title,overview=excluded.overview,release_year=excluded.release_year,
                                poster_path=excluded.poster_path,backdrop_path=excluded.backdrop_path,
                                added_at_ms=MIN(library_items.added_at_ms,excluded.added_at_ms),
                                updated_at_ms=excluded.updated_at_ms,is_favorite=excluded.is_favorite,
                                is_deleted=0,deleted_at_ms=NULL
                             WHERE excluded.updated_at_ms >= library_items.updated_at_ms",
                            (&item.profile_id,library_media_kind(item.media_kind),&item.media_id,&item.title,&item.overview,
                             item.release_year.map(i64::from),&item.poster_path,&item.backdrop_path,item.added_at_ms,
                             item.updated_at_ms,if item.is_favorite {1_i64}else{0_i64})
                        ).map_err(|error|RepositoryError::Unavailable(error.to_string()))?;
                    } else {
                        let media_id = payload
                            .get("media_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or(&mutation.entity_id);
                        let kind = payload
                            .get("media_kind")
                            .and_then(|v| v.as_str())
                            .unwrap_or("movie");
                        transaction.execute(
                            "UPDATE library_items SET is_deleted=1,deleted_at_ms=?4,updated_at_ms=MAX(updated_at_ms,?4)
                             WHERE profile_id=?1 AND media_kind=?2 AND media_id=?3",
                            (profile_id,kind,media_id,mutation.changed_at_ms)
                        ).map_err(|error|RepositoryError::Unavailable(error.to_string()))?;
                    }
                }
                "library_collection" => {
                    if mutation.operation == "upsert" {
                        let mut item: LibraryCollection = serde_json::from_value(payload)
                            .map_err(|_| RepositoryError::InvalidLibraryEntry)?;
                        item.profile_id = profile_id.to_owned();
                        if item.id.is_empty()
                            || item.id.len() > 128
                            || item.title.trim().is_empty()
                            || item.title.chars().count() > 80
                            || item.updated_at_ms < 0
                            || item.created_at_ms < 0
                        {
                            return Err(RepositoryError::InvalidLibraryEntry);
                        }
                        transaction.execute(
                            "INSERT INTO library_collections(id,profile_id,title,description,sort_order,created_at_ms,updated_at_ms,is_deleted,deleted_at_ms)
                             VALUES(?1,?2,?3,?4,?5,?6,?7,0,NULL)
                             ON CONFLICT(id) DO UPDATE SET title=excluded.title,description=excluded.description,
                                sort_order=excluded.sort_order,updated_at_ms=excluded.updated_at_ms,is_deleted=0,deleted_at_ms=NULL
                             WHERE excluded.updated_at_ms >= library_collections.updated_at_ms
                               AND library_collections.profile_id=excluded.profile_id",
                            (&item.id,&item.profile_id,&item.title,&item.description,item.sort_order,item.created_at_ms,item.updated_at_ms)
                        ).map_err(|error|RepositoryError::Unavailable(error.to_string()))?;
                    } else {
                        transaction.execute(
                            "UPDATE library_collections SET is_deleted=1,deleted_at_ms=?3,updated_at_ms=MAX(updated_at_ms,?3)
                             WHERE profile_id=?1 AND id=?2",
                            (profile_id,mutation.entity_id.as_str(),mutation.changed_at_ms)
                        ).map_err(|error|RepositoryError::Unavailable(error.to_string()))?;
                    }
                }
                "collection_item" => {
                    let collection_id = payload
                        .get("collection_id")
                        .and_then(|v| v.as_str())
                        .ok_or(RepositoryError::InvalidLibraryEntry)?;
                    let media_id = payload
                        .get("media_id")
                        .and_then(|v| v.as_str())
                        .ok_or(RepositoryError::InvalidLibraryEntry)?;
                    let kind = payload
                        .get("media_kind")
                        .and_then(|v| v.as_str())
                        .ok_or(RepositoryError::InvalidLibraryEntry)?;
                    let included = mutation.operation == "upsert"
                        && payload
                            .get("included")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(true);
                    if included {
                        let sort_order:i64=transaction.query_row(
                            "SELECT COALESCE(MAX(sort_order)+1,0) FROM library_collection_items WHERE profile_id=?1 AND collection_id=?2",
                            (profile_id,collection_id),|row|row.get(0)
                        ).map_err(|error|RepositoryError::Unavailable(error.to_string()))?;
                        transaction.execute(
                            "INSERT INTO library_collection_items(profile_id,collection_id,media_kind,media_id,sort_order,added_at_ms,updated_at_ms,is_deleted,deleted_at_ms)
                             VALUES(?1,?2,?3,?4,?5,?6,?6,0,NULL)
                             ON CONFLICT(profile_id,collection_id,media_kind,media_id) DO UPDATE SET
                                sort_order=CASE WHEN excluded.updated_at_ms>=library_collection_items.updated_at_ms THEN excluded.sort_order ELSE library_collection_items.sort_order END,
                                updated_at_ms=MAX(library_collection_items.updated_at_ms,excluded.updated_at_ms),is_deleted=0,deleted_at_ms=NULL",
                            (profile_id,collection_id,kind,media_id,sort_order,mutation.changed_at_ms)
                        ).map_err(|error|RepositoryError::Unavailable(error.to_string()))?;
                    } else {
                        transaction.execute(
                            "UPDATE library_collection_items SET is_deleted=1,deleted_at_ms=?5,updated_at_ms=MAX(updated_at_ms,?5)
                             WHERE profile_id=?1 AND collection_id=?2 AND media_kind=?3 AND media_id=?4",
                            (profile_id,collection_id,kind,media_id,mutation.changed_at_ms)
                        ).map_err(|error|RepositoryError::Unavailable(error.to_string()))?;
                    }
                }
                "collection_order" => {
                    let collection_id = payload
                        .get("collection_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&mutation.entity_id);
                    let items = payload
                        .get("items")
                        .and_then(|v| v.as_array())
                        .ok_or(RepositoryError::InvalidLibraryEntry)?;
                    if items.len() > 500 {
                        return Err(RepositoryError::InvalidLibraryEntry);
                    }
                    for (index, item) in items.iter().enumerate() {
                        let media_id = item
                            .get("media_id")
                            .and_then(|v| v.as_str())
                            .ok_or(RepositoryError::InvalidLibraryEntry)?;
                        let kind = item
                            .get("media_kind")
                            .and_then(|v| v.as_str())
                            .ok_or(RepositoryError::InvalidLibraryEntry)?;
                        transaction.execute(
                            "UPDATE library_collection_items SET sort_order=?5,updated_at_ms=MAX(updated_at_ms,?6)
                             WHERE profile_id=?1 AND collection_id=?2 AND media_kind=?3 AND media_id=?4",
                            (profile_id,collection_id,kind,media_id,index as i64,mutation.changed_at_ms)
                        ).map_err(|error|RepositoryError::Unavailable(error.to_string()))?;
                    }
                }
                "profile_setting" => {
                    let key = payload
                        .get("key")
                        .and_then(|v| v.as_str())
                        .ok_or(RepositoryError::InvalidLibraryEntry)?;
                    let value = payload
                        .get("value")
                        .and_then(|v| v.as_str())
                        .ok_or(RepositoryError::InvalidLibraryEntry)?;
                    if value.len() > 1_048_576 {
                        return Err(RepositoryError::InvalidLibraryEntry);
                    }
                    let key = SettingKey::new(key.to_owned())
                        .map_err(|_| RepositoryError::InvalidLibraryEntry)?;
                    transaction.execute(
                        "INSERT INTO profile_settings(profile_id,key,value,updated_at_ms) VALUES(?1,?2,?3,?4)
                         ON CONFLICT(profile_id,key) DO UPDATE SET value=excluded.value,updated_at_ms=excluded.updated_at_ms
                         WHERE excluded.updated_at_ms>=profile_settings.updated_at_ms",
                        (profile_id,key.as_str(),value,mutation.changed_at_ms)
                    ).map_err(|error|RepositoryError::Unavailable(error.to_string()))?;
                }
                "playback_progress" => {
                    let mut progress: PlaybackProgress = serde_json::from_value(payload)
                        .map_err(|_| RepositoryError::InvalidPlaybackProgress)?;
                    progress.profile_id = profile_id.to_owned();
                    validate_playback_progress(&progress)?;
                    let changed = transaction.execute(
                        "INSERT INTO playback_progress(profile_id,media_kind,media_id,season_number,episode_number,title,episode_title,poster_path,backdrop_path,still_path,position_ms,duration_ms,is_completed,updated_at_ms)
                         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)
                         ON CONFLICT(profile_id,media_kind,media_id,season_number,episode_number) DO UPDATE SET
                            title=excluded.title,episode_title=excluded.episode_title,poster_path=excluded.poster_path,
                            backdrop_path=excluded.backdrop_path,still_path=excluded.still_path,position_ms=excluded.position_ms,
                            duration_ms=excluded.duration_ms,is_completed=excluded.is_completed,updated_at_ms=excluded.updated_at_ms
                         WHERE excluded.updated_at_ms>=playback_progress.updated_at_ms",
                        (&progress.profile_id,media_kind_storage_value(progress.media_kind),&progress.media_id,
                         stored_episode_number(progress.season_number),stored_episode_number(progress.episode_number),
                         &progress.title,&progress.episode_title,&progress.poster_path,&progress.backdrop_path,&progress.still_path,
                         i64::try_from(progress.position_ms).map_err(|_|RepositoryError::InvalidPlaybackProgress)?,
                         i64::try_from(progress.duration_ms).map_err(|_|RepositoryError::InvalidPlaybackProgress)?,
                         if progress.is_completed {1_i64}else{0_i64},progress.updated_at_ms)
                    ).map_err(|error|RepositoryError::Unavailable(error.to_string()))?;
                    // A newer replay checkpoint must also clear an older watched marker.
                    if changed > 0 && progress.is_completed {
                        transaction.execute(
                            "INSERT INTO watched_items(profile_id,media_kind,media_id,season_number,episode_number,marked_at_ms)
                             VALUES(?1,?2,?3,?4,?5,?6)
                             ON CONFLICT(profile_id,media_kind,media_id,season_number,episode_number)
                             DO UPDATE SET marked_at_ms=MAX(watched_items.marked_at_ms,excluded.marked_at_ms)",
                            (&progress.profile_id,media_kind_storage_value(progress.media_kind),&progress.media_id,
                             stored_episode_number(progress.season_number),stored_episode_number(progress.episode_number),progress.updated_at_ms)
                        ).map_err(|error|RepositoryError::Unavailable(error.to_string()))?;
                    } else if changed > 0 {
                        transaction
                            .execute(
                                "DELETE FROM watched_items
                             WHERE profile_id=?1 AND media_kind=?2 AND media_id=?3
                               AND season_number=?4 AND episode_number=?5",
                                (
                                    &progress.profile_id,
                                    media_kind_storage_value(progress.media_kind),
                                    &progress.media_id,
                                    stored_episode_number(progress.season_number),
                                    stored_episode_number(progress.episode_number),
                                ),
                            )
                            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
                    }
                }
                "watched_item" => {
                    let mut item: WatchedItem = serde_json::from_value(payload)
                        .map_err(|_| RepositoryError::InvalidPlaybackProgress)?;
                    item.profile_id = profile_id.to_owned();
                    validate_watch_identity(
                        &item.profile_id,
                        &item.media_id,
                        item.media_kind,
                        item.season_number,
                        item.episode_number,
                    )?;
                    if mutation.operation == "upsert" {
                        transaction.execute(
                            "INSERT INTO watched_items(profile_id,media_kind,media_id,season_number,episode_number,marked_at_ms)
                             VALUES(?1,?2,?3,?4,?5,?6)
                             ON CONFLICT(profile_id,media_kind,media_id,season_number,episode_number)
                             DO UPDATE SET marked_at_ms=MAX(watched_items.marked_at_ms,excluded.marked_at_ms)",
                            (&item.profile_id,media_kind_storage_value(item.media_kind),&item.media_id,
                             stored_episode_number(item.season_number),stored_episode_number(item.episode_number),item.marked_at_ms)
                        ).map_err(|error|RepositoryError::Unavailable(error.to_string()))?;
                    } else {
                        transaction.execute(
                            "DELETE FROM watched_items WHERE profile_id=?1 AND media_kind=?2 AND media_id=?3 AND season_number=?4 AND episode_number=?5",
                            (&item.profile_id,media_kind_storage_value(item.media_kind),&item.media_id,
                             stored_episode_number(item.season_number),stored_episode_number(item.episode_number))
                        ).map_err(|error|RepositoryError::Unavailable(error.to_string()))?;
                    }
                }
                _ => return Err(RepositoryError::InvalidLibraryEntry),
            }
        }

        transaction.execute(
            "INSERT INTO sync_checkpoints(profile_id,provider,cursor,updated_at_ms) VALUES(?1,?2,?3,?4)
             ON CONFLICT(profile_id,provider) DO UPDATE SET cursor=excluded.cursor,updated_at_ms=MAX(sync_checkpoints.updated_at_ms,excluded.updated_at_ms)",
            (profile_id,provider,next_cursor,updated_at_ms)
        ).map_err(|error|RepositoryError::Unavailable(error.to_string()))?;
        transaction
            .commit()
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        Ok(())
    }

    fn list_pending_sync_mutations(
        &self,
        profile_id: &str,
        now_ms: i64,
        limit: usize,
    ) -> Result<Vec<SyncMutation>, RepositoryError> {
        validate_profile_id(profile_id)?;
        let limit = limit.clamp(1, 500) as i64;
        let connection = self.lock()?;
        let mut statement=connection.prepare(
            "SELECT id,profile_id,entity_type,entity_id,operation,payload_json,changed_at_ms,attempt_count,next_attempt_at_ms,last_error
             FROM sync_outbox WHERE profile_id=?1 AND next_attempt_at_ms<=?2
             ORDER BY changed_at_ms,id LIMIT ?3"
        ).map_err(|error|RepositoryError::Unavailable(error.to_string()))?;
        statement
            .query_map((profile_id, now_ms, limit), |row| {
                Ok(SyncMutation {
                    id: row.get(0)?,
                    profile_id: row.get(1)?,
                    entity_type: row.get(2)?,
                    entity_id: row.get(3)?,
                    operation: row.get(4)?,
                    payload_json: row.get(5)?,
                    changed_at_ms: row.get(6)?,
                    attempt_count: u32::try_from(row.get::<_, i64>(7)?).unwrap_or_default(),
                    next_attempt_at_ms: row.get(8)?,
                    last_error: row.get(9)?,
                })
            })
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))
    }
    fn mark_sync_mutations_applied(
        &self,
        profile_id: &str,
        mutation_ids: &[String],
    ) -> Result<(), RepositoryError> {
        validate_profile_id(profile_id)?;
        if mutation_ids.len() > 500 {
            return Err(RepositoryError::InvalidLibraryEntry);
        }
        let mut connection = self.lock()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| RepositoryError::Unavailable(e.to_string()))?;
        for id in mutation_ids {
            tx.execute(
                "DELETE FROM sync_outbox WHERE profile_id=?1 AND id=?2",
                (profile_id, id),
            )
            .map_err(|e| RepositoryError::Unavailable(e.to_string()))?;
        }
        tx.commit()
            .map_err(|e| RepositoryError::Unavailable(e.to_string()))?;
        Ok(())
    }
    fn record_sync_mutation_failure(
        &self,
        profile_id: &str,
        mutation_id: &str,
        error: &str,
        next_attempt_at_ms: i64,
    ) -> Result<(), RepositoryError> {
        validate_profile_id(profile_id)?;
        if error.len() > 1000 || next_attempt_at_ms < 0 {
            return Err(RepositoryError::InvalidLibraryEntry);
        }
        let connection = self.lock()?;
        let changed=connection.execute(
            "UPDATE sync_outbox SET attempt_count=attempt_count+1,next_attempt_at_ms=?3,last_error=?4
             WHERE profile_id=?1 AND id=?2",
            (profile_id,mutation_id,next_attempt_at_ms,error)
        ).map_err(|e|RepositoryError::Unavailable(e.to_string()))?;
        if changed == 0 {
            return Err(RepositoryError::NotFound);
        }
        Ok(())
    }
    fn get_sync_checkpoint(
        &self,
        profile_id: &str,
        provider: &str,
    ) -> Result<Option<String>, RepositoryError> {
        validate_profile_id(profile_id)?;
        if provider.trim().is_empty() || provider.len() > 100 {
            return Err(RepositoryError::InvalidLibraryEntry);
        }
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT cursor FROM sync_checkpoints WHERE profile_id=?1 AND provider=?2",
                (profile_id, provider),
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| RepositoryError::Unavailable(e.to_string()))
    }
    fn set_sync_checkpoint(
        &self,
        profile_id: &str,
        provider: &str,
        cursor: &str,
        updated_at_ms: i64,
    ) -> Result<(), RepositoryError> {
        validate_profile_id(profile_id)?;
        if provider.trim().is_empty()
            || provider.len() > 100
            || cursor.len() > 4096
            || updated_at_ms < 0
        {
            return Err(RepositoryError::InvalidLibraryEntry);
        }
        let connection = self.lock()?;
        connection.execute(
            "INSERT INTO sync_checkpoints(profile_id,provider,cursor,updated_at_ms) VALUES(?1,?2,?3,?4)
             ON CONFLICT(profile_id,provider) DO UPDATE SET cursor=excluded.cursor,updated_at_ms=MAX(sync_checkpoints.updated_at_ms,excluded.updated_at_ms)",
            (profile_id,provider,cursor,updated_at_ms)
        ).map_err(|e|RepositoryError::Unavailable(e.to_string()))?;
        Ok(())
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
        let progress_write = transaction
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
        if progress.is_completed && progress_write > 0 {
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
        } else if progress_write > 0 {
            transaction
                .execute(
                    "DELETE FROM watched_items
                     WHERE profile_id = ?1 AND media_kind = ?2 AND media_id = ?3
                       AND season_number = ?4 AND episode_number = ?5",
                    (
                        &progress.profile_id,
                        media_kind_storage_value(progress.media_kind),
                        &progress.media_id,
                        stored_episode_number(progress.season_number),
                        stored_episode_number(progress.episode_number),
                    ),
                )
                .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        }
        enqueue_sync_mutation(
            &transaction,
            &progress.profile_id,
            "playback_progress",
            &playback_sync_entity_key(progress),
            "upsert",
            progress,
            progress.updated_at_ms,
        )?;
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
        let payload = serde_json::to_value(item)
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        enqueue_sync_mutation(
            &transaction,
            &item.profile_id,
            "watched_item",
            &watched_sync_entity_key(item),
            if watched { "upsert" } else { "delete" },
            &payload,
            item.marked_at_ms,
        )?;
        transaction
            .commit()
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))
    }
}

fn playback_sync_entity_key(progress: &PlaybackProgress) -> String {
    format!(
        "{}:{}:{}:{}",
        media_kind_storage_value(progress.media_kind),
        progress.media_id,
        stored_episode_number(progress.season_number),
        stored_episode_number(progress.episode_number)
    )
}

fn watched_sync_entity_key(item: &WatchedItem) -> String {
    format!(
        "{}:{}:{}:{}",
        media_kind_storage_value(item.media_kind),
        item.media_id,
        stored_episode_number(item.season_number),
        stored_episode_number(item.episode_number)
    )
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
    use senzo_domain::{
        LibraryRepository, ProfileRepository, ProfileSecurityRepository, ProfileSettingsRepository,
        SettingKey, SettingsRepository, SyncOutboxRepository,
    };

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
    fn phase11_replaying_a_watched_episode_clears_its_watched_marker() {
        let database = SqliteDatabase::in_memory().unwrap();
        let watched = WatchedItem {
            profile_id: "default".to_owned(),
            media_id: "1399".to_owned(),
            media_kind: MediaKind::Series,
            season_number: Some(1),
            episode_number: Some(2),
            marked_at_ms: 200,
        };
        database.set_watched(&watched, true).unwrap();
        assert!(database.is_watched(&watched).unwrap());

        let replay = PlaybackProgress {
            profile_id: "default".to_owned(),
            media_id: "1399".to_owned(),
            media_kind: MediaKind::Series,
            title: "Game of Thrones".to_owned(),
            season_number: Some(1),
            episode_number: Some(2),
            episode_title: Some("The Kingsroad".to_owned()),
            poster_path: None,
            backdrop_path: None,
            still_path: None,
            position_ms: 45_000,
            duration_ms: 2_700_000,
            is_completed: false,
            updated_at_ms: 300,
        };
        database.save_playback_progress(&replay).unwrap();
        assert!(!database.is_watched(&watched).unwrap());
        assert_eq!(
            database.list_continue_watching("default", 20).unwrap(),
            vec![replay]
        );
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
        assert!(
            database
                .list_continue_watching("default", 20)
                .unwrap()
                .is_empty()
        );
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
        let completed = database
            .list_recent_completed_series("default", 20)
            .unwrap();
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].media_id, "1399");
        assert_eq!(
            (completed[0].season_number, completed[0].episode_number),
            (Some(1), Some(4))
        );
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
        assert!(
            database
                .list_recent_completed_series("default", 20)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn phase11_progress_and_watched_state_survive_database_reopen() {
        let path =
            std::env::temp_dir().join(format!("senzo-phase11-{}.sqlite", uuid::Uuid::new_v4()));
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
                database
                    .get_playback_progress("default", "1399", MediaKind::Series, Some(2), Some(3))
                    .unwrap(),
                Some(progress)
            );
            let watched = WatchedItem {
                profile_id: "default".to_owned(),
                media_id: "1399".to_owned(),
                media_kind: MediaKind::Series,
                season_number: Some(2),
                episode_number: Some(2),
                marked_at_ms: 400,
            };
            assert!(database.is_watched(&watched).unwrap());
            assert!(
                database
                    .list_recent_completed_series("default", 20)
                    .unwrap()
                    .is_empty(),
                "newer in-progress episode must suppress the Up Next projection"
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

    fn test_library_item(
        profile_id: &str,
        media_id: &str,
        title: &str,
        timestamp: i64,
    ) -> LibraryItem {
        LibraryItem {
            profile_id: profile_id.to_owned(),
            media_id: media_id.to_owned(),
            media_kind: MediaKind::Movie,
            title: title.to_owned(),
            overview: Some("A local-first test title".to_owned()),
            release_year: Some(2024),
            poster_path: Some("/poster.jpg".to_owned()),
            backdrop_path: Some("/backdrop.jpg".to_owned()),
            added_at_ms: timestamp,
            updated_at_ms: timestamp,
            is_favorite: false,
        }
    }

    #[test]
    fn phase12_library_and_collections_are_profile_scoped_and_ordered() {
        let database = SqliteDatabase::in_memory().unwrap();
        let guest = database.create_profile("Guest").unwrap();
        let first = test_library_item("default", "550", "Fight Club", 100);
        let second = test_library_item("default", "680", "Pulp Fiction", 200);
        let guest_item = test_library_item(&guest.id, "550", "Guest Fight Club", 300);
        database.save_library_item(&first).unwrap();
        database.save_library_item(&second).unwrap();
        database.save_library_item(&guest_item).unwrap();

        let mut favorite = second.clone();
        favorite.is_favorite = true;
        database.save_library_item(&favorite).unwrap();
        let items = database.list_library_items("default").unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].media_id, "680");
        assert!(items[0].is_favorite);
        assert_eq!(
            database.list_library_items(&guest.id).unwrap(),
            vec![guest_item]
        );

        let collection = database
            .create_library_collection("default", "Weekend", Some("Watch these together"), 400)
            .unwrap();
        database
            .set_collection_item(
                "default",
                &collection.id,
                "550",
                MediaKind::Movie,
                true,
                410,
            )
            .unwrap();
        database
            .set_collection_item(
                "default",
                &collection.id,
                "680",
                MediaKind::Movie,
                true,
                420,
            )
            .unwrap();
        let collection_items = database
            .list_collection_items("default", &collection.id)
            .unwrap();
        assert_eq!(
            collection_items
                .iter()
                .map(|item| item.media_id.as_str())
                .collect::<Vec<_>>(),
            vec!["550", "680"]
        );

        database
            .reorder_collection_items(
                "default",
                &collection.id,
                &[
                    ("680".to_owned(), MediaKind::Movie),
                    ("550".to_owned(), MediaKind::Movie),
                ],
                430,
            )
            .unwrap();
        let reordered = database
            .list_collection_items("default", &collection.id)
            .unwrap();
        assert_eq!(
            reordered
                .iter()
                .map(|item| item.media_id.as_str())
                .collect::<Vec<_>>(),
            vec!["680", "550"]
        );

        let renamed = database
            .update_library_collection("default", &collection.id, "Weekend queue", None, 440)
            .unwrap();
        assert_eq!(renamed.title, "Weekend queue");
        assert_eq!(
            database.list_library_collections("default").unwrap().len(),
            1
        );
        assert!(
            database
                .list_library_collections(&guest.id)
                .unwrap()
                .is_empty()
        );

        database
            .remove_library_item("default", "550", MediaKind::Movie, 450)
            .unwrap();
        assert_eq!(database.list_library_items("default").unwrap().len(), 1);
        let remaining_collection_items = database
            .list_collection_items("default", &collection.id)
            .unwrap();
        assert_eq!(
            remaining_collection_items
                .iter()
                .map(|item| item.media_id.as_str())
                .collect::<Vec<_>>(),
            vec!["680"],
            "removing a library title must remove only that title's collection membership"
        );
        database
            .delete_library_collection("default", &collection.id, 460)
            .unwrap();
        assert!(
            database
                .list_library_collections("default")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn phase12_profile_settings_and_management_do_not_cross_profile_boundaries() {
        let database = SqliteDatabase::in_memory().unwrap();
        let guest = database.create_profile("Guest").unwrap();
        let key = SettingKey::new("appearance.language").unwrap();
        database
            .set_profile_setting("default", &key, "en-US")
            .unwrap();
        database
            .set_profile_setting(&guest.id, &key, "bn-BD")
            .unwrap();
        assert_eq!(
            database
                .get_profile_setting("default", &key)
                .unwrap()
                .as_deref(),
            Some("en-US")
        );
        assert_eq!(
            database
                .get_profile_setting(&guest.id, &key)
                .unwrap()
                .as_deref(),
            Some("bn-BD")
        );

        let renamed = database.update_profile(&guest.id, "  Family  ").unwrap();
        assert_eq!(renamed.name, "Family");
        assert!(database.delete_profile("default").is_err());
        database.delete_profile(&guest.id).unwrap();
        assert_eq!(
            database.list_profiles().unwrap(),
            vec![Profile {
                id: "default".to_owned(),
                name: "Default".to_owned()
            }]
        );
        assert_eq!(database.get_profile_setting(&guest.id, &key).unwrap(), None);
    }

    #[test]
    fn phase12_profile_pin_uses_salted_hashes_and_temporary_lockout() {
        let database = SqliteDatabase::in_memory().unwrap();
        let profile = database.create_profile("Kids").unwrap();
        assert!(!database.has_profile_pin(&profile.id).unwrap());
        assert!(matches!(
            database.set_profile_pin(&profile.id, "12"),
            Err(RepositoryError::InvalidProfilePin)
        ));
        database.set_profile_pin(&profile.id, "4826").unwrap();
        assert!(database.has_profile_pin(&profile.id).unwrap());
        assert!(
            database
                .verify_profile_pin(&profile.id, "4826", 10_000)
                .unwrap()
        );
        for attempt in 0..5 {
            assert!(
                !database
                    .verify_profile_pin(&profile.id, "0000", 20_000 + attempt)
                    .unwrap()
            );
        }
        assert!(matches!(
            database.verify_profile_pin(&profile.id, "4826", 20_010),
            Err(RepositoryError::ProfilePinLocked)
        ));
        assert!(
            database
                .verify_profile_pin(&profile.id, "4826", 320_011)
                .unwrap()
        );
        database.clear_profile_pin(&profile.id).unwrap();
        assert!(!database.has_profile_pin(&profile.id).unwrap());
    }

    #[test]
    fn phase12_sync_outbox_retries_offline_mutations_and_persists_checkpoints() {
        let database = SqliteDatabase::in_memory().unwrap();
        database
            .save_library_item(&test_library_item("default", "550", "Fight Club", 100))
            .unwrap();
        let first = database
            .list_pending_sync_mutations("default", 100, 20)
            .unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].entity_type, "library_item");
        assert_eq!(first[0].operation, "upsert");

        database
            .record_sync_mutation_failure("default", &first[0].id, "offline", 1_000)
            .unwrap();
        assert!(
            database
                .list_pending_sync_mutations("default", 999, 20)
                .unwrap()
                .is_empty()
        );
        let retry = database
            .list_pending_sync_mutations("default", 1_000, 20)
            .unwrap();
        assert_eq!(retry.len(), 1);
        assert_eq!(retry[0].attempt_count, 1);
        assert_eq!(retry[0].last_error.as_deref(), Some("offline"));

        database
            .set_sync_checkpoint("default", "supabase", "cursor-42", 1_200)
            .unwrap();
        assert_eq!(
            database
                .get_sync_checkpoint("default", "supabase")
                .unwrap()
                .as_deref(),
            Some("cursor-42")
        );
        database
            .mark_sync_mutations_applied("default", &[retry[0].id.clone()])
            .unwrap();
        assert!(
            database
                .list_pending_sync_mutations("default", 2_000, 20)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn phase12_remote_sync_pages_reconcile_library_settings_and_watched_state_atomically() {
        let database = SqliteDatabase::in_memory().unwrap();
        let item = test_library_item("another-device", "550", "Fight Club", 100);
        let collection = LibraryCollection {
            id: "weekend-list".to_owned(),
            profile_id: "another-device".to_owned(),
            title: "Weekend".to_owned(),
            description: Some("Remote collection".to_owned()),
            sort_order: 0,
            created_at_ms: 110,
            updated_at_ms: 110,
        };
        let watched = WatchedItem {
            profile_id: "another-device".to_owned(),
            media_id: "1399".to_owned(),
            media_kind: MediaKind::Series,
            season_number: Some(1),
            episode_number: Some(2),
            marked_at_ms: 120,
        };
        let mutations = vec![
            senzo_domain::RemoteSyncMutation {
                cursor: 1,
                entity_type: "library_item".to_owned(),
                entity_id: "movie:550".to_owned(),
                operation: "upsert".to_owned(),
                payload_json: serde_json::to_string(&item).unwrap(),
                changed_at_ms: 100,
            },
            senzo_domain::RemoteSyncMutation {
                cursor: 2,
                entity_type: "library_collection".to_owned(),
                entity_id: collection.id.clone(),
                operation: "upsert".to_owned(),
                payload_json: serde_json::to_string(&collection).unwrap(),
                changed_at_ms: 110,
            },
            senzo_domain::RemoteSyncMutation {
                cursor: 3,
                entity_type: "watched_item".to_owned(),
                entity_id: "series:1399:1:2".to_owned(),
                operation: "upsert".to_owned(),
                payload_json: serde_json::to_string(&watched).unwrap(),
                changed_at_ms: 120,
            },
            senzo_domain::RemoteSyncMutation {
                cursor: 4,
                entity_type: "profile_setting".to_owned(),
                entity_id: "library.favorites_only".to_owned(),
                operation: "upsert".to_owned(),
                payload_json: serde_json::json!({"key":"library.favorites_only","value":"true"})
                    .to_string(),
                changed_at_ms: 130,
            },
        ];
        database
            .apply_remote_sync_page("default", "supabase", "4", &mutations, 140)
            .unwrap();

        let library = database.list_library_items("default").unwrap();
        assert_eq!(library.len(), 1);
        assert_eq!(library[0].media_id, "550");
        assert_eq!(
            database.list_library_collections("default").unwrap()[0].title,
            "Weekend"
        );
        assert!(
            database
                .is_watched(&WatchedItem {
                    profile_id: "default".to_owned(),
                    ..watched.clone()
                })
                .unwrap()
        );
        let key = SettingKey::new("library.favorites_only").unwrap();
        assert_eq!(
            database
                .get_profile_setting("default", &key)
                .unwrap()
                .as_deref(),
            Some("true")
        );
        assert_eq!(
            database
                .get_sync_checkpoint("default", "supabase")
                .unwrap()
                .as_deref(),
            Some("4")
        );

        // A newer incomplete replay arriving from another device removes the watched marker.
        let replay = PlaybackProgress {
            profile_id: "another-device".to_owned(),
            media_id: "1399".to_owned(),
            media_kind: MediaKind::Series,
            title: "Game of Thrones".to_owned(),
            season_number: Some(1),
            episode_number: Some(2),
            episode_title: Some("The Kingsroad".to_owned()),
            poster_path: None,
            backdrop_path: None,
            still_path: None,
            position_ms: 50_000,
            duration_ms: 2_700_000,
            is_completed: false,
            updated_at_ms: 200,
        };
        let second_page = [senzo_domain::RemoteSyncMutation {
            cursor: 5,
            entity_type: "playback_progress".to_owned(),
            entity_id: "series:1399:1:2".to_owned(),
            operation: "upsert".to_owned(),
            payload_json: serde_json::to_string(&replay).unwrap(),
            changed_at_ms: 200,
        }];
        database
            .apply_remote_sync_page("default", "supabase", "5", &second_page, 210)
            .unwrap();
        assert!(
            !database
                .is_watched(&WatchedItem {
                    profile_id: "default".to_owned(),
                    ..watched
                })
                .unwrap()
        );
        assert_eq!(
            database
                .get_sync_checkpoint("default", "supabase")
                .unwrap()
                .as_deref(),
            Some("5")
        );
    }

    #[test]
    fn phase12_remote_sync_page_rejects_bad_cursor_without_partial_writes() {
        let database = SqliteDatabase::in_memory().unwrap();
        let item = test_library_item("other", "550", "Fight Club", 100);
        let mutation = senzo_domain::RemoteSyncMutation {
            cursor: 3,
            entity_type: "library_item".to_owned(),
            entity_id: "movie:550".to_owned(),
            operation: "upsert".to_owned(),
            payload_json: serde_json::to_string(&item).unwrap(),
            changed_at_ms: 100,
        };
        assert!(
            database
                .apply_remote_sync_page("default", "supabase", "2", &[mutation], 120)
                .is_err()
        );
        assert!(database.list_library_items("default").unwrap().is_empty());
        assert_eq!(
            database.get_sync_checkpoint("default", "supabase").unwrap(),
            None
        );
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
    fn phase12_profile_scoped_settings_facade_switches_namespace_without_losing_data() {
        let database = Arc::new(SqliteDatabase::in_memory().unwrap());
        let guest = database.create_profile("Guest").unwrap();
        let scope = ProfileScope::new("default");
        let settings = ProfileScopedSettingsRepository::new(database.clone(), scope.clone());
        let language = SettingKey::new("appearance.language").unwrap();
        let active = SettingKey::new("profiles.active").unwrap();
        settings.set(&language, "en-US").unwrap();
        settings.set(&active, &guest.id).unwrap();
        scope.set(guest.id.clone());
        settings.set(&language, "bn-BD").unwrap();
        assert_eq!(settings.get(&language).unwrap().as_deref(), Some("bn-BD"));
        assert_eq!(
            settings.get(&active).unwrap().as_deref(),
            Some(guest.id.as_str())
        );
        scope.set("default");
        assert_eq!(settings.get(&language).unwrap().as_deref(), Some("en-US"));
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
