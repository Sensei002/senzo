<p align="center">
  <img src="assets/branding/senzo-logo.png" width="420" alt="Official SENZO logo" />
</p>

<h1 align="center">SENZO</h1>
<p align="center"><strong>A native Rust desktop media center.</strong></p>
<p align="center"><em>Nuvio's desktop experience, re-engineered for Rust and GPUI CE.</em></p>

SENZO is a native desktop rewrite of Nuvio Desktop. The goal is desktop feature and workflow parity, implemented in Rust under the SENZO name—not a browser wrapper or a loosely similar media player.

## Technology boundary

- **Language:** Rust 2024.
- **Native UI:** GPUI Community Edition (GPUI CE), pinned to 0.2.2.
- **Platforms:** Windows and Linux.
- **Playback:** Native MPV/libmpv behind a SENZO-owned Rust interface, with an MPV-owned native video window; immersive GPUI playback controls are Phase 10.
- **Not used:** WebView2, Electron, Tauri, egui, or embedded browser UI.
- **Build and validation:** GitHub Actions.

All application screens and player controls will be native GPUI. The previous prototype's WebView2/player-surface implementation is not part of the new application path.

## Rewrite status

- **Phase 0 — Source audit and architecture reset:** complete.
- **Phase 1 — Clean Rust workspace and technical foundation:** complete; Windows and Linux native builds, workspace checks and startup/render/shutdown smoke tests passed.
- **Phase 2 — Native GPUI application shell and design system:** complete for implementation and automated acceptance. Navigation, focus, pointer and synthesized mouse Navigate paths are tested; responsive geometry and fullscreen-key behavior are covered by the Phase 2/4 close-out checks. Physical HID side-button delivery and final human visual review remain tracked in Phase 18's release acceptance.
- **Phase 3 — Networking, metadata and artwork foundation:** complete. Native metadata/artwork behavior and protected hosted Preview acceptance passed on Windows and Linux in [Desktop CI #505](https://github.com/Sensei002/senzo/actions/runs/44244248291).
- **Phase 4 — Home, catalogues and discovery:** complete for built-in TMDB Home discovery, configurable shelves and hero, pagination/refresh, stable catalogue identities, independent loading, persisted Home layout and collection-row projection, and route-aware scroll/focus restoration.
- **Phase 5 — Search, details, seasons and episodes:** complete. Search history and discover filters, movie/TV/person details, cast/crew and related titles, season/specials/episode browsing, artwork, composite episode identity and detail back-stack navigation passed native Windows/Linux and hosted API acceptance in [Desktop CI #569](https://github.com/Sensei002/senzo/actions/runs/44364656528).
- **Phase 6 — Stremio add-on ecosystem:** complete. Windows/Linux native CI and live Cinemeta, Torrentio, and OpenSubtitles v3 acceptance passed in [CI #616](https://github.com/Sensei002/senzo/actions/runs/44401216814). See [Phase 6 implementation report](docs/PHASE_6_IMPLEMENTATION.md).
- **Phase 7 — Nuvio plugin/scraper runtime:** complete. Nuvio-compatible QuickJS execution, repository/scraper management, sandbox permissions and limits, profile-scoped state, CryptoJS/Web Crypto, binary fetch, Cheerio compatibility, and native Settings integration passed Windows and Linux acceptance in [CI #642](https://github.com/Sensei002/senzo/actions/runs/44477224036). See the [Phase 7 implementation report](docs/PHASE_7_IMPLEMENTATION.md).
- **Phase 8 — Source aggregation and provider integrations:** complete. Provider-neutral aggregation, debrid integrations, media-server source resolution and failure isolation are implemented.
- **Phase 9 — Native MPV playback engine:** complete for development implementation and automated acceptance. Linux CI verified actual video frames and non-silent audio output through Xvfb/PulseAudio; Windows and Linux native build/smoke gates passed in [Desktop CI #684](https://github.com/Sensei002/senzo/actions/runs/44557774636). Physical monitor/speaker acceptance is retained as a Phase 18 release check.
- **Phase 10 — Native player experience:** complete. Native GPUI playback controls, timeline, track/subtitle menus, chapter navigation, stream information, direct-source recovery and PiP-style window hints are implemented. See [Phase 10 implementation report](docs/PHASE_10_IMPLEMENTATION.md).
- **Phase 11 — TV playback, progress and episode continuity:** complete for Windows development implementation and automated acceptance. Episode source discovery/navigation, autoplay-next, watched-state/completion, durable per-episode resume, Home Continue Watching/Up Next projection, MPV chapter-based skip and Stremio binge-group continuity are covered by the Windows Phase 11 regression gate. See [Phase 11 implementation report](docs/PHASE_11_IMPLEMENTATION.md).
- **Next:** Phase 12 — Library, profiles, accounts and sync.

Phase 12 owns Library, profiles and cloud sync; Phase 13 owns download/P2P transport. Physical Windows/Linux playback and final visual acceptance remain Phase 18 release checks.
Current development CI runs on Windows only. Linux remains a supported target and will receive a separate validation pass after the Windows-first development stages.

The Phase 3 metadata client targets the hosted SENZO API at senzo-api.vercel.app. It models search, trending, popular, movie/TV/person details, combined cast/crew credits, external IDs, credits, videos, images, seasons and episodes. The Person profile and combined-credits route is implemented on the API `development` branch and was verified through the protected Preview acceptance; production promotion remains a separate release decision. The client validates inputs, coalesces duplicate requests, supports cancellation and bounded retries, caps response sizes, and applies a bounded in-memory response cache. It never sends a TMDB credential or Authorization header; TMDB authentication stays server-side.

The `senzo-artwork` crate resolves TMDB image paths, downloads without following redirects, validates MIME types and image signatures, enforces compressed/decompressed size limits, decodes JPEG/PNG/WebP to RGBA8, and maintains bounded memory and disk caches. The native Home screen requests real trending metadata and renders poster/backdrop artwork through native GPUI components. Search uses a native GPUI text input, a 350 ms debounce, cancellation of superseded requests, All/Movies/TV filters, hosted metadata results, validated poster/profile artwork and a quick-preview panel. Movie/TV Details provide metadata and TV season/episode browsing; Person Details provide profile, biography, personal facts and cast/crew filmography shelves. The Phase 3 hosted-service, native artwork and offline/provider-recovery acceptance gates have passed. Phase 5 search-history/discover improvements and expanded detail/episode workflows are complete; add-on catalogue integration is in Phase 6.

The previous monolithic prototype has been removed from the active source tree. Its history remains available in Git; it is not the architecture baseline for the rewrite.

## Initial workspace

| Crate | Responsibility |
|---|---|
| senzo-app | Process entry point, service composition and Tokio runtime ownership. |
| senzo-ui | Native GPUI application shell, design system, route navigation and GPUI-safe async updates. |
| senzo-domain | Typed domain models and repository contracts. |
| senzo-metadata | Typed SENZO API client, TMDB-shaped response models, request coalescing and metadata response cache. |
| senzo-artwork | TMDB artwork URL resolution, bounded download/decode, validation and memory/disk caches. |
| senzo-network | Shared HTTP client policy and URL validation. |
| senzo-storage | SQLite repositories and transactional migrations. |
| senzo-platform | OS paths and native credential storage. |
| senzo-diagnostics | Structured logging and credential-redaction helpers. |

The workspace will gain focused feature crates as later roadmap phases require them; it will not create a separate crate for every screen.

## Build locally

The supported toolchain is pinned in rust-toolchain.toml (Rust 1.95.0).

    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace --all-targets
    cargo run -p senzo-app

Linux builds need the GPUI development libraries listed in Development. GitHub Actions is the authoritative project validation environment.

## Documentation

- Phase 0 completion: docs/PHASE_0_COMPLETION.md
- Phase 1 implementation and acceptance record: docs/PHASE_1_COMPLETION.md
- Phase 2 shell implementation and acceptance record: docs/PHASE_2_IMPLEMENTATION.md
- Phase 3 implementation record: docs/PHASE_3_IMPLEMENTATION.md
- Phase 4 implementation and acceptance record: docs/PHASE_4_IMPLEMENTATION.md
- Phase 5 implementation and acceptance record: docs/PHASE_5_IMPLEMENTATION.md
- Phase 11 TV playback/progress implementation and Windows acceptance record: docs/PHASE_11_IMPLEMENTATION.md
- Nuvio source audit: docs/NUVIO_SOURCE_AUDIT.md
- Nuvio-to-SENZO feature mapping: docs/NUVIO_TO_SENZO_MAPPING.md
- Rewrite architecture: docs/ARCHITECTURE.md
- Product specification: docs/PRODUCT_SPEC.md
- Full roadmap: docs/ROADMAP.md
- Development workflow: docs/DEVELOPMENT.md
- Licensing policy: docs/LICENSING_POLICY.md
- Security policy: docs/SECURITY.md

## Repositories

- Desktop: https://github.com/Sensei002/senzo
- Metadata API: https://github.com/Sensei002/senzo-api
- Hosted metadata API: https://senzo-api.vercel.app

The existing SENZO API keeps the TMDB Read Access Token server-side. The desktop binary must never contain the shared TMDB secret.

## Branches

- main — stable releases only.
- development — active rewrite and integration work.

## Source and provider policy

SENZO ports Nuvio desktop behavior and architecture, not Nuvio branding or artwork. Torrentio remains an optional, explicitly user-installed Stremio add-on; it is never bundled, silently installed, or enabled by default. Provider responses, manifests, plugin data, subtitles and media URLs are untrusted input.

SENZO is licensed under GPL-3.0-only. See Licensing and preserve applicable third-party notices when dependencies are added.
