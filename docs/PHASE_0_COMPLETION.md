# Phase 0 completion record

**Phase:** 0 — Source audit and architecture reset  
**Status:** Complete  
**Date:** 2026-11-17  
**Implementation branch:** `development`

## Completed work

- [x] Pinned the Nuvio Desktop reference to `NuvioMedia/NuvioDesktop`, branch `Dev`, commit `65061186dde35fc0a0f90b05d380f2318133f227`.
- [x] Audited the upstream repository structure, shared feature modules, desktop source set, platform feature gates and relevant tests.
- [x] Inspected the upstream add-on, stream, metadata, Home, details, player-engine, native player, P2P, plugin, debrid, library, settings and desktop build areas.
- [x] Identified the Windows Nuvio player bridge's WebView2 dependency and recorded it as an explicit SENZO exclusion.
- [x] Inventoried the desktop-enabled Nuvio feature set, including accounts, profiles, cloud/sync, plugins, debrid, P2P, downloads, media servers, tracking and watch parties.
- [x] Audited the existing SENZO development branch at `fe071a5fba69caddcfc76d5bc4ff254e8be12eb6`.
- [x] Classified the current monolithic SENZO implementation as a legacy prototype, not a proven rewrite foundation.
- [x] Recorded the existing Windows playback acceptance failure; no claim of visible video or stable Stop behavior is made.
- [x] Audited the existing private `senzo-api` route structure and credential boundary; no API source or deployment was changed.
- [x] Confirmed the root SENZO license text is GPLv3 and `Cargo.toml` declares `GPL-3.0-only`.
- [x] Corrected the existing documentation and package-label mismatch that incorrectly called the project AGPL.
- [x] Published the pinned Nuvio source audit.
- [x] Published the complete Nuvio-to-SENZO feature mapping.
- [x] Published the new Rust workspace and native playback architecture.
- [x] Replaced the previous incremental roadmap with the full rewrite roadmap.
- [x] Updated the README and developer guidance to describe the fresh rewrite.

## Deliverables

- [Nuvio source audit](NUVIO_SOURCE_AUDIT.md)
- [Nuvio-to-SENZO feature mapping](NUVIO_TO_SENZO_MAPPING.md)
- [Rewrite architecture](ARCHITECTURE.md)
- [Product specification](PRODUCT_SPEC.md)
- [Full rewrite roadmap](ROADMAP.md)
- [Development workflow](DEVELOPMENT.md)
- [Licensing policy](LICENSING_POLICY.md)

## Architecture decisions locked for the rewrite

1. Rust is the implementation language.
2. GPUI CE is the only selected UI framework.
3. MPV/libmpv is the playback engine behind a SENZO-owned Rust interface.
4. No WebView2, Electron, Tauri, egui or embedded browser UI.
5. Player controls are native GPUI components.
6. Nuvio desktop feature parity is the product scope; mobile-only behavior is excluded.
7. Stremio add-ons and Nuvio plugins are separate systems.
8. Torrentio remains user-installed and is never bundled or enabled by default.
9. The current SENZO code is legacy reference material; Phase 1 starts with a clean workspace.
10. Native video rendering must pass a real Windows proof-of-concept before the full player is built.

## Exit criteria

All Phase 0 audit and documentation tasks are complete and committed to `development`. Phase 1 may begin only after the Phase 0 documentation commit is present on the branch.

Phase 0 is documentation and architecture work. It does not produce a new playable binary, modify the Vercel API, or claim that the current SENZO prototype's playback defect is fixed.
