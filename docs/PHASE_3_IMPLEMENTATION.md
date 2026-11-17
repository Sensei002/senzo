# Phase 3 — Networking, metadata and artwork foundation

**Status:** In progress — the metadata API client passed cross-platform CI; the native artwork service is implemented and awaiting cross-platform CI acceptance. GPUI image presentation and discovery workflows remain.

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

- GPUI image-cache integration and image presentation in Home, Search and Details.
- Search input, result navigation and movie/TV details screens.
- Live hosted API health, search, details and artwork acceptance on Windows and Linux.
- Provider outage and offline UX in the shell.

## Artwork pipeline implementation

The new `senzo-artwork` crate implements the second Phase 3 slice:

- Resolves TMDB file paths to the fixed image CDN with typed size variants (w92 through w1280 and original); rejects arbitrary hosts, malformed paths, traversal, credentials, query strings and fragments.
- Downloads through the shared HTTP client with redirects disabled, preventing image requests from escaping the allowed origin.
- Bounds concurrent downloads, request time, compressed response bytes, decoded dimensions, total pixel count and decoder allocation.
- Requires an allowed image MIME type and independently verifies the image signature; supports JPEG, PNG and WebP only.
- Decodes validated images to RGBA8 for the native GPUI image adapter while retaining validated encoded bytes.
- Adds an LRU in-memory cache with a 64 MiB default budget and a persistent disk cache with a 512 MiB default budget and oldest-entry eviction.
- Supports cancellation, bounded transient retries, corrupt disk-entry cleanup and cache directory isolation.
- Exposes cache statistics and composes the service into the application-owned service graph. GPUI image rendering remains the next integration slice.

The artwork cache lives below the OS cache directory in `SENZO/artwork`; it is disposable and contains no credentials or database records.

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
