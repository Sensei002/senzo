//! Secure, bounded image loading for TMDB artwork in the native GPUI image cache.

use std::{sync::Arc, time::Duration};

use anyhow::{Context, bail};
use futures::{channel::oneshot, future::BoxFuture};
use gpui::http_client::{HttpClient, HttpResponse};
use http::StatusCode;
use reqwest::{Client, redirect::Policy};
use tokio::runtime::Runtime;
use url::Url;

const MAX_ARTWORK_BYTES: usize = 8 * 1024 * 1024;

/// GPUI's executor is not a Tokio executor. Keep a small Tokio runtime alive and
/// dispatch reqwest work onto it; awaiting reqwest directly from GPUI can panic
/// because reqwest requires a Tokio reactor while GPUI polls its own futures.
#[derive(Clone)]
pub struct TmdbArtworkHttpClient {
    client: Client,
    runtime: Arc<Runtime>,
}

impl TmdbArtworkHttpClient {
    pub fn new() -> anyhow::Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("senzo-artwork-http")
            .enable_all()
            .build()
            .context("could not initialize the artwork Tokio runtime")?;
        let client = Client::builder()
            .user_agent(concat!("SENZO/", env!("CARGO_PKG_VERSION")))
            .https_only(true)
            .redirect(Policy::none())
            .timeout(Duration::from_secs(15))
            .build()
            .context("could not initialize the artwork HTTP client")?;
        Ok(Self {
            client,
            runtime: Arc::new(runtime),
        })
    }
}

impl HttpClient for TmdbArtworkHttpClient {
    fn get(
        &self,
        url: &str,
        _follow_redirects: bool,
    ) -> BoxFuture<'static, anyhow::Result<HttpResponse>> {
        let client = self.client.clone();
        let runtime = self.runtime.clone();
        let url = match validate_artwork_url(url) {
            Ok(url) => url,
            Err(error) => return Box::pin(async move { Err(error) }),
        };

        Box::pin(async move {
            let (sender, receiver) = oneshot::channel();
            runtime.spawn(async move {
                let result = async {
                    if let Some(body) = crate::cache::load_artwork_bytes(url.as_str())? {
                        return Ok(HttpResponse {
                            status: StatusCode::OK,
                            body,
                        });
                    }

                    let response = client
                        .get(url.clone())
                        .send()
                        .await
                        .with_context(|| format!("TMDB artwork request failed for {}", url))?;
                    let status = StatusCode::from_u16(response.status().as_u16())
                        .context("TMDB artwork returned an invalid HTTP status")?;
                    if !status.is_success() {
                        bail!("TMDB artwork server returned HTTP {status} for {url}");
                    }
                    if response
                        .content_length()
                        .is_some_and(|length| length > MAX_ARTWORK_BYTES as u64)
                    {
                        bail!("TMDB artwork exceeds the 8 MiB safety limit");
                    }
                    let body = response
                        .bytes()
                        .await
                        .context("could not read TMDB artwork")?;
                    if body.is_empty() || body.len() > MAX_ARTWORK_BYTES {
                        bail!("TMDB artwork is empty or exceeds the 8 MiB safety limit");
                    }

                    if let Err(error) = crate::cache::store_artwork_bytes(url.as_str(), &body) {
                        crate::diagnostics::record(
                            crate::diagnostics::Level::Warn,
                            "artwork",
                            &format!("could not cache downloaded TMDB artwork: {error:#}"),
                        );
                    }
                    crate::diagnostics::record(
                        crate::diagnostics::Level::Info,
                        "artwork",
                        &format!("TMDB artwork downloaded successfully ({})", body.len()),
                    );
                    Ok(HttpResponse {
                        status,
                        body: body.to_vec(),
                    })
                }
                .await;
                if let Err(error) = &result {
                    crate::diagnostics::record(
                        crate::diagnostics::Level::Error,
                        "artwork",
                        &format!("{error:#}"),
                    );
                }
                let _ = sender.send(result);
            });

            receiver
                .await
                .context("artwork HTTP worker stopped before returning a response")?
        })
    }
}

fn validate_artwork_url(input: &str) -> anyhow::Result<Url> {
    let url = Url::parse(input).context("TMDB artwork URL is invalid")?;
    if url.scheme() != "https" || url.host_str() != Some("image.tmdb.org") || url.port().is_some() {
        bail!("artwork requests are restricted to the official TMDB image host");
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artwork_urls_are_restricted_to_the_official_https_host() {
        assert!(validate_artwork_url("https://image.tmdb.org/t/p/w500/poster.jpg").is_ok());
        assert!(validate_artwork_url("http://image.tmdb.org/poster.jpg").is_err());
        assert!(validate_artwork_url("https://example.com/poster.jpg").is_err());
        assert!(validate_artwork_url("https://image.tmdb.org.evil.example/poster.jpg").is_err());
        assert!(validate_artwork_url("https://image.tmdb.org:8443/poster.jpg").is_err());
    }
}
