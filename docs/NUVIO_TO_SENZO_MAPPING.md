# Nuvio-to-SENZO feature and architecture mapping

## 1. Porting rule

SENZO is a behavior-first Rust rewrite of Nuvio Desktop. The upstream source is the reference for workflows, feature boundaries, data contracts, settings, and edge cases. Kotlin/Compose syntax and platform-specific implementation details are not copied mechanically.

The mapping is pinned to Nuvio Desktop commit `65061186dde35fc0a0f90b05d380f2318133f227`. See [the source audit](NUVIO_SOURCE_AUDIT.md).

## 2. Target and exclusions

**In scope:** Nuvio desktop features that are enabled or implemented for desktop, adapted for Windows and Linux.

**Not in the initial platform scope:** Android and iOS application shells, mobile-only services, Android ExoPlayer implementation, iOS playback implementation, mobile background/foreground services, and mobile-only UI.

**Hard technical exclusions:**
- WebView2 and `WebView2Loader.dll`.
- Electron, Tauri, browser-hosted UI, and embedded browser player controls.
- egui. GPUI CE is the selected UI framework.
- Nuvio logos, artwork, and brand identity.

macOS is not an initial release commitment. The Rust architecture should avoid unnecessary Windows-only assumptions so a macOS port can be considered later.

## 3. Feature mapping

| Nuvio feature area | Nuvio reference areas | SENZO Rust target | Required behavior / acceptance |
|---|---|---|---|
| App shell and navigation | `App.kt`, `AppGate`, `MainAppContent`, navigation, desktop chrome | `senzo-app`, `senzo-ui`, `senzo-platform` | Native window, startup, navigation stack, Back/Escape, focus restoration, fullscreen/windowed mode, keyboard/mouse input, close controls. |
| Design system | Theme, tokens, reusable UI, desktop navigation, posters and shelves | `senzo-ui` | SENZO identity, native GPUI components, artwork-led layout, focus states, loading/empty/error states. |
| Home and catalogues | Home repository/screen/components, catalog repository and models | `senzo-catalog`, `senzo-metadata`, `senzo-ui` | Hero, configurable catalog shelves, collections, continue watching, pagination, stable section identity, cached results. |
| Search and discovery | Search repository/models/screen, discover catalog selection | `senzo-catalog`, `senzo-metadata` | Movie/TV search, discover filters, history, paging, cancellation, retry, stable IDs. |
| Details and people | Metadata details repository/models/screen, season support, person details, ratings/comments/trailers | `senzo-catalog`, `senzo-metadata` | Movie/series details, cast/crew, production, ratings, related titles, trailers, seasons, episodes, artwork. |
| Metadata and artwork | TMDB services, poster/backdrop loaders, custom poster resolution | `senzo-metadata`, `senzo-artwork`, SENZO API | Typed TMDB API, external-ID mapping, images, bounded caches, fallbacks, cancellation and request coalescing. |
| Stremio add-ons | Add-on repository, manifest parser, HTTP client, transport URLs | `senzo-addons` | User install/preview/validation, capabilities, resource routing, enable/disable/remove, profile scoping, manifest refresh. |
| Stream discovery | Streams repository/parser/models/cards, badges, filters, launch/context stores | `senzo-sources`, `senzo-addons` | Parallel provider queries, stream grouping, metadata, headers, URL/torrent/external classifications, sorting, filtering, fallback and auto-selection. |
| Plugin/scraper runtime | Plugin repository/models, QuickJS runtime and desktop platform | `senzo-plugins` | Separate repository/scraper ecosystem, compatible runtime, settings, refresh, test/execute, stream result mapping, isolation and time limits. |
| Debrid | Debrid provider/API/models, direct resolver, local availability and cloud library | `senzo-debrid`, `senzo-sources` | Provider capability model, explicit credentials, device-code/API-key auth as applicable, cached-torrent checks, resolve and source formatting. |
| P2P/torrent | Desktop P2P engine, consent, cache and player overlays | `senzo-torrent` | Consent and settings, metadata/file selection, progress/cache state, streaming transport, cancellation, safe shutdown and recovery. |
| Player engine | Shared player-engine contract and desktop engine | `senzo-playback` | Engine-neutral commands/events, load, play/pause, seek, speed, volume, mute, state snapshots, errors and deterministic teardown. |
| Native video | Native player host/controller/bridge and platform native sources | `senzo-playback`, `senzo-platform` | Actual visible video in a GPUI window on Windows and Linux. No WebView2. Renderer path must pass a standalone proof-of-concept before full integration. |
| Player interface | Controls, overlays, source panel, side panel, tracks, timeline, chapters and gestures | `senzo-ui`, `senzo-playback` | Native GPUI controls, seek/volume, track selection, subtitles, chapters, resize mode, fullscreen, errors and auto-hide. |
| Subtitle/audio | Subtitle repository, matching, style, add-on subtitle loader, audio preferences and delay | `senzo-playback`, `senzo-subtitles` | Embedded/external/add-on subtitles, track matching, language preferences, appearance, delay, audio track selection and audio delay where supported. |
| Episode playback | Series playback resolver, seasons/episodes, next episode and auto-play rules | `senzo-catalog`, `senzo-playback` | Correct season/episode identity, episode source search, next episode, watched state and resume progress. |
| Library and collections | Library repository/screen, local/server content, list management, collections | `senzo-library`, `senzo-integrations` | Local library, collections, provider-backed lists, sorting, paging, import/export where supported. |
| Watch state and progress | Watched repository, watch progress, continue watching, resume prompts, tracking source coordinator | `senzo-library` | Watched state, progress updates, resume prompts, completion rules, continue-watching enrichment. |
| Profiles and accounts | Auth, profiles, PIN, credential storage and profile routing | `senzo-accounts`, `senzo-storage` | Profile selection, account state, secure credentials, profile-scoped settings and data isolation. |
| Cloud and sync | Cloud library, sync manager, profile/settings/provider credential sync | `senzo-sync`, `senzo-integrations` | Optional authenticated sync and reconciliation, conflict handling, pagination and offline behavior. |
| Media servers | Server repository, provider abstractions, matching, sessions and catalog/stream support | `senzo-integrations`, `senzo-sources` | Add/manage connections, credentials, browse, match media, resolve streams and report provider errors. |
| Tracking services | Trakt, Simkl, MDBList and tracking registry | `senzo-integrations` | Auth, library/list sync, watched-state mapping, refresh and provider-specific settings. |
| Watch party | Party API/channel/chat/presence/playback sync | `senzo-party`, `senzo-integrations` | Create/join sessions, sync playback state, chat/presence, handle reconnect and auth requirements. |
| Downloads | Download repository, platform downloader, progress and local playback | `senzo-downloads`, `senzo-storage` | Queue, pause/cancel/retry, progress, destination, completion, local-file playback and cleanup. |
| Notifications | Episode release notifications | `senzo-notifications` | Port only where desktop policy supports it; upstream desktop currently disables notifications. |
| Trailers | Trailer extraction/player and platform policy | `senzo-trailers` | Respect desktop platform behavior; Windows uses external trailer playback in the audited snapshot. |
| Discord presence | Desktop Discord IPC activity manager | `senzo-integrations` | Optional presence, playback-state updates, privacy controls and no title sharing without explicit consent. |
| Membership/community | Membership, supporter, contributor and donation pages/actions | `senzo-community` | Port desktop-enabled functionality if it remains part of SENZO product scope; do not let it block core playback. |
| Settings | General, appearance, player, source, integration and provider settings | `senzo-settings`, feature services | Persisted typed settings, defaults, validation, capability-aware controls and migration. |
| Updater and diagnostics | Desktop updater, Sentry settings, diagnostics and logs | `senzo-update`, `senzo-diagnostics` | Update channels, integrity checks, logs, redaction, support export and recovery. |

## 4. Product decisions carried into the rewrite

1. **Nuvio desktop parity replaces the previous narrow SENZO feature scope.** The previous accountless-only rule is superseded for the rewrite: desktop-exposed authentication, profiles, cloud library/sync, and watch-party capabilities are part of the parity inventory. They should remain optional where Nuvio permits, and basic local discovery/playback must not be needlessly blocked by account setup.
2. **Torrentio remains user-installed.** No add-on is bundled, silently installed, or enabled by default.
3. **Stremio add-ons and Nuvio plugins remain separate.** They have different manifests, runtimes, trust boundaries, and result contracts.
4. **All player controls are native GPUI.** The Nuvio WebView2 controls implementation is not ported.
5. **MPV is a backend only.** The Rust application owns lifecycle, commands, state, error handling and native presentation.
6. **Metadata remains independent of sources.** TMDB provides metadata; add-ons/plugins/debrid/media servers provide playback sources.
7. **Local data remains first-class.** Settings, profiles, history, cache and progress require explicit storage boundaries. Optional cloud sync is an integration, not a reason to remove local persistence.

## 5. Required end-to-end acceptance workflows

- Launch SENZO → Home loads → hero and shelves render artwork → navigate with keyboard and pointer.
- Search a movie → open details → inspect metadata/artwork → discover sources → select a source → visible video and audible audio.
- Search a TV show → select season → select episode → discover episode-specific sources → play → next episode → resume progress.
- Install an add-on from a user-supplied manifest URL → validate → enable → browse its catalogue → discover streams → disable/remove.
- Install/manage a plugin repository separately → test a scraper → map returned streams into source selection.
- Configure a debrid provider → resolve or check a cached source → playback.
- Start P2P playback → monitor progress → seek → stop → confirm engine and temporary data cleanup.
- Select audio/subtitle tracks → change subtitle appearance/delay → verify live playback.
- Add a media server/tracking provider → authenticate → browse/sync → handle disconnect/reconnect.
- Create/join a watch party → verify playback synchronization and teardown.
- Close and restart SENZO → verify settings, profile state, library and playback progress persistence.

## 6. Porting and review policy

For every feature, record: upstream source paths, behavior contract, Rust module, persisted data, platform differences, security implications, tests, and Windows/Linux runtime acceptance. Do not mark a feature complete merely because its screen exists or its code compiles.
