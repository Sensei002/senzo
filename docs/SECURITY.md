# SENZO security and privacy model

## Purpose
SENZO is a local-first desktop media application. Its security design must preserve local user data, protect credentials, and treat all external metadata, add-ons, network responses, and playable URLs as untrusted.

This document describes requirements and planned controls. A control is not implemented merely because it is documented.

## Data and privacy

- SENZO does not require a SENZO account, profile, or cloud history service.
- Settings, watch history, playback progress, and installed add-on records are stored locally.
- On Windows, the current SQLite database location is `%LOCALAPPDATA%\SENZO\senzo.db`.
- Diagnostic logs are stored separately under `%LOCALAPPDATA%\SENZO\logs\senzo.log`.
- The logger currently rotates the active log at 2 MiB and exports at most 1 MiB from the previous 72 hours.
- Diagnostic exports do not include SQLite database contents or watch-history records.
- Redaction of common credential-like fields is best-effort, not a guarantee. Never pass credentials, private manifest URLs, personal media activity, or sensitive query parameters to the logger.
- Future metadata and artwork caches must have size limits and eviction rules.
- Cache cleanup must not delete watch history, settings, or add-on configuration.
- Any future telemetry must be opt-in, documented, and designed to avoid collecting viewing history or personal media activity by default.

## Secrets and credentials

- Never commit TMDB tokens, provider credentials, signing keys, updater secrets, or Discord secrets.
- Never log authorization headers, API tokens, passwords, private manifest URLs containing credentials, or sensitive query parameters.
- Official build secrets must be injected through protected GitHub Actions configuration and must not be exposed in logs or public artifacts.
- Do not assume a client-embedded API key is confidential; desktop binaries can be inspected. Use provider-approved restrictions and rotate compromised credentials.
- User-supplied provider credentials, if supported, should be stored using an appropriate OS credential facility rather than plaintext settings.
- Signing keys must be kept outside the repository and CI logs.

## Add-on trust boundary

An add-on manifest URL is user-supplied remote input. A manifest does not become trusted merely because the user installs it.

Before production release, the add-on networking layer must:

- Require HTTPS by default; any exception must be explicit and clearly warned.
- Reject malformed URLs and unsupported schemes.
- Limit response size, redirects, request duration, and JSON nesting/shape.
- Validate manifest identity, resources, supported types, and response schemas.
- Avoid sending SENZO credentials to add-on servers.
- Validate returned stream and subtitle URLs before handing them to playback.
- Handle redirects deliberately and revalidate the final destination.
- Protect against requests to localhost, private, link-local, and other sensitive network ranges where appropriate; consider DNS rebinding and redirect-based bypasses.
- Avoid exposing local file paths or private application state to remote providers.
- Query only add-ons explicitly installed and enabled by the user.

Torrentio is not built in, preinstalled, or queried automatically. It can only be used if the user chooses to add a configured compatible manifest URL.

## Metadata and artwork

TMDB and other metadata endpoints are external services. Validate responses, handle unavailable services, apply timeouts, and avoid trusting remote text as executable content. Display required attribution. Do not let metadata content inject UI markup or commands.

## Playback and local resources

- Treat provider-returned media URLs and subtitle URLs as untrusted.
- Do not execute downloaded media or subtitle files as programs.
- Keep playback-engine processes and arguments constrained.
- Validate local paths and avoid path traversal when reading or writing cache files.
- Do not expose local files to an add-on or network service without explicit user action.

## Updates and release integrity

The future updater must verify signed update metadata and artifact integrity before installation. Downloads should be staged, installation should be recoverable, and rollback behavior should be tested. Do not execute an update solely because a remote endpoint returned a URL.

## Reporting a vulnerability

Until a dedicated private security contact is published, do not include exploit details or secrets in public issues. Report only a minimal description publicly and request a private reporting channel. A SECURITY.md contact process should be added before public stable releases.

## Status

This document is a design and implementation checklist. Many controls above are planned and must be implemented and tested before SENZO is considered production-ready.
