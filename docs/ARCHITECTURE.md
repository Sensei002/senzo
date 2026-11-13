# SENZO architecture

## 1. Product boundary

SENZO is a native desktop media center built with Rust + GPUI. The initial development target is Windows; Linux is deliberately deferred. Android, iOS, and other mobile platforms are out of scope. SENZO is not an Electron, Tauri, WebView2, or egui application.

SENZO is **accountless by design**. There is no SENZO sign-up, login, cloud profile, or required remote watch-history service. User settings, installed add-ons, watch history, and playback resume positions are stored in the local SENZO application-data directory.

### Product responsibilities

SENZO owns:
- The native desktop interface and navigation.
- TMDB-backed movie and television discovery and metadata presentation.
- A user-managed registry of compatible add-ons.
- Source selection and handoff to the playback layer.
- Local settings, watch history, and resume progress.
- Playback controls and optional Discord Rich Presence.

SENZO does not promise:
- That any particular third-party streaming source is available or lawful in every region.
- That every Stremio add-on or Nuvio plugin is compatible.
- A SENZO-hosted account, cloud sync, or remote history service.
- A built-in catalog of torrent sources or a preinstalled Torrentio integration.

## 2. Metadata and discovery

- **Primary metadata provider:** The Movie Database (TMDB), following the broad metadata workflow observed in Nuvio.
- **Search flow:** Search TMDB for movies or TV shows, retain the TMDB ID as the stable internal identity, then request full details for the selected result.
- **Detail enrichment:** Fetch credits, videos, images, and external IDs with TMDB `append_to_response` where supported. TV season and episode data will use the corresponding season/episode endpoints.
- **Artwork:** Convert TMDB poster, backdrop, logo, and episode-still paths into image CDN URLs. Artwork caching will be local and bounded.
- **Source ID mapping:** Map TMDB IDs to IMDb or other identifiers when required by a compatible add-on. Mapping must be explicit and tested; do not assume every provider accepts TMDB IDs.
- **Attribution:** Show the TMDB logo and required notice in About/Credits: "This product uses the TMDB API but is not endorsed or certified by TMDB."
- **Credentials:** TMDB API Read Access Token configuration is separate from user identity. SENZO must not require users to create a SENZO account to access metadata. The delivery mechanism for the application-level TMDB credential (for example, protected CI configuration for official builds or an explicit user-provided token for self-built versions) must be finalized before public distribution. Never commit credentials or log authorization headers.

## 3. Add-ons and streaming sources

SENZO uses a user-managed add-on registry inspired by Nuvio's Stremio-compatible add-on flow.

### Installation and management

- The Add-ons screen will contain an **Add Add-on** action and a URL field for a manifest URL.
- On installation, SENZO fetches and validates the add-on's `manifest.json`, then stores its identity, capabilities, manifest URL, and manifest locally.
- Add-ons can be enabled, disabled, removed, and reordered.
- Stream requests are sent only to add-ons the user explicitly installed and enabled.
- No streaming add-on is bundled or preinstalled. **Torrentio is not integrated into the default source list.** A user may manually paste their own configured Torrentio manifest URL, or a URL for another compatible provider.
- Add-on configuration and any third-party account requirements belong to that external service; they do not create a SENZO account.

### Protocol boundary

Initial compatibility targets Stremio's HTTP add-on protocol:
- Manifest discovery.
- Catalog resources.
- Metadata resources.
- Stream resources.
- Subtitle resources, where supported.

Each resource must be implemented and tested before being advertised as supported. This does not imply support for Nuvio's separate local JavaScript plugin runtime or every provider-specific extension.

### Trust and network safety

Add-on manifests and responses are untrusted remote input. Validate URL schemes, response sizes, JSON shapes, declared capabilities, and returned stream URLs. Avoid leaking secrets in logs. Consider redirect handling, private-network/localhost destinations, DNS rebinding, and other server-side request risks before enabling unrestricted add-on URL fetching in production. Never query an add-on unless the user installed and enabled it.

## 4. Planned high-level components

- **Desktop shell:** GPUI views, navigation, application state, keyboard interaction, and theming.
- **Metadata service:** TMDB search, details, season/episode retrieval, external-ID mapping, and image URL construction.
- **Artwork/cache service:** bounded local image and metadata cache with eviction and failure handling.
- **Source adapters:** Stremio-compatible add-ons behind a normalized source interface. Additional source types can be added later without coupling them to the UI.
- **Playback:** MPV behind an internal playback interface. Hardware decoding, subtitle selection, audio-track selection, seeking, speed, and fullscreen are playback-engine responsibilities.
- **Torrent transport:** Evaluate Rust librqbit behind a transport abstraction only when a user-selected source requires it. Validate licensing, seek behavior, piece prioritization, buffering, and platform support before adoption. It is not a bundled source or a default torrent catalog.
- **Local persistence:** SQLite stores settings, add-on registry, library state, playback progress, and watch history. On Windows, the current database path is `%LOCALAPPDATA%\SENZO\senzo.db`.
- **Networking and serialization:** Tokio, Reqwest, and Serde.
- **Presence:** Discord Rich Presence through Discord IPC, with a privacy setting and explicit playback-state mapping.
- **Updater:** Windows Stable and Development channels with signed update metadata, integrity verification, staged installation, and rollback strategy.

## 5. Local data model and lifecycle

The initial SQLite schema contains:
- `settings`: key/value application settings.
- `playback_progress`: latest resume position and optional duration per media ID.
- `watch_history`: watched media identity, title, media type, and timestamp.
- `addons`: add-on identity, version, manifest URL, enabled state, manifest JSON, and installation timestamp.

The database is created in the user's local application-data directory. The current implementation initializes it during application startup. Future migrations must be versioned and safe; do not discard user history or add-on configuration during upgrades.

Planned local data additions include metadata cache entries, artwork cache indexing, favorites/library state, and migration metadata. Cache data should be bounded and disposable; personal history and settings should not be treated as disposable cache.

## 6. Application data flow

1. The GPUI interface sends a search request to the metadata service.
2. The metadata service requests TMDB search results and returns stable TMDB IDs.
3. A selected title is enriched with detail, credits, images, videos, and external IDs.
4. When the user requests sources, SENZO checks only installed and enabled add-ons that declare support for the relevant media/resource type.
5. A selected playable result is handed to the playback service.
6. Playback progress and viewing events are persisted locally.
7. Discord presence, if enabled, reflects the active playback state without exposing private information beyond the user's chosen settings.

## 7. Architectural rules

1. UI code must not own torrent, HTTP, metadata, or MPV implementation details.
2. External providers and add-ons are untrusted input. Validate manifests and normalize responses.
3. Keep API tokens and provider credentials out of logs and source control.
4. Basic SENZO use must never require a SENZO account or cloud service.
5. Watch progress and history must work offline and persist locally.
6. No add-on may be installed or queried without the user's explicit action.
7. Torrentio must not be bundled, preinstalled, or automatically queried.
8. Keep Windows and future Linux behavior behind small platform adapters; Linux is not part of the current CI phase.
9. Pin and verify dependencies before adopting them in release builds.
10. Never claim an add-on ecosystem is compatible until its protocol and runtime requirements have been tested.
11. Treat cache eviction separately from deletion of personal history and settings.
12. Do not ship secrets in source control, logs, or unprotected build artifacts.

## 8. Current implementation snapshot

Implemented in the repository (not all yet verified by a successful Windows CI run):
- `src/storage.rs`: SQLite database initialization, local settings, playback progress, watch history, and installed add-on registry.
- `src/metadata.rs`: TMDB movie/TV search, movie/TV detail enrichment, and artwork URL construction.
- `src/addons.rs`: Stremio manifest URL normalization/validation, explicit URL installation, local registry persistence, and an initial stream endpoint request.
- `src/main.rs`: initializes the local database when SENZO starts.

Not yet connected or implemented:
- Full GPUI screens and navigation.
- TMDB-driven Home/Search/Detail experience.
- TV season/episode browsing and ID mapping.
- Persistent metadata/artwork cache.
- User-facing add-on manager and full catalog/meta/stream/subtitle routing.
- MPV playback integration.
- Torrent transport.
- Discord Rich Presence.
- Windows installers and update channels.

## 9. Implementation sequence

1. Verify the service and persistence foundation in Windows CI.
2. Build GPUI navigation, home/library/search/detail/settings screens.
3. Connect TMDB search, details, seasons/episodes, artwork, and local cache.
4. Build the user-managed Add-ons UI and source-selection flow.
5. Implement and test Stremio catalog, metadata, stream, and subtitle resource routing.
6. Integrate MPV behind a playback service and persist resume progress.
7. Evaluate optional torrent transport after source selection and playback boundaries are established.
8. Add Discord Rich Presence and privacy controls.
9. Add signed Windows Stable/Development update channels and installer packaging.
