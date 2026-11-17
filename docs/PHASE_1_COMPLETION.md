# Phase 1 — Clean Rust workspace and technical foundation

**Status: Complete.** Phase 1 implementation, automated checks, native Windows/Linux builds, startup/shutdown smoke tests and dependency-lockfile commit have passed.

## Implemented in this phase

- Replaced the root single-package prototype with a seven-crate Cargo workspace.
- Retired the old monolithic `src/main.rs` and prototype modules from the active source tree. Their history remains available in Git.
- Pinned GPUI CE 0.2.2, gpui_ce_platform 0.1.0 and Rust 1.95.0.
- Added typed domain identifiers, media types, route state and repository contracts.
- Added one application-owned Tokio runtime. The GPUI shell uses its handle for background work and applies results through GPUI entity updates.
- Added structured tracing, redacted secret formatting, URL query redaction and bearer-token redaction.
- Added SQLite settings/profile repositories and ordered, transactional schema migrations.
- Added Windows Credential Manager and Linux Secret Service/keyutils credential-store configuration through the keyring crate.
- Added a shared Reqwest client policy, bounded redirects, timeouts and absolute HTTP(S) URL validation.
- Added a minimal native GPUI shell with Home, Library and Settings navigation.
- Added Windows and Linux CI jobs for formatting, Clippy, tests, release builds and native startup/shutdown smoke tests.

## Acceptance record

**Successful CI:** [SENZO Foundation CI run #411](https://github.com/Sensei002/senzo/actions/runs/44081493351)

**Build commit:** `3aa376ac28d89f26f9e3e034eafa92b60f6a6147`  
**Dependency lockfile commit:** [`a29354627e26d11b5189125c25eec0b86b4cafe6`](https://github.com/Sensei002/senzo/commit/a29354627e26d11b5189125c25eec0b86b4cafe6)  
**Branch:** `development`

- [x] Rust formatting, Clippy and all workspace tests pass on Windows and Linux.
- [x] Windows x64 native release build succeeds.
- [x] Windows GPUI window opens, renders and exits through the smoke-test path.
- [x] Linux x64 native release build succeeds on Ubuntu 24.04.
- [x] Linux GPUI window opens, renders and exits under Xvfb.
- [x] Resolved Cargo.lock is committed to `development`.

**Artifacts uploaded by CI:**
- `senzo-windows-x64-foundation-3aa376ac28d89f26f9e3e034eafa92b60f6a6147`
- `senzo-linux-x64-foundation-3aa376ac28d89f26f9e3e034eafa92b60f6a6147`

The artifacts are available from the [Actions run](https://github.com/Sensei002/senzo/actions/runs/44081493351) and are retained by GitHub Actions until 2026-12-01.

The Home/Library/Settings navigation handlers are implemented. The automated smoke test verifies native window startup, rendering and shutdown; it does not simulate mouse clicks or verify the complete interactive route/focus experience. That broader navigation acceptance belongs to Phase 2.

## Explicitly not part of Phase 1

The foundation shell is not the final SENZO design. Home/catalogue UI, metadata, add-ons, plugins, torrent transport, MPV playback, video presentation and Nuvio feature parity belong to later phases. No claim is made that media playback works in this phase.
