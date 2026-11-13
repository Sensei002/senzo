# SENZO development guide

## Project stage

SENZO is in early development. The current focus is Windows only. Linux work is intentionally deferred; Android and iOS are out of scope.

## Branch strategy

- `development` is the active integration branch for all ongoing implementation.
- `main` is reserved for stable releases.
- Do not commit unfinished feature work directly to `main`.
- Keep changes focused and update relevant documentation in the same change.

## CI-first workflow

GitHub Actions is the project's build and verification environment. Do not rely on local compilation as the project's acceptance criterion.

For each implementation change:

1. Work on `development`.
2. Keep the change focused and document behavior or architecture changes.
3. Run formatting, compilation, and tests in Windows CI.
4. Inspect the actual workflow result and logs.
5. Fix failures rather than disabling meaningful checks.
6. Mark a feature verified only after a successful run for the relevant commit.

Rapid pushes may cancel or queue earlier workflow runs depending on workflow concurrency. A pending or cancelled run is not evidence that the code failed to compile. A green formatting step alone is not proof that the application builds and tests pass.

## Current code organization

- `src/main.rs` — application entry point and startup initialization.
- `src/lib.rs` — library module exports.
- `src/storage.rs` — SQLite persistence and local application data.
- `src/metadata.rs` — TMDB API client and metadata models.
- `src/addons.rs` — add-on manifest handling and source requests.

As the application grows, keep UI, metadata, persistence, source adapters, playback, and platform-specific code separated.

## Engineering rules

- Use Rust + GPUI for the native interface. Do not introduce Electron, Tauri, WebView2, or egui.
- Keep platform scope Windows-only until Linux work is explicitly started.
- Use stable internal media identifiers and explicit external-ID mapping.
- Treat remote manifests, JSON, URLs, and media metadata as untrusted input.
- Keep secrets out of source control, logs, and artifacts.
- Track the SQLite schema with `PRAGMA user_version`; implement schema changes as ordered, transactional migrations and test both repeat opens and unsupported future versions.
- Add tests for parsing, persistence, URL validation, and error handling.
- Do not bundle or automatically install streaming add-ons.
- Do not integrate Torrentio as a default source.
- Avoid claiming a protocol or provider is compatible until tested.
- Keep the exact official SENZO logo artwork; do not redraw or substitute it.
- Update README, Product Specification, Architecture, Security, and Roadmap when the relevant scope or decisions change.

## Feature completion criteria

A feature is considered complete only when:

- Its intended behavior is implemented.
- It is connected to the UI or application flow where applicable.
- Errors and unavailable external services are handled.
- Relevant tests exist and pass.
- Windows CI passes for the change.
- Documentation accurately describes the shipped behavior.
