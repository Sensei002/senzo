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
- User-supplied OpenSubtitles API credentials are stored through the Windows Credential Manager adapter (`src/subtitles/credentials.rs`), not in SQLite, settings JSON, logs, or support bundles. The application UI for entering/removing the key is still pending. Other provider credentials must follow the same OS-backed approach.
- Signing keys must be kept outside the repository and CI logs.

## Add-on trust boundary

An add-on manifest URL is user-supplied remote input. A manifest does not become trusted merely because the user installs it.

Before production release, the add-on networking layer must:

- Require HTTPS by default; any exception must be explicit and clearly warned.
- Reject malformed URLs and unsupported schemes.
- Limit response size and request duration; redirects are currently disabled for manifest requests.
- Reject obvious localhost/private/reserved IP literals and local hostname suffixes before manifest requests.
- Complete DNS-resolution-aware private-address blocking and revalidation against DNS rebinding before production release.
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

## Torrent transport and temporary data

- Accept torrent identifiers only from an explicitly selected stream returned by a user-installed add-on. Validate 40-character BitTorrent v1 info hashes before constructing a magnet URI; v2-only hashes are not supported yet.
- Keep torrent engine state and payloads in a dedicated session directory under `%LOCALAPPDATA%\\SENZO\\torrent-temp`; never reuse metadata, subtitle, or application cache directories.
- Bind any local streaming HTTP endpoint to loopback only. Do not expose the torrent HTTP API to the LAN.
- Do not persist torrent session state across application launches. A new playback session must not silently resume downloading or seeding from a prior run.
- On normal exit, stop MPV, stop and terminate the torrent engine, then remove the session directory. On startup, remove abandoned session directories from prior crashes; do not follow symlinks or delete unrelated directories.
- Keep playback progress and watch history in SQLite independently of temporary payload files.
- Never create a permanent-download or background-seeding task as part of playback. Any future permanent download capability must be a separate explicit opt-in feature.


## Implemented torrent transport safeguards

The current Windows implementation embeds the pinned librqbit engine with upload/seeding disabled, persistence and fast-resume disabled, and local service discovery disabled. Only user-selected torrent results from explicitly installed add-ons enter the transport. A random token protects the HTTP stream path, which binds to IPv4 loopback on an ephemeral port and supports byte ranges. MPV's general provider URL validation remains strict; a separate narrow validator permits only SENZO's tokenized loopback path. The application owns a dedicated torrent runtime for the life of the playback session, waits for MPV to acknowledge stopping before torrent teardown, deletes temporary payloads during orderly shutdown, and recovers abandoned session directories on next launch. Current info-hash support is BitTorrent v1 only. The Windows CI package statically includes the engine in the SENZO executable and bundles its Apache-2.0 notice alongside the MPV runtime notices. Real-world peer discovery, buffering, seek behavior, and playback still require manual validation.
