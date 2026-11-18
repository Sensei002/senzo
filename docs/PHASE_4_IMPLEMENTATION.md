# Phase 4 — Home, catalogues and discovery

**Status:** First implementation slice on `development`: local Home layout preferences are persisted and consumed by the native GPUI Home screen. Phase 4 remains in progress.

## Slice 1 — Configurable built-in Home layout

- Added the `senzo-catalog` Rust workspace crate to isolate Home/catalogue preferences from GPUI and SQLite implementation details.
- Added versioned `HomeSettings` and stable-key `HomeSectionPreference` models.
- Defaults preserve the current three shelves: Trending, Popular Movies and Popular TV.
- Added bounded preference normalization, duplicate-key rejection, safe namespaced keys, title bounds, and a maximum of two enabled hero-source preferences.
- Reused SENZO's existing `SettingsRepository` and SQLite `app_settings` key/value table; no second database or new schema migration is introduced.
- Wired Settings to toggle the featured hero and show/hide built-in shelves, and reorder shelves using keyboard-focusable native GPUI controls.
- Home now renders enabled built-in shelves in the saved order and uses persisted featured-hero visibility.
- Preferences remain local-first and do not require account sign-in or cloud sync.

## Still outstanding

- Actual multi-item hero/backdrop carousel and source selection.
- Custom section titles in the Settings UI.
- Add-on catalogue discovery/registration, collection rows and Continue Watching.
- Pagination, refresh, section loading isolation and scroll/focus preservation across reorder.
- Full Windows/Linux visual and interaction acceptance.

## Verification

GitHub Actions validates formatting, Clippy, Rust tests and native Windows/Linux builds. Phase 3 live Vercel Preview acceptance remains a separate gate because `/api/health` must report `tmdbConfigured: true`.
