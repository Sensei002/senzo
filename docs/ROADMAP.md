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

The Settings experience will follow Nuvio's simple, user-friendly approach: clear everyday categories, plain-language labels, sensible defaults, and progressive disclosure for technical controls. TMDB and OpenSubtitles are built-in integrations in official builds; ordinary users must not need to supply API keys. Advanced diagnostics and technical playback controls stay out of the main settings flow.

- [x] Build main SENZO navigation rail with compact fixed-height pills, cinematic hero carousel, and poster shelves.
- [x] Build Home welcome and onboarding hero with zero-credential SENZO metadata API discovery and add-on quick actions.
- [ ] Add reusable poster, backdrop, title, and detail components.
- [x] Build Home, Search, Library, Details, Player, Add-ons, and Settings screens with permanent sidebar access.
- [ ] Add a Settings diagnostics section with a Copy Logs action.
- [ ] Add keyboard navigation, focus behavior, and accessible controls.
- [ ] Apply the exact official SENZO transparent logo throughout the application.
- [ ] Add TMDB logo and required attribution in About/Credits.

## Phase 4 — Metadata, discovery, and local library

- [x] Replace the legacy desktop TMDB token prompt with the hosted SENZO metadata API; the desktop no longer stores or requests a TMDB token (Batch 8.4).
- [x] Connect TMDB popular catalogues, clipboard-assisted movie/TV search, and title detail requests to GPUI screens.
- [x] Add TMDB TV season/episode detail endpoints and typed response models (UI browsing remains pending).
- [x] Add TMDB movie/TV external-ID lookup models and endpoints (source-provider integration remains pending).
- [x] Add bounded local TMDB artwork disk caching with HTTPS-only host validation, 8 MiB per-image limit, 128 MiB total budget, and GPUI decoded-image cache integration; bounded metadata caches are tracked separately below.
- [ ] Add continue-watching, watch history, favorites, and local library views.
- [ ] Add safe cache cleanup that preserves personal history and settings.

## Phase 3 — Playback engine

- [x] Build Add-ons UI with clipboard-assisted manual manifest URL entry and explicit install confirmation.
- [x] Add manifest preview/validation and add-on enable/disable/remove controls (provider reordering deferred).
- [x] Implement Stremio catalog, metadata, stream, and subtitle resource routing.
- [x] Add direct HTTP(S), torrent-info-hash, and external-link source classification with user-controlled direct-stream selection.
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
- [ ] Validate visible video output on Windows with real direct and user-installed Torrentio streams; confirm MPV `vo-configured`, decoded dimensions, and frame counters.
- [x] Add an immersive Player destination that hides the discovery navigation rail while media is playing, uses a black edge-to-edge playback canvas, expands the native video viewport to consume available player space, and provides an explicit Back action to return to the previous browsing context.
- [ ] Validate immersive player sizing and native child-HWND alignment on Windows at multiple window sizes and display scale factors; verify Back restores the previous screen without stopping playback.
- [x] Force MPV's embedded window mode for caller-owned HWND playback and disable MPV's standalone OSC/default key bindings in the embedded surface.
- [x] Raise the native child video surface to the top of its sibling z-order whenever its viewport is resized.
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
- [x] Surface live MPV video diagnostics in the playback panel: codec, decoder, active hardware decoder, video output/configuration state, video format, source/output dimensions, estimated frame rate, decoder drops, renderer drops, and mistimed frames; log changes to the diagnostic snapshot without recording stream URLs or credentials.
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
- [x] Add a clipboard-assisted manifest URL preview and explicit install confirmation flow to the Add-ons screen.
- [x] Add Torrentio-style manifest and stream-response fixture compatibility tests without bundling or enabling Torrentio.
- [x] Connect Stremio catalog browsing, TMDB-backed title details, stream discovery, and user-selected direct-source MPV playback into the discovery UI (TV source lookup currently uses S01E01; richer season/episode selection is a follow-up).

## Phase 6 — Torrent playback transport (Batch 5)

- [x] Add a pinned, license-reviewed librqbit engine behind a SENZO-owned transport boundary (Apache-2.0 dependency; upload support compiled out).
- [x] Accept validated BitTorrent v1 info hashes and optional tracker sources from user-installed Stremio add-ons; never bundle or enable a streaming add-on by default.
- [x] Enumerate torrent files, honor Stremio `fileIdx`, and let the user select a video file when no valid file index is supplied.
- [x] Stream the selected file to MPV over an authenticated loopback-only HTTP endpoint with HTTP Range/seek support; rqbit's file stream provides on-demand piece scheduling.
- [x] Create isolated per-session temporary storage, remove it on normal shutdown, and recover abandoned session directories at next startup.
- [x] Stop MPV and the torrent engine before deleting session payloads; terminate the session-owned runtime on orderly exit and recover abandoned sessions after crashes.
- [x] Keep playback progress and watch history in SQLite, separate from temporary torrent data.
- [x] Surface live downloaded/total/uploaded byte counts, engine state, and engine errors in the UI without blocking playback; peer-count detail remains a follow-up because the current stats API does not expose a stable peer-count field. Peer-count detail remains a follow-up because the current engine stats API does not expose a stable peer-count field.
- [x] Include the upstream Apache-2.0 license text and attribution notice for the embedded `librqbit` engine.
- [x] Build the Windows x64 release executable in CI with the torrent engine statically linked, and upload a package containing the executable, MPV runtime, branding asset, and license notices.
- [ ] Manually validate real peer discovery, buffering, seeking, and orderly shutdown on Windows hardware before release.

## Phase 7 — Console-first media home and discovery experience (Batch 6)

**Batch 6 implementation scope is delivered in the current development line.** The Home screen is now the default media-discovery surface, with the first native GPUI implementation of Nuvio's hero-and-shelf browsing model. SENZO adapts the upstream interaction patterns; it does not embed Kotlin/Compose code or Nuvio assets.

- [x] Make Home the default landing screen and primary browsing experience.
- [x] Build a cinematic featured hero from real TMDB catalog data, with backdrop, title, year, media type, rating, overview, and a Details action.
- [x] Add an eight-second hero rotation, previous/next controls, direct indicator selection, and a user pause/resume control. Rotation pauses while the user is outside Home.
- [x] Add keyboard support for hero left/right navigation and Escape to return from Details or close Advanced Settings.
- [x] Make shared action chips and media poster cards keyboard tab stops with click activation.
- [x] Build horizontally scrolling poster shelves for Trending, Popular Movies, Popular TV, and enabled user-installed add-on catalogs.
- [x] Add View All navigation for built-in TMDB shelves and dynamic add-on catalog shelves.
- [x] Show Continue Watching only when local in-progress media exists, with saved progress and a resume action.
- [x] Support shelf visibility and ordering, persisted locally.
- [x] Add artwork fallbacks, readable title/year/rating metadata, partial-provider status, a discovery retry action, and a clear no-results hero state.
- [x] Connect poster and hero Details actions to the existing metadata/detail flow; add-on shelf View All routes to the existing catalog browser.
- [x] Keep add-on shelves sourced only from enabled, explicitly user-installed manifests; no add-on is bundled or enabled by default.
- [x] Keep simple Settings categories separate from technical Advanced Settings and preserve local persistence for user preferences.
- [x] Update the public README, roadmap, and UI design specification to reflect the implemented Home experience.

**Batch 6 boundary:** this is the first complete Home/Discovery and Settings UX milestone, not a claim that every later media-center feature is finished. Full directional focus movement between every shelf/card, game-controller support, rich pointer-hover video previews, persistent scroll/focus restoration, and bounded image/catalog caching remain post-milestone enhancements. They must be implemented and tested before a final production-readiness claim, but do not block beginning the Stremio ecosystem batch.

**Nuvio reference:** NuvioMedia/NuvioDesktop on branch Dev, especially HomeRepository.kt, HomeHeroSection.kt, HomePosterCard.kt, and HomeContinueWatchingSection.kt. Nuvio's eight-second hero rotation and data-driven Home composition informed this native GPUI implementation. Preserve SENZO branding and the upstream license/attribution requirements for any future code reuse.

## Phase 8 — Stremio add-on ecosystem (Batch 7)

**Batch 7 delivers the core add-on management and protocol-compatibility milestone.** It builds on the existing manifest/catalog/meta/stream/subtitle services and Batch 5 torrent transport, with Nuvio's native add-on management concepts used as a product reference rather than copying Kotlin/Compose code.

- [x] Honor Stremio resource-level and manifest-level media types and ID prefixes before requesting catalogs, metadata, or streams.
- [x] Validate manifest resource/catalog declarations and bound catalog extra-property keys, values, and counts.
- [x] Discover compatible stream providers concurrently while isolating each provider's failure.
- [x] Preserve path-configured manifest URLs, including user-configured Torrentio URLs; allow multiple configurations of the same provider to be installed independently.
- [x] Add explicit manifest refresh while preserving the configured URL, enabled state, installation date, and provider identity.
- [x] Add persistent provider ordering with Move Up/Move Down controls and a versioned SQLite migration that preserves existing installations.
- [x] Surface configuration-required provider hints and never guess required catalog filter values.
- [x] Keep Torrentio and every other streaming add-on absent by default; only explicitly installed and enabled providers are queried.
- [x] Add fixture/unit coverage for resource types, ID prefixes, configured Torrentio instance identity, catalog extras, provider ordering, and schema upgrade.
- [x] Add a native catalog-filter editor for required/optional extra fields, including option limits and clipboard-backed free-text values.
- [x] Add catalog pagination using Stremio's `skip` extra parameter, with previous/next page controls.
- [x] Discover subtitle tracks across all enabled compatible add-ons, including providers using either Stremio's `subtitle` or `subtitles` resource name.
- [x] Integrate add-on subtitle results into the native player through bounded HTTPS downloads and the existing native MPV subtitle-file command.
- [x] Add safe HTTPS external-link handoff through the Windows default browser without shell command construction.
- [ ] Add runtime validation with real user-configured Torrentio manifests and live provider responses on Windows.

**Batch 7 boundary:** The planned native add-on ecosystem implementation is complete and Windows CI-verified. It covers Stremio manifest/catalog/meta/stream/subtitle requests, catalog filters and pagination, multi-provider stream and subtitle discovery, direct/external/torrent source classification, and Torrentio-style BitTorrent v1 handoff. This is not a claim of universal Stremio parity: provider-specific configuration screens beyond configured manifest URLs, BitTorrent v2, and live provider/runtime validation remain follow-ups. Live provider validation requires the user to install their own provider configuration; no provider is bundled or enabled by default.

**Nuvio reference:** AddonModels.kt, AddonManifestParser.kt, AddonRepository.kt, AddonTransportUrls.kt, and AddonsScreen.kt in NuvioMedia/NuvioDesktop (branch Dev). SENZO adapts the product and protocol concepts to Rust/GPUI and its local-first security model; no Kotlin/Compose source or Nuvio assets are copied.

## Phase 9 — Native Playback & Media Experience (Batch 8)

Batch 8 turns the existing MPV worker integration into a dedicated, usable native player. The Player destination owns the native video viewport and presents essential transport, track, subtitle, and diagnostics controls without routing ordinary playback through Advanced Settings.

- [x] Deliver the dedicated Player screen with a native MPV video viewport and child-window visibility/lifecycle handling.
- [x] Connect seek, play/pause, stop, volume, mute, fullscreen, audio-track, and subtitle-track controls to the worker.
- [x] Add keyboard playback controls: Space, Left/Right, Up/Down, M, F, and Escape.
- [x] Preserve selected TMDB/Stremio media identity across direct-stream and torrent handoff; persist metadata-backed titles and media types with progress.
- [x] Resume metadata-backed network streams from their saved position after MPV confirms the loaded URL and duration; Continue Watching returns to title details and source selection.
- [x] Record completion-aware watch history atomically and remove completed items from Continue Watching.
- [x] Detect the active MPV renderer and hardware decoder, inspect source/output transfer functions, query Windows Advanced Color state for every active display, and expose manual renderer/decoder/HDR overrides only when the live runtime confirms support.
- [x] Complete Settings → Diagnostics with Copy Logs and Open Log Folder; exports remain bounded and exclude personal history/database contents.
- [ ] Validate native MPV video presentation, fullscreen behavior, track switching, seeking, buffering, and shutdown on Windows hardware.

**Batch 8 boundary:** Runtime capability detection is implemented conservatively: unavailable or unknown values are never treated as supported. HDR passthrough is offered only when GPU Next is the active renderer and Windows Advanced Color is enabled on every active display; tone mapping is offered only with an active GPU renderer. Decoder and renderer overrides are shown only after MPV reports the corresponding active path. CI validates code, tests, and packaging, but native video presentation, display targeting on multi-monitor systems, track switching, seeking, buffering, and shutdown still require real Windows hardware validation.

**Nuvio reference:** PlayerScreen.kt, PlayerControls.kt, PlayerTimeline.kt, PlayerTrackSelection.kt, PlayerMediaInfo.kt, and PlayerLoadingStatus.kt in NuvioMedia/NuvioDesktop (branch Dev). Adapt interaction patterns and user-facing behavior to Rust + GPUI and native libmpv; do not port Kotlin/Compose UI code.

## Phase 9.1 — Experience and Discovery Remediation (Batch 8.1)

**Status: In progress.** Windows user testing exposed gaps that were not covered by compile-time CI: the installed UI did not yet match the approved SENZO media-center design, Torrentio manifest preview failed, and Home discovery had no zero-configuration metadata service. Batch 8.4 replaces the original per-user TMDB credential path with the hosted SENZO API.

- [x] Add a direct Torrentio standard-manifest preview action, independent of clipboard behavior.
- [x] Keep configured Torrentio URLs supported through the existing Paste manifest URL flow.
- [x] Show the actual manifest request / validation error in the Add-ons screen while keeping user-supplied URLs out of diagnostic logs.
- [x] Provide a direct Home empty-state path for metadata discovery; the former credential prompt is superseded by the hosted SENZO API in Batch 8.4.
- [x] Redesign the real GPUI Home and navigation UI to match the approved SENZO visual reference (media-center navigation rail, cinematic hero carousel with Play/More Info pills, dot/arrow controls, shelf headers with vertical cyan accent bars, View All routes, and 2:3 cinematic poster cards).
- [x] Establish larger cinematic hero proportions, stronger title hierarchy, larger poster cards, and on-artwork rating badges in native GPUI shelves.
- [x] Implement polished hero transitions, carousel motion, and shelf interaction animations.
- [x] Improve Home discovery error recovery, preserve catalog state during transient refresh failures, and add media-center keyboard navigation (1-7, h, /).
- [ ] Verify Torrentio installation and stream discovery on Windows with the standard and configured manifest URLs.
- [ ] Verify Home recommendations through the hosted SENZO API on Windows hardware.
- [ ] Validate the complete experience on Windows before starting Batch 9.

**Boundary:** This remediation does not bundle Torrentio or any other streaming provider. Its original user-supplied TMDB credential flow is superseded by Batch 8.4; the desktop now calls the hosted SENZO API. CI success alone does not mark these user-facing tasks complete.

## Phase 9.2 — TV Seasons, Episodes & Metadata Caching (Batch 8.2)

**Status: Implementation complete; Windows CI verified. Runtime acceptance remains pending on Windows hardware.** This batch closes the TV-details gap left by the first discovery implementation. TMDB remains the metadata source of record; add-ons remain the source of playable streams.

- [x] Replace automatic S01E01 source lookup with user-selected season and episode browsing.
- [x] Display TMDB-listed seasons, including Specials (season 0), and load episode lists on demand.
- [x] Show episode-specific synopsis, still artwork, air date, runtime, and rating.
- [x] Request Stremio streams using the selected episode's IMDb/TMDB provider identifier and exact season/episode numbers.
- [x] Cache TMDB title details, external IDs, and season responses locally with language-aware keys and TTL expiry.
- [x] Cache official TMDB artwork locally with HTTPS host validation, an 8 MiB per-image limit, and a 128 MiB total cache budget.
- [x] Keep cache files separate from personal settings, playback progress, and watch history.
- [x] Verify the Windows build, tests, Windows x64 release build, and package assembly in GitHub Actions.
- [ ] Validate season switching, Specials, episode-specific source discovery, cache hits, and offline/error behavior on Windows.

**Boundary:** TMDB season numbering is passed through to compatible Stremio providers without guessing or silently remapping provider-specific numbering. Cache contents are disposable and may be pruned; they are never treated as user library data. Runtime provider correctness still depends on the metadata numbering matching the installed add-on's expectations.

**Nuvio reference:** NuvioMedia/NuvioDesktop series/episode detail and player episode-stream selection flows. SENZO uses the native Rust + GPUI presentation and its own TMDB/Stremio/MPV service boundaries.

## Phase 9.3 — Native Artwork Pipeline (Batch 8.3C)

**Status: Implementation pushed; Windows CI verification pending.** The initial artwork cache existed, but the GPUI application had not registered the custom HTTP client that connects remote image elements to the secure TMDB artwork loader. As a result, valid TMDB poster URLs could remain blank.

- [x] Register the secure TMDB artwork HTTP client with GPUI at application startup.
- [x] Serve cached artwork bytes to GPUI and fetch cache misses asynchronously over HTTPS from the official TMDB image host.
- [x] Enforce the 8 MiB per-image response limit and reject empty, failed, and non-success HTTP responses.
- [x] Cache only successful complete image responses; report cache-write failures without breaking the current image response.
- [x] Preserve the bounded 128 MiB disk cache and existing GPUI image decoding/rendering path for posters, backdrops, and episode stills.
- [ ] Verify posters, backdrops, and episode stills render on Windows across Home, Search, Details, and TV episode screens.
- [ ] Verify cache hits on relaunch, cache miss downloads, offline placeholders, and recovery after a failed request.

**Boundary:** The artwork client is restricted to HTTPS on image.tmdb.org; it does not fetch arbitrary user-supplied image URLs. Artwork is disposable cache data and does not contain personal watch history or credentials.

## Phase 9.4 — Hosted Metadata API & Zero-Configuration OOBE (Batch 8.4)

**Status: Implementation pushed; API and Windows CI verification pending.** SENZO now uses the hosted SENZO API as its only TMDB metadata gateway. The TMDB Read Access Token stays in the Vercel server environment; ordinary desktop users do not need a TMDB account, API token, or setup step.

- [x] Add allowlisted SENZO API routes for trending, popular movies/TV, movie/TV search, enriched details, external IDs, TV seasons, and individual TV episodes.
- [x] Route the native Rust metadata client through https://senzo-api.vercel.app/api/tmdb over HTTPS.
- [x] Remove TMDB token storage and masked token prompts from the desktop Settings and discovery onboarding.
- [x] Remove SENZO_TMDB_READ_ACCESS_TOKEN injection from the desktop Windows CI build.
- [x] Add a zero-credential first-run experience and a metadata-service status/retry path.
- [x] Keep TMDB artwork downloads on the official image.tmdb.org host through the existing secure GPUI artwork client.
- [x] Add unit coverage requiring an HTTPS SENZO metadata API base URL.
- [ ] Verify the expanded API routes and API repository CI after the latest route changes.
- [ ] Verify desktop Windows CI, including formatting, compilation, tests, release build, and package assembly.
- [ ] Validate first launch, Home recommendations, movie/TV search, title details, credits/videos, external IDs, TV seasons/episodes, artwork, and retry/offline behavior on Windows.
- [ ] Remove the obsolete SENZO_TMDB_READ_ACCESS_TOKEN secret from the desktop repository after confirming no remaining workflow references it.
- [ ] Add shared rate limiting and abuse monitoring to the public API before broad distribution; the current per-instance in-memory rate limiter/cache are best-effort only.

**Boundary:** The desktop is a public client and contains no shared TMDB credential. The API exposes fixed read-only routes, not an arbitrary URL proxy. Its public URL is not authentication; CORS is not access control. Vercel deployment health and successful TMDB search have been manually verified, but the expanded endpoint set and desktop runtime behavior still require validation. TMDB attribution and terms review remain required before public distribution.
## Phase 10 — Additional sources and presence (Batch 9)

- [x] Define a stable source-adapter interface and route the discovery UI through a provider registry; the Stremio adapter preserves the existing user-installed add-on behavior.
- [ ] Add more authorized source adapters without changing metadata or playback layers. Do not add a bundled/default provider; the dedicated Direct Sources provider system remains a later roadmap item.
- [ ] Add Discord Rich Presence, playback-state updates, and privacy controls. The opt-in IPC worker and title-sharing privacy control are implemented; Windows CI and Discord application-ID configuration remain to be verified.

## Phase 11 — Windows distribution

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
