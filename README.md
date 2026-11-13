# SENZO

<p align="center">
  <img src="assets/branding/senzo-logo.png" width="420" alt="Official SENZO logo" />
</p>

<p align="center"><strong>A native, accountless desktop media center.</strong></p>
<p align="center"><em>Discover. Organize. Watch. — Your media experience, stored locally.</em></p>

SENZO is a desktop media center in development, designed to bring movie and television discovery, personal watch history, optional streaming sources, and playback into one focused application. It is being built in **Rust + GPUI** for a cinematic, responsive native desktop experience.

SENZO is an independent project with its own architecture and product decisions. Its central idea is simple: **no SENZO account, no mandatory cloud profile, and no streaming provider silently installed for the user.**

> [!IMPORTANT]
> **Early development:** The current implementation phase targets **Windows only**. The interface and several planned features are not complete. Linux is a later target; Android and iOS are not targets. This repository describes both the current implementation and the intended product—planned features are clearly labelled as such.

## Project at a glance

- **Application:** Native desktop media center.
- **Language and UI:** Rust + GPUI.
- **Current platform:** Windows (MSVC).
- **Later platform:** Linux, after the Windows application is established.
- **Metadata:** TMDB is the primary metadata provider.
- **Sources:** Optional, user-installed add-ons using the Stremio HTTP add-on protocol as the initial compatibility target.
- **Playback:** MPV through a dynamically loaded libmpv C API adapter; the Windows runtime is not yet packaged.
- **Personal data:** Local SQLite database under the SENZO app-data directory.
- **Accounts:** No SENZO sign-up, login, or cloud account.
- **Builds and tests:** GitHub Actions; no local build is required for the project's CI workflow.

## What SENZO is being built to do

### Discover movies and television

SENZO will use **The Movie Database (TMDB)** as its primary metadata provider. The planned discovery experience includes:

- Search for movies or TV series.
- Browse artwork-led home and discovery shelves.
- View posters, backdrops, overviews, release dates, genres, ratings, and runtime.
- Explore cast, crew, trailers, and videos when available.
- Browse TV seasons and episodes, including episode descriptions and stills.
- Keep stable TMDB IDs and map external IDs such as IMDb IDs when a source provider requires them.

TMDB supplies metadata and artwork; it is not a streaming source. SENZO will display the required TMDB attribution. See [TMDB attribution](#tmdb-attribution).

### Choose your own streaming sources

SENZO is designed around a user-managed add-on list. The planned **Add-ons** screen will let users paste a compatible add-on manifest URL, inspect the add-on's declared capabilities, and install or manage it.

- No streaming add-on is bundled or preinstalled.
- **Torrentio is not integrated into SENZO's default source list.**
- SENZO will not silently install, select, or query a streaming provider.
- A user who deliberately chooses to use Torrentio may manually add their own configured manifest URL, just like any other compatible add-on.
- Only add-ons that the user has installed and enabled should be queried.

Initial compatibility targets the Stremio add-on HTTP protocol. Compatibility with every Stremio add-on and provider-specific extension is not guaranteed and will be tested incrementally.

### Keep personal data on your device

SENZO is **accountless and local-first**. The application will not require a SENZO account or a SENZO cloud service for personal viewing data.

The current SQLite database is created at:

`%LOCALAPPDATA%\SENZO\senzo.db`

The initial schema includes local settings, playback progress, watch history, and installed add-on records. A bounded metadata and artwork cache, favorites, and a fuller local library experience are planned. Database upgrades must preserve personal history and settings.

### Watch with a dedicated player

MPV is the planned playback engine. Target player capabilities include:

- Play, pause, seek, and resume.
- Subtitle selection.
- Audio-track selection.
- Playback speed.
- Fullscreen.
- Hardware decoding with supported Windows decoder preferences.
- Renderer selection, scaling algorithms, HDR handling, and optional image processing.
- Buffer and memory controls, subtitle appearance, and playback diagnostics.

These controls will be capability-aware: SENZO will only apply options supported by the active playback backend, GPU, and display. Defaults will favor automatic hardware selection and balanced quality with low resource use. The exact native video-surface integration will be validated during implementation. The current runtime layer dynamically loads a caller-supplied libmpv DLL, applies the saved MPV options before initialization, and exposes basic local-file loading, pause, seek, speed, stop, and property access. It is not yet connected to the GPUI interface, and SENZO does not yet package the Windows MPV runtime. Playback is not yet available as an end-user feature.

### Additional planned capabilities

- **Continue watching and history:** Resume media from locally saved progress.
- **Discord Rich Presence:** Show the current title and playback state, with privacy controls.
- **More source integrations:** Add future providers behind a stable source-adapter interface.
- **Update channels:** Windows Stable and Development channels, with integrity verification and rollback-safe installation.
- **Cinematic interface:** Dark, media-focused presentation, poster and backdrop artwork, and restrained transitions.

## Diagnostics and support

SENZO now has an initial local diagnostics service. It records operational audit, information, warning, and error events to:

`%LOCALAPPDATA%\SENZO\logs\senzo.log`

The active log rotates at 2 MiB. The support-bundle API is designed to collect recent events (up to 72 hours, capped at 1 MiB) without exporting the SQLite database or watch history. Credential redaction is best-effort, so secrets must never be sent to the logger. The current Settings screen includes a user-facing **Copy Logs** clipboard action.

## How SENZO is intended to work

1. **Discover:** The GPUI interface requests a movie or series search from the TMDB metadata service.
2. **Enrich:** SENZO retrieves details, credits, images, videos, external IDs, and TV season/episode data.
3. **Find sources:** When the user asks for available sources, SENZO queries only installed and enabled add-ons that support the relevant resource.
4. **Select:** The user chooses a source. SENZO does not silently pick or install one.
5. **Play:** The selected playable result is handed to the playback layer, planned around MPV. Any transport required by a selected source remains behind a separate abstraction.
6. **Remember locally:** Playback progress and viewing history are written to the local SQLite database.

Metadata, add-ons, transport, playback, persistence, and UI are separate components so that future providers can be added without rewriting the rest of the application.

## Current implementation status

The repository currently contains early service and persistence foundations. It is not yet a complete, user-ready media center.

| Area | Status |
|---|---|
| Rust + GPUI application shell | Initial scaffold |
| Windows CI | Configured; successful CI run #66 verifies formatting, compilation, and tests |
| Local SQLite storage | Initial settings, progress, history, and add-on tables |
| Local diagnostics service | Initial bounded event logging and recent-log export API; Settings/clipboard UI pending |
| Typed playback preferences | Persisted settings model and initial GPUI controls; settings persist locally |
| MPV configuration adapter | Pure MPV option mapping with capability-aware fallbacks and unit tests |\n| MPV runtime adapter | Dynamic libmpv loader with basic commands and property access; not connected to GPUI or packaged |
| TMDB service | Initial movie/TV search, detail requests, season/episode and external-ID services, image URL construction |
| Add-on service | Initial manifest URL validation/installation storage and stream request foundation |
| GPUI media-center screens | Settings shell only; Home/Search/Library/Details/Player/Add-ons remain planned |
| Metadata-driven Home/Search/Details | Planned |
| User-facing add-on manager | Planned |
| Full Stremio catalog/meta/stream/subtitle routing | Planned |
| End-user MPV playback | Planned |
| Torrent transport | Not integrated; evaluate only behind a transport abstraction if needed |
| Discord Rich Presence | Planned |
| Windows installer and updater | Planned |

A feature is not considered complete merely because a service module exists. It must be connected to the application where appropriate, tested, and verified by Windows CI. See the [Roadmap](docs/ROADMAP.md) for phase-by-phase status.

## Technology

- **Rust, Edition 2024** — application and service implementation.
- **GPUI** — native desktop interface.
- **Tokio** — asynchronous runtime.
- **Reqwest + Rustls** — HTTP requests.
- **Serde / Serde JSON** — structured data and protocol parsing.
- **SQLite / rusqlite** — local persistence.
- **TMDB API** — movie and television metadata.
- **Stremio-compatible HTTP add-ons** — optional external sources.
- **MPV / libmpv** — playback engine and C API.
- **GitHub Actions** — Windows formatting, compilation, tests, and future release builds.

Dependencies and integration choices may change when implementation, CI, security review, or license compatibility requires it.

## Platform and branch policy

- **Windows:** The only active implementation and CI target at this stage.
- **Linux:** A later target. Do not add Linux build jobs until Linux implementation begins.
- **Android and iOS:** Out of scope.
- **`development`:** Active development and development builds.
- **`main`:** Stable releases only.

All builds and tests are intended to run through GitHub Actions. Keep implementation work on `development`; do not place unfinished feature work directly on `main`.

## Privacy and security principles

- No SENZO user account or mandatory cloud watch-history service.
- Personal history, progress, settings, and add-on configuration are stored locally.
- No bundled or automatically installed streaming add-ons.
- Add-on manifests, responses, redirects, and stream URLs are untrusted input and must be validated.
- API tokens and provider credentials must never be committed or written to logs.
- External providers are independent services with their own availability, privacy policies, account requirements, and terms.
- Cache cleanup must not silently delete personal history or settings.

See [Security](docs/SECURITY.md) for the threat model and implementation requirements.

## Documentation

- [Product specification](docs/PRODUCT_SPEC.md) — product vision, scope, and non-negotiable decisions.
- [Architecture](docs/ARCHITECTURE.md) — system boundaries, components, data flow, and technical decisions.
- [Roadmap](docs/ROADMAP.md) — implementation phases and completion status.
- [Security](docs/SECURITY.md) — trust boundaries, local data, credentials, and network safety.
- [Development guide](docs/DEVELOPMENT.md) — branch rules, CI workflow, code organization, and contribution expectations.
- [Licensing policy](docs/LICENSING_POLICY.md) — license decision and restrictions under consideration.

These documents are living project documentation. Update them whenever implementation, scope, or technical decisions change. Keep the README detailed enough to explain the project to a new visitor, while clearly distinguishing shipped behavior from plans.

## TMDB attribution

This product uses the TMDB API but is not endorsed or certified by TMDB.

TMDB provides metadata and images under its applicable API terms and attribution requirements. SENZO will include the required TMDB branding and attribution in its About/Credits area. Commercial use may require separate permission or licensing from TMDB.

## License

SENZO is licensed under the **GNU Affero General Public License v3.0 only (AGPL-3.0-only)**.

- The complete license text is provided in the repository-root [`LICENSE`](LICENSE) file.
- Cargo package metadata declares `AGPL-3.0-only`.
- You may use, study, modify, and redistribute SENZO under the terms and conditions of AGPL-3.0.
- Modified versions that users interact with over a network are subject to the source-offer requirement in section 13 of the AGPL.
- Commercial use is permitted when the license terms are followed. AGPL-3.0 does not prohibit commercial use or impose field-of-use restrictions.
- Third-party services, metadata, media, trademarks, add-ons, and dependencies remain subject to their own terms and licenses.

See the [Licensing Policy](docs/LICENSING_POLICY.md) for a practical explanation. This project uses the SPDX identifier `AGPL-3.0-only`, not `UNLICENSED`.

## Project status

SENZO is under active development. Product scope, UI, technical choices, and implementation details may evolve. This README will be updated as real features land; the Architecture and Roadmap documents will track deeper implementation details and outstanding work.
