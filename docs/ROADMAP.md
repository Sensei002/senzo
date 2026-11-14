# SENZO roadmap

The active implementation phase targets **Windows only**. Linux will be addressed later. Android and iOS are out of scope. A feature is complete only when its implementation is connected where appropriate and its checks pass in Windows CI.

Status definitions:

- **Done:** implemented and verified.
- **In progress:** implementation exists but is incomplete or awaiting CI verification.
- **Planned:** not implemented yet.

## Phase 0 — Repository and service foundation

- [x] Establish repository and branch strategy (`main` stable, `development` active work).
- [x] Add initial native GPUI application shell.
- [x] Configure Windows-only GitHub Actions CI.
- [x] Add local SQLite database initialization at application startup.
- [x] Add initial local settings, playback progress, watch history, and add-on registry storage.
- [x] Add initial TMDB movie/TV search and detail client.
- [x] Add TMDB artwork URL construction.
- [x] Add an initial Stremio manifest installation service and local add-on registry.
- [x] Harden manifest fetching with HTTPS-only URLs, redirect rejection, bounded response size, request timeouts, and local/reserved IP-literal and hostname checks.
- [x] Add DNS-resolution-aware SSRF protection, typed catalog/metadata/stream routing, initial add-on management UI, and validated MPV URL handoff.
- [x] Add database schema versioning and migration tests.
- [x] Add local rolling diagnostic log service and bounded recent-log export API (Settings/clipboard UI remains pending).

## Phase 1 — Native desktop experience

The Settings experience will use progressive disclosure: essential controls first, advanced playback controls in a separate expandable section. Defaults prioritize automatic hardware selection, balanced image quality, low background overhead, and bounded memory use.

- [ ] Build main SENZO navigation and cinematic layout.
- [ ] Add reusable poster, backdrop, title, and detail components.
- [ ] Build Home, Search, Library, Details, Player, Add-ons, Settings, and About/Credits screens.
- [ ] Add a Settings diagnostics section with a Copy Logs action.
- [ ] Add keyboard navigation, focus behavior, and accessible controls.
- [ ] Apply the exact official SENZO transparent logo throughout the application.
- [ ] Add TMDB logo and required attribution in About/Credits.

## Phase 4 — Metadata, discovery, and local library

- [x] Add masked Windows Credential Manager storage and Settings controls for the TMDB Read Access Token; secrets are excluded from SQLite and logs. Official-build credential injection remains a distribution task.
- [ ] Connect TMDB search and detail services to GPUI screens.
- [x] Add TMDB TV season/episode detail endpoints and typed response models (UI browsing remains pending).
- [x] Add TMDB movie/TV external-ID lookup models and endpoints (source-provider integration remains pending).
- [ ] Add bounded local metadata and artwork caches.
- [ ] Add continue-watching, watch history, favorites, and local library views.
- [ ] Add safe cache cleanup that preserves personal history and settings.

## Phase 3 — Playback engine

- [ ] Build Add-ons UI with manual manifest URL entry.
- [ ] Add manifest preview/validation and add-on enable/disable/remove/reorder.
- [ ] Implement Stremio catalog, metadata, stream, and subtitle resource routing.
- [ ] Add source capability matching and user-controlled source selection.
- [ ] Keep all streaming add-ons absent by default; Torrentio may only be added manually by the user.
- [ ] Harden remote manifest fetching, redirects, and stream URL validation.
- [x] Add a pure MPV option-mapping layer with capability-aware fallback reporting and unit tests.
- [x] Add a dynamic libmpv loader that applies options before initialization and exposes local-file load, pause, seek, speed, stop, and property access.
- [x] Package the pinned upstream Windows x64 LGPL libmpv runtime (`libmpv-2.dll`) and sibling dependencies in a separate GitHub Actions artifact; preserve upstream licence notices. Runtime packaging is CI-verified; in-app playback still requires runtime validation. The package workflow pins the upstream mpv release version and records archive/runtime SHA-256 values.
- [x] Add a dedicated MPV playback worker with a bounded command queue, non-blocking events, orderly shutdown, and graceful missing-runtime reporting.
- [x] Add a playback-state snapshot with change-only 500 ms polling for playback position, duration, pause, buffering, title, volume, mute, and fullscreen.
- [x] Add validated worker/runtime commands for volume, mute, fullscreen, audio-track selection, and subtitle-track selection/disable.
- [x] Connect the playback worker to GPUI through a cancellable monitor task and bounded command queue; MPV operations remain on the dedicated worker thread.
- [x] Enumerate audio/subtitle tracks from MPV and display the active session's available tracks in GPUI.
- [x] Add GPUI audio/subtitle track selection, subtitle-off control, and local external subtitle-file loading.
- [x] Add live subtitle synchronization and font-size controls with locally persisted preferences.
- [x] Add a validated MPV configuration boundary for a caller-owned Win32 child HWND (`wid`) before MPV initialization.
- [x] Add a Windows child-HWND surface owner with validated parent/size inputs, resize support, and RAII teardown.
- [x] Acquire the parent HWND through GPUI 0.2.2 public raw-window-handle APIs and retain it in the app lifecycle.
- [x] Add a tested GPUI logical-pixel to Win32 physical-pixel conversion helper for child-surface geometry, including fractional scale factors, invalid geometry, and integer overflow checks.
- [x] Add a dedicated GPUI video viewport and capture its prepaint layout bounds, converting logical coordinates to physical pixels on each layout pass.
- [x] Create and resize the native child surface from the GPUI viewport layout callback; validate client-coordinate alignment, z-order, and runtime behavior on Windows.
- [x] Pass the child HWND to MPV before runtime initialization; validate actual video presentation, hardware decoding, and GPU/renderer compatibility on Windows.
- [x] Wire the native MPV child surface into the GPUI video viewport, including parent-HWND acquisition, DPI-aware viewport bounds, child creation, and resize updates. Actual Windows playback, z-order, and teardown behavior still require runtime validation.
- [x] Add native local-file selection and queue file-load commands through the GPUI playback bridge (requires a trusted runtime DLL).
- [x] Add initial GPUI pause/resume, stop, mute, 10-second seek-step, and fullscreen command controls.
- [x] Add GPUI volume down/up controls in 5% steps using the live MPV volume snapshot.
- [x] Add an interactive seek bar with click/drag seeking and a continuous volume slider.
- [x] Add live playback-speed controls (0.5×, 0.75×, 1.0×, 1.25×, 1.5×, and 2.0×) with the active speed read from MPV.
- [x] Add a UI-independent OpenSubtitles.com API v1 client for explicit title/IMDb/TMDB searches, language filtering, typed results, download-link retrieval, HTTPS host validation, and bounded subtitle downloads.
- [x] Add a Windows Credential Manager adapter for saving, loading, checking, and clearing the OpenSubtitles API key without SQLite or log persistence.
- [x] Add collision-safe local subtitle-cache persistence for validated OpenSubtitles downloads; cache writes enforce size and extension limits and never overwrite existing files.
- [x] Add playback-screen OpenSubtitles search using the current title, show English/Bangla file results, download a user-selected file into the local cache, and queue it into MPV.
- [x] Add Settings controls for masked native Windows API-key entry, replacement, and removal; the key is never redisplayed or written to SQLite/logs.
- [x] Surface live MPV video diagnostics in the playback panel: codec, decoder, active hardware decoder, video output, output dimensions, estimated frame rate, and dropped-frame count.
- [ ] Detect runtime decoder/renderer/HDR capabilities and apply settings to a live MPV instance.
- [x] Persist local-file playback progress periodically, flush before stop/media switch, and resume from the saved position when the same file is reopened.
- [ ] Extend progress persistence to metadata-backed movie/episode IDs and watch-history completion rules.
- [x] Add live playback diagnostics and visible runtime/command failure states.
- [x] Defer resume seeking until MPV reports the newly loaded file and its duration, with bounded timeout and end-of-file clamping.
- [x] Apply subtitle font size, vertical margin, and bold changes to the active MPV session.

## Phase 5 — Add-ons and streaming sources

- [x] Add bounded, timeout-limited HTTPS manifest fetching with redirects disabled and basic local-destination URL validation.
- [x] Build an initial native Add-ons management screen listing locally installed providers with enable/disable/remove controls.
- [x] Add a UI-independent manifest preview and validation API; user-facing manifest URL entry and confirmation flow remain in progress.
- [x] Implement Stremio catalog, metadata, stream, and subtitle resource request services.
- [x] Add stream result classification for direct HTTP(S), torrent info-hash, and external-link entries; actual torrent transport is Batch 5.
- [x] Keep all streaming add-ons absent by default; users must explicitly install their own manifest URLs.
- [x] Harden remote manifest fetching, DNS pinning, redirects, response limits, and direct stream URL validation.

## Phase 6 — Additional sources and presence

- [ ] Define a stable source-adapter interface for additional providers.
- [ ] Evaluate optional torrent engine choices behind a transport abstraction.
- [ ] Validate engine licensing, Windows compatibility, buffering, piece prioritization, and seek behavior before adoption.
- [ ] Add more source adapters without changing metadata or UI layers.
- [ ] Add Discord Rich Presence, playback-state updates, and privacy controls.

## Phase 7 — Windows distribution

- [ ] Build optimized Windows release binaries in GitHub Actions.
- [ ] Build a per-machine Windows installer targeting `%ProgramFiles%\\SENZO` (normally `C:\\Program Files\\SENZO`), with UAC elevation, Start Menu shortcuts, and a Windows uninstall entry.
- [ ] Brand the installer with the canonical transparent `assets/branding/senzo-logo.png` artwork; derive required icon formats without altering or replacing the original logo.
- [ ] Acquire a publicly trusted Authenticode code-signing certificate with the intended verified publisher display name `Sensei.STGR` (subject to CA identity validation); sign the installer and distributed executable binaries using SHA-256 and trusted timestamping. Keep signing keys/certificates protected in GitHub Actions secrets or an equivalent secure signing service.
- [ ] Bundle the application binaries, resources, and validated MPV runtime under the installation directory; keep all mutable user data under `%LOCALAPPDATA%\\SENZO` and secrets in Windows Credential Manager.
- [ ] Ensure upgrades preserve user data and uninstall offers an explicit retain/remove-data choice.
- [ ] Add Stable and Development update channels.
- [ ] Sign update metadata and verify downloaded artifacts.
- [ ] Add staged installation and rollback-safe update behavior.
- [ ] Publish release notes and version information.

## Later platform work

- [ ] Evaluate Linux support after the Windows application is stable.
- [ ] Add Linux CI and packaging only when Linux implementation begins.

## Ongoing documentation

The README is the detailed public project introduction. Update it whenever product scope, supported platforms, implemented features, or major user-facing capabilities change. Keep this roadmap honest about what is implemented, what awaits CI verification, and what remains planned. Update Architecture, Product Specification, Security, and Development docs whenever relevant decisions change.
