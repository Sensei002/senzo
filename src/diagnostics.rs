use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};

const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;
const MAX_EXPORT_BYTES: usize = 1024 * 1024;
const DEFAULT_LOOKBACK_SECONDS: u64 = 72 * 60 * 60;

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();
static LOG_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Audit,
    Info,
    Warn,
    Error,
}

impl Level {
    fn as_str(self) -> &'static str {
        match self {
            Self::Audit => "AUDIT",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

/// Initializes the per-user diagnostic log. Existing logs are retained across launches.
pub fn initialize() -> Result<()> {
    let app_data = dirs::data_local_dir()
        .context("the operating system did not provide a local app-data directory")?;
    let directory = app_data.join("SENZO").join("logs");
    fs::create_dir_all(&directory)
        .with_context(|| format!("failed to create {}", directory.display()))?;
    let path = directory.join("senzo.log");
    let _ = LOG_PATH.set(path);

    record(
        Level::Audit,
        "application",
        &format!(
            "SENZO session started; version={}; platform={}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS
        ),
    );
    Ok(())
}

/// Records an application event. Callers should never pass credentials or private media URLs.
pub fn record(level: Level, target: &str, message: &str) {
    let Some(path) = LOG_PATH.get() else {
        return;
    };
    let _guard = LOG_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let _ = append_record(path, level, target, message);
}

/// Returns a support bundle containing recent diagnostic events, not personal watch history.
pub fn collect_recent_logs() -> Result<String> {
    let path = LOG_PATH
        .get()
        .context("diagnostic logging has not been initialized")?;
    collect_from(path, DEFAULT_LOOKBACK_SECONDS)
}

fn append_record(path: &Path, level: Level, target: &str, message: &str) -> Result<()> {
    if fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0)
        >= MAX_LOG_BYTES
    {
        let rotated = path.with_extension("log.1");
        let _ = fs::remove_file(&rotated);
        if path.exists() {
            fs::rename(path, rotated).context("failed to rotate diagnostic log")?;
        }
    }

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let safe_target = sanitize(target);
    let safe_message = sanitize(message);
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open diagnostic log {}", path.display()))?;
    writeln!(
        file,
        "[unix={timestamp}][{}][{}] {}",
        level.as_str(),
        safe_target,
        safe_message
    )
    .context("failed to append diagnostic event")
}

fn collect_from(path: &Path, lookback_seconds: u64) -> Result<String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let cutoff = now.saturating_sub(lookback_seconds);
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read diagnostic log {}", path.display()))?;
    let mut selected = Vec::new();
    let mut bytes = 0usize;

    for line in content.lines().rev() {
        let timestamp = line
            .strip_prefix("[unix=")
            .and_then(|rest| rest.split(']').next())
            .and_then(|value| value.parse::<u64>().ok());
        if timestamp.is_some_and(|value| value >= cutoff) {
            let line_bytes = line.len() + 1;
            if bytes + line_bytes > MAX_EXPORT_BYTES {
                break;
            }
            selected.push(line);
            bytes += line_bytes;
        }
    }
    selected.reverse();

    Ok(format!(
        "SENZO diagnostic report\nVersion: {}\nPlatform: {}\nLookback: last 72 hours\nPersonal watch history and database contents are not included.\nSecrets and common credential fields are redacted where detected.\n\n{}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        selected.join("\n")
    ))
}

fn sanitize(input: &str) -> String {
    let mut output = input.to_owned();
    for key in [
        "authorization",
        "token",
        "api_key",
        "apikey",
        "password",
        "secret",
    ] {
        let lower = output.to_ascii_lowercase();
        let mut cursor = 0;
        while let Some(relative) = lower[cursor..].find(key) {
            let start = cursor + relative;
            let value_start = start + key.len();
            let separator = output[value_start..]
                .find(['=', ':'])
                .map(|offset| value_start + offset);
            if let Some(separator) = separator {
                let end = output[separator + 1..]
                    .find(['&', ' ', ',', ';'])
                    .map(|offset| separator + 1 + offset)
                    .unwrap_or(output.len());
                output.replace_range(separator + 1..end, "[REDACTED]");
                cursor = separator + "[REDACTED]".len() + 1;
            } else {
                cursor = value_start;
            }
            if cursor >= output.len() {
                break;
            }
        }
    }
    output.replace(['\r', '\n'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_audit_and_error_events_and_exports_recent_entries() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("senzo.log");
        append_record(&path, Level::Audit, "settings", "settings opened")
            .expect("write audit event");
        append_record(&path, Level::Error, "metadata", "request failed")
            .expect("write error event");

        let report = collect_from(&path, DEFAULT_LOOKBACK_SECONDS).expect("collect logs");
        assert!(report.contains("[AUDIT][settings] settings opened"));
        assert!(report.contains("[ERROR][metadata] request failed"));
        assert!(report.contains("Personal watch history and database contents are not included."));
    }

    #[test]
    fn redacts_common_credentials() {
        let safe = sanitize("token=abc123 api_key:xyz password=hunter2");
        assert!(safe.contains("token=[REDACTED]"));
        assert!(safe.contains("api_key:[REDACTED]"));
        assert!(safe.contains("password=[REDACTED]"));
        assert!(!safe.contains("abc123"));
        assert!(!safe.contains("hunter2"));
    }

    #[test]
    fn ignores_entries_older_than_the_requested_window() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("senzo.log");
        let old = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .saturating_sub(10_000);
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .expect("open log");
        writeln!(file, "[unix={old}][INFO][old] old event").expect("write old event");
        drop(file);

        let report = collect_from(&path, 60).expect("collect recent logs");
        assert!(!report.contains("old event"));
    }
}
