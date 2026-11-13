# SENZO architecture

## 1. Product boundary

SENZO is a native desktop media center built with Rust + GPUI. Windows is the only active implementation and CI target. Linux is deliberately deferred until the Windows application is established. Android and iOS are out of scope. SENZO is not an Electron, Tauri, WebView2, or egui application.

SENZO is **accountless by design**. There is no SENZO sign-up, login, cloud profile, or required remote watch-history service. User settings, installed add-ons, watch history, and resume positions are stored in the local SENZO application-data directory.

### SENZO owns

- Native desktop interface, navigation, and application state.
- TMDB-backed movie and television discovery and metadata presentation.
- User-managed registry of compatible add-ons.
- Source selection and handoff to playback.
- Local settings, watch history, and resume progress.
- Playback controls and optional Discord Rich Presence.
- Local diagnostic event logging and user-initiated support-bundle export.

### SENZO does not promise

- That any third-party source is available or lawful in every region.
- That every third-party add-on or provider-specific extension is compatible.
- A SENZO-hosted account, cloud sync, or remote history service.
- A built-in catalog of torrent sources or preinstalled Torrentio integration.

## 2. Metadata and discovery

- **Primary provider:** The Movie Database (TMDB) for movie and television metadata.
- **Search:** Search movies or TV shows and retain the TMDB ID as stable internal identity.
- **TV navigation:** Fetch typed season responses (including episode lists) and individual episode details. Season zero is supported for specials; episode numbers must be positive.
- **External IDs:** Resolve movie and TV external identifiers, including IMDb and TVDB IDs, through dedicated TMDB endpoints for source-provider mapping.
- **Enrichment:** Retrieve details, credits, videos, images, and external IDs with `append_to_response` where supported. TV season and episode information uses the relevant season/episode endpoints.
- **Artwork:** Convert TMDB poster, backdrop, logo, and episode-still paths into image CDN URLs. Cache artwork locally with bounded storage and eviction.
- **Source identifiers:** Map TMDB IDs to IMDb or other identifiers when an add-on requires them. Never assume every provider accepts TMDB IDs.
- **Attribution:** Include the TMDB logo and the required notice in About/Credits: "This product uses the TMDB API but is not endorsed or certified by TMDB."
- **Credential model:** A TMDB API Read Access Token is an application integration credential, not a SENZO user identity. Official builds may receive it through protected CI configuration. It must not be committed to source control, printed in logs, or exposed through diagnostic output. The approach for self-built binaries must be documented before public distribution; possible options include a user-supplied token or building without metadata access.

## 3. Add-ons and streaming sources

SENZO uses a user-managed registry for add-ons that implement the supported Stremio HTTP protocol.

### Installation and management

- The Add-ons screen will accept a user-entered manifest URL.
- SENZO fetches and validates the add-on manifest and records its identity, version, capabilities, URL, and manifest data locally.
- The UI will support enable, disable, remove, and reorder.
- Requests are sent only to add-ons explicitly installed and enabled by the user.
- No streaming add-on is bundled or preinstalled. **Torrentio is not integrated into the default source list.** Users may manually add their own configured Torrentio manifest URL or another compatible provider.
- External add-ons may have their own configuration, accounts, privacy policies, availability, and terms. They do not create a SENZO account.

### Protocol boundary

Initial compatibility targets the Stremio HTTP add-on protocol:

- Manifest discovery.
- Catalog resources.
- Metadata resources.
- Stream resources.
- Subtitle resources, where supported.

Each resource must be implemented and tested before being advertised as supported. This does not imply support for unrelated local plugin runtimes or every provider-specific extension.

### Trust and network safety

Manifests and responses are untrusted remote input. Validate URL schemes, response sizes, JSON shapes, declared capabilities, redirects, and returned stream URLs. Production fetching must account for localhost/private-network destinations, DNS rebinding, redirect chains, and other SSRF-style risks. Do not query an add-on unless the user installed and enabled it. See [Security](SECURITY.md).

## 4. Planned high-level components

- **Desktop shell:** GPUI views, navigation, application state, keyboard interaction, and theming.
- **Diagnostics:** bounded local event log, severity levels (audit/info/warn/error), credential redaction, and a user-initiated recent-log export. Diagnostic exports intentionally exclude SQLite watch history and database contents.
- **Metadata service:** TMDB search, details, seasons/episodes, external-ID mapping, and image URL construction.
- **Artwork/cache service:** Bounded local image and metadata cache with eviction and failure handling.
- **Source adapters:** Stremio-compatible add-ons behind a normalized source interface. Future source types should not be coupled to UI code.
- **Playback:** MPV behind an internal playback interface. The pure configuration adapter translates persisted preferences into MPV options and records capability fallbacks. The dynamic runtime loader owns the libmpv DLL and exposes a narrow command/property API; it does not expose MPV pointers to GPUI. The playback worker creates and owns the runtime on a dedicated thread, accepts commands through a bounded queue, and returns non-blocking command and state events. It polls a small playback-state snapshot (position, duration, pause, cache-wait, and media title) every 500 ms and emits only changes. MPV calls never run on the UI thread. Hardware decoding, renderer selection, scaling, HDR handling, subtitles/audio selection, buffering, seeking, speed, and fullscreen belong to this layer.
- **Torrent transport:** Evaluate a Rust torrent engine such as librqbit only if a user-selected source requires it. Validate license, Windows compatibility, buffering, piece prioritization, seek behavior, and platform support before adoption. It is not a bundled source or default torrent catalog.
- **Local persistence:** SQLite stores settings, add-on registry, playback progress, watch history, and future library state. Current Windows database path: `%LOCALAPPDATA%\SENZO\senzo.db`.
- **Networking:** Tokio, Reqwest, and Serde.
- **Presence:** Discord Rich Presence through Discord IPC, with privacy settings and explicit playback-state mapping.
- **Updater:** Windows Stable and Development channels with signed update metadata, integrity verification, staged installation, and rollback strategy.

## 5. Local data model and lifecycle

The initial SQLite schema contains:

- `settings`: key/value application settings, including a typed JSON playback-preferences record.
- `playback_progress`: latest resume position and optional duration per media ID.
- `watch_history`: watched media identity, title, media type, and timestamp.
- `addons`: add-on identity, version, manifest URL, enabled state, manifest JSON, and installation timestamp.

The database is created under the user's local application-data directory and initialized during application startup. SQLite `PRAGMA user_version` tracks schema version 1. Schema creation runs transactionally; future schema changes must add ordered migrations and must not discard history or settings. A build refuses to open a database whose schema version is newer than it supports.

The diagnostic log is stored separately under `%LOCALAPPDATA%\SENZO\logs\senzo.log`. The current logger rotates the active file at 2 MiB and exports up to 1 MiB of events from the previous 72 hours. It records operational events, not watch history. Redaction is best-effort; callers must never pass credentials, private URLs, or personal media activity to the logger. The native Settings screen exposes a user-initiated Copy Logs action; clipboard export excludes watch history and database contents.

Planned additions include metadata cache entries, artwork cache indexing, favorites/library state, and migration metadata. Cache data is bounded and disposable; personal history and settings are not disposable cache.

## 6. Application data flow

1. GPUI sends a search request to the metadata service.
2. The metadata service requests TMDB results and returns stable TMDB IDs.
3. A selected title is enriched with details, credits, images, videos, and external IDs.
4. When the user requests sources, SENZO checks only installed and enabled add-ons that declare support for the relevant resource.
5. The user selects a playable result; it is handed to the playback service.
6. Playback progress and viewing events are persisted locally.
7. Operational events are written to the diagnostic log; the user may later export recent logs explicitly.

## 7. Architectural rules

1. UI code must not own torrent, HTTP, metadata, or MPV implementation details.
2. External providers and add-ons are untrusted input; validate manifests, URLs, and responses.
3. Keep secrets out of source control, logs, and artifacts.
4. Basic SENZO use must never require a SENZO account or cloud service.
5. Watch progress and history must persist locally.
6. No add-on may be installed or queried without explicit user action.
7. Torrentio must not be bundled, preinstalled, or automatically queried.
8. Keep platform-specific behavior behind small adapters; Linux is not part of current CI.
9. Pin and verify dependencies before adopting them in release builds.
10. Do not claim protocol compatibility until the relevant resources and runtime requirements are tested.
11. Treat cache eviction separately from deletion of personal history and settings.
12. Do not ship secrets in source control, logs, or unprotected build artifacts.
13. Diagnostics must be local-first, bounded, redacted, and explicitly exported by the user. Do not log watch history or media titles by default.
14. Do not describe planned functionality as implemented.

## 8. Current implementation snapshot

Initial foundations exist in the repository. Windows CI is the source of truth for formatting, compilation, and tests.

- `src/storage.rs`: SQLite initialization, local settings, playback progress, watch history, and installed add-on registry.
- `src/diagnostics.rs`: local diagnostic event logging, bounded log rotation, recent-event collection, and basic credential redaction.
- `src/metadata.rs`: TMDB movie/TV search, detail enrichment, seasons/episodes, external-ID lookup, and image URL construction.
- `src/settings.rs`: typed playback preferences, conservative defaults, and resource-limit normalization.
- `src/playback/mpv_config.rs`: pure mapping from user preferences to MPV option values, runtime-capability-aware fallback reporting, and unit tests.
- `src/playback/mpv_runtime.rs`: dynamic libmpv loading, pre-initialization option application, local-file loading, pause, seek, speed, stop, and string-property access. It requires an application-supplied libmpv DLL and is not yet packaged.
- `src/playback/mpv_worker.rs`: dedicated playback thread, bounded command queue, non-blocking command/state event receiver, 500 ms change-only playback-state polling, orderly shutdown, and graceful runtime-load failure reporting. It is not yet connected to GPUI.
- `src/addons.rs`: Stremio manifest URL normalization/validation, explicit installation, local registry persistence, and an initial stream request method.
- `src/main.rs`: initializes diagnostics and SQLite, loads persisted playback preferences, and renders the initial GPUI Settings screen with local persistence and Copy Logs.

Not yet connected or implemented:

- Remaining GPUI screens and navigation beyond the initial Settings experience.
- TMDB-powered Home/Search/Details, season/episode browsing, and ID mapping.
- Persistent metadata/artwork cache.
- User-facing add-on manager and complete catalog/meta/stream/subtitle routing.
- GPUI playback screen and UI subscription to worker state events.
- Packaging and validating a trusted Windows libmpv runtime.
- Runtime GPU/decoder/HDR capability detection.
- Live MPV video-surface integration and playback UI.
- Torrent transport.
- Discord Rich Presence.
- Windows installers and updater.

## 9. Implementation sequence

1. Verify service and persistence foundations in Windows CI.
2. Build GPUI navigation and Home, Search, Library, Details, Settings, Add-ons, Player, and About/Credits screens.
3. Connect TMDB search, details, seasons/episodes, artwork, and local cache.
4. Build the user-managed Add-ons UI and source-selection flow.
5. Implement and test Stremio catalog, metadata, stream, and subtitle routing.
6. Connect the dedicated MPV playback worker to GPUI, package a trusted Windows libmpv runtime, add event/property observation, and persist resume progress.
7. Evaluate optional torrent transport only after source selection and playback boundaries are established.
8. Add Discord Rich Presence and privacy controls.
9. Add signed Windows Stable/Development update channels and installer packaging.
