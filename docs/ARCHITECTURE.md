# SENZO rewrite architecture

## 1. Product and platform boundary

SENZO is a native desktop rewrite of Nuvio Desktop in Rust. GPUI CE is the only selected UI framework. The initial desktop targets are Windows and Linux; macOS remains a future portability target, not a release commitment. Android and iOS are out of scope.

SENZO does not use WebView2, Electron, Tauri, egui, or browser-hosted UI. Every application screen and player control is rendered by GPUI. MPV/libmpv is a media engine only.

The audited Nuvio source snapshot is pinned in [NUVIO_SOURCE_AUDIT.md](NUVIO_SOURCE_AUDIT.md). The full feature mapping is in [NUVIO_TO_SENZO_MAPPING.md](NUVIO_TO_SENZO_MAPPING.md).

## 2. Rewrite boundary

The current `src/main.rs`-centered SENZO implementation is a legacy prototype. It remains in repository history and may be consulted for isolated, reviewed behavior or tests, but it is not the architecture baseline. Phase 1 establishes a clean Rust workspace. No legacy module is reused without review against the contracts in this document.

The rewrite preserves Nuvio desktop behavior and feature boundaries, not Kotlin syntax or platform-specific implementation accidents. The Windows Nuvio player bridge contains WebView2 code and is explicitly excluded.

## 3. Proposed Rust workspace

Phase 1 should create a Cargo workspace with a small number of cohesive crates. Split only where ownership, platform boundaries, or independent tests justify it; avoid one crate per screen.

| Crate | Responsibility |
|---|---|
| `senzo-app` | Binary entry point, application lifecycle, dependency composition and startup/shutdown. |
| `senzo-ui` | GPUI application shell, routes, views, design system, focus/navigation and player controls. |
| `senzo-domain` | Stable media, source, profile, playback, library and settings domain types/use cases. |
| `senzo-platform` | Native windowing, OS integration, input, filesystem locations, keyring and platform capabilities. |
| `senzo-network` | Shared HTTP policy, timeouts, redirect policy, URL validation, response limits and safe request execution. |
| `senzo-metadata` | SENZO API/TMDB client, typed metadata models, external-ID mapping and catalogue metadata. |
| `senzo-artwork` | Image URL resolution, secure image fetching, cache, decoding and GPUI image integration. |
| `senzo-addons` | Stremio manifest/resource protocol, installed add-on registry and add-on transport. |
| `senzo-plugins` | Separate Nuvio-compatible plugin/scraper repository and sandboxed QuickJS-compatible runtime. |
| `senzo-sources` | Provider-neutral source contracts, stream aggregation, filtering, sorting, selection and fallback. |
| `senzo-debrid` | Debrid provider auth, API clients, cache checks and URL resolution. |
| `senzo-torrent` | P2P/torrent transport, selected-file streaming, local range server and session lifecycle. |
| `senzo-playback` | MPV/libmpv adapter, worker, playback state, tracks, subtitles, chapters and lifecycle. |
| `senzo-library` | Local library, collections, watch history, watched state, resume progress and profiles. |
| `senzo-integrations` | Accounts, sync, media servers, Trakt, Simkl, MDBList, Discord and external service adapters. |
| `senzo-party` | Watch-party protocol, presence, chat and playback synchronization. |
| `senzo-storage` | SQLite schema/migrations, repositories, cache metadata and secure credential references. |
| `senzo-settings` | Typed settings, defaults, validation, migrations and capability-aware preferences. |
| `senzo-diagnostics` | Structured logs, crash context, redaction and user-initiated support export. |
| `senzo-update` | Update metadata, channel selection, integrity verification and safe update workflow. |

A crate may be merged with a closely related crate if the actual implementation proves a separate boundary unnecessary. The public contracts between UI, services, storage and playback must remain explicit.

## 4. Dependency and runtime policy

- **UI:** GPUI CE, pinned to a tested version.
- **Async:** Tokio; never block the GPUI event/render thread.
- **HTTP:** Reqwest with Rustls and a shared network policy.
- **Serialization:** Serde / Serde JSON.
- **Persistence:** SQLite with ordered, transactional migrations.
- **Secrets:** OS credential storage where supported; never plaintext secrets in logs or repository files.
- **Playback:** MPV/libmpv through a narrow Rust-owned interface.
- **Torrent:** Evaluate Nuvio P2P behavior against Rust transport options; keep transport behind a trait and validate licenses.
- **Plugin runtime:** Evaluate a Rust QuickJS binding for Nuvio-compatible plugin behavior; sandbox network, filesystem, CPU, memory and execution time.
- **Build:** Cargo and GitHub Actions. Windows and Linux CI must be separate explicit targets.

The UI must not call provider APIs, MPV, SQLite, or torrent internals directly. It sends typed commands to services and renders state/events.

## 5. Native playback architecture — critical gate

Playback is a first-class subsystem, not a child window added after the UI is complete.

### Required ownership model

1. The GPUI layer creates a playback session request with media identity, resolved source, headers, subtitles and preferences.
2. The playback service validates the request and creates one session owner.
3. A dedicated worker owns the MPV handle and all MPV calls.
4. A bounded command channel carries load, pause, seek, volume, track, subtitle, speed and stop commands.
5. A non-blocking event/state channel returns playback status, position, duration, buffering, tracks, errors and renderer diagnostics.
6. The UI observes state and renders all controls in GPUI.
7. Stop, source replacement, route navigation and application shutdown share one serialized teardown path. The MPV handle is destroyed exactly once after worker shutdown is acknowledged.
8. Torrent sessions own their local HTTP server and temporary files; they are shut down only after MPV no longer references their stream.

### Video presentation proof-of-concept

Before building the full player, Phase 1 must prove actual visible video in a GPUI window on Windows. It must compare:
- MPV's render API / `mpv_render_context` integration with GPUI's rendering/compositing path.
- A native OS video surface integrated into the GPUI layout, only if the first approach is unsuitable.

Do not assume a valid HWND, configured VO, decoded frames, or zero dropped frames means that the user can see video. Acceptance requires visible frames on screen, audible audio, seek, pause/resume, resize/fullscreen, repeated sessions, and clean stop/shutdown. The previous child-HWND implementation is not accepted as proven.

Linux playback must receive its own native presentation proof before the Linux UI is declared supported. Avoid designing the core player around Win32-only handles.

## 6. Application data flow

### Discovery
GPUI view → catalog/search use case → metadata API client → typed metadata → artwork service/cache → GPUI image components.

### Source discovery
Details/episode identity → source coordinator → enabled user-installed Stremio add-ons, plugin scrapers, configured debrid services and media servers → provider-neutral stream results → source selection UI.

### Playback
Selected source → source resolver:
- Direct HTTP(S): validated URL + headers → MPV.
- Torrent info hash: torrent session → selected file → authenticated loopback HTTP Range endpoint → MPV.
- External provider: provider-specific resolver → validated playable target → MPV or explicitly supported external destination.

Subtitles are resolved and validated independently, then attached through MPV's track/subtitle interface.

### Personal data
Feature use cases → repositories → SQLite/local files/OS credential store. Optional account-backed sync is an integration with explicit reconciliation and offline behavior; local storage remains authoritative for local-only data.

## 7. Data and security boundaries

- Treat add-on manifests, plugin repositories, scraper code, stream responses, subtitle URLs, media-server responses and metadata as untrusted input.
- Validate schemes, redirects, DNS resolution, private/reserved destinations, response sizes, JSON shape, content type and timeouts.
- Prevent SSRF and DNS-rebinding bypasses. Do not allow arbitrary local-network destinations except narrowly scoped, user-configured server integrations.
- Torrent loopback endpoints must bind only to loopback, use unguessable session tokens, enforce Range semantics and close before temporary data deletion.
- Plugin execution must have explicit permissions, resource limits, cancellation and a narrow host API.
- Secrets must be stored in OS credential facilities where possible, redacted from logs, and never committed.
- The TMDB Read Access Token remains only in the hosted SENZO API environment. The desktop binary receives no shared TMDB secret.
- Keep provider failures isolated; one failed add-on must not block other sources or the UI.

## 8. Persistence model

Use SQLite with versioned migrations and repository boundaries for:
- Application and playback settings.
- Profiles and profile-scoped preferences.
- Installed add-ons and plugin repositories.
- Watch history, watched state and playback progress.
- Library, favorites, collections and user-defined ordering.
- Download metadata and task state.
- Cached metadata/artwork indexes where useful.

Large media, transient torrent payloads, logs and disposable image caches are stored outside SQLite. Secrets are stored separately. Migrations must preserve user data and reject unsupported future schema versions safely.

## 9. Licensing

Nuvio Desktop is GPL-3.0 and SENZO is GPL-3.0-only. Preserve upstream notices when adapting covered source. Audit each native/runtime dependency and ship required notices. See [LICENSING_POLICY.md](LICENSING_POLICY.md).
