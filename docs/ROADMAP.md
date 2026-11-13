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
- [x] Add Stremio manifest URL normalization/validation and explicit add-on installation service.
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

- [ ] Configure TMDB credentials for official builds without exposing secrets in source control or logs.
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
- [ ] Package and validate the upstream Windows x64 LGPL libmpv runtime (`libmpv-2.dll`) and its sibling dependencies in a separate GitHub Actions artifact; preserve upstream licence notices. The package workflow pins the upstream mpv release version and records archive/runtime SHA-256 values.
- [x] Add a dedicated MPV playback worker with a bounded command queue, non-blocking events, orderly shutdown, and graceful missing-runtime reporting.
- [x] Add a playback-state snapshot with change-only 500 ms polling for playback position, duration, pause, buffering, title, volume, mute, and fullscreen.
- [x] Add validated worker/runtime commands for volume, mute, fullscreen, audio-track selection, and subtitle-track selection/disable.
- [x] Connect the playback worker to GPUI through a cancellable monitor task and bounded command queue; MPV operations remain on the dedicated worker thread.
- [x] Enumerate audio/subtitle tracks from MPV and display the active session's available tracks in GPUI.
- [x] Add GPUI audio/subtitle track selection, subtitle-off control, and local external subtitle-file loading.
- [x] Add live subtitle synchronization and font-size controls with locally persisted preferences.
- [ ] Implement a native video presentation surface and validate GPU/renderer compatibility.
- [ ] Embed the MPV video output into the GPUI player layout; verify HWND/window lifetime, resize, DPI, and z-order behavior before enabling playback by default.
- [x] Add native local-file selection and queue file-load commands through the GPUI playback bridge (requires a trusted runtime DLL).
- [x] Add initial GPUI pause/resume, stop, mute, 10-second seek-step, and fullscreen command controls.
- [x] Add GPUI volume down/up controls in 5% steps using the live MPV volume snapshot.
- [ ] Add a seek bar and continuous volume adjustment.
- [x] Add a UI-independent OpenSubtitles.com API v1 client for explicit title/IMDb/TMDB searches, language filtering, typed results, download-link retrieval, HTTPS host validation, and bounded subtitle downloads.
- [x] Add a Windows Credential Manager adapter for saving, loading, checking, and clearing the OpenSubtitles API key without SQLite or log persistence.
- [x] Add collision-safe local subtitle-cache persistence for validated OpenSubtitles downloads; cache writes enforce size and extension limits and never overwrite existing files.
- [x] Add playback-screen OpenSubtitles search using the current title, show English/Bangla file results, download a user-selected file into the local cache, and queue it into MPV.
- [x] Add Settings controls for masked native Windows API-key entry, replacement, and removal; the key is never redisplayed or written to SQLite/logs.
- [x] Surface live MPV video diagnostics in the playback panel: codec, decoder, active hardware decoder, video output, output dimensions, estimated frame rate, and dropped-frame count.
- [ ] Detect runtime decoder/renderer/HDR capabilities and apply settings to a live MPV instance.
- [x] Persist local-file playback progress periodically, flush before stop/media switch, and resume from the saved position when the same file is reopened.
- [ ] Extend progress persistence to metadata-backed movie/episode IDs and watch-history completion rules.
- [ ] Add live playback diagnostics and graceful error/recovery states.

## Phase 5 — Add-ons and streaming sources

- [ ] Build Add-ons UI with manual manifest URL entry.
- [ ] Add manifest preview/validation and add-on enable/disable/remove/reorder.
- [ ] Implement Stremio catalog, metadata, stream, and subtitle resource routing.
- [ ] Add source capability matching and user-controlled source selection.
- [ ] Keep all streaming add-ons absent by default; users may add their own configured manifest URLs.
- [ ] Harden remote manifest fetching, redirects, and stream URL validation.
- [ ] Connect selected add-on streams to the playback service through a transport abstraction.

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
