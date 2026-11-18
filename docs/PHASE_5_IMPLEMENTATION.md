# Phase 5 — Search, details, seasons and episodes

**Status:** Implementation committed to `development`; final Windows/Linux and live hosted API acceptance is pending.

**Reference baseline:** Nuvio Desktop commit `65061186dde35fc0a0f90b05d380f2318133f227`, audited in `docs/NUVIO_SOURCE_AUDIT.md` and mapped in `docs/NUVIO_TO_SENZO_MAPPING.md`. This is a Rust/GPUI implementation of the upstream workflows and boundaries, not a Kotlin/Compose port.

## Scope delivered

### Search and search history

- Retained the existing multi, movie-only, and TV-only Search request types and connected visible result paging to the requested page.
- Changing a query or media filter returns to page 1. The existing request generation/cancellation guards prevent an older query from replacing newer results.
- Added local recent-search persistence through the existing `SettingsRepository` under `search.recent`; no additional database or cloud account is introduced.
- History is normalized, case-insensitively deduplicated, capped at 10 entries, rejects control characters and values outside 2–120 characters, and refuses persisted payloads larger than 8 KiB.
- The Search empty state presents selectable recent terms and a Clear action.

### Movie and TV discovery

- Added hosted API routes `/api/tmdb/genre/movie/list`, `/api/tmdb/genre/tv/list`, `/api/tmdb/discover/movie`, and `/api/tmdb/discover/tv`.
- The public desktop client uses typed `GenreListResponse`, `DiscoverSort`, and `DiscoverRequest` models. It never receives or stores the TMDB credential.
- Discovery filters include genre, year (2017–2026 in the current native filter row), and sort: popularity, rating, newest; movie discovery also exposes highest grossing.
- API input is bounded and validated: pages 1–500, genre IDs 1–10,000, years 1870–2100, and an explicit sort allowlist. Adult results are excluded by the hosted route.
- Movie and TV destinations load independently. Initial requests, genre requests, pagination, retry, error and empty states remain separate from the other destination.
- Filter changes cancel the current request and restart at page 1. Page additions preserve existing cards, reject duplicate `(media_type, id)` identities, and ignore stale generations. A filter reset starts with a fresh identity set, so results from the previous filter cannot suppress matching cards.
- Each destination provides skeleton, loading-more, empty, error/retry and poster-card states. Cards open the matching movie or series Details view.

### Movie, series and people details

- Movie and TV detail responses include credits, videos, images, external IDs, recommendations and similar-title metadata.
- Native detail views render title metadata, synopsis/tagline, release information, runtime/status, genre labels, rating and vote count, poster/backdrop artwork, cast, selected crew departments, production companies/countries, trailer/teaser metadata, and related title cards.
- Related titles use recommendations when available and fall back to similar titles. Their poster paths are reused from the response and decoded through the native artwork service; the detail page does not issue an extra metadata call per recommendation.
- Cast and crew entries navigate to the existing person details and combined filmography view. Filmography and related title cards navigate to their corresponding movie/series detail type.
- Related title/person navigation uses a bounded 32-entry detail stack. Back returns to the immediately previous detail route instead of discarding the user's exploration context.

### Seasons, episodes and identity

- Series details expose season browsing, including season zero displayed as **Specials**.
- Selecting a season fetches its detail payload and poster artwork. Switching seasons cancels both the prior season and episode requests and clears the prior selection.
- Episode rows show season/episode number, title, runtime/date and synopsis. Selecting an episode fetches full episode metadata and still artwork and displays an episode detail panel with rating/runtime/date and synopsis.
- Episode identity is the composite tuple `(series_id, season_number, episode_number)`. It is explicitly represented and unit-tested so identical episode numbers in different seasons or series cannot collide.
- Cancellation tokens and selected series/season checks reject stale episode responses.

## Automated coverage added or extended

- Metadata-client tests cover genre retrieval, filtered movie discovery, media-specific newest-sort mapping, and rejection of invalid media type, genre and year inputs.
- UI helper tests cover bounded/normalized search history, case-insensitive duplicate handling, movie-vs-TV identity, season-zero labels and composite episode identity.
- The native `--metadata-smoke-test` now exercises live movie/TV genre and filtered discovery responses as well as existing multi/movie/TV search, movie/TV/person details, season/episode metadata and native artwork download/decode.
- `scripts/live-api-acceptance.mjs` now checks both genre endpoints, filtered movie/TV discovery, invalid sort/year/zero-genre rejection, and recommendation/similar arrays in movie/TV detail responses.
- The CI workflow is invoked with `[live-api-acceptance]` so hosted API acceptance runs on Windows and Linux, alongside formatting, Clippy, workspace tests, native release builds and startup/render/shutdown smoke tests.

## Explicit boundaries

- **No playback is implemented in Phase 5.** Trailer/teaser metadata is shown, but trailer playback, source discovery, stream selection and episode playback remain outside this phase's gate.
- **Phase 6** owns Stremio manifest/resource/capability matching and user-authorized add-on catalogue execution. Phase 5 only supplies built-in TMDB discovery/details.
- **Phase 11** owns watched/resume progress and Continue Watching projections.
- The existing Phase 18 human visual and physical-device acceptance requirements remain unchanged.

## Completion acceptance

This document will be changed to **Complete** and linked to the final CI run only after Windows and Linux checks, live hosted API acceptance, native smoke tests and lockfile housekeeping have all passed for the final Phase 5 commit.
