# SENZO architecture

## 1. Product boundary

SENZO is a native desktop media center built with Rust + GPUI. Windows is the only active implementation and CI target. Linux is deliberately deferred until the Windows application is established. Android and iOS are out of scope. SENZO is not an Electron, Tauri, WebView2, or egui application.

SENZO is **accountless by design**. There is no SENZO sign-up, login, cloud profile, or required remote watch-history service. User settings, installed add-ons, watch history, and resume positions are stored in the local SENZO application-data directory.

### Windows installation and writable data

The Windows distribution is a **per-machine system installation** under `%ProgramFiles%\\SENZO` (normally `C:\\Program Files\\SENZO`). The installer requests elevation through Windows UAC. Application binaries, bundled resources, and the trusted MPV runtime belong in the installation directory; the running application must never write mutable user data there. The installer UI must use the canonical transparent SENZO logo at `assets/branding/senzo-logo.png` for its branding (including the installer icon and appropriate welcome/header artwork); preserve the supplied artwork and do not substitute or recreate the logo. The installer must be Authenticode-signed with a publicly trusted code-signing certificate so Windows UAC does not show `Unknown publisher`. The intended publisher display name is **`Sensei.STGR`**; obtain a certificate whose validated subject/publisher identity can display that exact name, subject to the certificate authority's identity-verification rules. Sign the installer and distributed executable binaries, use SHA-256 and a trusted timestamp, and keep signing credentials/private keys out of the repository and build logs. Per-user writable data remains separate under `%LOCALAPPDATA%\\SENZO` (database, settings, logs, and disposable caches). Secrets remain in Windows Credential Manager. The installer should create Start Menu shortcuts and register a normal Windows uninstall entry. Uninstall must offer a clear choice about retaining or removing user data; updates must preserve it. Do not install the application itself under AppData or use a per-user-only installation by default.

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
- **Metadata service:** TMDB search, popular movie/TV catalogues, details, seasons/episodes, external-ID mapping, and image URL construction. TMDB Read Access Tokens are configured through a masked native Windows credential prompt and stored only in Windows Credential Manager; they are never stored in SQLite or logs.
- **Artwork/cache service:** Bounded local image and metadata cache with eviction and failure handling.
- **Source adapters:** Stremio-compatible add-ons behind a normalized source interface. Future source types should not be coupled to UI code.
- **Playback:** MPV behind an internal playback interface. The pure configuration adapter translates persisted preferences into MPV options and records capability fallbacks. The dynamic runtime loader owns the libmpv DLL and exposes a narrow command/property API; it does not expose MPV pointers to GPUI. The playback worker creates and owns the runtime on a dedicated thread, accepts commands through a bounded queue, and returns non-blocking command and state events. It polls a compact state snapshot (position, duration, pause, cache-wait, media title, volume, mute, fullscreen, video codec/decoder, active hardware decoder, video output, output dimensions, estimated frame rate, and dropped-frame count) every 500 ms and emits only changes. The GPUI playback panel exposes these live diagnostics so decoder and renderer behavior can be verified on real Windows hardware. Its command boundary covers local-file load, pause/resume, seek, speed, stop, volume, mute, fullscreen, audio-track selection, and subtitle-track selection/disable. The playback snapshot also reads MPV's live `speed` property, and GPUI exposes selectable 0.5×, 0.75×, 1.0×, 1.25×, 1.5×, and 2.0× speed presets. Volume and track IDs are validated before commands enter the queue. The GPUI bridge stores local-file progress in SQLite at a bounded interval, force-flushes the latest known position before stop or media switch, and queues a resume seek after reopening the same canonical file path. MPV calls never run on the UI thread. Audio/subtitle track enumeration is read from MPV's track-list properties and surfaced in GPUI, with selection controls. The GPUI bridge also provides a native video viewport backed by a child Win32 HWND passed to MPV as `wid`; actual rendering and lifecycle behavior still require runtime validation. The player UI includes pointer-driven seek and volume controls. Renderer, hardware-decoder, image-processing, HDR, and buffer preferences are supplied to MPV at session initialization; subtitle typography and timing controls are applied live. Local-file playback position is persisted periodically and restored on reopen. GPUI is connected to the worker for local-file selection and basic transport, seek, mute, fullscreen, and volume-step commands. Hardware decoding, renderer selection, scaling, HDR handling, subtitles/audio selection, buffering, seeking, speed, and fullscreen belong to this layer.
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
- `src/playback/mpv_runtime.rs`: dynamic libmpv loading, pre-initialization option application, local-file loading, pause, seek, speed, stop, and string-property access. The Windows CI packaging workflow separately produces a pinned LGPL runtime artifact; app-side runtime installation and playback validation remain outstanding.
- `src/playback/mpv_worker.rs`: dedicated playback thread, bounded command queue, non-blocking command/state event receiver, 500 ms change-only playback-state polling, orderly shutdown, and graceful runtime-load failure reporting. GPUI queues file-load, transport, seek, mute, and fullscreen commands through this boundary.
- `src/addons.rs`: Stremio manifest URL normalization/validation, explicit installation, local registry persistence, and an initial stream request method.
- `src/main.rs`: initializes diagnostics and SQLite, loads persisted playback preferences, and renders the initial GPUI Settings screen with local persistence and Copy Logs.

Not yet connected or implemented:

- Remaining GPUI screens and navigation beyond the initial Settings experience.
- TMDB-powered Home/Search/Details, season/episode browsing, and ID mapping.
- Persistent metadata/artwork cache.
- User-facing add-on manager and complete catalog/meta/stream/subtitle routing.
- Runtime validation of native video presentation, child-window alignment/lifetime, and GPU/decoder behavior on Windows hardware.
- Integrating the packaged MPV runtime into the final installer and validating playback on Windows hardware.
- Runtime GPU/decoder/HDR capability detection.
- Runtime verification of native MPV video rendering, child-window alignment/lifetime, and GPU/decoder behavior.
- Torrent transport.
- Discord Rich Presence.
- Windows installers and updater.

## 9. Implementation sequence

1. Verify service and persistence foundations in Windows CI.
2. Build GPUI navigation and Home, Search, Library, Details, Settings, Add-ons, Player, and About/Credits screens.
3. Connect TMDB search, details, seasons/episodes, artwork, and local cache.
4. Build the user-managed Add-ons UI and source-selection flow.
5. Implement and test Stremio catalog, metadata, stream, and subtitle routing.
6. Complete Windows MPV runtime deployment and validate native video rendering, lifecycle, GPU/decoder behavior, and seek-bar interaction.
7. Evaluate optional torrent transport only after source selection and playback boundaries are established.
8. Add Discord Rich Presence and privacy controls.
9. Add signed Windows Stable/Development update channels and installer packaging.

## GPUI playback bridge

The GPUI application owns the playback worker handle behind a shared mutex. The mutex protects only short, non-blocking queue operations and event reads; all libmpv calls remain on the dedicated playback thread. A cancellable GPUI task checks for worker events every 200 ms using background executor work, then applies snapshots and status changes to the UI entity. The controls expose a native Open Media file picker, pause/resume, stop, mute, a draggable seek bar, a continuous volume slider plus 5% steps, 10-second seek steps, and fullscreen. Resume seeking is deferred until MPV reports the newly loaded path and a valid duration, with a bounded timeout and end-of-file clamping. Subtitle font size, vertical margin, and bold preferences can be changed during playback. SENZO loads `runtime/libmpv-2.dll` beside the executable; the separate Windows CI packaging workflow produces the pinned LGPL runtime artifact and preserves its notices. The native child HWND is created/resized from the GPUI viewport callback and passed to MPV before initialization. This is CI-verified integration only; actual video rendering, coordinate alignment, window lifetime/teardown, and GPU behavior still require runtime testing.

### Native Windows video presentation boundary

The Windows presentation path uses a caller-owned native child HWND as MPV's `wid` target. `MpvConfig::set_native_window_id` validates the non-zero 32-bit Win32 handle and passes it to libmpv before initialization. The owner must keep that child window alive until the MPV instance is destroyed. GPUI-to-HWND acquisition, DPI-aware viewport geometry, child-surface creation/resizing, and MPV `wid` wiring are implemented. Actual video rendering, client-coordinate alignment, z-order, and lifecycle behavior remain to be verified in a running Windows app.

### Native Windows child-surface primitive

`src/playback/windows_surface.rs` provides a Windows-only RAII wrapper around a child `STATIC` HWND. It validates the parent HWND and positive pixel dimensions, exposes the child handle as the documented non-zero 32-bit MPV `wid` value, supports explicit resize/show operations without activation or z-order changes, and destroys the child HWND on drop. All surface operations must run on the owning UI thread. The GPUI Settings playback panel now includes a dedicated video viewport. Its child-prepaint callback captures the viewport bounds and converts logical pixels to physical pixels on each layout pass. The callback creates the child surface on first layout and updates its bounds on later layouts; the surface is retained in application state and its HWND is supplied to MPV before initialization. Actual alignment, z-order, teardown ordering, and rendered output remain to be validated in a running Windows application.

## GPUI native window handle

SENZO obtains its Windows parent HWND through GPUI's public `HasWindowHandle` implementation and the `raw-window-handle` 0.6 `RawWindowHandle::Win32` variant. The value is retained in application state and used as the parent for the native video child surface.
