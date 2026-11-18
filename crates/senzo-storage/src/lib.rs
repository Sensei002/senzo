//! SQLite persistence with ordered, transactional schema migrations.

use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
    time::Duration,
};

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};
use senzo_addons::{AddonRecord, AddonStore, AddonStoreError};
use senzo_domain::{Profile, ProfileRepository, RepositoryError, SettingKey, SettingsRepository};
use thiserror::Error;

pub const CURRENT_SCHEMA_VERSION: i64 = 3;

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

#[cfg(test)]
mod tests {
    use super::*;
    use senzo_domain::{ProfileRepository, SettingKey, SettingsRepository};

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
