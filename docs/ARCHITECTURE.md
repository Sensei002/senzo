# SENZO architecture

## Product boundary

SENZO is a native desktop media center for Windows and Linux. The application UI is built with Rust and GPUI. It is not an Electron, Tauri, WebView2, or egui application. Mobile platforms are out of scope.

## Planned high-level components

- **Desktop shell:** GPUI views, navigation, application state, keyboard interaction, and theming.
- **Playback:** MPV behind an internal playback interface. Hardware decoding, subtitle selection, audio-track selection, seeking, speed, and fullscreen are playback-engine responsibilities.
- **Sources and add-ons:** HTTP-facing provider integrations behind a normalized source interface. Stremio-compatible manifest/catalog/metadata/stream endpoints are the first compatibility target. Compatibility with other plugin ecosystems must be verified before being promised.
- **Torrent transport:** Evaluate the Rust `librqbit` engine behind a transport abstraction. Validate licensing, seek behavior, piece prioritization, buffering, and platform support before adoption.
- **Metadata:** TMDB-compatible metadata provider adapter, subject to API terms and user-supplied credentials where required.
- **Persistence:** SQLite for local library, settings, playback progress, and provider configuration.
- **Networking and serialization:** Tokio, Reqwest, and Serde are planned for asynchronous I/O and data contracts.
- **Presence:** Discord Rich Presence through Discord IPC, with a privacy setting and explicit playback-state mapping.

## Architectural rules

1. UI code must not own torrent, HTTP, metadata, or MPV implementation details.
2. External providers are untrusted input. Validate and normalize all provider responses.
3. Keep secrets and provider credentials out of logs.
4. Persist playback progress and settings locally; avoid requiring an account for basic local use.
5. Keep Windows and Linux behavior behind small platform adapters.
6. Pin and verify dependencies before adopting them in release builds.
7. Never claim an add-on ecosystem is compatible until its protocol and runtime requirements have been tested.

## Initial implementation sequence

1. Establish a compiling GPUI shell on Windows and Linux.
2. Build the navigation, home/library/search/detail views and design system.
3. Define domain models and SQLite migrations.
4. Integrate MPV behind a playback service.
5. Add metadata and source adapters.
6. Evaluate torrent streaming and buffering.
7. Add Discord Rich Presence.
8. Add signed stable/development update channels and release packaging.
