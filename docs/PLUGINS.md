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
- A provider **Test** fetches the script and verifies that it exposes a
  `getStreams` entry point. It does not execute downloaded JavaScript.

## Status

The repository manager, manifest validation, provider list, persistence and
toggles are implemented. Executing Nuvio JavaScript providers and merging their
results into the stream drawer still requires a sandboxed JavaScript runtime;
until that is wired, the manager does not claim provider results.

Provider results, when implemented, belong in the provider-grouped stream
drawer alongside Torrentio and Stremio add-ons. SENZO never hosts provider
content and never bypasses logins, paywalls, DRM or CAPTCHAs.
