# Phase 3 — Networking, metadata and artwork foundation

**Status:** In progress — typed hosted-API client and metadata response cache implemented; artwork and native discovery workflows remain.

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

## Acceptance status

Automated validation is pending the first Windows and Linux CI run for this commit. Phase 3 is not complete until Search, details and artwork work from the native app without a desktop TMDB secret and the documented runtime checks pass.
