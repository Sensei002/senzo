# SENZO architecture

## Product boundary

SENZO is a native desktop media center built with Rust + GPUI. The initial development target is Windows; Linux is deliberately deferred. It is not an Electron, Tauri, WebView2, or egui application. Mobile platforms are out of scope.

SENZO is **accountless by design**. There is no SENZO sign-up, login, cloud profile, or required remote watch-history service. A user's settings, installed add-ons, watch history, and playback resume positions are stored locally in the SENZO application-data directory.

## Metadata and discovery

- **Primary metadata provider:** The Movie Database (TMDB), following the same broad metadata workflow used by Nuvio.
- **Search flow:** Search TMDB for movies or TV shows, retain the TMDB ID as the stable internal identity, then request full details for the selected result.
- **Detail enrichment:** Fetch credits, videos, images, and external IDs with TMDB's append_to_response support. TV seasons and episodes will use the corresponding season/episode endpoints.
- **Artwork:** TMDB poster, backdrop, logo, and episode-still paths are converted to image CDN URLs. Artwork caching will be local and bounded.
- **Attribution:** The app must show the TMDB logo and the required notice in its About/Credits screen: "This product uses the TMDB API but is not endorsed or certified by TMDB."
- **Credentials:** TMDB API Read Access Token configuration is separate from a user account. It must never be logged. The settings UI and secure Windows credential storage will be added before the metadata feature is exposed to users.

## Add-ons and streaming sources

SENZO will use a user-managed add-on registry inspired by Nuvio's Stremio-compatible add-on flow.

- The Add-ons screen will contain an **Add Add-on** action and a URL field for a manifest URL.
- On install, SENZO fetches and validates the add-on's manifest.json, then stores its identity, capabilities, manifest URL, enabled state, and manifest locally.
- Add-ons can be enabled, disabled, removed, and reordered.
- Stream requests are sent only to add-ons the user explicitly installed and enabled.
- **No streaming add-on is bundled or preinstalled. Torrentio is not integrated into the default app.** A user may manually paste a configured Torrentio manifest URL, or a manifest URL for another compatible provider.
- Initial protocol compatibility targets Stremio's HTTP manifest, catalog, metadata, and stream resources. Each resource will be implemented and tested as needed; this does not imply support for Nuvio's separate local JavaScript plugin runtime.

## Planned high-level components

- **Desktop shell:** GPUI views, navigation, application state, keyboard interaction, and theming.
- **Playback:** MPV behind an internal playback interface. Hardware decoding, subtitle selection, audio-track selection, seeking, speed, and fullscreen are playback-engine responsibilities.
- **Source adapters:** Stremio-compatible add-ons behind a normalized source interface. Additional source types can be added later without coupling them to the UI.
- **Torrent transport:** Evaluate Rust librqbit behind a transport abstraction only when a user-selected source requires it. Validate licensing, seek behavior, piece prioritization, buffering, and platform support before adoption.
- **Local persistence:** SQLite stores settings, add-on registry, library state, playback progress, and watch history. On Windows, the default database path is %LOCALAPPDATA%\SENZO\senzo.db.
- **Networking and serialization:** Tokio, Reqwest, and Serde.
- **Presence:** Discord Rich Presence through Discord IPC, with a privacy setting and explicit playback-state mapping.

## Architectural rules

1. UI code must not own torrent, HTTP, metadata, or MPV implementation details.
2. External providers and add-ons are untrusted input. Validate manifests and normalize responses.
3. Keep API tokens and provider credentials out of logs; use secure OS credential storage for secrets.
4. Basic app use must never require a SENZO account or cloud service.
5. Watch progress and history must work offline and persist locally.
6. No add-on may be installed or queried without the user's explicit action.
7. Keep Windows and future Linux behavior behind small platform adapters.
8. Pin and verify dependencies before adopting them in release builds.
9. Never claim an add-on ecosystem is compatible until its protocol and runtime requirements have been tested.

## Current implementation

- src/storage.rs: SQLite database initialization, local settings, playback progress, watch history, and installed add-on registry.
- src/metadata.rs: TMDB search, movie/TV detail enrichment, and artwork URL construction.
- src/addons.rs: Stremio manifest validation, explicit URL installation, local registry persistence, and stream endpoint requests.
- src/main.rs: initializes the local database when SENZO starts.

## Implementation sequence

1. Complete local persistence, metadata and add-on service tests.
2. Build the GPUI navigation, home/library/search/detail/settings screens.
3. Connect the TMDB service to search, details, artwork and local cache.
4. Build the user-managed Add-ons UI and source-selection flow.
5. Integrate MPV behind a playback service.
6. Evaluate torrent streaming and buffering after the optional add-on flow is established.
7. Add Discord Rich Presence.
8. Add signed Windows Stable/Development update channels and installer packaging.
