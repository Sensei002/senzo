//! Persistent, bounded and redacted diagnostics for SENZO Desktop.

use std::{
    error::Error,
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

use regex::Regex;
use tracing_subscriber::{EnvFilter, fmt::MakeWriter};
use url::Url;

const MAX_LOG_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_RECENT_LOG_BYTES: usize = 4 * 1024 * 1024;
const MAX_EXPORT_BYTES: usize = 3 * 1024 * 1024;
static ACTIVE_SINK: OnceLock<Arc<DiagnosticsSink>> = OnceLock::new();

pub struct Secret<'a>(pub &'a str);

impl fmt::Debug for Secret<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

struct SinkState {
    file: File,
    recent: String,
}

struct DiagnosticsSink {
    state: Mutex<SinkState>,
}

impl DiagnosticsSink {
    fn open(path: PathBuf) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let recent = read_recent(&path, MAX_RECENT_LOG_BYTES)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(&path)?;
        if file.metadata()?.len() > MAX_LOG_FILE_BYTES {
            file.set_len(0)?;
            file.seek(SeekFrom::Start(0))?;
            file.write_all(recent.as_bytes())?;
            file.flush()?;
        }
        Ok(Self {
            state: Mutex::new(SinkState { file, recent }),
        })
    }

    fn append(&self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let safe = redact_text(&String::from_utf8_lossy(bytes));
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state.recent.push_str(&safe);
        trim_recent(&mut state.recent, MAX_RECENT_LOG_BYTES);
        if state.file.write_all(safe.as_bytes()).is_err() {
            return;
        }
        let _ = state.file.flush();
        if state
            .file
            .metadata()
            .is_ok_and(|metadata| metadata.len() > MAX_LOG_FILE_BYTES)
            && state.file.set_len(0).is_ok()
            && state.file.seek(SeekFrom::Start(0)).is_ok()
        {
            let recent = state.recent.clone();
            let _ = state.file.write_all(recent.as_bytes());
            let _ = state.file.flush();
        }
    }

    fn export(&self) -> String {
        let recent = self
            .state
            .lock()
            .map(|state| state.recent.clone())
            .unwrap_or_else(|_| "[Diagnostics buffer unavailable]".to_owned());
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_default();
        let mut report = format!(
            "SENZO Diagnostic Report\nApp version: {}\nOperating system: {} ({})\nArchitecture: {}\nGenerated at (Unix UTC): {}\nRecent log buffer: {} MiB\nPersistent log cap: {} MiB\nSensitive values are redacted before storage and export.\n\n--- Recent application log ---\n{}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::FAMILY,
            std::env::consts::ARCH,
            timestamp,
            MAX_RECENT_LOG_BYTES / (1024 * 1024),
            MAX_LOG_FILE_BYTES / (1024 * 1024),
            recent
        );
        if report.len() > MAX_EXPORT_BYTES {
            let mut start = report.len() - MAX_EXPORT_BYTES;
            while !report.is_char_boundary(start) {
                start += 1;
            }
            report = format!(
                "[Older diagnostic lines omitted to keep this export bounded.]\n{}",
                &report[start..]
            );
        }
        report
    }
}

fn read_recent(path: &Path, limit: usize) -> io::Result<String> {
    if !path.exists() {
        return Ok(String::new());
    }
    let mut file = File::open(path)?;
    let length = file.metadata()?.len();
    file.seek(SeekFrom::Start(length.saturating_sub(limit as u64)))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let mut recent = redact_text(&String::from_utf8_lossy(&bytes));
    trim_recent(&mut recent, limit);
    Ok(recent)
}

fn trim_recent(value: &mut String, limit: usize) {
    if value.len() <= limit {
        return;
    }
    let mut start = value.len() - limit;
    while !value.is_char_boundary(start) {
        start += 1;
    }
    value.drain(..start);
}

struct DiagnosticsWriter {
    sink: Arc<DiagnosticsSink>,
    pending: Vec<u8>,
}

impl Write for DiagnosticsWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.sink.append(&self.pending);
        self.pending.clear();
        Ok(())
    }
}

impl Drop for DiagnosticsWriter {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

struct DiagnosticsSinkWriter(Arc<DiagnosticsSink>);

impl<'a> MakeWriter<'a> for DiagnosticsSinkWriter {
    type Writer = DiagnosticsWriter;

    fn make_writer(&'a self) -> Self::Writer {
        DiagnosticsWriter {
            sink: self.0.clone(),
            pending: Vec::new(),
        }
    }
}

pub fn init() -> Result<(), Box<dyn Error + Send + Sync>> {
    init_with_file(std::env::temp_dir().join("senzo-diagnostics/senzo.log"))
}

/// Initialize structured tracing with a bounded persistent file and in-memory export buffer.
pub fn init_with_file(path: impl Into<PathBuf>) -> Result<(), Box<dyn Error + Send + Sync>> {
    let sink = Arc::new(DiagnosticsSink::open(path.into())?);
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_thread_ids(true)
        .with_ansi(false)
        .compact()
        .with_writer(DiagnosticsSinkWriter(sink.clone()))
        .try_init()?;
    let _ = ACTIVE_SINK.set(sink);

    std::panic::set_hook(Box::new(|panic| {
        let message = panic
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| panic.payload().downcast_ref::<String>().map(String::as_str))
            .unwrap_or("non-string panic payload");
        let location = panic
            .location()
            .map(|location| {
                format!(
                    "{}:{}:{}",
                    location.file(),
                    location.line(),
                    location.column()
                )
            })
            .unwrap_or_else(|| "unknown location".to_owned());
        let backtrace = std::backtrace::Backtrace::force_capture();
        tracing::error!(
            panic_message = %redact_text(message),
            location = %redact_text(&location),
            backtrace = %redact_text(&backtrace.to_string()),
            "SENZO panic captured"
        );
    }));

    tracing::info!(
        app_version = env!("CARGO_PKG_VERSION"),
        operating_system = std::env::consts::OS,
        architecture = std::env::consts::ARCH,
        "SENZO diagnostics initialized"
    );
    Ok(())
}

/// Return a bounded support report suitable for copying into a bug report.
pub fn export_text() -> String {
    ACTIVE_SINK
        .get()
        .map(|sink| sink.export())
        .unwrap_or_else(|| {
            "SENZO Diagnostic Report\nDiagnostics are not initialized in this process.\n".to_owned()
        })
}

pub fn redact_url(value: &str) -> String {
    let Ok(mut url) = Url::parse(value) else {
        return "[invalid-url]".to_owned();
    };
    if !url.username().is_empty() {
        let _ = url.set_username("[REDACTED]");
    }
    if url.password().is_some() {
        let _ = url.set_password(Some("[REDACTED]"));
    }
    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(key, value)| {
            let replacement = if is_sensitive_name(&key) {
                "[REDACTED]".to_owned()
            } else {
                value.into_owned()
            };
            (key.into_owned(), replacement)
        })
        .collect();
    url.set_query(None);
    if !pairs.is_empty() {
        url.query_pairs_mut().extend_pairs(
            pairs
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str())),
        );
    }
    url.to_string()
}

fn redaction_patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            r#"(?i)(authorization\s*[:=]\s*bearer\s+)[^\s,;\"']+"#,
            r#"(?i)(\bbearer\s+)[A-Za-z0-9._~+/=-]+"#,
            r#"(?i)([\"']?(?:api[_-]?key|apikey|access[_-]?token|refresh[_-]?token|id[_-]?token|token|password|passwd|secret|client[_-]?secret|cookie|set-cookie|x-api-key)[\"']?\s*[:=]\s*[\"']?)[^\"'&,;\s}]+([\"']?)"#,
            r#"(?i)([?&](?:api[_-]?key|apikey|access[_-]?token|refresh[_-]?token|id[_-]?token|token|secret|password)=)[^&#\s\"'<>]+"#,
            r#"(?i)https?://[^\s\"'<>]+"#,
        ]
        .into_iter()
        .filter_map(|pattern| Regex::new(pattern).ok())
        .collect()
    })
}

pub fn redact_text(value: &str) -> String {
    let mut output = value.to_owned();
    for (index, pattern) in redaction_patterns().iter().enumerate() {
        output = pattern
            .replace_all(&output, |captures: &regex::Captures<'_>| match index {
                0 | 1 => format!(
                    "{}[REDACTED]",
                    captures.get(1).map_or("", |capture| capture.as_str())
                ),
                2 => format!(
                    "{}[REDACTED]{}",
                    captures.get(1).map_or("", |capture| capture.as_str()),
                    captures.get(2).map_or("", |capture| capture.as_str())
                ),
                3 => format!(
                    "{}[REDACTED]",
                    captures.get(1).map_or("", |capture| capture.as_str())
                ),
                4 => {
                    let raw = captures.get(0).map_or("", |capture| capture.as_str());
                    let trailing = raw
                        .chars()
                        .rev()
                        .take_while(|character| {
                            matches!(character, '.' | ',' | ';' | ')' | ']' | '}')
                        })
                        .count();
                    let (url, suffix) = raw.split_at(raw.len().saturating_sub(trailing));
                    format!("{}{}", redact_url(url), suffix)
                }
                _ => "[REDACTED]".to_owned(),
            })
            .into_owned();
    }
    output
}

fn is_sensitive_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    matches!(
        name.as_str(),
        "token"
            | "api_key"
            | "apikey"
            | "password"
            | "passwd"
            | "secret"
            | "authorization"
            | "access_token"
            | "refresh_token"
            | "id_token"
            | "client_secret"
            | "cookie"
            | "set-cookie"
            | "x-api-key"
    ) || name.ends_with("_token")
        || name.ends_with("_secret")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn secret_debug_output_is_redacted() {
        assert_eq!(format!("{:?}", Secret("private-value")), "[REDACTED]");
    }

    #[test]
    fn url_query_credentials_are_redacted() {
        let redacted = redact_url("https://example.test/items?api_key=private&lang=en");
        let parsed = Url::parse(&redacted).unwrap();
        let pairs: Vec<_> = parsed.query_pairs().collect();
        assert!(
            pairs
                .iter()
                .any(|(key, value)| key == "api_key" && value == "[REDACTED]")
        );
        assert!(
            pairs
                .iter()
                .any(|(key, value)| key == "lang" && value == "en")
        );
        assert!(!redacted.contains("private"));
    }

    #[test]
    fn redact_text_handles_json_authorization_and_inline_secrets() {
        let input =
            r#"Authorization: Bearer private-token "api_key":"inline-secret" password=example"#;
        let redacted = redact_text(input);
        assert!(!redacted.contains("private-token"));
        assert!(!redacted.contains("inline-secret"));
        assert!(!redacted.contains("password=example"));
        assert!(redacted.contains("[REDACTED]"));
    }

    #[test]
    fn text_url_query_credentials_are_redacted_without_hiding_safe_parameters() {
        let redacted = redact_text("request=https://example.test/api?token=private&language=en");
        assert!(!redacted.contains("private"));
        assert!(redacted.contains("language=en"));

        // URL serialization percent-encodes the brackets in the marker. Verify
        // the decoded query value rather than requiring a particular encoding.
        let url_text = redacted
            .split_once("request=")
            .map(|(_, value)| value)
            .expect("redacted request URL should remain present");
        let parsed = Url::parse(url_text).expect("redacted request URL should remain valid");
        let pairs: Vec<_> = parsed.query_pairs().collect();
        assert!(
            pairs
                .iter()
                .any(|(key, value)| key == "token" && value == "[REDACTED]")
        );
        assert!(
            pairs
                .iter()
                .any(|(key, value)| key == "language" && value == "en")
        );
    }

    #[test]
    fn persistent_log_file_and_export_are_bounded_and_redacted() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logs/senzo.log");
        let sink = Arc::new(DiagnosticsSink::open(path.clone()).unwrap());
        let mut writer = DiagnosticsWriter {
            sink: sink.clone(),
            pending: Vec::new(),
        };
        writer.write_all(b"event token=private-token\n").unwrap();
        writer.flush().unwrap();

        let exported = sink.export();
        assert!(exported.contains("SENZO Diagnostic Report"));
        assert!(exported.contains("event token=[REDACTED]"));
        assert!(!exported.contains("private-token"));
        let persisted = fs::read_to_string(path).unwrap();
        assert!(persisted.contains("[REDACTED]"));
        assert!(!persisted.contains("private-token"));
    }

    #[test]
    fn recent_log_buffer_trims_on_utf8_boundaries() {
        let mut value = "é".repeat(10);
        trim_recent(&mut value, 5);
        assert!(value.len() <= 5);
        assert!(value.is_char_boundary(0));
    }
}
