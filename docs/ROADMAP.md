# SENZO roadmap

The initial implementation phase targets Windows only. Linux will be addressed later. A feature is not complete until its implementation and tests pass in Windows CI.

## Phase 0 — Foundation

- [x] Initialize repository and branch strategy.
- [x] Add the first native GPUI application shell.
- [x] Establish Windows-only CI.
- [x] Add a local SQLite database initialized at app startup.
- [x] Add local settings, playback progress, watch-history, and add-on registry storage.
- [x] Add the initial TMDB search/detail client.
- [x] Add Stremio manifest URL validation and explicit add-on installation service.
- [ ] Verify the new metadata/storage/add-on implementation in Windows CI.

## Phase 1 — Desktop experience

- [ ] Build the main SENZO navigation and cinematic layout.
- [ ] Add reusable poster, backdrop, title, and detail components.
- [ ] Build Home, Search, Library, Detail, Player, Add-ons, and Settings screens.
- [ ] Add keyboard navigation and accessible controls.
- [ ] Add the TMDB logo and required attribution in About/Credits.

## Phase 2 — Metadata and local library

- [ ] Connect TMDB credentials through secure Windows credential storage.
- [ ] Add TMDB movie/TV search and detail pages to the GPUI UI.
- [ ] Add season and episode browsing.
- [ ] Add artwork downloads and a bounded local image cache.
- [ ] Add stable TMDB/IMDb ID mapping for compatible stream add-ons.
- [ ] Add continue-watching, history, favorites, and local library views.

## Phase 3 — Add-ons and playback

- [ ] Build the Add-ons UI with manual manifest URL entry.
- [ ] Support add-on enable/disable/remove/reorder.
- [ ] Implement catalog, metadata, stream, and subtitle resource routing.
- [ ] Keep Torrentio absent by default; users may add it manually.
- [ ] Integrate MPV with subtitles, audio tracks, seeking, speed, and fullscreen.
- [ ] Connect player progress to local resume storage.

## Phase 4 — Additional sources and presence

- [ ] Evaluate torrent engine options behind a transport abstraction.
- [ ] Add source selection and source capability reporting.
- [ ] Add more streaming source adapters without changing the metadata or UI layers.
- [ ] Add Discord Rich Presence and privacy controls.

## Phase 5 — Distribution

- [ ] Build Windows Release binaries in GitHub Actions.
- [ ] Package EXE and MSI installers.
- [ ] Add Stable and Development update channels.
- [ ] Sign update metadata and verify downloaded artifacts.
- [ ] Publish release notes and support rollback-safe updates.
