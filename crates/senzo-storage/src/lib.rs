//! SQLite persistence with ordered, transactional schema migrations.

use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
    time::Duration,
};

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};
use senzo_domain::{Profile, ProfileRepository, RepositoryError, SettingKey, SettingsRepository};
use thiserror::Error;

pub const CURRENT_SCHEMA_VERSION: i64 = 2;

const MIGRATIONS: &[(i64, &str)] = &[
    (1, "CREATE TABLE app_settings (
        key TEXT PRIMARY KEY NOT NULL,
        value TEXT NOT NULL,
        updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
    );"),
    (2, "CREATE TABLE profiles (
        id TEXT PRIMARY KEY NOT NULL,
        name TEXT NOT NULL,
        created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
    );
    INSERT OR IGNORE INTO profiles (id, name) VALUES ('default', 'Default');"),
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

pub struct SqliteDatabase { connection: Mutex<Connection> }

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
        Ok(Self { connection: Mutex::new(connection) })
    }

    pub fn schema_version(&self) -> Result<i64, StorageError> {
        let connection = self.connection.lock().map_err(|_| StorageError::LockPoisoned)?;
        read_schema_version(&connection).map_err(StorageError::Sqlite)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, RepositoryError> {
        self.connection.lock()
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
        return Err(StorageError::UnsupportedSchema { found: current, supported: CURRENT_SCHEMA_VERSION });
    }
    apply_migrations(&transaction, current)?;
    transaction.commit()?;
    Ok(())
}

fn apply_migrations(transaction: &Transaction<'_>, current: i64) -> Result<(), rusqlite::Error> {
    for (version, sql) in MIGRATIONS {
        if *version > current {
            transaction.execute_batch(sql)?;
            transaction.execute("INSERT INTO schema_migrations (version) VALUES (?1)", [version])?;
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

impl SettingsRepository for SqliteDatabase {
    fn get(&self, key: &SettingKey) -> Result<Option<String>, RepositoryError> {
        let connection = self.lock()?;
        connection.query_row("SELECT value FROM app_settings WHERE key = ?1", [key.as_str()], |row| row.get(0))
            .optional()
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))
    }

    fn set(&self, key: &SettingKey, value: &str) -> Result<(), RepositoryError> {
        let connection = self.lock()?;
        connection.execute(
            "INSERT INTO app_settings (key, value, updated_at)
             VALUES (?1, ?2, CURRENT_TIMESTAMP)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP",
            (key.as_str(), value),
        ).map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        Ok(())
    }
}

impl ProfileRepository for SqliteDatabase {
    fn list_profiles(&self) -> Result<Vec<Profile>, RepositoryError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare("SELECT id, name FROM profiles ORDER BY created_at, id")
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))?;
        let profiles = statement.query_map([], |row| Ok(Profile { id: row.get(0)?, name: row.get(1)? }))
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
        let profile = Profile { id: uuid::Uuid::new_v4().to_string(), name: name.to_owned() };
        let connection = self.lock()?;
        connection.execute("INSERT INTO profiles (id, name) VALUES (?1, ?2)", (&profile.id, &profile.name))
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
        assert!(matches!(database.create_profile(" "), Err(RepositoryError::InvalidProfileName)));
    }
}
