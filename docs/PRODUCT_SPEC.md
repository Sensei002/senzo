# SENZO product specification

## Vision

SENZO is a native, accountless desktop media center for discovering movies and television, managing optional streaming sources, watching supported media, and keeping personal viewing data on the user's own device.

SENZO combines familiar media-discovery, metadata, and user-managed add-on workflows in its own Rust + GPUI application with a local-first data model.

## Target users and experience

SENZO is for desktop users who want one interface for media discovery and playback while retaining control over which external sources they use. The intended experience is cinematic and media-focused, with artwork-led browsing, responsive navigation, clear source selection, and a dedicated player.

## Non-negotiable product decisions

- Native desktop UI built with Rust + GPUI.
- Windows is the only active implementation and CI target at this stage.
- Linux is a later target. Android and iOS are not targets.
- No SENZO account, signup, login, or required cloud service.
- Watch history, resume progress, settings, and installed add-ons are stored locally.
- TMDB is the primary metadata provider, accessed through the hosted SENZO API. The desktop app must not require users to provide a TMDB token.
- Streaming providers are optional user-installed add-ons.
- No streaming add-on is bundled or preinstalled.
- Torrentio is not integrated into the default source list; users may manually add their own configured manifest URL.
- MPV is the playback engine, with user-adjustable decoding, rendering, image processing, subtitles, and buffering.
- All builds and tests are run through GitHub Actions.

## Functional scope

### Discovery and metadata

The application is planned to provide:
- Movie and TV search.
- Home/discovery shelves.
- Movie and series detail pages.
- Posters, backdrops, overviews, release dates, genres, ratings, and runtime.
- Cast and crew information.
- Trailers and videos where TMDB provides them.
- TV season and episode browsing, including episode artwork and descriptions.
- Stable TMDB IDs and external-ID mapping for source lookup.

TMDB is a metadata provider, not a streaming provider.

### Personal library and history

The application is planned to provide:
- Continue watching.
- Resume from the last saved playback position.
- Watch history.
- Favorites and a local library.
- Local settings and preferences.

Personal data is stored in the SENZO app-data directory. The application must not require a SENZO account or remote history service for these functions.

### Add-on manager

The planned Add-ons screen will:
- Accept a user-entered manifest URL.
- Fetch and validate the manifest.
- Show provider identity and declared capabilities.
- Store the add-on in the local registry.
- Allow enable, disable, remove, and reorder operations.
- Query only sources the user installed and enabled.
- Support compatible Stremio HTTP resources incrementally.

The app must not silently install a source or automatically add Torrentio. Third-party add-ons may have independent configuration, account requirements, availability, and terms.

### Playback and picture quality

MPV is the planned playback engine. The playback experience must expose meaningful controls without making the default experience complicated or resource-heavy.

Target capabilities include:
- Play, pause, seek, resume, playback progress, and fullscreen.
- Volume and mute.
- Subtitle and audio-track selection, playback speed, and subtitle appearance.
- Automatic, enabled, or disabled hardware decoding, with supported Windows decoder preferences such as D3D11VA and DXVA2.
- Renderer selection and capability-aware fallback.
- Configurable upscaling and downscaling algorithms, debanding, dithering, and other supported image-processing options.
- HDR auto-detection, tone mapping, passthrough, or disablement where the output path supports them.
- Buffer duration, back-buffer duration, and a bounded memory budget.
- Playback diagnostics showing codec, resolution, active decoder, renderer, dropped frames, and relevant capability information.

Settings must represent real backend capabilities. Unsupported options must be hidden or clearly unavailable, not presented as working controls. The default profile should favor automatic hardware selection, balanced scaling, disabled expensive optional processing, and bounded buffering. RTX-specific enhancement features must only appear when compatible hardware and a supported processing path are detected.

The native libmpv runtime is owned by a dedicated worker thread with a bounded command queue and non-blocking events. GPUI now has a dedicated Player destination with a native Win32 child-window video viewport, transport and seek controls, volume/mute/fullscreen controls, audio/subtitle track selection, local subtitle loading, online subtitle search, and live decoder/output diagnostics. Keyboard controls cover play/pause, seek, volume, mute, fullscreen, and leaving the Player view. The player surface is created with the application and resized/shown only for the Player destination; MPV calls remain off the UI thread. These controls and the native presentation boundary are CI-tested, but actual video rendering, child-window alignment/z-order, fullscreen behavior, track switching, and GPU compatibility still require Windows runtime validation. Local-file progress is saved; selected TMDB/Stremio media identity is retained across direct and torrent stream handoff so metadata-backed network progress can be persisted. Automatic network-stream resume and completion-aware watch history remain Batch 8 follow-ups. Hardware/renderer/HDR capability detection is not yet complete.

### Discord Rich Presence

Planned presence includes:
- SENZO application identity and approved artwork.
- Current movie or series title.
- Season and episode when relevant.
- Watching, paused, and stopped states.
- Automatic updates on playback transitions.
- A privacy toggle and sensible handling of unavailable metadata.

### Updates and distribution

Planned Windows distribution includes:
- Stable channel sourced from `main`.
- Development channel sourced from `development`.
- Version information and release notes.
- Update checks and download/install flow.
- Integrity verification and signed update metadata.
- Staged installation and rollback-safe behavior.

## Technical architecture

- Rust + GPUI desktop shell.
- Tokio asynchronous runtime.
- Reqwest HTTP client.
- Serde and Serde JSON for protocol/data handling.
- SQLite for local persistence.
- Hosted SENZO API for TMDB metadata.
- Stremio-compatible HTTP add-ons for optional sources.
- MPV through a dynamically loaded libmpv C API adapter.
- Discord IPC for presence.
- GitHub Actions for Windows formatting, compilation, tests, and future release builds.

Provider-specific logic must remain behind service or adapter interfaces. UI components should not directly own network, metadata, torrent, or playback internals.

## Privacy and trust expectations

- No SENZO user account or mandatory cloud storage.
- Local personal history and resume data.
- No preinstalled or automatically queried streaming source.
- Explicit user action required to install and enable an add-on.
- Validate untrusted remote manifests, responses, and stream URLs.
- Never log or commit credentials. The TMDB Read Access Token remains server-side in the SENZO API deployment and is never shipped in the desktop binary.
- Clearly communicate that external providers are independent services.

## Current status

The repository now has SQLite persistence, TMDB discovery, the Stremio add-on ecosystem, torrent transport, typed playback preferences, a dedicated GPUI Player destination, and a dynamically loaded libmpv worker. The Player destination provides a native Win32 video viewport, transport/seek/volume/mute/fullscreen controls, audio/subtitle track selection, subtitle loading/search, keyboard shortcuts, and live playback diagnostics. The CI artifact contains the pinned MPV runtime and notices. Native video presentation, GPU/decoder compatibility, and real provider playback still require Windows runtime validation. Metadata-backed network resume, completion-aware watch history, runtime capability detection, Discord Rich Presence, installers, and updater remain unfinished.

See [README](../README.md), [Architecture](ARCHITECTURE.md), and [Roadmap](ROADMAP.md).
