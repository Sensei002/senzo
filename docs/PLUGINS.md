# Plugins

SENZO's **Plugins** tab installs Nuvio-compatible provider repositories. It
replaces the old sidebar entry for Direct Sources; the Direct Sources provider
subsystem remains in the codebase, but it is no longer exposed as a separate
navigation tab.

## Repository format

A repository is an HTTPS URL to a manifest. The manifest lists individual
provider scripts:

```json
{
  "name": "All-in-One-Nuvio",
  "version": "1.0.0",
  "scrapers": [
    {
      "id": "example",
      "name": "Example",
      "description": "Example provider",
      "version": "1.0.0",
      "filename": "providers/example.js",
      "supportedTypes": ["movie", "tv"],
      "enabled": true,
      "hasSettings": false,
      "contentLanguage": ["en"],
      "formats": ["mp4"]
    }
  ]
}
```

The default repository is
`https://raw.githubusercontent.com/D3adlyRocket/All-in-One-Nuvio/refs/heads/main/manifest.json`.

## Behaviour

- Installing a repository fetches and validates its manifest. It never runs a
  provider script during installation.
- Providers are listed with their repository, version, supported types and
  formats. Each provider has its own switch; a provider disabled in its
  manifest cannot be enabled locally.
- Refreshing preserves the user's enabled/disabled choices by stable provider
  key. Removing a repository removes its providers.
- Repository and provider URLs must be public HTTPS hosts. Local/private
  addresses, redirects to private addresses, oversized manifests and
  oversized scripts are rejected.
- A provider **Test** loads the script in the sandbox and verifies that it
  exports a usable `getStreams` function. It does not call `getStreams`.
- Providers with settings expose a JSON field in their row. The JSON is passed
  to the script as `SCRAPER_SETTINGS`.

## Execution

When the details page selects a movie or episode, SENZO resolves a TMDB id,
loads the enabled providers' scripts in parallel (bounded concurrency), and
runs `getStreams(tmdbId, mediaType, season, episode)` in a QuickJS runtime.
Results are merged into the normal stream drawer under the **Plugins** filter
and are tagged with their provider name and a `PLUGIN` badge. Direct URLs use
the existing MPV/streaming-server playback path; `infoHash` results use the
existing torrent path.

The runtime provides the compatibility surface Nuvio providers use: `fetch`,
`AbortController`, timers, `URL`/`URLSearchParams`, base64, `TextEncoder`/
`TextDecoder`, `crypto-js`, a WebCrypto subset, `Buffer`, and a Cheerio-compatible
HTML selector facade backed by `scraper`/`html5ever`. `api.themoviedb.org`
requests are rewritten to SENZO's metadata gateway; the script's placeholder
`TMDB_API_KEY` is never sent and is stripped from the request.

### Sandbox limits

- One QuickJS runtime per provider query, created on a blocking worker thread.
- 128 MB memory limit, 2 MB stack limit, 60-second execution deadline.
- At most eight providers run at once and at most twelve providers are queried
  per title. Response bodies are capped at 12 MB; provider scripts at 4 MB.
- No filesystem, database, credential-store, process, or UI access. Only the
  explicit `__senzo_*` native bridges are exposed.
- Provider HTTP requests must be public HTTP/HTTPS hosts. DNS is checked before
  each request and each redirect; private, loopback, link-local, CGNAT and
  documentation addresses are rejected.
- Provider display fields and headers are sanitized. URLs are limited to
  HTTP/HTTPS or torrent hashes; `javascript:`, `data:` and other schemes are
  discarded.

### Limitations

- Providers that depend on WebAssembly are rejected by this build.
- WebCrypto RSA/ECDSA signatures are not available; HMAC, PBKDF2, AES-CBC,
  AES-GCM and AES-ECB are available.
- Provider scripts that call the unsupported TMDB `/find` gateway route fall
  back to SENZO's title/year resolution where possible.
- Some providers require their own settings JSON; use the provider row editor.

SENZO never hosts provider content and never bypasses logins, paywalls, DRM or
CAPTCHAs. Only enable providers and repositories you trust: a provider has
network access while it runs, even though it cannot read local SENZO data.
