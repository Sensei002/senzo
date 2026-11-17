# Phase 3 — Networking, metadata and artwork foundation

**Status:** In progress — the typed metadata API client has passed cross-platform CI; artwork and native discovery workflows remain.

## Scope

Phase 3 creates the native data path:

SENZO GPUI → typed metadata client → hosted SENZO API → TMDB

The hosted API remains the only TMDB credential boundary. The desktop application must not contain a TMDB Read Access Token, API key or other shared provider secret.

## Implemented in the first slice

- Added the senzo-metadata workspace crate and composed its client into the application service graph.
- Added typed responses for API health, search results, trending/popular catalogues, movie and TV details, external IDs, credits, videos, images, seasons and episodes.
- Implemented every read-only metadata route currently exposed by the hosted senzo-api service.
- Validated API base URLs: remote endpoints require HTTPS; cleartext HTTP is accepted only for loopback development and tests.
- Validated IDs, page ranges, search text and language tags before sending requests.
- Added bounded JSON response reads and typed HTTP/API/decode/cancellation errors.
- Added a bounded least-recently-used in-memory response cache with per-resource TTLs.
- Added in-flight request coalescing so identical concurrent requests share one GET.
- Added bounded retries for transient transport and upstream failures; 429 responses are retried only when Retry-After is short.
- Added cancellation-aware requests, response-body reads and retry delays.
- Added local HTTP test coverage proving the route shape and absence of Authorization headers; the test uses a loopback server and does not depend on a live TMDB account.
- Added the SENZO_API_BASE_URL development override. The default is https://senzo-api.vercel.app.

## Not yet implemented

- Artwork URL resolution, download limits, content-type and format validation, and image decoding.
- Bounded memory and disk artwork caches with eviction.
- GPUI image-cache integration and image presentation in Home, Search and Details.
- Search input, result navigation and movie/TV details screens.
- Live hosted API health, search, details and artwork acceptance on Windows and Linux.
- Provider outage and offline UX in the shell.

## First-slice automated acceptance

The metadata-client foundation passed [CI run #431](https://github.com/Sensei002/senzo/actions/runs/44098712863) on commit 072ff362e84c37b6ca45f439cad39c8fe562ab73.

- Windows Server 2022: formatting, Clippy, workspace tests, native release build and GPUI startup/render/shutdown smoke test passed.
- Ubuntu 24.04: formatting, Clippy, workspace tests, native release build and GPUI startup/render/shutdown smoke test passed.
- Both development binaries were uploaded:
  - senzo-windows-x64-development-072ff362e84c37b6ca45f439cad39c8fe562ab73
  - senzo-linux-x64-development-072ff362e84c37b6ca45f439cad39c8fe562ab73
- CI regenerated and committed Cargo.lock in commit 66df5a3c75281fc65698e7f69a7fa94b19736cf8.

These checks validate the service foundation and native application build. They do not claim that the hosted API was live-tested or that Search, details and artwork are already wired into the UI.

## Remaining Phase 3 acceptance

Phase 3 is not complete until artwork is downloaded, decoded and cached safely, and metadata-backed search/details workflows operate from the native app without a desktop TMDB secret. Offline and provider-error states also need runtime acceptance on Windows and Linux.
