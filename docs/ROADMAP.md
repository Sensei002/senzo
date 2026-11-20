# SENZO rewrite roadmap

**Project target:** Nuvio Desktop feature parity, rewritten in Rust with GPUI CE and native MPV/libmpv.

**Platform target:** Windows first, Linux second. macOS is a future portability target. Android and iOS are excluded.

**Branch strategy:** main = stable releases; development = active rewrite.

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

**Deliverables:** Source audit, feature mapping, architecture, product specification and Phase 0 record.

## Phase 1 — Clean Rust workspace and technical foundation

<badge>Complete</badge>

- [x] Establish a clean Cargo workspace and crate boundaries.
- [x] Retire the legacy monolithic src/main.rs from the active application path; preserve Git history.
- [x] Pin GPUI CE and validate the native Windows and Linux startup paths.
- [x] Establish typed domain models, service interfaces and dependency composition.
- [x] Establish Tokio task ownership and GPUI-safe async state updates.
- [x] Establish structured errors, logging, diagnostics and credential redaction.
- [x] Establish SQLite repositories and transactional migrations.
- [x] Establish secure credential storage per platform.
- [x] Establish shared HTTP policy and URL validation.
- [x] Establish CI for format, lint, test, Windows build and Linux build.
- [x] Create a minimal native shell that opens, navigates and shuts down cleanly.

**Gate:** Passed on Windows Server 2022 and Ubuntu 24.04 in CI run #411. Both native release builds, workspace checks and startup/render/shutdown smoke tests succeeded. The Home, Library and Settings shell paths are implemented.

**Completion record:** docs/PHASE_1_COMPLETION.md.

## Phase 2 — Native GPUI application shell and design system

<badge color="success">Complete</badge>

- [x] Implement native window lifecycle, windowed/fullscreen modes and close controls.
- [x] Implement persistent navigation, route stack, Back/Escape and focus restoration.
- [x] Implement keyboard, pointer and supported mouse side-button event handling.
- [x] Implement SENZO theme, typography, spacing, icons and reusable components.
- [x] Implement dialogs, menus, notifications, loading, empty and error states.
- [x] Integrate the canonical SENZO logo without altering the artwork.
- [x] Implement accessibility and directional focus behavior.
- [x] Establish deterministic structural visual-regression fixtures using rendered geometry at 1024×680, 1280×720 and 1920×1080. The pinned GPUI CE 0.2.2 test-support API does not expose pixel-frame readback, so geometry/input fixtures are used instead of unsupported screenshot baselines.

**Automated gate:** The GPUI interaction test exercises all seven destinations, directional wraparound, Back/Forward, Escape, dialog dismissal and focus restoration. It clicks the rendered Movies navigation control and dispatches mouse Navigate Back/Forward events. The Phase 2/4 close-out acceptance also checks viewport containment for the sidebar, Home hero, page viewport and refresh control at the three supported sizes, plus F11/Alt+Enter fullscreen state transitions. Windows and Linux formatting, Clippy, workspace tests, native release builds and startup/render/shutdown smoke tests are required by the close-out CI.

**Completion boundary:** Phase 2's implementation and automated acceptance are complete. Physical HID side-button delivery and final human visual/interactive review on actual Windows and Linux desktops are tracked as Phase 18 release checks; no unimplemented Phase 2 code blocks Phase 5. See docs/PHASE_2_IMPLEMENTATION.md.



## Phase 3 — Networking, metadata and artwork foundation

<badge>Complete</badge>

- [x] Implement typed SENZO API client for the existing hosted route contract.
- [x] Implement search, trending, popular, movie/TV details, external-ID, season and episode request methods.
- [x] Add typed credits, videos, images and enrichment response models.
- [x] Implement request coalescing, cancellation, bounded JSON responses, retry and timeout handling.
- [x] Implement bounded metadata response caching with TTL and LRU eviction.
- [x] Implement TMDB artwork URL resolution with fixed size variants and strict host/path validation.
- [x] Implement bounded, cancellation-aware image downloads with redirects disabled.
- [x] Validate MIME type and image signatures; decode JPEG/PNG/WebP under dimension, pixel and allocation caps.
- [x] Implement bounded in-memory LRU and persistent disk cache with oldest-entry eviction.
- [x] Render decoded RGBA artwork as native GPUI BGRA RenderImage frames.
- [x] Add reusable native poster and backdrop components with loading/unavailable placeholders.
- [x] Connect the Home trending, popular movie and popular TV shelves to real hosted metadata and the bounded artwork service.
- [x] Keep metadata requests, artwork downloads and pixel conversion off the GPUI render thread.
- [x] Add native hosted-metadata Search with All/Movies/TV filters, debounce, request cancellation, poster artwork and quick preview.
- [x] Implement dedicated movie/TV Details workflows, including TV season and episode navigation.
- [x] Implement dedicated person Details workflow, including profile, biography, personal facts and movie/TV filmography.
- [x] Implement explicit retry and offline/provider-outage UX across Home, Search and Details.
- [x] Complete live Preview acceptance of the person profile and combined-credits route from SENZO API PR #1 on the protected `development` Preview.
- [x] Verify live hosted Home, Search, movie/TV/person details and artwork behavior on Windows and Linux using the compiled native client.
- [x] Preserve TMDB credentials exclusively on the hosted API.

**Completion gate — passed:** CI run [#505](https://github.com/Sensei002/senzo/actions/runs/44244248291) passed all Windows and Linux native jobs, both protected live hosted API acceptance jobs, and repository automation validation. Both platforms passed formatting, Clippy, workspace tests, release builds, native GPUI startup/render/shutdown smoke tests, and the compiled native metadata/artwork smoke test. Native acceptance verified all three Home catalogues; multi/movie/TV Search; Movie and TV details with credits, videos, images and external IDs; TV season and episode; Person Details with combined filmography; and a real TMDB poster download/decode (342 × 513 JPEG). The live hosted acceptance jobs also passed on Windows and Linux. The Preview's server-side `TMDB_READ_ACCESS_TOKEN` is confirmed by `/api/health` (`tmdbConfigured: true`) and successful live TMDB-backed requests. The person endpoint remains in SENZO API `development` via [API PR #1](https://github.com/Sensei002/senzo-api/pull/1); production promotion/merge is a separate release decision and was not needed for this Preview acceptance. No `main` branch or production deployment was changed.

See docs/PHASE_3_IMPLEMENTATION.md.

**Phase 3 completion record:** November 18, 2026 — all automated and live Preview acceptance gates passed in [SENZO Desktop CI run #505](https://github.com/Sensei002/senzo/actions/runs/44244248291). Phase 4 can now continue from its existing first-slice baseline.

## Phase 4 — Home, catalogues and discovery

<badge color="success">Complete — built-in Home and catalogue scope</badge>

- [x] Implement first-slice Home layout preferences: show/hide the three built-in catalogue sections, reorder shelves, toggle the featured hero, and persist preferences locally.
- [x] Implement a native featured carousel that pulls backdrops from up to two enabled built-in catalogue sources, with keyboard-focusable wraparound navigation and a title-specific Details action.
- [x] Implement built-in Home catalogue shelves, horizontal poster rails, loading skeletons, empty/error states, and title-specific Details navigation.
- [x] Implement custom Home section titles with validation and local persistence.
- [x] Implement built-in shelf pagination, explicit Home refresh, bounded page/item limits, duplicate-safe page merging and stable media-based card identity.
- [x] Implement the provider-neutral catalogue descriptor schema and stable built-in request/cache identities, including revision-aware stale-response protection.
- [x] Implement the validated provider-definition registry and built-in TMDB adapter.
- [x] Implement independently cancellable initial loads and incremental publication of each built-in shelf, with isolated errors and hero ordering independent of response completion.
- [x] Implement stable provider/media-type/catalogue/revision descriptors and reject initial/page results whose descriptor identity is no longer active.
- [x] Implement reusable native collection rows and a versioned, bounded local Home collection-settings projection.
- [x] Implement per-page scroll-offset preservation and stable Home card/featured-action focus restoration across Back/Forward, sidebar navigation, and title/person details return.
- [x] Implement cinematic hero and backdrop carousel with user-selected built-in catalogue sources (up to eight items, wraparound controls).
- [x] Record and assign later-phase boundaries for add-on catalogue execution, playback-progress shelves and collection authoring to their owning phases.

**Automated slice acceptance:**
- Featured carousel and persisted Home layout: [CI run #510](https://github.com/Sensei002/senzo/actions/runs/44248395951).
- Built-in pagination, refresh and stable card identity: [CI run #515](https://github.com/Sensei002/senzo/actions/runs/44253403129).
- Editable shelf titles: [CI run #517](https://github.com/Sensei002/senzo/actions/runs/44254968935).
- Independent built-in shelf loading and ordered hero publication: [CI run #519](https://github.com/Sensei002/senzo/actions/runs/44256725204).
- Provider-neutral descriptor schema and stale-response identity guards: [CI run #523](https://github.com/Sensei002/senzo/actions/runs/44259302648).
- Route-aware Home scroll/focus preservation: [CI run #527](https://github.com/Sensei002/senzo/actions/runs/44261770211).
- Provider-definition registry, persisted collection rows, pointer-click and mouse Navigate Back/Forward interaction tests, and Windows/Linux native build/smoke gates: [CI run #535](https://github.com/Sensei002/senzo/actions/runs/44271228972).
- Responsive Home geometry and window-mode regression checks: included in the Phase 2/4 close-out CI.

**Gate — passed for Phase 4's scope:** Home is populated by real built-in TMDB metadata and every built-in hero/card opens the correct Details destination. Catalogue loading, pagination/refresh, error isolation, local layout/title persistence, the bounded collection-row projection, stable identities and route-aware scroll/focus behavior are implemented and automated-accepted.

**Explicit later-phase hand-offs (not Phase 4 blockers):**
- **Phase 6:** Stremio add-on manifest/resource/capability contracts and actual user-installed add-on catalogue execution. No add-on is bundled or enabled by default.
- **Phase 11:** watched state and playback-progress domain, then real Continue Watching and release-aware Home shelves. No fabricated progress is shown.
- **Phase 12:** collection/folder creation, edit/delete, ordering and Library ownership; Phase 4 consumes the validated local projection only.
- **Phase 18:** actual hardware mouse side-button tests and final human visual/interactive review on native Windows/Linux desktops.

See docs/PHASE_4_IMPLEMENTATION.md for the slice-by-slice implementation and acceptance record.



## Phase 5 — Search, details, seasons and episodes

<badge color="success">Complete</badge>

- [x] Implement movie/TV search, discover filters and search history.
- [x] Implement movie, series and person detail views.
- [x] Implement cast/crew, ratings, production, trailers and related titles.
- [x] Implement season and episode browsing, specials and episode artwork.
- [x] Implement episode identity and metadata mapping.
- [x] Implement navigation between related media and back-stack restoration.

**Completion gate — passed:** [SENZO Desktop CI run #569](https://github.com/Sensei002/senzo/actions/runs/44364656528) passed all six jobs, including Windows and Linux native formatting, Clippy, workspace tests, release builds, startup/render/shutdown smoke tests, live hosted API acceptance on both platforms, repository automation validation, and Cargo.lock housekeeping. Playback remains explicitly outside Phase 5; it is addressed by later playback phases.

See docs/PHASE_5_IMPLEMENTATION.md.

## Phase 6 — Stremio add-on ecosystem

- [x] Implement manifest parsing and validation.
- [x] Implement explicit install/preview/confirm/remove/enable/disable workflows.
- [x] Implement profile-scoped add-on registry and ordering.
- [x] Implement catalog, metadata, stream and subtitle resources.
- [x] Implement resource/type/ID-prefix capability matching.
- [x] Implement request transport, timeouts, response limits and safe redirects.
- [x] Implement manifest refresh and failure isolation.
- [x] Implement add-on configuration and catalog extras.
- [x] Validate compatibility with live public Stremio providers on Windows and Linux.

**Gate:** Passed in [CI run #616](https://github.com/Sensei002/senzo/actions/runs/44401216814). Windows Server 2022 and Ubuntu 24.04 native checks and live Cinemeta, Torrentio, and OpenSubtitles v3 acceptance all passed. Torrentio remains user-installed only.

**Implementation report:** [docs/PHASE_6_IMPLEMENTATION.md](PHASE_6_IMPLEMENTATION.md).

## Phase 7 — Nuvio plugin/scraper runtime

- [x] Audit Nuvio QuickJS plugin API and compatibility contracts in detail.
- [x] Select a Rust QuickJS runtime binding.
- [x] Implement plugin repository manifest parsing and management.
- [x] Implement sandboxed execution, permission boundaries and resource/time limits.
- [x] Implement plugin settings and profile-scoped state.
- [x] Implement scraper test/execute workflows.
- [x] Map plugin results into the provider-neutral stream model.
- [x] Add fixtures and compatibility tests.

**Gate:** Passed. Native Windows/Linux acceptance, plugin compatibility fixtures, and automation validation are green in [CI run #642](https://github.com/Sensei002/senzo/actions/runs/44477224036). Plugin work runs outside the GPUI render path and remains separate from the Phase 6 Stremio add-on subsystem.

**Implementation report:** [docs/PHASE_7_IMPLEMENTATION.md](PHASE_7_IMPLEMENTATION.md).

## Phase 8 — Source aggregation and provider integrations

**Status: Complete**

- [x] Implement provider-neutral source adapter interface and registry.
- [x] Implement concurrent source discovery and result grouping.
- [x] Implement stream classification, parsing, headers and external targets.
- [x] Implement sorting, filtering, badges and auto-selection policies.
- [x] Implement source caching, fallback and retry.
- [x] Implement debrid provider capability model and credentials.
- [x] Port Torbox, Premiumize and applicable Real-Debrid behavior.
- [x] Implement media-server provider connections, matching, catalogs and stream resolution.
- [x] Implement source diagnostics and provider isolation.

**Gate — passed:** The native movie and episode Details workflow is connected to the provider-neutral aggregator. Results from independent adapters are normalized, grouped and rendered in the source list; adapter failures are isolated and reported without discarding successful providers. Automated tests verify aggregation across multiple independent providers and failure isolation.

**Acceptance:** [SENZO Desktop CI #661](https://github.com/Sensei002/senzo/actions/runs/44500080651) passed Windows and Linux native formatting, Clippy, workspace tests, release builds, native startup/shutdown smoke tests, repository automation validation and Cargo.lock housekeeping.

**Implementation report:** [docs/PHASE_8_IMPLEMENTATION.md](PHASE_8_IMPLEMENTATION.md).

**Phase boundary:** Source discovery and debrid resolution prepare normalized direct sources for Phase 9's native MPV playback engine. Actual playback, source handoff, torrent transport and downloads are not Phase 8 deliverables. Live authentication against real third-party accounts is configuration-dependent; CI does not store or use personal provider credentials.

## Phase 9 — Native MPV playback proof and engine

<badge color="success">Complete — automated playback acceptance</badge>

**Status:** Native MPV engine and GPUI source handoff complete. Linux headless audiovisual acceptance and Windows/Linux native CI gates passed; physical-device output checks are deferred to Phase 18.

- [x] Design the Rust-owned MPV lifecycle and narrow command/event interface.
- [x] Prototype and document MPV render-context feasibility against the pinned GPUI/WGPU stack; retain the native MPV-owned output-window fallback because there is no supported shared-texture import path in the pinned GPUI release.
- [x] Implement the native MPV-owned output-window fallback. The current fallback is a separate native video window, not an embedded GPUI child surface.
- [x] Prove real libmpv video decode/render and audio output through generated media, Xvfb frame capture and a PulseAudio virtual sink in Linux CI.
- [x] Pass Windows native release build and application startup/shutdown acceptance; physical Windows output is a Phase 18 release check.
- [x] Implement direct HTTP(S) playback with bounded request headers.
- [x] Implement play/pause/seek/volume/mute/speed controls and native GPUI actions.
- [x] Implement audio/subtitle track enumeration and selection.
- [x] Implement MPV state snapshots, change events and bounded diagnostics.
- [x] Implement a single-owner MPV worker, stop acknowledgement and ordered teardown.
- [x] Wire source replacement and route-navigation stop requests; add a local acceptance command for repeated sessions and clean shutdown.
- [x] Keep MPV initialization, loading and controls off the GPUI event thread; use a background reaper for final-owner teardown.
- [x] Add a Linux headless libmpv audiovisual acceptance run with generated media, a virtual X11 display, frame-motion checks and PulseAudio PCM capture.

**Automated AV acceptance:** CI builds and runs `crates/senzo-player/examples/headless_acceptance.rs` against locally generated H.264/AAC HTTP fixtures served by the range-capable test server. It uses the production `MpvPlayer` owner-thread API to verify stream decode, playback progress, audio-track selection, pause/seek/resume, volume/mute/speed, source replacement, stop, repeated playback and clean shutdown. Xvfb frame capture is sampled at 1 fps and checked for visible content and frame-to-frame motion; PulseAudio monitor capture is checked for non-silent PCM. The complete diagnostics are uploaded as a workflow artifact.

**Current limitation:** GPUI CE 0.2.2 uses its own WGPU render path. SENZO does not yet ship a working shared-GPU render-context bridge from WGPU into libmpv. The implemented fallback lets libmpv own its native output window; it does not render into an inline GPUI rectangle.

**Manual device acceptance:** Build/run with libmpv available (`mpv-2.dll`/compatible DLL on Windows or `libmpv.so.2`/compatible library on Linux). Set `SENZO_LIBMPV_PATH` when the library is not in the platform loader path, then run `senzo --mpv-smoke-test <direct-http-or-https-media-url>` with a video source that contains audio. The command validates video dimensions, audio-track metadata, controls, source replacement, a second playback session, stop acknowledgement and owner-thread shutdown. During the run, a human must confirm that the native video window is actually visible and audio is actually audible on each OS.

**Gate — passed:** [SENZO Desktop CI #684](https://github.com/Sensei002/senzo/actions/runs/44557774636) passed Linux headless libmpv audiovisual runtime acceptance plus Windows and Linux native builds, workspace checks and startup/shutdown smoke tests. The generated fixture produced decoded 640×360 H.264 video and AAC audio; captured virtual output contained 4 visible frames among 12 samples (maximum frame difference 3.21) and 5.49 seconds of non-silent stereo PCM. Physical Windows/Linux monitor-and-speaker checks are explicitly moved to Phase 18 and are not claimed as completed.

## Phase 10 — Native player experience

<badge color="success">Development implementation complete</badge>

- [x] Implement immersive GPUI player controls and a full-screen control plane over the existing native MPV video window.
- [x] Implement a seek timeline, elapsed/remaining time and cache-buffering state.
- [x] Implement five-second control auto-hide, pointer reveal and player-scoped keyboard controls.
- [x] Implement native audio/subtitle track menus, including explicit subtitle disable.
- [x] Implement external HTTP(S) subtitle loading, previously fetched Stremio add-on subtitle entries, visibility, font-size and timing controls.
- [x] Implement Fit/Fill/Stretch/Zoom modes, native MPV fullscreen and window restoration on player exit.
- [x] Implement chapter navigation, ±10-second skip controls and stream/content information overlays. Show an explicit unavailable state when age-classification data is not supplied; never infer a parental rating from the metadata score.
- [x] Implement direct-source selection, immediate load-command fallback to the next candidate, manual next-source and retry actions, and playback error states.
- [x] Implement a best-effort native always-on-top, borderless, scaled output-window mode for picture-in-picture on platforms/window managers that support those MPV window hints.
- [x] Implement playback failure messaging and recovery actions.

**Gate — passed for the implemented native-window architecture:** Player controls, timeline, menus, chapter/skip actions, source recovery and subtitle operations are rendered natively with GPUI. No WebView or external control page is used.

**Architecture boundary:** The GPUI layer is the immersive control plane. The video picture remains in libmpv's separate native output window; it is not composited into a GPUI rectangle because the pinned GPUI CE/WGPU renderer has no supported libmpv shared-texture import route. PiP is a best-effort native window mode rather than a desktop-shell-managed system PiP API.

**Known metadata/subtitle limitations:** The current metadata contract does not contain parental content-age classifications, so the player explicitly displays that the rating is unavailable. Subtitle appearance controls currently cover visibility, font size and delay; font family, colour, outline, background, and other advanced styling remain a later preference-scope improvement. Add-on subtitle entries are offered when a subtitle search has already populated the add-on subtitle results; subtitle discovery is not yet automatically run for each playback item.

**Implementation report:** [docs/PHASE_10_IMPLEMENTATION.md](PHASE_10_IMPLEMENTATION.md).

## Phase 11 — TV playback, progress and episode continuity

<badge color="success">Windows development acceptance complete</badge>

- [x] Implement episode-specific source discovery and playback.
- [x] Implement next/previous episode navigation, including adjacent-season transitions and unreleased-episode guards.
- [x] Implement autoplay-next behavior and locally persisted preferences.
- [x] Implement watched-state tracking, explicit watched markers and the 90% completion rule.
- [x] Implement resume prompts and durable playback-position persistence.
- [x] Implement Continue Watching enrichment and next-up ordering.
- [x] Project real watched/resume progress into Home Continue Watching and release-aware Up Next shelves (consumes the Phase 4 Home collection/shelf contracts).
- [x] Implement intro/recap/credits chapter skipping where the selected MPV source exposes named chapters.
- [x] Preserve Stremio `behaviorHints.bingeGroup` and prefer the same group for next-episode playback before provider and discovery-order fallbacks.

**Automated gate — passed:** Windows CI runs source-normalization, storage and UI Phase 11 regression tests in addition to the full workspace checks, native release build and application startup/shutdown smoke test. Tests cover metadata/source arrival ordering, exact episode identity before autoplay, binge-group priority and fallback ordering, completion-threshold filtering, manually watched episodes without existing progress, replay clearing a watched marker, database reopen persistence, and transactional watched/progress behavior.

**Implementation report:** [docs/PHASE_11_IMPLEMENTATION.md](PHASE_11_IMPLEMENTATION.md).

**Explicit boundaries:** Intro/recap/credits skipping uses MPV chapter metadata when supplied by a source; no remote IntroDB/SkipIntro service is queried by the desktop player. Binge-group continuity auto-selects matching direct HTTP sources; torrent/P2P transport remains Phase 13 and unsupported source kinds continue to require their appropriate manual/provider flow. Profile/cloud watched-state synchronization remains Phase 12. Physical Windows monitor/speaker and final human visual acceptance remain Phase 18 release checks.

## Phase 12 — Library, profiles, accounts and sync

<badge color="success">Implementation complete · Windows regression gate enabled</badge>

- [x] Implement local library, favorites, collections and list management.
- [x] Implement collection creation, editing, deletion, membership and ordering; publish the validated local projection consumed by Phase 4 Home rows.
- [x] Implement profiles, active-profile switching, startup/switch PIN protection and profile-scoped settings/provider state.
- [x] Implement optional email/password authentication, refresh and sign-out lifecycle through the SENZO API.
- [x] Implement authenticated cloud Library, collection, profile-preference, watched-state and playback-progress synchronization.
- [x] Implement cursor-based append-only sync pages, transactional local reconciliation, timestamp conflict handling, idempotent push and retry/backoff.
- [x] Implement local/offline behavior: mutations remain in SQLite until accepted; sign-out clears the profile's local cloud session without deleting local Library or watch history.
- [x] Implement watched-state and playback-progress sync across signed-in SENZO devices.

**Automated gate:** Windows CI runs the Phase 12 profile-scope, Library/collection, PIN lockout, profile settings, outbox retry, remote-page reconciliation, cloud-client validation and Home projection regression tests, in addition to all workspace checks and the native Windows build/smoke test.

**Cloud activation required:** The account/sync API and Supabase SQL migration are committed in [Sensei API](https://github.com/Sensei002/senzo-api). Cloud sign-in/sync remain disabled until the project owner applies `supabase/migrations/0001_senzo_sync.sql` to their Supabase project and configures `SUPABASE_URL` and `SUPABASE_ANON_KEY` in the Vercel project. No service-role key is embedded in SENZO or exposed to the desktop. The local/offline Library works without this setup.

**Credential boundary:** Add-on URLs/configuration tokens, plugin secrets and media-server passwords remain in the local OS credential store and are not uploaded as sync payloads. Profile-scoped provider state is isolated locally; only non-secret profile preferences and supported Library/watch entities are cloud-synced.

**Implementation report:** [docs/PHASE_12_IMPLEMENTATION.md](PHASE_12_IMPLEMENTATION.md).

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
- [ ] Verify native MPV playback on a physical Windows desktop: visible moving video, audible audio, pause/seek and clean stop/application teardown.
- [ ] Verify native MPV playback on a physical Linux desktop/display server: visible moving video, audible audio, pause/seek and clean stop/application teardown.
- [ ] Verify physical mouse side-button Back/Forward delivery with real hardware on Windows and Linux.
- [ ] Conduct human visual/interactive review of windowed/fullscreen behavior, Home hero/shelves and common display sizes/scales on real Windows and Linux desktops.
- [ ] Publish SENZO 1.0 release notes and source.

**Gate:** Nuvio desktop parity inventory is closed or every remaining difference is explicitly documented and accepted.


**Current sequencing (November 20, 2026):** Phases 0–11 are implemented on development. Phase 12 owns Library, profiles and sync; Phase 13 owns download/P2P transport; Phase 18 owns final cross-platform packaging, physical-device playback/visual acceptance and release. Stremio add-ons remain user-installed, Nuvio scrapers remain a separate sandboxed runtime, and source aggregation remains separate from playback transport.