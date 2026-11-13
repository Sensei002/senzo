# SENZO roadmap

The initial implementation phase targets **Windows only**. Linux will be addressed later. Android and iOS are out of scope. A feature is not complete until its implementation is connected where appropriate and its checks pass in Windows CI.

Status labels:
- **Done:** implemented and verified.
- **In progress:** implementation exists but is incomplete or awaiting CI verification.
- **Planned:** not implemented yet.

## Phase 0 — Repository and service foundation

- [x] Initialize repository and branch strategy (`main` stable, `development` active work).
- [x] Add the first native GPUI application shell.
- [x] Establish Windows-only GitHub Actions CI.
- [x] Add a local SQLite database initialized at app startup.
- [x] Add local settings, playback progress, watch-history, and add-on registry storage.
- [x] Add the initial TMDB movie/TV search and detail client.
- [x] Add TMDB artwork URL construction.
- [x] Add Stremio manifest URL normalization/validation and explicit add-on installation service.
- [ ] Verify the latest service/persistence changes in a successful Windows CI run.
- [ ] Add database schema versioning and migration tests.

## Phase 1 — Native desktop experience

- [ ] Build the main SENZO navigation and cinematic layout.
- [ ] Add reusable poster, backdrop, title, and detail components.
- [ ] Build Home, Search, Library, Detail, Player, Add-ons, Settings, and About/Credits screens.
- [ ] Add keyboard navigation, focus behavior, and accessible controls.
- [ ] Apply the exact official SENZO transparent logo throughout the application.
- [ ] Add TMDB logo and required attribution in About/Credits.

## Phase 2 — Metadata and local library

- [ ] Configure the TMDB API credential for official builds without exposing it in source control or logs.
- [ ] Connect TMDB search and detail services to GPUI screens.
- [ ] Add TV season and episode browsing.
- [ ] Add external-ID mapping (including TMDB/IMDb) for compatible source providers.
- [ ] Add bounded local metadata and artwork caches.
- [ ] Add continue-watching, watch history, favorites, and local library views.
- [ ] Add safe cache cleanup without deleting personal history or settings.

## Phase 3 — Add-ons and playback

- [ ] Build the Add-ons UI with manual manifest URL entry.
- [ ] Add manifest preview/validation and add-on enable/disable/remove/reorder.
- [ ] Implement Stremio catalog, metadata, stream, and subtitle resource routing.
- [ ] Add source capability matching and user-controlled source selection.
- [ ] Keep all streaming add-ons absent by default; Torrentio may only be added manually by the user.
- [ ] Harden remote manifest fetching and stream URL validation.
- [ ] Integrate MPV with subtitles, audio tracks, seeking, speed, and fullscreen.
- [ ] Connect player progress and history events to local storage.

## Phase 4 — Additional sources and presence

- [ ] Define a stable source-adapter interface for additional providers.
- [ ] Evaluate optional torrent engine choices behind a transport abstraction.
- [ ] Validate engine licensing, Windows compatibility, buffering, piece prioritization, and seek behavior before adoption.
- [ ] Add more streaming source adapters without changing the metadata or UI layers.
- [ ] Add Discord Rich Presence, playback-state updates, and privacy controls.

## Phase 5 — Windows distribution

- [ ] Build optimized Windows release binaries in GitHub Actions.
- [ ] Package Windows installers.
- [ ] Add Stable and Development update channels.
- [ ] Sign update metadata and verify downloaded artifacts.
- [ ] Add staged installation and rollback-safe update behavior.
- [ ] Publish release notes and version information.

## Later platform work

- [ ] Evaluate and plan Linux support after the Windows application is stable.
- [ ] Add Linux CI and packaging only when Linux implementation begins.

## Ongoing documentation

The README is the detailed public project introduction. Update it whenever the product scope, supported platforms, implemented features, or major user-facing capabilities change. Keep this roadmap honest about what is implemented, what is awaiting CI verification, and what remains planned. Update the Architecture document when technical decisions or system boundaries change.
