//! Network access for Direct Sources.
//!
//! Every request a provider makes goes through this type. Provider URLs are
//! untrusted input: they must be HTTPS, must not resolve to a private or
//! link-local address, redirects are re-checked, responses are size-capped and
//! every request has a deadline. Query strings are never logged because
//! authorized M3U and JSON feeds commonly carry access tokens there.

use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    time::Duration,
};

use reqwest::{
    Client, Url,
    header::{ACCEPT, CONTENT_LENGTH, CONTENT_TYPE, LOCATION},
    redirect::Policy,
};
use thiserror::Error;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_REDIRECTS: usize = 4;
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const USER_AGENT: &str = "SENZO-DirectSources/1.0 (+https://github.com/Sensei002/senzo)";

#[derive(Debug, Error)]
pub enum NetError {
    #[error("only HTTPS sources are allowed")]
    NotHttps,
    #[error("the source URL is not valid")]
    InvalidUrl,
    #[error("private, loopback, and link-local addresses are not allowed")]
    PrivateAddress,
    #[error("the source did not respond in time")]
    Timeout,
    #[error("the source returned HTTP {0}")]
    Status(u16),
    #[error("the source response is larger than the allowed limit")]
    TooLarge,
    #[error("the source returned an unsupported content type")]
    ContentType,
    #[error("the source could not be reached")]
    Request,
    #[error("the source redirected to an unsafe address")]
    UnsafeRedirect,
}

impl From<reqwest::Error> for NetError {
    fn from(error: reqwest::Error) -> Self {
        if error.is_timeout() {
            Self::Timeout
        } else {
            Self::Request
        }
    }
}

#[derive(Clone)]
pub struct SafeHttp {
    client: Client,
}

pub struct FetchedText {
    pub body: String,
    pub final_url: Url,
    pub content_type: String,
}

impl SafeHttp {
    pub fn new() -> Self {
        let client = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .redirect(Policy::none())
            .user_agent(USER_AGENT)
            .no_proxy()
            .build()
            .unwrap_or_else(|_| Client::new());

        Self { client }
    }

    /// Cheap, synchronous checks used before a provider is saved.
    pub fn validate_url(url: &str) -> Result<Url, NetError> {
        let parsed = Url::parse(url).map_err(|_| NetError::InvalidUrl)?;
        if parsed.scheme() != "https" {
            return Err(NetError::NotHttps);
        }
        if parsed.host_str().is_none() || !parsed.username().is_empty() {
            return Err(NetError::InvalidUrl);
        }
        if parsed.port() == Some(0) || is_blocked_host(&parsed) {
            return Err(NetError::PrivateAddress);
        }
        Ok(parsed)
    }

    /// Resolve the host and reject any address that could reach the local
    /// network. DNS is re-checked for every request and for every redirect.
    pub async fn validate_public_url(url: &str) -> Result<Url, NetError> {
        let parsed = Self::validate_url(url)?;
        let host = parsed.host_str().ok_or(NetError::InvalidUrl)?;
        let port = parsed.port_or_known_default().unwrap_or(443);
        let addresses = tokio::net::lookup_host((host, port))
            .await
            .map_err(|_| NetError::Request)?;
        let mut found = false;
        for address in addresses {
            found = true;
            if is_blocked_ip(address.ip()) {
                return Err(NetError::PrivateAddress);
            }
        }
        if !found {
            return Err(NetError::Request);
        }
        Ok(parsed)
    }

    /// Fetch a small textual document (catalog, manifest, M3U, feed).
    pub async fn get_text(&self, url: &str, max_bytes: usize) -> Result<FetchedText, NetError> {
        let mut checked = Self::validate_public_url(url).await?;
        for _ in 0..=MAX_REDIRECTS {
            let response = tokio::time::timeout(
                REQUEST_TIMEOUT,
                self.client
                    .get(checked.clone())
                    .header(ACCEPT, "text/plain, application/json, application/xml, text/xml, application/vnd.apple.mpegurl, audio/mpegurl, */*;q=0.5")
                    .send(),
            )
            .await
            .map_err(|_| NetError::Timeout)??;
            let status = response.status();
            if status.is_redirection() {
                let location = response
                    .headers()
                    .get(LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or(NetError::UnsafeRedirect)?;
                let next = checked
                    .join(location)
                    .map_err(|_| NetError::UnsafeRedirect)?;
                // Follow one hop manually so DNS is resolved and checked before
                // the request, not after a client-level redirect already fired.
                checked = Self::validate_public_url(next.as_str()).await?;
                continue;
            }
            if !status.is_success() {
                return Err(NetError::Status(status.as_u16()));
            }
            let final_url = checked;

            if let Some(length) = response
                .headers()
                .get(CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<usize>().ok())
                && length > max_bytes.min(MAX_RESPONSE_BYTES)
            {
                return Err(NetError::TooLarge);
            }
            let content_type = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("")
                .to_ascii_lowercase();
            if !content_type.is_empty()
                && !content_type.starts_with("text/")
                && !content_type.contains("json")
                && !content_type.contains("xml")
                && !content_type.contains("mpegurl")
                && !content_type.contains("octet-stream")
            {
                return Err(NetError::ContentType);
            }

            let limit = max_bytes.min(MAX_RESPONSE_BYTES);
            let mut body = Vec::new();
            let mut response = response;
            while let Some(chunk) = response.chunk().await? {
                if body.len() + chunk.len() > limit {
                    return Err(NetError::TooLarge);
                }
                body.extend_from_slice(&chunk);
            }

            return Ok(FetchedText {
                body: String::from_utf8_lossy(&body).into_owned(),
                final_url,
                content_type,
            });
        }
        Err(NetError::UnsafeRedirect)
    }

    /// Fetch JSON and deserialize it in one step. The body is never logged.
    pub async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        max_bytes: usize,
    ) -> Result<T, NetError> {
        let fetched = self.get_text(url, max_bytes).await?;
        serde_json::from_str(&fetched.body).map_err(|_| NetError::ContentType)
    }
}

impl Default for SafeHttp {
    fn default() -> Self {
        Self::new()
    }
}

/// A label safe to put in logs or the UI: origin and path, never the query.
pub fn display_url(url: &str) -> String {
    let Ok(parsed) = Url::parse(url) else {
        return "invalid source".to_owned();
    };
    let host = parsed.host_str().unwrap_or("unknown");
    match parsed.port() {
        Some(port) => format!("{}://{}:{}{}", parsed.scheme(), host, port, parsed.path()),
        None => format!("{}://{}{}", parsed.scheme(), host, parsed.path()),
    }
}

pub fn is_blocked_host(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return true;
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty()
        || host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
        || host.ends_with(".home.arpa")
    {
        return true;
    }
    if let Ok(address) = host.parse::<IpAddr>() {
        return is_blocked_ip(address);
    }
    false
}

pub fn is_blocked_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_blocked_v4(address),
        IpAddr::V6(address) => is_blocked_v6(address),
    }
}

fn is_blocked_v4(address: Ipv4Addr) -> bool {
    let octets = address.octets();
    address.is_private()
        || address.is_loopback()
        || address.is_link_local()
        || address.is_broadcast()
        || address.is_documentation()
        || address.is_unspecified()
        || address.is_multicast()
        || octets[0] == 0
        || (octets[0] == 100 && (64..128).contains(&octets[1]))
        || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
        || (octets[0] == 198 && (18..20).contains(&octets[1]))
        || (octets[0] == 198 && octets[1] == 51 && octets[2] == 100)
        || (octets[0] == 203 && octets[1] == 0 && octets[2] == 113)
}

fn is_blocked_v6(address: Ipv6Addr) -> bool {
    address.is_loopback()
        || address.is_unspecified()
        || address.is_multicast()
        || address.is_unique_local()
        || address.is_unicast_link_local()
        || (address.segments()[0] & 0xffc0) == 0xfe80
        || (address.segments()[0] & 0xff00) == 0xff00
        || address.to_ipv4_mapped().is_some_and(is_blocked_v4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_plain_http() {
        assert!(matches!(
            SafeHttp::validate_url("http://example.com/feed.json"),
            Err(NetError::NotHttps)
        ));
    }

    #[test]
    fn rejects_loopback_and_private_hosts() {
        for url in [
            "https://127.0.0.1/feed",
            "https://10.0.0.2/feed",
            "https://192.168.1.10/feed",
            "https://172.16.0.1/feed",
            "https://169.254.169.254/latest/meta-data",
            "https://[::1]/feed",
            "https://localhost/feed",
            "https://router.local/feed",
        ] {
            assert!(
                SafeHttp::validate_url(url).is_err(),
                "{url} should be blocked"
            );
        }
    }

    #[test]
    fn accepts_public_https() {
        assert!(SafeHttp::validate_url("https://example.com/feed.json").is_ok());
    }

    #[test]
    fn display_url_drops_the_query() {
        assert_eq!(
            display_url("https://example.com/list.m3u?token=secret"),
            "https://example.com/list.m3u"
        );
    }
}
