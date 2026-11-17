# SENZO rewrite architecture

## 1. Product and platform boundary

SENZO is a native desktop rewrite of Nuvio Desktop in Rust. GPUI CE is the only selected UI framework. The initial desktop targets are Windows and Linux; macOS remains a future portability target, not a release commitment. Android and iOS are out of scope.

SENZO does not use WebView2, Electron, Tauri, egui, or browser-hosted UI. Every application screen and player control is rendered by GPUI. MPV/libmpv is a media engine only.

The audited Nuvio source snapshot is pinned in NUVIO_SOURCE_AUDIT.md. The full feature mapping is in NUVIO_TO_SENZO_MAPPING.md.

## 2. Rewrite boundary

The former root src/main.rs implementation was a legacy prototype. It has been removed from the active branch; Git history preserves it for reference. It is not the architecture baseline. No legacy module is reused without review against the contracts in this document.

The rewrite preserves Nuvio desktop behavior and feature boundaries, not Kotlin syntax or platform-specific implementation accidents. The Windows Nuvio player bridge contains WebView2 code and is explicitly excluded.

## 3. Workspace ownership

| Crate | Responsibility |
|---|---|
| senzo-app | Binary entry point, dependency composition, application lifecycle and Tokio runtime ownership. |
| senzo-ui | GPUI application shell, routes and GPUI-safe async state updates. |
| senzo-domain | Typed media/settings/profile models and repository contracts. |
| senzo-metadata | Typed SENZO API client, TMDB-shaped response models, request coalescing and metadata response cache. |
| senzo-artwork | Validated TMDB artwork resolution, bounded download/decode, memory/disk caches and safe image data for GPUI. |
| senzo-storage | SQLite connection ownership, repositories and ordered transactional migrations. |
| senzo-platform | OS data/config/cache directories and native credential storage. |
| senzo-network | Shared HTTP client policy, timeouts, redirect limits and URL validation. |
| senzo-diagnostics | Structured tracing and redaction helpers for secrets, URLs and bearer tokens. |

Feature-specific crates are introduced when their phase begins. Avoid one crate per screen and avoid premature splitting of tightly coupled code.

## 4. Dependency and runtime policy

- UI: GPUI CE 0.2.2 with gpui_ce_platform 0.1.0; both are pinned.
- Toolchain: Rust 1.95.0, edition 2024.
- Async: One application-owned Tokio runtime; the UI receives a cloneable handle. Never block the GPUI event/render thread.
- HTTP: Reqwest with Rustls, shared timeouts, limited redirects and validated HTTP(S) URLs.
- Serialization: Serde and serde_json.
- Persistence: SQLite with versioned, ordered, transactional migrations.
- Secrets: Windows Credential Manager and Linux Secret Service/keyutils through keyring; never plaintext secrets in SQLite or logs.
- Playback: MPV/libmpv through a narrow Rust-owned interface, introduced in the playback phase.
- Build: Cargo and GitHub Actions. Windows and Linux CI are separate explicit jobs.

The UI must not call provider APIs, MPV, SQLite, or torrent internals directly. It sends typed commands to services and renders state/events.

## 5. Discovery data flow

GPUI view → typed catalog/search use case → senzo-metadata → hosted SENZO API → TMDB.

The metadata client uses fixed API routes and typed request methods. It validates search terms, media identifiers, page numbers and language tags before transport. Responses are size-limited and decoded into typed domain-facing models. Identical concurrent requests are coalesced, successful responses are cached with per-resource TTLs and a bounded LRU policy, and retry waits and response reads can be cancelled.

The hosted API remains the TMDB credential boundary. Remote API origins must use HTTPS; HTTP is accepted only for loopback development/test servers. The desktop client never sends a TMDB Authorization header. Artwork paths pass through the bounded senzo-artwork service before GPUI image presentation; raw provider paths are never treated as arbitrary image URLs. The UI converts validated RGBA8 pixels into BGRA RenderImage frames on Tokio's blocking pool and passes cached frames to GPUI's native img element. The UI never loads arbitrary remote image URLs directly.

## 6. Native playback architecture — critical gate

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

Before building the full player, the playback phase must prove actual visible video in a GPUI window on Windows. It must compare:
- MPV render API / mpv_render_context integration with GPUI's rendering/compositing path.
- A native OS video surface integrated into the GPUI layout, only if the first approach is unsuitable.

Do not assume a valid HWND, configured VO, decoded frames, or zero dropped frames means that the user can see video. Acceptance requires visible frames on screen, audible audio, seek, pause/resume, resize/fullscreen, repeated sessions, and clean stop/shutdown. The previous child-HWND implementation is not accepted as proven.

Linux playback must receive its own native presentation proof before the Linux UI is declared supported. Avoid designing the core player around Win32-only handles.

## 7. Data and security boundaries

- Treat add-on manifests, plugin repositories, scraper code, stream responses, subtitle URLs, media-server responses and metadata as untrusted input.
- Validate schemes, redirects, DNS resolution, private/reserved destinations, response sizes, content type and timeouts.
- Prevent SSRF and DNS-rebinding bypasses. Do not allow arbitrary local-network destinations except narrowly scoped, user-configured server integrations.
- Torrent loopback endpoints must bind only to loopback, use unguessable session tokens, enforce Range semantics and close before temporary data deletion.
- Plugin execution must have explicit permissions, resource limits, cancellation and a narrow host API.
- Secrets must be stored in OS credential facilities where possible, redacted from logs, and never committed.
- The TMDB Read Access Token remains only in the hosted API environment. The desktop binary receives no shared TMDB secret.
- Keep provider failures isolated; one failed add-on must not block other sources or the UI.

## 8. Persistence model

Use SQLite with versioned migrations and repository boundaries for:
- Application and playback settings.
- Profiles and profile-scoped preferences.
- Installed add-ons and plugin repositories.
- Watch history, watched state and playback progress.
- Library, favorites, collections and user-defined ordering.
- Cached metadata/artwork indexes where useful.

Large media, transient torrent payloads, logs and disposable image caches are stored outside SQLite. Secrets are stored separately. Migrations must preserve user data and reject unsupported future schema versions safely.

## 9. Licensing

Nuvio Desktop is GPL-3.0 and SENZO is GPL-3.0-only. Preserve upstream notices when adapting covered source. See LICENSING_POLICY.md.
