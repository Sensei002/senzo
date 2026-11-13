# SENZO

<p align="center">
  <img src="assets/branding/senzo-logo.png" width="420" alt="Official SENZO logo" />
</p>

<p align="center"><strong>A native, accountless desktop media center.</strong></p>

<p align="center">
  <em>Discover. Organize. Watch. — Your media experience, stored locally.</em>
</p>

SENZO is an open-development desktop media center designed to bring movie and television discovery, personal watch history, optional streaming sources, and playback together in one focused application. It is being built in **Rust with GPUI** for a cinematic, responsive native desktop experience.

The project takes inspiration from the discovery and add-on workflows found in applications such as Nuvio and Stremio, while establishing its own accountless, local-first design. SENZO is not intended to require a SENZO account, synchronize personal viewing data to a SENZO cloud service, or silently install a streaming provider.

> **Development status:** SENZO is in early development. The current implementation phase targets **Windows only**. The interface and several planned user-facing features are not complete. Linux is a later target, not part of the current build and test phase.

## What SENZO is aiming to become

SENZO is intended to be a personal media hub with a clear separation between metadata, discovery, streaming sources, playback, and local user data.

- **Discover films and series:** Search and browse movie and television information, artwork, cast, crew, ratings, trailers, seasons, and episodes using TMDB as the primary metadata provider.
- **Bring your own sources:** Add compatible streaming providers yourself by entering their add-on manifest URL. SENZO will not silently choose or install a provider on your behalf.
- **Keep your data on your device:** Watch history, resume positions, settings, and installed add-on information are stored in SENZO's local application-data directory.
- **Resume where you stopped:** The planned library experience includes continue-watching and playback progress for supported media.
- **Use a dedicated player:** MPV is the planned playback engine, with support targeted for subtitles, audio-track selection, seeking, playback speed, and fullscreen.
- **Grow through source adapters:** New streaming integrations should be addable without rebuilding the metadata layer or coupling provider-specific logic to the interface.
- **See what you are watching:** Discord Rich Presence is planned, with playback state and a privacy control.
- **Stay in control of updates:** Stable and Development update channels are planned for Windows distribution.

## Core principles

### 1. Accountless by design

SENZO itself will not require registration, sign-in, a SENZO profile, or a cloud account. The application is intended to work as a personal desktop app.

This does not prevent a user from choosing an external provider that has its own account, configuration, or terms. Those are separate services and are not SENZO accounts.

### 2. Local-first personal data

Personal application data belongs in the user's local SENZO app-data directory. On Windows, the current database location is:

`%LOCALAPPDATA%\SENZO\senzo.db`

SQLite is used for local settings, playback progress, watch history, and the installed add-on registry. A local artwork and metadata cache is planned. SENZO will not require a remote SENZO service to store or retrieve personal watch history.

### 3. TMDB-powered metadata

TMDB is the primary planned source for movie and TV metadata. The intended workflow is to search by title, retain stable TMDB identifiers, and enrich selected titles with details, credits, images, videos, external IDs, seasons, and episodes.

Metadata and streaming are separate concerns: TMDB provides information and artwork; it is not a streaming provider. TMDB attribution and applicable API terms will be respected. See [TMDB attribution](#tmdb-attribution).

### 4. Add-ons are explicit and optional

SENZO is designed around a user-managed add-on list. The planned Add-ons screen will let a user paste a Stremio-compatible manifest URL, inspect the returned manifest, and install or manage that source.

**No streaming add-on is bundled or preinstalled. Torrentio is not built into SENZO's default source list.** A user who deliberately chooses to use Torrentio may add their own configured manifest URL in the same way as any other compatible add-on. SENZO will not automatically install or query it.

The initial compatibility target is the Stremio add-on HTTP protocol, including manifest and supported catalog, metadata, stream, and subtitle resources. Compatibility will be implemented and verified incrementally; support for Nuvio's separate local JavaScript plugin runtime is not implied.

### 5. Native desktop application

The intended stack is Rust + GPUI. SENZO is not an Electron, Tauri, WebView2, or egui application. MPV is the planned playback engine and will sit behind an internal playback interface.

## How the pieces fit together

At a high level, the planned application flow is:

1. **Discovery:** The GPUI interface asks the metadata service to search TMDB.
2. **Details:** The metadata service retrieves title information and artwork using TMDB identifiers.
3. **Source selection:** When a user opens a title's available sources, SENZO queries only compatible add-ons the user has explicitly installed and enabled.
4. **Playback:** A selected playable result is handed to the playback layer, planned around MPV. Torrent transport, if needed by a selected source, will remain behind a separate abstraction.
5. **Local progress:** Playback progress and viewing history are written to the local SQLite database so the user can resume later.

This separation is important: metadata, add-on providers, transport, playback, and the UI should remain independently maintainable.

## Current implementation

The repository currently contains the first service and persistence foundations. This is not yet a complete usable media-center experience.

| Area | Current state |
|---|---|
| Native application shell | Initial GPUI application scaffold |
| Platform target | Windows CI and development first |
| Local database | SQLite initialization and initial settings, progress, history, and add-on tables |
| TMDB | Initial movie/TV search and detail client, including artwork URL construction |
| Add-ons | Manifest URL normalization/validation, manifest storage, and an initial stream request method |
| User interface | Placeholder shell; full screens are not connected yet |
| Playback | MPV integration is planned, not yet implemented |
| Torrent transport | Not integrated; engine evaluation is a later step |
| Discord Rich Presence | Planned |
| Updates and installers | Planned |

Implementation status is tracked in the [Roadmap](docs/ROADMAP.md). A feature should not be described as complete until it is implemented, connected to the app where applicable, and verified by Windows CI.

## Platform and branch strategy

- **Current target:** Windows (MSVC).
- **Later target:** Linux. Linux implementation and CI are intentionally deferred.
- **Out of scope:** Android, iOS, and mobile app development.
- **`development`:** Active feature development and development builds.
- **`main`:** Stable releases only.

All current builds, formatting checks, compilation checks, and tests are intended to run through GitHub Actions. The project does not rely on local developer builds as its release verification path.

## Technology direction

- **Language:** Rust (Edition 2024).
- **Desktop UI:** GPUI.
- **Local persistence:** SQLite via rusqlite.
- **Async runtime:** Tokio.
- **HTTP and serialization:** Reqwest, Serde, and Serde JSON.
- **Metadata:** TMDB API.
- **Streaming-source compatibility:** Stremio-compatible HTTP add-ons, managed explicitly by the user.
- **Playback:** MPV (planned).
- **CI:** GitHub Actions on Windows.

Dependencies and integration choices may evolve as implementation and CI testing expose platform, licensing, or compatibility constraints. See [Architecture](docs/ARCHITECTURE.md) for boundaries and technical decisions.

## Documentation

- [Architecture](docs/ARCHITECTURE.md) — system boundaries, services, data flow, security, and integration rules.
- [Roadmap](docs/ROADMAP.md) — phased implementation plan and current completion status.

These documents will be updated as SENZO gains features, changes implementation details, and moves toward stable releases.

## Privacy and security direction

- No SENZO account or cloud watch-history requirement.
- Local personal data stored under the user's app-data directory.
- No bundled or automatically installed streaming add-ons.
- Add-on manifests and responses are external input and must be validated.
- API tokens and provider credentials must never be written to logs.
- Secrets must not be committed to the repository or embedded in public source code.
- External providers may have their own privacy policies, availability, account requirements, and terms; users choose whether to install them.

## TMDB attribution

This product uses the TMDB API but is not endorsed or certified by TMDB.

TMDB provides metadata and images under its applicable API terms and attribution requirements. SENZO will include the required TMDB branding and attribution in its About/Credits area. Commercial use may require separate permission or licensing from TMDB.

## Project status and contributions

SENZO is under active development and its architecture, feature set, and documentation may change. The README is intended to describe the project's current direction; the Roadmap and Architecture documents provide more detailed implementation status and technical context.

No contribution or distribution license has been selected yet. Until a license is added, all rights are reserved by the repository owner. Do not assume the source is available under an open-source license merely because the repository is publicly visible.
