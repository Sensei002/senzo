//! Secure, bounded image loading for TMDB artwork in the native GPUI image cache.

use std::time::Duration;

use anyhow::{Context, bail};
use futures::future::BoxFuture;
use gpui::http_client::{HttpClient, HttpResponse};
use http::StatusCode;
use reqwest::{Client, redirect::Policy};
use url::Url;

const MAX_ARTWORK_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone)]
pub struct TmdbArtworkHttpClient {
    client: Client,
}

impl TmdbArtworkHttpClient {
    pub fn new() -> anyhow::Result<Self> {
        let client = Client::builder()
            .user_agent(concat!("SENZO/", env!("CARGO_PKG_VERSION")))
            .https_only(true)
            .redirect(Policy::none())
            .timeout(Duration::from_secs(15))
            .build()
            .context("could not initialize the artwork HTTP client")?;
        Ok(Self { client })
    }
}

impl HttpClient for TmdbArtworkHttpClient {
    fn get(
        &self,
        url: &str,
        _follow_redirects: bool,
    ) -> BoxFuture<'static, anyhow::Result<HttpResponse>> {
        let client = self.client.clone();
        let url = match validate_artwork_url(url) {
            Ok(url) => url,
            Err(error) => return Box::pin(async move { Err(error) }),
        };

        Box::pin(async move {
            let response = client
                .get(url)
                .send()
                .await
                .context("TMDB artwork request failed")?;
            let status = StatusCode::from_u16(response.status().as_u16())
                .context("TMDB artwork returned an invalid HTTP status")?;
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
            if body.len() > MAX_ARTWORK_BYTES {
                bail!("TMDB artwork exceeds the 8 MiB safety limit");
            }
            Ok(HttpResponse {
                status,
                body: body.to_vec(),
            })
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
