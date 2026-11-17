# SENZO rewrite roadmap

**Project target:** Nuvio Desktop feature parity, rewritten in Rust with GPUI CE and native MPV/libmpv.

**Platform target:** Windows first, Linux second. macOS is a future portability target. Android and iOS are excluded.

**Branch strategy:** `main` = stable releases; `development` = active rewrite.

A phase is complete only when its documented deliverables, automated checks, and relevant runtime acceptance criteria pass. CI success verifies the checks actually run; it does not prove native video presentation or end-to-end user workflows.

## Phase 0 — Source audit and architecture reset

<badge>Complete</badge>

- [x] Pin the Nuvio Desktop source snapshot and record its branch/commit.
- [x] Audit Nuvio repository structure and desktop-specific source sets.
- [x] Inventory shared and desktop feature modules.
- [x] Inspect add-on, stream, metadata, player, P2P, plugin and desktop platform boundaries.
- [x] Identify Nuvio Windows WebView2 player-controls dependency and explicitly exclude it.
- [x] Audit the current SENZO Rust prototype and classify it as legacy reference code.
- [x] Audit the existing SENZO metadata API without changing or redeploying it.
- [x] Audit GPLv3 licensing compatibility and correct GPL/AGPL documentation mismatch.
- [x] Produce the Nuvio source audit.
- [x] Produce the complete Nuvio-to-SENZO feature map.
- [x] Define the Rust workspace and native playback architecture.
- [x] Replace the old implementation roadmap with the full rewrite roadmap.
- [x] Publish Phase 0 completion record.

**Deliverables:** [Source audit](NUVIO_SOURCE_AUDIT.md), [feature mapping](NUVIO_TO_SENZO_MAPPING.md), [architecture](ARCHITECTURE.md), [product specification](PRODUCT_SPEC.md), [Phase 0 record](PHASE_0_COMPLETION.md).

## Phase 1 — Clean Rust workspace and technical foundation

<badge>Complete</badge>

- [x] Establish a clean Cargo workspace and crate boundaries.
- [x] Retire the legacy monolithic `src/main.rs` from the active application path; preserve Git history.
- [x] Pin GPUI CE and validate the native Windows and Linux startup paths.
- [x] Establish typed domain models, service interfaces and dependency composition.
- [x] Establish Tokio task ownership and GPUI-safe async state updates.
- [x] Establish structured errors, logging, diagnostics and credential redaction.
- [x] Establish SQLite repositories and transactional ordered migrations.
- [x] Establish secure credential storage per platform.
- [x] Establish shared HTTP policy and URL validation.
- [x] Establish CI for format, lint, test, Windows build and Linux build.
- [x] Create a minimal native shell that opens, navigates and shuts down cleanly.

**Gate:** Passed on Windows Server 2022 and Ubuntu 24.04 in [CI run #411](https://github.com/Sensei002/senzo/actions/runs/44081493351). Both native release builds, workspace checks and startup/render/shutdown smoke tests succeeded. Windows and Linux foundation artifacts were uploaded. The basic Home/Library/Settings navigation is implemented; automated smoke tests validate window startup/render/shutdown, while interactive route behavior will receive broader acceptance coverage in Phase 2.

**Completion record:** [Phase 1 completion](PHASE_1_COMPLETION.md).

## Phase 2 — Native GPUI application shell and design system

- [ ] Implement native window lifecycle, windowed/fullscreen modes and close controls.
- [ ] Implement persistent navigation, route stack, Back/Escape and focus restoration.
- [ ] Implement keyboard, pointer and supported mouse side-button navigation.
- [ ] Implement SENZO theme, typography, spacing, icons and reusable components.
- [ ] Implement dialogs, menus, notifications, loading, empty and error states.
- [ ] Integrate the canonical SENZO logo without altering the artwork.
- [ ] Implement accessibility and directional focus behavior.
- [ ] Establish screenshot/visual regression fixtures where practical.

**Gate:** Every primary route can be navigated without dead ends or loss of focus state.

## Phase 3 — Networking, metadata and artwork foundation

- [ ] Implement typed SENZO API client.
- [ ] Implement TMDB search, trending, popular, movie/TV details, external IDs, seasons and episodes.
- [ ] Implement credits, videos, images and enrichment data supported by the API.
- [ ] Implement artwork URL resolution, image decoding and GPUI image integration.
- [ ] Implement memory/disk caches with bounds and eviction.
- [ ] Implement request coalescing, cancellation, retry and timeout handling.
- [ ] Implement offline and provider-error states.
- [ ] Preserve TMDB credentials exclusively on the hosted API.

**Gate:** Search/details/artwork work from the native app with no desktop TMDB secret.

## Phase 4 — Home, catalogues and discovery

- [ ] Implement Home data orchestration and configurable sections.
- [ ] Implement cinematic hero and backdrop carousel.
- [ ] Implement catalog shelves, poster rails, collection rows and skeleton states.
- [ ] Implement Continue Watching and release-aware sections.
- [ ] Implement add-on-provided catalogues.
- [ ] Implement pagination, refresh and stable section identity.
- [ ] Implement focus/scroll preservation across routes.

**Gate:** Home is populated by real metadata and every hero/card opens the correct destination.

## Phase 5 — Search, details, seasons and episodes

- [ ] Implement movie/TV search, discover filters and search history.
- [ ] Implement movie, series and person detail views.
- [ ] Implement cast/crew, ratings, production, trailers and related titles.
- [ ] Implement season and episode browsing, specials and episode artwork.
- [ ] Implement episode identity and metadata mapping.
- [ ] Implement navigation between related media and back-stack restoration.

**Gate:** Movie and TV discovery flows work end-to-end without playback yet.

## Phase 6 — Stremio add-on ecosystem

- [ ] Implement manifest parsing and validation.
- [ ] Implement explicit install/preview/confirm/remove/enable/disable workflows.
- [ ] Implement profile-scoped add-on registry and ordering.
- [ ] Implement catalog, metadata, stream and subtitle resources.
- [ ] Implement resource/type/ID-prefix capability matching.
- [ ] Implement request transport, timeouts, response limits and safe redirects.
- [ ] Implement manifest refresh and failure isolation.
- [ ] Implement add-on configuration and catalog extras.
- [ ] Validate compatibility with real user-provided manifests.

**Gate:** User can install an add-on, browse its catalogue and retrieve streams/subtitles. Torrentio remains user-installed only.

## Phase 7 — Nuvio plugin/scraper runtime

- [ ] Audit Nuvio QuickJS plugin API and compatibility contracts in detail.
- [ ] Select a Rust QuickJS runtime binding.
- [ ] Implement plugin repository manifest parsing and management.
- [ ] Implement sandboxed execution, permission boundaries and resource/time limits.
- [ ] Implement plugin settings and profile-scoped state.
- [ ] Implement scraper test/execute workflows.
- [ ] Map plugin results into the provider-neutral stream model.
- [ ] Add fixtures and compatibility tests.

**Gate:** Plugin runtime works independently from Stremio add-ons and cannot block the UI.

## Phase 8 — Source aggregation and provider integrations

- [ ] Implement provider-neutral source adapter interface and registry.
- [ ] Implement concurrent source discovery and result grouping.
- [ ] Implement stream classification, parsing, headers and external targets.
- [ ] Implement sorting, filtering, badges and auto-selection policies.
- [ ] Implement source caching, fallback and retry.
- [ ] Implement debrid provider capability model and credentials.
- [ ] Port Torbox, Premiumize and applicable Real-Debrid behavior.
- [ ] Implement media-server provider connections, matching, catalogs and stream resolution.
- [ ] Implement source diagnostics and provider isolation.

**Gate:** Details/episode source lists show accurate results from multiple independent providers.

## Phase 9 — Native MPV playback proof and engine

<badge color="warning">Critical acceptance gate</badge>

- [ ] Design the Rust-owned MPV lifecycle and narrow command/event interface.
- [ ] Prototype MPV render-context integration with GPUI.
- [ ] Prototype native surface fallback only if GPUI compositing requires it.
- [ ] Prove visible video and audible audio on Windows before UI expansion.
- [ ] Prove visible video and audible audio on Linux before Linux playback support.
- [ ] Implement direct HTTP(S) playback with headers.
- [ ] Implement play/pause/seek/volume/mute/speed.
- [ ] Implement track enumeration and selection.
- [ ] Implement MPV state snapshots and useful diagnostics.
- [ ] Implement single-owner session teardown and stop acknowledgment.
- [ ] Test source replacement, route navigation, repeated sessions and application exit.
- [ ] Verify no UI-thread MPV calls, leaks, deadlocks or stop-triggered crashes.

**Gate:** Actual visible video, audible audio, working seek/pause, and crash-free stop/shutdown on the target OS. Decoder statistics alone do not pass.

## Phase 10 — Native player experience

- [ ] Implement immersive player screen with GPUI controls.
- [ ] Implement seek timeline, elapsed/remaining time and buffering.
- [ ] Implement control auto-hide and pointer/keyboard interaction.
- [ ] Implement audio/subtitle track menus.
- [ ] Implement external/add-on subtitles and subtitle appearance.
- [ ] Implement resize modes, fullscreen and window restoration.
- [ ] Implement chapters, skip controls and parental/stream information overlays.
- [ ] Implement source fallback and retry UX.
- [ ] Implement picture-in-picture where supported by the native platform.
- [ ] Implement playback errors and recovery.

**Gate:** Player workflows are usable without WebView or external control pages.

## Phase 11 — TV playback, progress and episode continuity

- [ ] Implement episode-specific source discovery and playback.
- [ ] Implement next/previous episode navigation.
- [ ] Implement autoplay-next behavior and preferences.
- [ ] Implement watched-state tracking and completion rules.
- [ ] Implement resume prompts and playback position persistence.
- [ ] Implement continue-watching enrichment and next-up ordering.
- [ ] Implement intro/recap/credits skip integrations where supported.
- [ ] Implement binge-group/source reuse where supported.

**Gate:** A series can be watched across episodes with correct progress and resume state.

## Phase 12 — Library, profiles, accounts and sync

- [ ] Implement local library, favorites, collections and list management.
- [ ] Implement profiles, profile switching, PIN and profile-scoped settings.
- [ ] Implement authentication and session lifecycle where required by Nuvio parity.
- [ ] Implement cloud library and settings/provider sync.
- [ ] Implement sync reconciliation, paging, conflict handling and retry.
- [ ] Implement local/offline behavior and account sign-out cleanup.
- [ ] Implement watched-state and progress sync with supported providers.

**Gate:** Local data remains safe across profile changes, sync failures and application restarts.

## Phase 13 — Downloads and P2P transport

- [ ] Implement download queue, progress, retry, cancel and local playback.
- [ ] Implement P2P consent and settings.
- [ ] Port Nuvio P2P engine behavior behind a Rust transport boundary.
- [ ] Evaluate/reuse Rust torrent transport only after protocol and behavior comparison.
- [ ] Implement selected-file streaming and HTTP Range support.
- [ ] Implement cache and engine statistics.
- [ ] Implement orderly cancellation, cleanup and crash recovery.
- [ ] Verify seek, buffering and shutdown with real sources.

**Gate:** Downloads and P2P do not corrupt playback or leave active temporary sessions.

## Phase 14 — Subtitles, audio and advanced playback preferences

- [ ] Implement subtitle provider/search integration.
- [ ] Implement subtitle matching and forced/SDH preferences.
- [ ] Implement subtitle styling, font, size, margin and delay.
- [ ] Implement audio language preferences and delay.
- [ ] Implement audio amplification/downmix controls where supported.
- [ ] Implement video decoder, renderer, HDR and image-processing preferences.
- [ ] Implement buffer controls with capability-aware defaults.
- [ ] Hide unsupported controls rather than presenting nonfunctional options.

**Gate:** Preferences persist and reflect actual backend capabilities.

## Phase 15 — Nuvio integrations and social features

- [ ] Implement Trakt authentication, library and watched-state sync.
- [ ] Implement Simkl authentication and sync.
- [ ] Implement MDBList lists, metadata and sync.
- [ ] Implement Discord Rich Presence with privacy controls.
- [ ] Implement watch-party create/join, presence, chat and playback sync.
- [ ] Implement membership/community desktop functionality where retained.
- [ ] Implement trailer extraction and platform-specific playback behavior.
- [ ] Implement media-server and debrid settings integration.
- [ ] Implement release notifications only where desktop support is enabled.

**Gate:** Each integration is independently configurable and failure-isolated.

## Phase 16 — Settings, diagnostics and updater

- [ ] Port general, appearance, navigation, player and integration settings.
- [ ] Implement typed settings models, defaults and migrations.
- [ ] Implement settings search and progressive disclosure.
- [ ] Implement diagnostic logs, redaction and support export.
- [ ] Implement update channels, metadata validation and integrity checks.
- [ ] Implement safe update staging, rollback and recovery.
- [ ] Implement About, licenses and third-party attribution.

**Gate:** Settings accurately represent supported features; updates preserve user data.

## Phase 17 — Performance, security and reliability

- [ ] Profile startup, memory, CPU, GPU and image decoding.
- [ ] Bound network concurrency, caches and background tasks.
- [ ] Audit add-on/plugin SSRF and sandbox boundaries.
- [ ] Audit credential handling and log redaction.
- [ ] Test malformed manifests, responses, subtitles and media URLs.
- [ ] Test database migration, corruption recovery and interrupted writes.
- [ ] Test repeated playback, route changes and shutdown.
- [ ] Test long-running sessions and memory/resource leaks.
- [ ] Add regression tests for all previously fixed defects.

**Gate:** No known critical data-loss, playback-lifecycle or security defects.

## Phase 18 — Cross-platform packaging and Nuvio parity release

- [ ] Complete Windows x64 CI, package and installer.
- [ ] Complete Linux CI and distribution packages.
- [ ] Bundle/document native MPV runtime dependencies and notices.
- [ ] Verify clean install, upgrade, uninstall and user-data retention.
- [ ] Verify application signing/update integrity where configured.
- [ ] Run full Nuvio feature-parity audit against the pinned mapping.
- [ ] Complete missing desktop-enabled features or document justified exclusions.
- [ ] Run Windows and Linux end-to-end acceptance suites.
- [ ] Publish SENZO 1.0 release notes and source.

**Gate:** Nuvio desktop parity inventory is closed or every remaining difference is explicitly documented and accepted.
