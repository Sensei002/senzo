//! Blocking HTTP bridge for provider scripts.
//!
//! Provider scripts run on a dedicated worker thread, so a blocking client is
//! simpler than driving an async runtime from inside QuickJS. Every redirect
//! is resolved and checked manually so a public URL cannot bounce into the
//! local network. Response bodies and scripts are size-capped.

use std::{
    collections::HashMap,
    io::Read,
    net::ToSocketAddrs,
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use reqwest::{
    Url,
    blocking::{Client, Response},
    header::{HeaderName, HeaderValue, LOCATION},
    redirect::Policy,
};
use serde::{Deserialize, Serialize};

use crate::direct_sources::net::{is_blocked_host, is_blocked_ip};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_REDIRECTS: usize = 6;
const MAX_RESPONSE_BYTES: usize = 12 * 1024 * 1024;
const USER_AGENT: &str = "SENZO-PluginRuntime/1.0 (+https://github.com/Sensei002/senzo)";
const GATEWAY_ORIGIN: &str = "https://senzo-api.vercel.app";
const TMDB_HOST: &str = "api.themoviedb.org";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct FetchRequest {
    pub url: String,
    #[serde(default)]
    pub method: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub body_kind: String,
    #[serde(default)]
    pub body: String,
    #[serde(default = "default_true")]
    pub follow_redirects: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct FetchResponse {
    pub ok: bool,
    pub status: u16,
    pub status_text: String,
    pub url: String,
    pub redirected: bool,
    pub headers: HashMap<String, String>,
    pub body: String,
    pub body_base64: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl FetchResponse {
    fn error(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            status: 0,
            status_text: String::new(),
            url: String::new(),
            redirected: false,
            headers: HashMap::new(),
            body: String::new(),
            body_base64: String::new(),
            error: Some(message.into()),
        }
    }
}

pub(super) struct SafeHttp {
    client: Client,
}

impl SafeHttp {
    pub(super) fn new() -> Self {
        let client = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .redirect(Policy::none())
            .cookie_store(true)
            .user_agent(USER_AGENT)
            .build()
            .unwrap_or_else(|_| Client::new());
        Self { client }
    }

    pub(super) fn fetch(&self, request: FetchRequest) -> FetchResponse {
        let mut url = match Url::parse(request.url.trim()) {
            Ok(url) => url,
            Err(_) => return FetchResponse::error("The provider requested an invalid URL."),
        };
        if url.scheme() != "https" && url.scheme() != "http" {
            return FetchResponse::error("Provider requests must use HTTP or HTTPS.");
        }
        if url.host_str().is_none() || !url.username().is_empty() {
            return FetchResponse::error("The provider requested an invalid URL.");
        }
        if is_blocked_host(&url) {
            return FetchResponse::error("Provider requests to private hosts are blocked.");
        }

        let method = if request.method.trim().is_empty() {
            "GET".to_owned()
        } else {
            request.method.trim().to_ascii_uppercase()
        };
        if !matches!(method.as_str(), "GET" | "POST" | "PUT" | "DELETE" | "PATCH" | "HEAD") {
            return FetchResponse::error("That HTTP method is not allowed.");
        }

        let body = match request.body_kind.as_str() {
            "base64" => match BASE64.decode(request.body.as_bytes()) {
                Ok(bytes) => Some(bytes),
                Err(_) => return FetchResponse::error("The provider sent an invalid request body."),
            },
            "text" => Some(request.body.into_bytes()),
            _ => None,
        };
        let mut redirected = false;

        for hop in 0..=MAX_REDIRECTS {
            let mut target = url.clone();
            let gateway_rewritten = rewrite_tmdb_url(&mut target);
            if is_blocked_host(&target) {
                return FetchResponse::error("Provider requests to private hosts are blocked.");
            }
            if !dns_is_public(&target) {
                return FetchResponse::error("Provider requests to private addresses are blocked.");
            }

            let mut builder = self.client.request(
                reqwest::Method::from_bytes(method.as_bytes()).unwrap_or(reqwest::Method::GET),
                target.clone(),
            );
            for (name, value) in &request.headers {
                let lower = name.to_ascii_lowercase();
                if gateway_rewritten && (lower == "authorization" || lower == "x-api-key") {
                    continue;
                }
                if matches!(lower.as_str(), "host" | "content-length" | "connection") {
                    continue;
                }
                let Ok(name) = HeaderName::from_bytes(lower.as_bytes()) else {
                    continue;
                };
                let Ok(value) = HeaderValue::from_str(value) else {
                    continue;
                };
                builder = builder.header(name, value);
            }
            if let Some(bytes) = body.as_ref() {
                builder = builder.body(bytes.clone());
            }

            let response = match builder.send() {
                Ok(response) => response,
                Err(error) => {
                    let message = if error.is_timeout() {
                        "The provider request timed out."
                    } else if error.is_connect() {
                        "The provider host could not be reached."
                    } else {
                        "The provider request failed."
                    };
                    return FetchResponse::error(message);
                }
            };

            if response.status().is_redirection() {
                if !request.follow_redirects {
                    return self.read_response(response, target.as_str(), redirected);
                }
                let Some(location) = response
                    .headers()
                    .get(LOCATION)
                    .and_then(|value| value.to_str().ok())
                else {
                    return FetchResponse::error("The provider redirect was missing its target.");
                };
                let next = match target.join(location) {
                    Ok(next) => next,
                    Err(_) => return FetchResponse::error("The provider redirect was invalid."),
                };
                if next.scheme() != "https" && next.scheme() != "http" {
                    return FetchResponse::error("The provider redirected to an unsupported URL.");
                }
                if is_blocked_host(&next) || !dns_is_public(&next) {
                    return FetchResponse::error("The provider redirected to a private address.");
                }
                url = next;
                redirected = true;
                if hop == MAX_REDIRECTS {
                    return FetchResponse::error("The provider redirected too many times.");
                }
                continue;
            }

            return self.read_response(response, target.as_str(), redirected);
        }

        FetchResponse::error("The provider redirected too many times.")
    }

    fn read_response(&self, response: Response, final_url: &str, redirected: bool) -> FetchResponse {
        let status = response.status();
        let mut headers = HashMap::new();
        for (name, value) in response.headers() {
            if let Ok(value) = value.to_str() {
                headers.insert(name.as_str().to_ascii_lowercase(), value.to_owned());
            }
        }
        if let Some(length) = response
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<usize>().ok())
            && length > MAX_RESPONSE_BYTES
        {
            return FetchResponse::error("The provider response is larger than the allowed limit.");
        }
        let mut bytes = Vec::new();
        let mut limited = response.take((MAX_RESPONSE_BYTES + 1) as u64);
        if limited.read_to_end(&mut bytes).is_err() {
            return FetchResponse::error("Could not read the provider response.");
        }
        if bytes.len() > MAX_RESPONSE_BYTES {
            return FetchResponse::error("The provider response is larger than the allowed limit.");
        }
        FetchResponse {
            ok: status.is_success(),
            status: status.as_u16(),
            status_text: status.canonical_reason().unwrap_or_default().to_owned(),
            url: final_url.to_owned(),
            redirected,
            headers,
            body: String::from_utf8_lossy(&bytes).into_owned(),
            body_base64: BASE64.encode(&bytes),
            error: None,
        }
    }
}

pub(crate) fn is_public_url(url: &Url) -> bool {
    !is_blocked_host(url) && dns_is_public(url)
}

fn dns_is_public(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let port = url.port_or_known_default().unwrap_or(443);
    let Ok(addresses) = (host, port).to_socket_addrs() else {
        return false;
    };
    let mut found = false;
    for address in addresses {
        found = true;
        if is_blocked_ip(address.ip()) {
            return false;
        }
    }
    found
}

/// Metadata-only rewrite. Provider scripts assume a real TMDB key; SENZO's
/// gateway owns the key server-side. Stream endpoints are never routed here.
fn rewrite_tmdb_url(url: &mut Url) -> bool {
    if url.host_str() != Some(TMDB_HOST) {
        return false;
    }
    let path = url.path().to_owned();
    let Some(rest) = path.strip_prefix("/3") else {
        return false;
    };
    if !rest.is_empty() && !rest.starts_with('/') {
        return false;
    }
    let gateway_path = format!("/api/tmdb{rest}");
    let Ok(mut gateway) = Url::parse(GATEWAY_ORIGIN) else {
        return false;
    };
    gateway.set_path(&gateway_path);
    let retained = url
        .query_pairs()
        .filter(|(key, _)| key != "api_key" && key != "append_to_response")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    gateway.query_pairs_mut().extend_pairs(retained);
    *url = gateway;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_requests_use_the_script_wire_format() {
        let request: FetchRequest = serde_json::from_str(
            r#"{"url":"https://example.com","bodyKind":"text","body":"a=1","followRedirects":false}"#,
        )
        .unwrap();
        assert_eq!(request.url, "https://example.com");
        assert_eq!(request.body_kind, "text");
        assert_eq!(request.body, "a=1");
        assert!(!request.follow_redirects);
    }

    #[test]
    fn tmdb_requests_are_rewritten_to_the_gateway() {
        let mut url = Url::parse(
            "https://api.themoviedb.org/3/movie/550?api_key=secret&language=en-US",
        )
        .unwrap();
        assert!(rewrite_tmdb_url(&mut url));
        assert_eq!(url.host_str(), Some("senzo-api.vercel.app"));
        assert_eq!(url.path(), "/api/tmdb/movie/550");
        assert_eq!(url.query(), Some("language=en-US"));
    }

    #[test]
    fn non_tmdb_requests_are_untouched() {
        let mut url = Url::parse("https://example.com/3/movie/550").unwrap();
        assert!(!rewrite_tmdb_url(&mut url));
    }

    #[test]
    fn private_hosts_are_blocked() {
        let mut url = Url::parse("http://127.0.0.1:8080/").unwrap();
        assert!(is_blocked_host(&url));
        url = Url::parse("http://[::1]/").unwrap();
        assert!(is_blocked_host(&url));
    }
}
