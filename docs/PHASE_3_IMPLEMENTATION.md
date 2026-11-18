# Phase 3 — Networking, metadata and artwork foundation

**Status:** In progress — metadata, artwork, native Home/Search, Movie/TV/Person Details and unified retryable metadata-error UX are implemented on `development`. The Vercel `development` Preview now has `TMDB_READ_ACCESS_TOKEN` configured and rebuilt from the correct branch; `/api/health` reports `tmdbConfigured: true`, and a real TMDB-backed trending request returns HTTP 200. Final Windows/Linux desktop CI and native live acceptance are being rerun after fixing a stale test constructor call. The Person endpoint remains a development Preview integration via [API PR #1](https://github.com/Sensei002/senzo-api/pull/1); production promotion remains a separate release decision.

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

## Phase 3 scope boundary

The remaining full Nuvio discovery features—recent-search persistence, discover filters, pagination, add-on/server catalogues and configurable Home sections—belong to the later discovery phases. Phase 3 establishes the hosted metadata and artwork transport, native integration baseline and reliable recovery behavior; it does not claim full Nuvio discovery parity.


## Artwork pipeline implementation

The new `senzo-artwork` crate implements the second Phase 3 slice:

- Resolves TMDB file paths to the fixed image CDN with typed size variants (w92 through w1280 and original); rejects arbitrary hosts, malformed paths, traversal, credentials, query strings and fragments.
- Downloads through the shared HTTP client with redirects disabled, preventing image requests from escaping the allowed origin.
- Bounds concurrent downloads, request time, compressed response bytes, decoded dimensions, total pixel count and decoder allocation.
- Requires an allowed image MIME type and independently verifies the image signature; supports JPEG, PNG and WebP only.
- Decodes validated images to RGBA8 for native GPUI rendering while retaining validated encoded bytes.
- Adds an LRU in-memory cache with a 64 MiB default budget and a persistent disk cache with a 512 MiB default budget and oldest-entry eviction.
- Supports cancellation, bounded transient retries, corrupt disk-entry cleanup and cache directory isolation.
- Exposes cache statistics and composes the service into the application-owned service graph. Native poster/backdrop components and all three built-in Home catalogue shelves now consume this service.

The artwork cache lives below the OS cache directory in `SENZO/artwork`; it is disposable and contains no credentials or database records.

## Native GPUI image integration

The SENZO UI now consumes the typed metadata client and artwork service directly:

- Home requests real hosted catalogue metadata asynchronously; no hardcoded titles or stock artwork are used.
- Trending, popular movies and popular TV are loaded concurrently and rendered as independent native shelves.
- Poster requests use the bounded artwork service at w342; the featured trending title's backdrop is resolved at w1280.
- Decoded RGBA8 pixels are converted on Tokio's blocking pool into GPUI RenderImage frames in BGRA order. The GPUI renderer receives cached render frames, not arbitrary remote URLs.
- Reusable poster and backdrop components provide fixed layout dimensions and explicit loading/unavailable placeholders.
- Metadata requests, downloads and pixel conversion do not block the GPUI render thread. The UI is updated only after the background task completes.
- A cancellation token is owned by the shell and cancelled during teardown.

The current built-in Home baseline shows up to 12 cards in each of the three catalogue shelves, selects a trending item with a backdrop for the hero, and routes movie, TV and person cards to their matching native Details workflow. Each shelf has loading, empty/error and retry states. Configurable/add-on catalogues, collections, Continue Watching, pagination and a full cinematic carousel remain later Home/discovery work.

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
- Deterministic local-server tests cover transient 5xx recovery, the short `Retry-After` rule for 429 responses, non-caching of failed responses, identical-request coalescing and cancellation during retry backoff.

## First-slice automated acceptance

The metadata-client foundation passed [CI run #431](https://github.com/Sensei002/senzo/actions/runs/44098712863) on commit 072ff362e84c37b6ca45f439cad39c8fe562ab73.

- Windows Server 2022: formatting, Clippy, workspace tests, native release build and GPUI startup/render/shutdown smoke test passed.
- Ubuntu 24.04: formatting, Clippy, workspace tests, native release build and GPUI startup/render/shutdown smoke test passed.
- Both development binaries were uploaded:
  - senzo-windows-x64-development-072ff362e84c37b6ca45f439cad39c8fe562ab73
  - senzo-linux-x64-development-072ff362e84c37b6ca45f439cad39c8fe562ab73
- CI regenerated and committed Cargo.lock in commit 66df5a3c75281fc65698e7f69a7fa94b19736cf8.

These first-slice checks validate the initial service foundation and native application build only. Later desktop CI runs cover the expanded Home, Search, Details and resilience implementations; no desktop build or mock-server test proves that the production hosted API is reachable.

## Opt-in live hosted API acceptance

The live API acceptance job checks API health and server-side TMDB configuration; multi/movie/TV Search; trending, popular movie and popular TV catalogues; movie/TV details, credits, videos, images and external IDs; TV season and episode details; person details and combined cast/crew credits; and the TMDB artwork CDN with byte-limit and file-signature checks. The job runs on both Windows and Linux. It can be started from Actions → SENZO Desktop CI → Run workflow by enabling `run_live_api_acceptance` and setting `api_base_url`, or automatically from a `development` push whose commit message contains `[live-api-acceptance]`. Marker-triggered pushes use the SENZO API development Preview URL by default; ordinary commits do not invoke external live services.

SENZO API PR #1 currently has a ready Vercel Preview deployment at `https://senzo-api-git-development-stream-anything.vercel.app`. This allows the new Person route and the rest of the API contract to be tested before merging the PR. The preview does not change the production API or the desktop's default production base URL.

The live acceptance harness first exposed and fixed a URL-joining bug that could construct `https://api/...` instead of the configured Preview origin. URL creation is now isolated in `scripts/live-api-url.mjs` and covered by Node's built-in test runner for root and subpath deployments, loopback HTTP, unsafe URL credentials/query/fragments, and cross-origin path attempts; automation CI runs these tests. Preview Deployment Protection is successfully bypassed by the GitHub Actions secret `VERCEL_AUTOMATION_BYPASS_SECRET`; the secret is injected only into the test proxy and never logged or passed to SENZO. The Vercel `senzo-api` Preview now contains `TMDB_READ_ACCESS_TOKEN`, targeted to branch `development`. An initial redeploy accidentally targeted `main`; that did not update the development alias. The correct development commit `5f4df13765bdddaf4b11355f4365cf62540de7cd` was then rebuilt as deployment `dpl_53fYJS1zxuVR8BwSUjo6oqFKZ97f`. Its health endpoint reports `tmdbConfigured: true`, and `GET /api/tmdb/trending/all/week` returned HTTP 200 with live metadata, confirming that TMDB accepts the configured token. Do not put this TMDB token into GitHub Actions or the desktop. An explicit offline connection-failure test has also been added to the metadata client.

The Windows and Linux native build jobs now also have an opt-in Rust integration smoke mode (`senzo --metadata-smoke-test`). It runs the app's real `MetadataClient` and `ArtworkService` against the hosted Preview, including Home catalogues, all Search filters, Movie/TV/Person Details, seasons/episodes and a real TMDB image decode. `scripts/native-metadata-smoke.mjs` runs the compiled native binary through a loopback-only Node proxy that adds the Vercel bypass header upstream. The bypass secret is removed from the child process environment; the SENZO binary receives only a loopback API URL and no deployment secret. The ordinary GPUI window startup/shutdown smoke check remains separate and still runs on every normal CI build.

## Remaining Phase 3 acceptance

Phase 3 remains open until the expanded acceptance suite passes against the protected API Preview on Windows and Linux, and native checks confirm the integrated metadata/artwork workflows plus the local Retry/stale-request regression coverage. The Preview token is now configured, the correct development deployment has been rebuilt, and direct health plus TMDB-backed API calls both pass. The latest desktop CI failure was a test call site that omitted the new `SettingsRepository` constructor argument; the test now uses a minimal in-memory repository. The next `[live-api-acceptance]` run validates hosted routes and the compiled native metadata/artwork smoke test on Windows and Linux. Offline, 429, 5xx, cache, coalescing and cancellation rules have deterministic local HTTP coverage. API PR #1 remains intentionally unmerged: production Person Details is still a separate release decision, and no production branch has been modified.
