# Phase 6 — Stremio add-on ecosystem

**Status:** Implementation complete; full live acceptance is running on Windows and Linux.  
**Branch:** `development`  
**Nuvio reference:** `NuvioMedia/NuvioDesktop` commit `65061186dde35fc0a0f90b05d380f2318133f227`.

## Scope and architecture

Phase 6 ports the Nuvio desktop Stremio add-on boundary into the native Rust workspace. The implementation is owned by `senzo-addons`, persisted by `senzo-storage`, composed by `senzo-app`, and exposed through the native GPUI Add-ons page in `senzo-ui`. It does not introduce a browser runtime, WebView, Electron, Tauri, or a bundled provider.

Stremio add-ons remain separate from the Nuvio QuickJS plugin/scraper runtime (Phase 7), from the provider-neutral source aggregation layer (Phase 8), and from torrent transport (Phase 13).

## Implemented deliverables

- [x] Parse and validate Stremio manifests, including string-form and object-form resources, global and resource-specific type/ID-prefix capabilities, catalog declarations, extra-property constraints, and behavior hints.
- [x] Normalize HTTPS, HTTP-with-explicit-user-approval, `stremio://` share links, and bare provider host inputs into validated manifest URLs.
- [x] Provide manifest preview and an explicit user confirmation before installation.
- [x] Keep configured manifest URLs (which can contain provider configuration tokens) in the platform credential store; persisted manifest JSON and UI labels do not expose those raw URLs.
- [x] Persist add-ons per profile in SQLite; support duplicate detection, ordered listing, reorder, enable, disable, refresh, and removal.
- [x] Preserve the previous valid manifest and enabled state when a refresh fails; surface the refresh error without destroying the working configuration.
- [x] Support Stremio catalogue, metadata, stream, and subtitle resources with capability checks and typed/validated response handling.
- [x] Support catalogue extras, including required fields, options, option limits, validation, bounded values, and protocol-compatible extra path segments.
- [x] Validate returned stream, external and subtitle URLs; validate torrent info-hash metadata without starting a torrent engine.
- [x] Enforce request timeouts, response-size limits, redirect limits, HTTPS downgrade rules, DNS resolution checks, public-destination checks, and DNS-pinned connections to reduce SSRF/rebinding exposure.
- [x] Keep provider work off the GPUI render path and isolate loading, operation, and refresh failures in the UI.
- [x] Keep Torrentio strictly user-installed and opt-in. No add-on is preinstalled or enabled by default.

## Validation added

- Unit coverage for malformed manifests, manifest size and URL validation, capability matching/inheritance, profile isolation, duplicate installs, enable/disable, ordering, secret redaction, catalogue extras, resource URL encoding, stream/subtitle protocol fields, private/reserved IP ranges, magnet metadata validation, and refresh-failure preservation.
- Live acceptance is configured to run on both Windows Server 2022 and Ubuntu 24.04. It verifies:
  - public Cinemeta manifest, movie catalogue and metadata;
  - public Torrentio manifest and stream resource response, installed only into an ephemeral test profile;
  - public OpenSubtitles v3 manifest and subtitle resource response, installed only into an ephemeral test profile.
- The live acceptance command runs only the explicitly ignored `live_*` tests. It does not install providers into a user's SENZO profile and does not bundle or enable Torrentio.

## Phase boundary

Phase 6 does not start torrent transport, playback, QuickJS plugins, parallel provider aggregation, debrid resolution, or subtitle file playback. Torrent streams and subtitles are validated and exposed as Stremio resource results; their integration into the source/playback pipeline belongs to later phases.

## Completion gate

**Pending live acceptance.** Do not mark Phase 6 complete or advance the roadmap until the Windows and Linux jobs pass the expanded live Cinemeta, Torrentio, and OpenSubtitles v3 acceptance tests together with the native Windows/Linux format, Clippy, workspace-test, build, and startup/shutdown checks.
