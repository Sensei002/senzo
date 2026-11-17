# Nuvio Desktop source audit

## 1. Audit baseline

This audit is pinned to a reproducible upstream source snapshot rather than a moving branch name.

| Item | Audited value |
|---|---|
| Repository | [NuvioMedia/NuvioDesktop](https://github.com/NuvioMedia/NuvioDesktop) |
| Branch | `Dev` |
| Commit | `65061186dde35fc0a0f90b05d380f2318133f227` |
| Commit date | 2026-11-17 source snapshot |
| Repository tree | 3,331 entries; not truncated by GitHub's tree response |
| README product status | Alpha software / testers only |
| Upstream license | GNU GPL version 3.0 |
| SENZO audit date | 2026-11-17 |

The upstream repository README describes Nuvio Desktop as a free, open-source media application for Windows, macOS, and Linux. Its shared application is Kotlin Multiplatform with Compose Multiplatform. This audit concerns the desktop product; Android and iOS source sets are used only to distinguish mobile-only behavior and are not SENZO implementation targets.

## 2. Repository and platform structure

The upstream source is split across several layers:

- `composeApp/src/commonMain`: shared product logic, domain models, repositories, and Compose UI.
- `composeApp/src/desktopMain`: JVM desktop implementations and desktop integrations.
- `composeApp/src/windowsDesktopMain`: Windows-specific desktop implementations.
- `composeApp/src/nonWindowsDesktopMain`: non-Windows desktop implementations.
- `composeApp/src/desktopTest`: desktop lifecycle and behavior tests.
- `composeApp/src/commonTest`: shared logic tests.
- `composeApp/src/desktopMain/native/windows`: Windows native player bridge source.
- `composeApp/src/desktopMain/native/linux` and `macos`: platform-native support.
- Root Android/iOS modules and source sets: excluded from the SENZO desktop port.

The desktop implementation is not just a single UI screen. It combines shared feature repositories with desktop-specific storage, network, player, native-window, updater, plugin, and P2P implementations.

## 3. Feature inventory

The following file counts are from the audited tree and count source files under each feature directory. They are an inventory aid, not a measure of feature complexity.

### Shared feature modules (`commonMain`)

| Feature | Files |
|---|---:|
| Add-ons | 9 |
| Authentication | 4 |
| Auto-sync | 20 |
| Catalog | 6 |
| Cloud library | 5 |
| Collections | 14 |
| Debrid | 18 |
| Details | 46 |
| Downloads | 8 |
| Home | 26 |
| Library | 26 |
| MDBList | 52 |
| Membership | 10 |
| Notifications | 5 |
| P2P | 3 |
| Watch party | 16 |
| Player | 109 |
| Plugins | 3 |
| Profiles | 20 |
| Search | 7 |
| Media servers | 18 |
| Settings | 83 |
| Shuffle | 6 |
| Simkl | 31 |
| Streams | 29 |
| TMDB | 8 |
| Tracking | 13 |
| Trailers | 6 |
| Trakt | 25 |
| Updater | 8 |
| Watched state | 8 |
| Watching/release state | 11 |
| Watch progress | 18 |

### Desktop feature modules (`desktopMain`)

The desktop source set has platform implementations for add-ons, auto-sync, collections, debrid, details, Discord Rich Presence, downloads, Home, library, MDBList, membership, notifications, P2P, player, plugins, profiles, search, media servers, settings, shuffle, Simkl, streams, TMDB, trailers, Trakt, updater, watched state, and watch progress.

The largest desktop-specific areas are the native player controller, player platform effects/settings, P2P streaming engine, plugin platform/runtime, add-on networking/storage, and updater. These are separate implementation responsibilities and must not be collapsed into one SENZO UI module.

## 4. Product feature findings

### Discovery and metadata
- Home is data-driven and combines configurable catalog sections, hero metadata, poster rails, collections, continue watching, and release-aware sections.
- Search has its own repository, history, discover selection, and pagination behavior.
- Details include movie and series metadata, cast, production, trailers, ratings, comments, season/episode content, and source availability.
- Catalog resources can come from installed add-ons as well as built-in metadata sources.
- TMDB is a central metadata integration, but Nuvio also has metadata and enrichment integrations.

### Add-ons and sources
- Stremio add-ons are managed through manifest parsing, a registry, resource/capability checks, and HTTP transport.
- Supported resource families include catalog, metadata, stream, and subtitle resources.
- Stream parsing handles direct URLs, torrent info hashes, file indexes, external links, YouTube identifiers, headers, and behavior hints.
- Stream discovery is coordinated separately from the UI and includes source grouping, filtering, badges, auto-selection policies, caching, and fallback behavior.
- The plugin system is distinct from Stremio add-ons. The desktop platform uses a QuickJS-based plugin runtime; SENZO must preserve this distinction rather than treating every extension as an add-on.
- Debrid support is modeled by provider and capability. The audited provider registry includes Torbox, Premiumize, and Real-Debrid; Real-Debrid is not shown in the normal UI in this snapshot.
- P2P streaming is a separate desktop engine integration, not merely a stream URL parser.

### Playback
- Shared player behavior is represented through a player-engine interface and a large set of player state, control, overlay, subtitle, audio, track, chapter, skip, episode, source-fallback, and party components.
- Desktop playback has a dedicated `PlayerEngine.desktop.kt`, `NativePlayerHost`, `NativePlayerController`, and `NativePlayerBridge`.
- The desktop controller contains explicit attach, replacement, release, teardown, timeout, and pre-navigation lifecycle handling. A desktop teardown test verifies that navigation does not complete before native player disposal.
- The Windows native implementation is `composeApp/src/desktopMain/native/windows/player_bridge.cpp`. It uses Win32, D3D11, JNI, and MPV APIs.
- **Important SENZO constraint:** this Windows bridge includes `<WebView2.h>`; the Kotlin bridge's runtime dependency list includes `WebView2Loader.dll`, and the player controller refers to an embedded controls WebView. This is a dependency of Nuvio's native player-controls implementation, not evidence that the entire Nuvio UI is a WebView.
- SENZO must not port that WebView2 path. All SENZO player controls must be rendered by GPUI. MPV remains the playback engine behind a Rust-owned native integration.

### Personal data and integrations
The audited desktop feature inventory includes authentication, profiles, cloud library, local library, collections, watch progress, watched state, downloads, media-server connections, watch parties, tracking integrations, MDBList, Trakt, Simkl, debrid providers, Discord Rich Presence, membership/community pages, and in-app updating.

The upstream desktop feature policy reports:
- Enabled: plugins, downloads, supporter/contributor page, donation actions, P2P, in-app updater, custom server connections.
- Disabled: desktop notifications, account deletion, personal-media add-on copy, donation progress, external-player support, playback foreground service, and download foreground service.
- Windows uses external trailer playback; non-Windows desktop uses in-app trailer playback. Hero trailer playback is disabled on Windows and enabled on non-Windows desktop.

These platform gates matter: shared source files do not automatically mean every feature is enabled on every desktop OS.

## 5. SENZO legacy repository audit

The existing `Sensei002/senzo` development branch at the start of Phase 0 was:

- Branch head: `fe071a5fba69caddcfc76d5bc4ff254e8be12eb6`.
- Rust 2024 single-package application using `gpui-ce = 0.2.2`.
- Existing dependencies include Tokio, Reqwest, Serde, SQLite, libloading, Windows APIs, and `librqbit`.
- Existing modules include add-ons, artwork/cache, diagnostics, Discord, metadata, MPV configuration/runtime/worker, settings, sources, storage, subtitles, torrent handling, and torrent transport.
- `src/main.rs` is approximately 361 KB and contains the bulk of the UI/application implementation.
- Existing Windows CI formats, checks, tests, builds, packages MPV, and uploads a Windows artifact.
- The existing `senzo-api` repository is a private Next.js service with `/api/health` and a validated `/api/tmdb/*` route family. Its TMDB token is read server-side from `TMDB_READ_ACCESS_TOKEN`. Phase 0 makes no API deployment changes.

The existing prototype has useful research material, including Stremio manifest/stream handling, metadata API calls, SQLite persistence, torrent transport, and MPV worker code. However, the current native playback path has not passed the user's Windows visual-video acceptance: previous runtime reports showed audio without visible video and a Stop-triggered crash. It is therefore a **legacy reference**, not a proven playback foundation. No module is automatically approved for reuse; every candidate must pass the new architecture boundary and tests.

## 6. Licensing and attribution findings

- Nuvio Desktop's root license is GNU GPL version 3.0.
- SENZO's root `LICENSE` is byte-identical to the audited Nuvio root license text.
- SENZO's `Cargo.toml` declares `GPL-3.0-only`.
- SENZO's existing licensing document and Windows CI package filename incorrectly used AGPL terminology. Phase 0 corrects those inconsistencies to GPL-3.0-only.
- A code port derived from Nuvio must preserve applicable upstream copyright and license notices, include required notices, and comply with GPLv3 when distributing covered derivative work. Do not copy Nuvio artwork, logos, or unrelated third-party assets.
- The authoritative license text is the repository-root `LICENSE`; this audit is not legal advice.

## 7. Audit conclusions

1. The rewrite target is the **Nuvio desktop product**, not the Android/iOS applications.
2. Nuvio's feature scope is much broader than the current SENZO prototype. Account/profile/cloud and social features must not be silently discarded under the old accountless-only SENZO specification.
3. Stremio add-ons, Nuvio plugins, debrid integrations, media servers, and P2P are distinct subsystems with separate trust and lifecycle boundaries.
4. The player must be redesigned independently. Nuvio's Windows bridge is not a suitable code base for SENZO because it contains WebView2; the previous SENZO child-window approach is also not accepted until visible video is proven.
5. GPUI CE remains the selected Rust UI framework. All user-facing controls remain native GPUI components.
6. Phase 1 must create a clean Rust workspace and establish a playback proof-of-concept before the rewrite expands into the full UI.

## 8. Source links

- [Nuvio Desktop README](https://github.com/NuvioMedia/NuvioDesktop/blob/65061186dde35fc0a0f90b05d380f2318133f227/README.md)
- [Nuvio desktop build](https://github.com/NuvioMedia/NuvioDesktop/blob/65061186dde35fc0a0f90b05d380f2318133f227/composeApp/build.gradle.kts)
- [Nuvio add-on repository](https://github.com/NuvioMedia/NuvioDesktop/blob/65061186dde35fc0a0f90b05d380f2318133f227/composeApp/src/commonMain/kotlin/com/nuvio/app/features/addons/AddonRepository.kt)
- [Nuvio stream repository](https://github.com/NuvioMedia/NuvioDesktop/blob/65061186dde35fc0a0f90b05d380f2318133f227/composeApp/src/commonMain/kotlin/com/nuvio/app/features/streams/StreamsRepository.kt)
- [Nuvio player engine contract](https://github.com/NuvioMedia/NuvioDesktop/blob/65061186dde35fc0a0f90b05d380f2318133f227/composeApp/src/commonMain/kotlin/com/nuvio/app/features/player/PlayerEngine.kt)
- [Nuvio desktop native player controller](https://github.com/NuvioMedia/NuvioDesktop/blob/65061186dde35fc0a0f90b05d380f2318133f227/composeApp/src/desktopMain/kotlin/com/nuvio/app/features/player/desktop/NativePlayerController.kt)
- [Nuvio Windows native player bridge](https://github.com/NuvioMedia/NuvioDesktop/blob/65061186dde35fc0a0f90b05d380f2318133f227/composeApp/src/desktopMain/native/windows/player_bridge.cpp)
- [Nuvio desktop feature policy](https://github.com/NuvioMedia/NuvioDesktop/blob/65061186dde35fc0a0f90b05d380f2318133f227/composeApp/src/desktopMain/kotlin/com/nuvio/app/core/build/AppFeaturePolicy.desktop.kt)
