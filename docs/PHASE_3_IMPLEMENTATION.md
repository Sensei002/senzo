# Phase 3 — Networking, metadata and artwork foundation

**Status:** In progress — metadata, artwork, native Home/Search, Movie/TV/Person Details and unified retryable metadata-error UX are implemented on `development`. Desktop CI #457 passed on Windows and Linux. Live hosted-service runtime acceptance remains; the Person endpoint is implemented on SENZO API `development` and awaits merge/deployment via [API PR #1](https://github.com/Sensei002/senzo-api/pull/1).

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

- Live hosted API health, Home, Search, Movie/TV/Person Details and artwork acceptance on Windows and Linux.
- Merge/deploy SENZO API PR #1 before validating the new production person endpoint.
- Runtime verification of connection loss, rate limiting and provider outages with recovery on both desktop platforms.
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

## Native Movie and TV Details

The native Search preview can open a dedicated GPUI Details page for movies and TV series:

- Movie and TV details are loaded through the typed hosted metadata client.
- The page presents overview, genres, release information, runtime, status, rating and available cast credits.
- TV series expose season selection and episode metadata browsing. This does not initiate playback.
- Details requests are cancellation-aware and stale results cannot replace a newer selection.
- The Details retry action uses the page's own media identity, not the mutable Search result list.

## Native Person Details

The native Search workflow opens a dedicated GPUI Person Details page for person results:

- Profile artwork, name, known-for department, biography, birthday/deathday and place of birth.
- Movie and TV filmography shelves built from combined cast and crew credits.
- Credit selection prefers crew for non-acting departments, cast for actors, and falls back to the other credit group when the preferred group is empty.
- Credits are de-duplicated by TMDB media ID and ordered by popularity; entries without usable artwork are excluded from the visual shelves.
- Credit artwork uses the bounded artwork service and is loaded off the GPUI render path.
- Selecting a filmography item opens its native Movie/TV Details page.
- Person requests and artwork jobs are cancellation-aware; stale responses cannot overwrite a newer page.
- The Person Details page has loading, unavailable, empty-filmography and retry states.

The desktop implementation and typed client are on `development` (native Person Details: [commit 8763055](https://github.com/Sensei002/senzo/commit/8763055e6054b702284cb192bc78da2e85ed70ff); metadata person client: [commit 4a520d4](https://github.com/Sensei002/senzo/commit/4a520d480c3cbe95497a3bf349b138e19f158b2d)). The backend route is implemented on SENZO API `development`; [API PR #1](https://github.com/Sensei002/senzo-api/pull/1) passed API CI #10 but has not been merged into `main`, so live production Person Details remains blocked on deployment.

## Consistent metadata failure and recovery UX

- Home, Search, Movie/TV Details and TV season requests map typed metadata errors to consistent user-facing messages.
- Connection failures, provider 5xx outages, 429 rate limits, missing titles, invalid responses and local configuration/cache failures receive distinct guidance.
- Home, Search and Details retain explicit Retry actions; individual TV season requests also provide a dedicated retry action.
- Selecting another TV season cancels the previous request so a late response cannot overwrite the currently selected season.
- API diagnostics and raw upstream response text are not exposed as the primary UI message. The desktop still contains no TMDB credential.

## First-slice automated acceptance

The metadata-client foundation passed [CI run #431](https://github.com/Sensei002/senzo/actions/runs/44098712863) on commit 072ff362e84c37b6ca45f439cad39c8fe562ab73.

- Windows Server 2022: formatting, Clippy, workspace tests, native release build and GPUI startup/render/shutdown smoke test passed.
- Ubuntu 24.04: formatting, Clippy, workspace tests, native release build and GPUI startup/render/shutdown smoke test passed.
- Both development binaries were uploaded:
  - senzo-windows-x64-development-072ff362e84c37b6ca45f439cad39c8fe562ab73
  - senzo-linux-x64-development-072ff362e84c37b6ca45f439cad39c8fe562ab73
- CI regenerated and committed Cargo.lock in commit 66df5a3c75281fc65698e7f69a7fa94b19736cf8.

These checks validate the service foundation and native application build. They do not claim that the hosted API was live-tested or that Search, details and artwork are already wired into the UI.

## Opt-in live hosted API acceptance

An opt-in live API acceptance workflow is available through Actions → SENZO Desktop CI → Run workflow → enable `run_live_api_acceptance`. It checks API health, Search, trending, movie details/credits, TV details/seasons, person details/combined credits and a TMDB artwork CDN response from both Windows and Linux runners. Keep it disabled during normal CI to avoid making every commit depend on external services.

## Remaining Phase 3 acceptance

Phase 3 is not complete until the native Home, Search, Movie/TV/Person Details and artwork flows are verified against the live hosted API on Windows and Linux without a desktop TMDB secret. First merge/deploy SENZO API PR #1 so the hosted person profile/combined-credits route exists in production. Run the opt-in live API acceptance workflow after deployment. Runtime acceptance must also simulate offline connectivity, HTTP 429 and provider 5xx outages, verify that every Retry action recovers cleanly, and confirm that cancelled or superseded requests never restore stale data. Desktop CI #457 and API CI #10 are green, but neither substitutes for this live runtime gate.
