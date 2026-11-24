use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

/// A small, profile-scoped local playback ledger.
///
/// The core library only exposes a resume offset when the title is already in
/// the user's library. Direct sources and catalogue previews are not, so they
/// used to start from zero after every restart. This ledger fills that gap
/// without adding a second database dependency to the playback hot path.
const RESUME_MIN_SECONDS: i64 = 10;
const COMPLETED_GUARD_SECONDS: i64 = 12;
const FLUSH_INTERVAL: Duration = Duration::from_secs(5);
const MAX_ENTRIES: usize = 2_000;

#[derive(Debug, Default, Serialize, Deserialize)]
struct ProgressFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    entries: HashMap<String, Entry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Entry {
    position: i64,
    duration: i64,
    updated_at: i64,
}

struct Store {
    path: PathBuf,
    profile_id: String,
    entries: HashMap<String, Entry>,
    last_flush: Instant,
    dirty: bool,
}

static STORE: OnceLock<Mutex<Store>> = OnceLock::new();

pub(crate) fn initialize(profile_id: &str) {
    let path = crate::paths::get().root().join("watch-progress.json");
    if let Some(store) = STORE.get() {
        let Ok(mut store) = store.lock() else {
            return;
        };
        if store.profile_id == profile_id {
            return;
        }
        // Switching profiles is a scope change, not a process restart. Persist
        // the outgoing profile first, then reload the shared file so the next
        // profile sees its own entries without losing the previous one.
        flush_locked(&mut store);
        store.entries = load_entries(&path);
        store.profile_id = profile_id.to_owned();
        store.path = path;
        store.dirty = false;
        store.last_flush = Instant::now();
        return;
    }
    let store = Store {
        path,
        profile_id: profile_id.to_owned(),
        entries: load_entries(&path),
        last_flush: Instant::now(),
        dirty: false,
    };
    let _ = STORE.set(Mutex::new(store));
}

fn load_entries(path: &std::path::Path) -> HashMap<String, Entry> {
    match fs::read_to_string(path) {
        Ok(contents) => match serde_json::from_str::<ProgressFile>(&contents) {
            Ok(file) => file.entries,
            Err(error) => {
                tracing::warn!(%error, path = %path.display(), "ignored invalid local watch-progress file");
                HashMap::new()
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "could not read local watch-progress file");
            HashMap::new()
        }
    }
}

fn key(profile_id: &str, meta_id: &str, video_id: &str) -> String {
    format!("{profile_id}\u{1f}{meta_id}\u{1f}{video_id}")
}

fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

/// Whether this profile has any saved position for the item. A present but
/// ineligible entry (too early, or effectively finished) is still authoritative:
/// it must not fall through to a stale core-library resume point.
pub(crate) fn has_entry(meta_id: &str, video_id: &str) -> bool {
    let Some(store) = STORE.get() else {
        return false;
    };
    let Ok(store) = store.lock() else {
        return false;
    };
    store
        .entries
        .contains_key(&key(&store.profile_id, meta_id, video_id))
}

/// Return a safe resume point for the active profile.
///
/// The first and final few seconds are deliberately ignored: resuming at 0:03
/// is indistinguishable from starting over, and resuming into the credits is
/// worse than starting cleanly. A saved point is also kept a couple of seconds
/// behind the last observed time so the viewer gets a short running start.
pub(crate) fn resume_position(meta_id: &str, video_id: &str) -> Option<f64> {
    let store = STORE.get()?.lock().ok()?;
    let entry = store
        .entries
        .get(&key(&store.profile_id, meta_id, video_id))?;
    if entry.duration <= 0
        || entry.position < RESUME_MIN_SECONDS
        || entry.position >= entry.duration - COMPLETED_GUARD_SECONDS
    {
        return None;
    }
    Some(entry.position.saturating_sub(2) as f64)
}

pub(crate) fn record(meta_id: &str, video_id: &str, position: i64, duration: i64) {
    if meta_id.is_empty() || video_id.is_empty() || position <= 0 || duration <= 0 {
        return;
    }
    let Some(store) = STORE.get() else {
        return;
    };
    let Ok(mut store) = store.lock() else {
        return;
    };
    let key = key(&store.profile_id, meta_id, video_id);
    if position >= duration - COMPLETED_GUARD_SECONDS {
        if store.entries.remove(&key).is_some() {
            store.dirty = true;
        }
    } else {
        let entry = Entry {
            position,
            duration,
            updated_at: now_seconds(),
        };
        let unchanged = store.entries.get(&key).is_some_and(|previous| {
            previous.position == entry.position && previous.duration == entry.duration
        });
        if !unchanged {
            store.entries.insert(key, entry);
            store.dirty = true;
        }
    }

    if store.dirty && store.last_flush.elapsed() >= FLUSH_INTERVAL {
        flush_locked(&mut store);
    }
}

/// Persist the latest position immediately. Called when playback unloads so a
/// normal close does not lose the last few seconds of progress.
pub(crate) fn flush() {
    let Some(store) = STORE.get() else {
        return;
    };
    let Ok(mut store) = store.lock() else {
        return;
    };
    flush_locked(&mut store);
}

fn flush_locked(store: &mut Store) {
    if !store.dirty {
        return;
    }
    if store.entries.len() > MAX_ENTRIES {
        let mut newest: Vec<_> = store
            .entries
            .iter()
            .map(|(key, entry)| (key.clone(), entry.updated_at))
            .collect();
        newest.sort_unstable_by_key(|(_, updated_at)| *updated_at);
        let remove_count = store.entries.len() - MAX_ENTRIES;
        for (key, _) in newest.into_iter().take(remove_count) {
            store.entries.remove(&key);
        }
    }
    let file = ProgressFile {
        version: 1,
        entries: store.entries.clone(),
    };
    match serde_json::to_vec_pretty(&file) {
        Ok(contents) => {
            if let Err(error) = fs::write(&store.path, contents) {
                tracing::warn!(%error, path = %store.path.display(), "could not save local watch progress");
            } else {
                store.dirty = false;
                store.last_flush = Instant::now();
            }
        }
        Err(error) => tracing::warn!(%error, "could not serialize local watch progress"),
    }
}

#[cfg(test)]
mod tests {
    use super::{COMPLETED_GUARD_SECONDS, RESUME_MIN_SECONDS};

    #[test]
    fn resume_guards_leave_a_clean_start_and_end() {
        assert!(RESUME_MIN_SECONDS > 0);
        assert!(COMPLETED_GUARD_SECONDS > 0);
    }
}
