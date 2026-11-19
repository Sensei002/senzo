# Phase 8 — Source aggregation and provider integrations

**Status:** Complete  
**Branch:** development  
**Source implementation tested by:** [CI #661](https://github.com/Sensei002/senzo/actions/runs/44500080651)  
**Tested source commit:** 54b839ba0017dde8cddb7fe1d97ffb8ab5988f82  
**Post-run lockfile commit:** 4ec24f9655802a9ad2d895baf889c7589a3101c6

## Delivered

### Provider-neutral source pipeline

- Added the senzo-sources crate and a SourceAdapter contract. The application composition root registers Stremio add-on, Nuvio scraper and media-server adapters with the shared SourceAggregator.
- Connected source discovery to the native movie and episode Details flows. Discovery runs outside the GPUI render path; the UI can find or refresh sources without blocking interaction.
- Run independent adapters concurrently with a bounded provider semaphore, per-provider timeout and one retry. A failing adapter does not discard successful results from other adapters.
- Normalize HTTP, torrent, external and YouTube targets while retaining supported provider metadata: titles, filenames, quality, size, language, codec, seeders/peers, request headers and subtitles.
- Validate source targets and metadata, bound headers/subtitles/URL sizes, group and de-duplicate normalized results, and generate badges only from known metadata.
- Add deterministic Recommended, Quality, Seeders and Size sorting; direct/torrent/external filters; and Disabled, Recommended, Highest Quality, Cached First and Highest Seeders selection policies.
- Add per-adapter diagnostics for elapsed time, returned source count, cache origin and failures. The Details source panel shows provider groups and reports isolated provider failures.

### Cache, fallback and retry

- Fresh source results are cached for 45 seconds.
- The bounded in-memory cache retains up to 128 request entries.
- If every adapter fails and the current request has a prior successful result, the aggregator can return that stale result for up to 10 minutes.
- A source refresh bypasses the fresh cache and retries provider discovery.

### Debrid providers

- Added provider identity and capability contracts for Torbox, Premiumize and Real-Debrid.
- Implemented account validation, connect/disconnect, masked account state and profile-scoped credentials through the platform credential-store abstraction. API keys are not stored in SQLite.
- Implemented Torbox cached-hash checks, cached-only torrent creation, file selection and download-link resolution.
- Implemented Premiumize cache checks and transfer/directdl resolution with explicit video-file matching.
- Implemented applicable Real-Debrid behavior: add magnet, inspect torrent files, select the exact matching file, wait for its generated link and unrestrict it. Real-Debrid cache availability remains Unknown because this provider adapter does not advertise the same hash-cache check capability.
- Preserve explicit filename, file-index and season/episode hints when choosing a torrent file; do not silently substitute a different episode when an exact episode hint cannot be satisfied.
- Added native Settings controls to connect and disconnect each optional debrid account. A torrent source can be resolved through any configured provider.

### Jellyfin and Emby media servers

- Added Jellyfin and Emby connection management with provider-appropriate authorization headers.
- Exchange server username/password for an access token, then store the token in the operating-system credential store. Persist connection metadata and library selections in the profile-scoped settings store.
- Implement connection validation, library refresh, library browsing, catalogue search, provider-ID/title/year matching, TV season/episode matching, playback-info resolution and stream URL construction.
- Preserve the authentication headers and external subtitle stream metadata needed by the later native player.
- Isolate a failed server lookup so another configured server can still return a result.
- Added native Settings controls to connect and remove media-server connections. Connected server results are included in movie and episode source discovery.

## Resource and safety bounds

- Source discovery: at most 48 registered adapters and 12 concurrently active provider calls.
- Per-provider discovery timeout: 28 seconds, with one bounded retry.
- Fresh cache: 45 seconds; stale fallback: 10 minutes; maximum cache entries: 128.
- Returned stream processing: bounded per provider; header count, individual header size, subtitle count and URL length are capped.
- Debrid and media-server HTTP clients use request/connect timeouts, disable automatic redirects and enforce bounded response-body reads.
- Private/LAN media-server addresses are accepted only as explicitly user-configured hosts; embedded URL credentials and fragments are rejected.
- Provider API tokens and media-server access tokens are stored through the OS credential-store interface, not in the application database.

## Regression coverage

The workspace tests include:
- Source normalization and classification for supported target types.
- Stremio behavior-hint, header and file-index preservation.
- Grouping across independent providers, deterministic sorting/filtering/selection, bounded header parsing and episode request validation.
- An asynchronous aggregator test with two successful independent providers and one failing provider, asserting that successful sources remain grouped and the failed provider is diagnosed.
- Debrid capability declarations, magnet/hash validation, Premiumize cache-item construction, exact filename/episode/file-index selection and profile-scoped credential namespaces.
- Jellyfin and Emby authorization-header construction, provider parsing, safe user-configured server URLs, title/year matching and profile-scoped connection namespaces.
- Native Settings and Details source-list wiring, plus workspace compilation and startup/shutdown smoke coverage.

The source, debrid and media-server integration crates passed their workspace tests on both Windows and Linux in CI #661.

## Acceptance evidence

[CI #661 — successful run](https://github.com/Sensei002/senzo/actions/runs/44500080651)

- Windows native desktop: formatting, Clippy with warnings denied, workspace tests, release build and GPUI startup/shutdown smoke test — passed.
- Linux native desktop: formatting, Clippy with warnings denied, workspace tests, release build and GPUI startup/shutdown smoke test — passed.
- Repository automation validation — passed.
- Cargo.lock housekeeping — passed.
- The live hosted API and live Stremio jobs were skipped by workflow conditions. They are not Phase 8 acceptance gates; Phase 6's live Stremio acceptance was separately completed in [CI #616](https://github.com/Sensei002/senzo/actions/runs/44401216814).

## External-service verification boundary

CI intentionally does not contain personal debrid API keys or a user's Jellyfin/Emby credentials, and this run did not perform authenticated live-account acceptance against those services. The provider implementations, credential flows, request/response handling and source aggregation are covered by repository code and automated tests, but a real account must still be connected in SENZO Settings to verify that account's current permissions, server contents, subscription state and provider-side availability. This is an environment-specific smoke check, not a claim that CI logged into user accounts.

## Phase boundaries

Phase 8 discovers, normalizes, filters, groups and resolves sources. A resolved direct URL is prepared for the native playback service; actual MPV handoff and visible/audio playback belong to Phase 9. P2P transport, selected-file streaming and download lifecycle belong to Phase 13.