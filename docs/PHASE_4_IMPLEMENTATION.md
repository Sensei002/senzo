# Phase 4 — Home, catalogues and discovery

**Status:** In progress on `development`. Slice 1 established persisted Home layout preferences. Slice 2 adds a native, multi-title featured carousel driven by user-selected built-in catalogue sources.

## Slice 1 — Configurable built-in Home layout

- Added the `senzo-catalog` Rust workspace crate to isolate Home/catalogue preferences from GPUI and SQLite implementation details.
- Added versioned `HomeSettings` and stable-key `HomeSectionPreference` models.
- Defaults preserve the current three shelves: Trending, Popular Movies and Popular TV.
- Added bounded preference normalization, duplicate-key rejection, safe namespaced keys, title bounds, and a maximum of two enabled hero-source preferences.
- Reused SENZO's existing `SettingsRepository` and SQLite `app_settings` key/value table; no second database or new schema migration is introduced.
- Wired Settings to toggle the featured hero and show/hide built-in shelves, and reorder shelves using keyboard-focusable native GPUI controls.
- Home renders enabled built-in shelves in the saved order.
- Preferences remain local-first and do not require account sign-in or cloud sync.

## Slice 2 — Native featured hero carousel

- Added per-section featured-source controls to Settings; no more than two enabled shelves can feed the hero, and hidden shelves cannot remain selected as hero sources.
- The Home loader now resolves hero candidates from the selected sources in saved section order, with a hard cap of eight featured entries.
- Each selected backdrop is validated and decoded through the existing native Rust artwork pipeline in background tasks; poster loading remains parallel and independent.
- Added wraparound previous/next controls, a visible position indicator, title-specific metadata/overview, and a More info action that opens the matching movie or TV details route.
- Added unit tests for source-selection order/visibility and carousel wraparound behavior.
- Kept the hero local-first and provider-neutral. Torrent/add-on providers are not bundled, installed, or enabled by this slice.

## Still outstanding

- Custom section title editing in the Settings UI (the model and persistence validation already exist).
- Generic catalogue request definitions, independent per-section loading/cache/error state, stable request identity, and refresh/pagination.
- Add-on catalogue registration and collection rows; this will plug into the dedicated Stremio add-on contracts in Phase 6.
- Continue Watching and release-aware shelves; these require the playback/progress domain planned for Phase 11.
- Focus/scroll preservation across Home reorder and route changes.
- Full Windows/Linux visual and interactive acceptance of the new hero controls.

## Verification

The Phase 3 hosted Preview acceptance remains a separate gate and is already recorded in `docs/PHASE_3_IMPLEMENTATION.md`. The new Phase 4 slice must pass the Windows and Linux formatting, Clippy, workspace-test, native build, and GPUI smoke-test jobs before it is considered accepted.
