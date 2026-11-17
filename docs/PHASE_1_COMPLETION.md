# Phase 1 — Clean Rust workspace and technical foundation

**Status:** Implementation committed; automated and native runtime validation pending.

## Implemented in this phase

- Replaced the root single-package prototype with a seven-crate Cargo workspace.
- Removed the old monolithic src/main.rs and prototype modules from the active source tree. Their history remains available in Git.
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

The following items remain pending until the Phase 1 workflow reports success:

- [ ] Rust formatting, Clippy and all workspace tests pass.
- [ ] Windows release build succeeds.
- [ ] Windows GPUI window opens, renders and exits through the smoke-test path.
- [ ] Linux release build succeeds.
- [ ] Linux GPUI window opens, renders and exits under Xvfb.
- [ ] Resolved Cargo.lock is committed to development.

The workflow must be green before Phase 1 is marked complete. A compile-only result is not a native startup acceptance result.

## Explicitly not part of Phase 1

The foundation shell is not the final SENZO design. Home/catalogue UI, metadata, add-ons, plugins, torrent transport, MPV playback, video presentation and Nuvio feature parity belong to later phases. No claim is made that media playback works in this phase.
