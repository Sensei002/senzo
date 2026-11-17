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
- **Playback target:** MPV/libmpv behind a SENZO-owned Rust interface; playback integration is a later phase.
- **Not used:** WebView2, Electron, Tauri, egui, or embedded browser UI.
- **Build and validation:** GitHub Actions.

All application screens and player controls will be native GPUI. The previous prototype's WebView2/player-surface implementation is not part of the new application path.

## Rewrite status

- **Phase 0 — Source audit and architecture reset:** complete.
- **Phase 1 — Clean Rust workspace and technical foundation:** implementation is on development; Windows/Linux CI and native startup smoke tests are the acceptance gate.
- Later product features are intentionally not included in this foundation shell.

The previous monolithic prototype has been removed from the active source tree. Its history remains available in Git; it is not the architecture baseline for the rewrite.

## Initial workspace

| Crate | Responsibility |
|---|---|
| senzo-app | Process entry point, service composition and Tokio runtime ownership. |
| senzo-ui | Minimal native GPUI shell and GPUI-safe async updates. |
| senzo-domain | Typed domain models and repository contracts. |
| senzo-storage | SQLite repositories and transactional migrations. |
| senzo-platform | OS paths and native credential storage. |
| senzo-network | Shared HTTP client policy and URL validation. |
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
