//! Shared HTTP client policy and validated HTTP(S) URL boundary.

use std::time::Duration;

use reqwest::{Client, Response, redirect::Policy};
use thiserror::Error;
use url::Url;

#[derive(Debug, Clone)]
pub struct HttpPolicy {
    pub request_timeout: Duration,
    pub connect_timeout: Duration,
    pub max_redirects: usize,
}

impl Default for HttpPolicy {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(20),
            connect_timeout: Duration::from_secs(7),
            max_redirects: 5,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedHttpUrl(Url);

impl ValidatedHttpUrl {
    pub fn parse(value: &str) -> Result<Self, NetworkError> {
        let url = Url::parse(value).map_err(|_| NetworkError::InvalidUrl)?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(NetworkError::InvalidUrl);
        }
        Ok(Self(url))
    }

    pub fn as_url(&self) -> &Url {
        &self.0
    }
}

#[derive(Debug, Error)]
pub enum NetworkError {
    #[error("only absolute HTTP(S) URLs without embedded credentials are accepted")]
    InvalidUrl,
    #[error("HTTP client configuration failed: {0}")]
    ClientConfiguration(String),
    #[error("HTTP request failed: {0}")]
    Transport(String),
}

#[derive(Clone)]
pub struct HttpClient {
    client: Client,
    no_redirect_client: Client,
}

impl HttpClient {
    pub fn new(policy: HttpPolicy) -> Result<Self, NetworkError> {
        let client = build_client(&policy, Policy::limited(policy.max_redirects))?;
        let no_redirect_client = build_client(&policy, Policy::none())?;
        Ok(Self { client, no_redirect_client })
    }

    pub async fn get(&self, url: &ValidatedHttpUrl) -> Result<Response, NetworkError> {
        self.client
            .get(url.as_url().clone())
            .send()
            .await
            .map_err(|error| NetworkError::Transport(error.without_url().to_string()))
    }

    /// Sends a GET without following redirects. Use for allow-listed resources,
    /// such as artwork, to prevent redirect-based SSRF.
    pub async fn get_without_redirects(
        &self,
        url: &ValidatedHttpUrl,
    ) -> Result<Response, NetworkError> {
        self.no_redirect_client
            .get(url.as_url().clone())
            .send()
            .await
            .map_err(|error| NetworkError::Transport(error.without_url().to_string()))
    }
}

fn build_client(policy: &HttpPolicy, redirect: Policy) -> Result<Client, NetworkError> {
    Client::builder()
        .timeout(policy.request_timeout)
        .connect_timeout(policy.connect_timeout)
        .redirect(redirect)
        .user_agent(concat!("SENZO/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| NetworkError::ClientConfiguration(error.without_url().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_http_and_https_urls() {
        assert!(ValidatedHttpUrl::parse("https://example.test/catalog").is_ok());
        assert!(ValidatedHttpUrl::parse("http://127.0.0.1:11470/health").is_ok());
    }

    #[test]
    fn rejects_unsafe_url_schemes_and_embedded_credentials() {
        assert!(ValidatedHttpUrl::parse("file:///etc/passwd").is_err());
        assert!(ValidatedHttpUrl::parse("javascript:alert(1)").is_err());
        assert!(ValidatedHttpUrl::parse("https://user:pass@example.test/").is_err());
        assert!(ValidatedHttpUrl::parse("/relative/path").is_err());
    }

    #[test]
    fn builds_a_shared_client_with_default_policy() {
        assert!(HttpClient::new(HttpPolicy::default()).is_ok());
    }
}
