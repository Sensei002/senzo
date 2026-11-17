# SENZO

<p align="center">
  <img src="assets/branding/senzo-logo.png" width="420" alt="Official SENZO logo" />
</p>

<p align="center"><strong>A native Rust desktop media center.</strong></p>
<p align="center"><em>Nuvio's desktop experience, re-engineered for Rust and GPUI CE.</em></p>

SENZO is being rebuilt as a native desktop application that ports the **desktop features and workflows of Nuvio** into Rust under the SENZO name. The goal is product and feature parity—not a loosely similar media player.

## Non-negotiable technology boundary

- **Language:** Rust.
- **UI:** GPUI CE.
- **Playback:** MPV / libmpv behind a SENZO-owned Rust interface.
- **Native UI and controls:** GPUI; no browser-based player controls.
- **Platforms:** Windows first, Linux as the second desktop target. The architecture must remain portable; macOS is not part of the initial release commitment.
- **Not used:** WebView2, Electron, Tauri, egui, or an embedded browser UI.
- **Builds and validation:** GitHub Actions.

MPV is the playback engine, not the application UI. The player surface and every control belong to SENZO's native Rust application.

## Rewrite status

**Phase 0 — Source audit and architecture reset: complete.** The Nuvio Desktop source baseline has been pinned and audited, the Nuvio-to-SENZO feature map is documented, and the new Rust architecture and phase roadmap are published.

The current source tree is the earlier SENZO prototype. It is retained in Git history as a reference, but it is **not** the architecture baseline for the rewrite. Its current playback implementation has not passed Windows visual-video acceptance. Phase 1 will establish the clean Rust workspace and the new implementation foundation; no Phase 1 application code is claimed complete yet.

## Nuvio reference

- Upstream: https://github.com/NuvioMedia/NuvioDesktop
- Audited branch: `Dev`
- Audited commit: `65061186dde35fc0a0f90b05d380f2318133f227`
- The upstream project is GPL-3.0. SENZO remains GPL-3.0-only; see the licensing policy and source audit.

SENZO ports desktop behavior and architectural concepts. It does not reuse Nuvio branding or artwork. The canonical SENZO logo in `assets/branding/senzo-logo.png` remains unchanged.

## Documentation

- [Phase 0 completion](docs/PHASE_0_COMPLETION.md)
- [Nuvio source audit](docs/NUVIO_SOURCE_AUDIT.md)
- [Nuvio-to-SENZO feature mapping](docs/NUVIO_TO_SENZO_MAPPING.md)
- [Rewrite architecture](docs/ARCHITECTURE.md)
- [Product specification](docs/PRODUCT_SPEC.md)
- [Full roadmap](docs/ROADMAP.md)
- [Development workflow](docs/DEVELOPMENT.md)
- [Licensing policy](docs/LICENSING_POLICY.md)
- [Security policy](docs/SECURITY.md)

## Repositories

- Desktop: https://github.com/Sensei002/senzo
- Metadata API: https://github.com/Sensei002/senzo-api
- Hosted metadata API: https://senzo-api.vercel.app

The existing SENZO API is a small Next.js service that exposes validated TMDB routes and keeps the TMDB Read Access Token server-side. Phase 0 does not change or redeploy that service.

## Branches

- `main` — stable releases.
- `development` — active rewrite and integration work.

## Important source policy

All streaming add-ons are explicitly user-installed. Torrentio is never bundled, silently installed, or enabled by default. Add-on manifests and returned resources are untrusted network input and must pass the SENZO validation and network-safety boundaries.

SENZO is an independent project. Third-party providers, metadata services, add-ons, trademarks, and media remain subject to their own rights and terms.
