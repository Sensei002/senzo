//! Structured diagnostics helpers. Never format raw credentials into log fields.

use std::{error::Error, fmt};

use tracing_subscriber::EnvFilter;
use url::Url;

pub struct Secret<'a>(pub &'a str);

impl fmt::Debug for Secret<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

pub fn init() -> Result<(), Box<dyn Error + Send + Sync>> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_thread_ids(true)
        .compact()
        .try_init()?;
    Ok(())
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
        url.query_pairs_mut()
            .extend_pairs(pairs.iter().map(|(key, value)| (key.as_str(), value.as_str())));
    }
    url.to_string()
}

pub fn redact_text(value: &str) -> String {
    let mut output = Vec::new();
    let mut redact_next = false;
    for part in value.split_whitespace() {
        if redact_next {
            output.push("[REDACTED]".to_owned());
            redact_next = false;
            continue;
        }
        if part.eq_ignore_ascii_case("bearer") {
            output.push("Bearer".to_owned());
            redact_next = true;
            continue;
        }
        if let Some((key, _)) = part.split_once('=') {
            if is_sensitive_name(key.trim_end_matches(':')) {
                output.push(format!("{key}=[REDACTED]"));
                continue;
            }
        }
        output.push(part.to_owned());
    }
    output.join(" ")
}

fn is_sensitive_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    matches!(
        name.as_str(),
        "token" | "api_key" | "apikey" | "password" | "secret" | "authorization"
            | "access_token" | "refresh_token" | "client_secret"
    ) || name.ends_with("_token") || name.ends_with("_secret")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_debug_output_is_redacted() {
        assert_eq!(format!("{:?}", Secret("private-value")), "[REDACTED]");
    }

    #[test]
    fn url_query_credentials_are_redacted() {
        let redacted = redact_url("https://example.test/items?api_key=private&lang=en");
        let parsed = Url::parse(&redacted).unwrap();
        let pairs: Vec<_> = parsed.query_pairs().collect();
        assert!(pairs.iter().any(|(key, value)| key == "api_key" && value == "[REDACTED]"));
        assert!(pairs.iter().any(|(key, value)| key == "lang" && value == "en"));
        assert!(!redacted.contains("private"));
    }

    #[test]
    fn bearer_and_inline_credentials_are_redacted() {
        assert_eq!(
            redact_text("Authorization: Bearer private-token api_key=private"),
            "Authorization: Bearer [REDACTED] api_key=[REDACTED]"
        );
    }
}
