# SENZO product specification

## Vision

SENZO is a native Rust desktop rewrite of Nuvio Desktop. It brings Nuvio's desktop discovery, source management, integrations, personal media organization, and playback workflows into an independent SENZO-branded application.

The goal is functional parity with the audited Nuvio desktop product, adapted to Rust and GPUI CE—not a simplified media player or a screen-by-screen visual imitation.

## User experience

- Cinematic, artwork-led Home with a featured hero, configurable shelves, collections and Continue Watching.
- Global search and discovery.
- Rich movie, series, season, episode and person details.
- Dedicated source selection with provider, quality, language, codec, size and availability information where the source provides it.
- Immersive player with visible native video and native GPUI controls.
- Library, watch history, resume progress, profiles and collections.
- Add-on/plugin management and optional external integrations.
- Complete settings with sensible defaults and advanced controls progressively disclosed.

## Non-negotiable technical decisions

- Rust application and services.
- GPUI CE for all application UI and player controls.
- MPV/libmpv as the playback engine behind a Rust-owned interface.
- No WebView2, Electron, Tauri, embedded browser UI or egui.
- Windows first; Linux as the second desktop target. macOS is a future portability target.
- GitHub Actions for build, test, package and release workflows.
- Native input, navigation, windowing and video presentation.

## Nuvio desktop parity scope

The rewrite includes the desktop-enabled Nuvio feature inventory: Home/catalogs, search, metadata/details, artwork, Stremio add-ons, plugin/scraper runtime, stream discovery, debrid, P2P, player controls, subtitles/audio tracks, TV episodes, downloads, library, collections, watched state, watch progress, profiles, authentication, cloud/sync, media servers, watch parties, Trakt, Simkl, MDBList, trailers, Discord Rich Presence, settings, diagnostics and updater.

The exact module mapping and desktop platform gates are maintained in [NUVIO_TO_SENZO_MAPPING.md](NUVIO_TO_SENZO_MAPPING.md) and [NUVIO_SOURCE_AUDIT.md](NUVIO_SOURCE_AUDIT.md).

The previous SENZO accountless-only restriction is superseded by the parity goal. Nuvio desktop exposes authentication, profiles, cloud library/sync and watch-party features. Port these as optional account-backed capabilities where appropriate, while retaining local persistence and avoiding unnecessary account requirements for basic local discovery and playback.

## Add-on and source policy

- Stremio-compatible add-ons are installed only by explicit user action.
- Torrentio is never bundled, preinstalled, silently installed or enabled by default.
- Nuvio's plugin/scraper system remains distinct from Stremio add-ons.
- Debrid and media-server integrations are separate source providers.
- Provider failures are isolated; returned data is untrusted and validated.

## Playback quality bar

Playback is the central acceptance gate:
- Video is visibly rendered; audio is audible.
- Play, pause, seek, volume, mute, speed, track selection and subtitles work.
- Stop, source replacement, navigation and application shutdown do not crash or leak native resources.
- Resize, fullscreen, repeated sessions and recovery are verified.
- Windows and Linux each pass their own native rendering tests.

A successful CI build or MPV decoder status is not proof of visible playback.

## Metadata API

The existing private `senzo-api` service remains the hosted TMDB credential boundary. Its current Next.js API exposes health and validated search, trending, popular, movie/TV detail, external-ID, season and episode routes. The TMDB Read Access Token remains server-side. Phase 0 does not change the API; later phases may extend it only when a documented requirement needs it.

## Brand and licensing

Use the official SENZO logo and SENZO visual identity. Do not reuse Nuvio logos or assets. SENZO remains GPL-3.0-only and must preserve applicable Nuvio GPLv3 notices when distributing derivative code.
