# Phase 3 — Networking, metadata and artwork foundation

**Status:** In progress — metadata and artwork-service CI passed on Windows and Linux. Native GPUI Home artwork and a hosted-metadata Search integration are implemented on development; this application integration is awaiting Windows/Linux CI and live hosted-service acceptance. Dedicated Details and offline/provider-error workflows remain.

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

- Dedicated movie and TV Details screens, including metadata enrichment and season/episode navigation.
- Live hosted API health, Search and Details acceptance on Windows and Linux.
- Provider outage and offline UX in the shell.
- Full Nuvio Search parity: recent-search persistence, discover catalogues, pagination and add-on/server aggregation.


## Artwork pipeline implementation

The new `senzo-artwork` crate implements the second Phase 3 slice:

- Resolves TMDB file paths to the fixed image CDN with typed size variants (w92 through w1280 and original); rejects arbitrary hosts, malformed paths, traversal, credentials, query strings and fragments.
- Downloads through the shared HTTP client with redirects disabled, preventing image requests from escaping the allowed origin.
- Bounds concurrent downloads, request time, compressed response bytes, decoded dimensions, total pixel count and decoder allocation.
- Requires an allowed image MIME type and independently verifies the image signature; supports JPEG, PNG and WebP only.
- Decodes validated images to RGBA8 for native GPUI rendering while retaining validated encoded bytes.
- Adds an LRU in-memory cache with a 64 MiB default budget and a persistent disk cache with a 512 MiB default budget and oldest-entry eviction.
- Supports cancellation, bounded transient retries, corrupt disk-entry cleanup and cache directory isolation.
- Exposes cache statistics and composes the service into the application-owned service graph. Native poster/backdrop components and the initial Home trending shelf now consume this service.

The artwork cache lives below the OS cache directory in `SENZO/artwork`; it is disposable and contains no credentials or database records.

## Native GPUI image integration

The SENZO UI now consumes the typed metadata client and artwork service directly:

- Home requests the real hosted trending catalogue asynchronously; no hardcoded titles or stock artwork are used.
- Poster requests use the bounded artwork service at w342; the featured title's backdrop is resolved at w1280.
- Decoded RGBA8 pixels are converted on Tokio's blocking pool into GPUI RenderImage frames in BGRA order. The GPUI renderer receives cached render frames, not arbitrary remote URLs.
- Reusable poster and backdrop components provide fixed layout dimensions and explicit loading/unavailable placeholders.
- Metadata requests, downloads and pixel conversion do not block the GPUI render thread. The UI is updated only after the background task completes.
- A cancellation token is owned by the shell and cancelled during teardown.

The initial Home rail is intentionally limited to eight current trending items as an integration proof. Full Home discovery shelves, Search and Details remain separate work.

## Native hosted-metadata Search integration

The Search route now exercises the metadata and artwork foundation through a native GPUI workflow:

- Uses the GPUI CE native Input component and InputState from the companion package compatible with the pinned GPUI CE 0.2.2 runtime. No WebView or browser input is used.
- Normalizes the query and debounces typing by 350 ms; a new query or filter cancels the previous metadata request and outstanding artwork downloads.
- Supports All (multi-search), Movies and TV Shows through the existing hosted metadata API contract.
- Renders up to 24 actual API results with title, media type, release year, rating and validated poster/profile artwork. Results remain usable when artwork is unavailable.
- Provides a selectable quick-preview panel for the overview and rating, plus explicit idle, loading, empty and retryable error states.
- Uses the bounded artwork service and performs downloads and RGBA-to-GPUI conversion outside the GPUI render path.

This is an initial hosted-API integration, not full Nuvio Search parity. Recent-search persistence, discover catalogues, pagination, add-on/server aggregation and navigation to full movie/TV Details remain future work. The desktop still contains no TMDB credential.

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
