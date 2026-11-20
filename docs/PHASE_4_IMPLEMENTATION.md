# Phase 4 — Home, catalogues and discovery

**Status:** Complete on `development` for the built-in Home, TMDB catalogue/discovery, local Home preferences, and persisted collection-row projection scope. All in-scope features are implemented and covered by Windows/Linux automated acceptance. Add-on catalogue execution, playback-progress shelves and collection authoring are explicitly handed to their owning later phases.

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
- The Home loader resolves hero candidates from the selected sources in saved section order, with a hard cap of eight featured entries.
- Each selected backdrop is validated and decoded through the existing native Rust artwork pipeline in background tasks; poster loading remains parallel and independent.
- Added wraparound previous/next controls, a visible position indicator, title-specific metadata/overview, and a More info action that opens the matching movie or TV details route.
- Added unit tests for source-selection order/visibility and carousel wraparound behavior.
- Hiding a selected hero source or reordering selected shelves refreshes the featured sequence so hidden sources disappear and saved priority takes effect immediately.
- Kept the hero local-first and provider-neutral. Torrent/add-on providers are not bundled, installed, or enabled by this slice.

### Slice 2 automated acceptance

Passed in [SENZO Desktop CI run #510](https://github.com/Sensei002/senzo/actions/runs/44248395951) on November 18, 2026:

- Windows and Linux formatting and Clippy.
- All workspace tests on both platforms, including the new hero-source and carousel-navigation tests.
- Native Windows x64 and Linux release builds.
- GPUI native window startup/render/shutdown smoke tests on both platforms.
- Repository automation validation and lockfile housekeeping.

## Slice 3 — Built-in shelf pagination, refresh and stable card identity

- Each built-in shelf now tracks its own current page, total pages, loading-more state, and next-page error independently.
- Added explicit Home catalogue refresh. Refresh cancels the previous catalogue generation before starting new requests so stale responses cannot overwrite the newer Home state.
- Added page-by-page requests for Trending, Popular Movies and Popular TV, with native poster artwork decoded in background tasks.
- Added duplicate-safe page merging keyed by `(media_type, id)`, preserving the existing shelf order and avoiding collisions between movie and TV IDs.
- Poster row element IDs now use the stable shelf key plus media identity rather than the card's current position, so appended pages do not rename existing cards.
- Added bounded pagination (500 pages / 120 cards per shelf), returned-page validation, load-more feedback, and retry for a failed next page.
- Added unit tests for page limits and duplicate-safe page merging.

### Slice 3 automated acceptance

Passed in [SENZO Desktop CI run #515](https://github.com/Sensei002/senzo/actions/runs/44253403129) on November 18, 2026:

- Windows and Linux formatting, Clippy and all workspace tests.
- Native Windows x64 and Linux release builds.
- GPUI native window startup/render/shutdown smoke tests on both platforms.
- Repository automation validation.
- The live hosted API acceptance job was not required for this Home-only change and was skipped.

## Slice 4 — Editable Home shelf titles

- Added a retained native GPUI input for each persisted Home section.
- Custom titles are saved on Enter or when the input loses focus; they are not written to SQLite on every keystroke.
- Reuses `HomeSettings::set_section_title` validation: whitespace is trimmed, empty titles restore the default label, control characters are rejected, and titles are limited to 80 characters.
- Invalid titles display inline validation feedback. The editor preserves keyboard tab order across visibility, reorder and hero-source controls.
- Custom titles remain local-first through the existing settings repository.

### Slice 4 automated acceptance

Passed in [SENZO Desktop CI run #517](https://github.com/Sensei002/senzo/actions/runs/44254968935) on November 18, 2026:

- Windows and Linux formatting and Clippy.
- All workspace tests on both platforms.
- Native Windows x64 and Linux release builds.
- GPUI native window startup/render/shutdown smoke tests on both platforms.
- Repository automation validation and Cargo.lock housekeeping.

## Slice 5 — Independent Home shelf loading and ordered hero publication

- Replaced the aggregate first-load barrier with one independent metadata/artwork worker per built-in Home shelf.
- Each shelf publishes its own loaded cards, pagination metadata, skeleton/error resolution, and poster frames as soon as that shelf finishes; a slow or unavailable catalogue no longer blocks successful shelves.
- Shelf API errors remain isolated and visible on the affected shelf. Unexpected worker termination is converted into a retryable per-shelf error without discarding already published shelves.
- Home refresh cancels the prior generation. Late results from a superseded generation are discarded before touching current UI state.
- Featured artwork is cached per source for the current generation and merged by the user's selected source order, not request completion order. Later sources stay pending until earlier selected sources resolve, then the carousel expands up to the eight-item cap.
- Preserves the currently selected featured media item when the available hero sequence is rebuilt.
- Added a unit test proving that a later hero source resolving first cannot appear ahead of the earlier selected source.

### Slice 5 automated acceptance

Passed in [SENZO Desktop CI run #519](https://github.com/Sensei002/senzo/actions/runs/44256725204) on November 18, 2026:

- Windows and Linux formatting, Clippy and all workspace tests.
- Native Windows x64 and Linux release builds.
- GPUI native window startup/render/shutdown smoke tests on both platforms.
- Windows and Linux binary artifacts uploaded.
- Repository automation validation and Cargo.lock housekeeping.

## Slice 6 — Provider-neutral catalogue descriptors and request identity

- Added `HomeCatalogDescriptor` with provider, media type, catalogue ID and descriptor revision. Fields are bounded and restricted to safe identifier characters.
- Added the canonical `BuiltInHomeCatalog` key type and `HomeCatalogDefinition` metadata for the three TMDB-backed Home sources.
- Added deterministic, delimiter-safe descriptor/cache identities. Display title and shelf ordering do not affect the identity; provider, media type, catalogue ID, revision and pagination capability do.
- The GPUI Home loader now resolves built-in requests through the shared catalogue definitions rather than maintaining a duplicate local key enum.
- Initial and paginated results carry their descriptor identity. Results are applied only while that identity still matches the active shelf contract, preventing stale data from being appended after a catalogue definition changes.
- Added tests for stable built-in keys, deterministic identities, revision/provider/media-type changes and invalid/oversized descriptor fields.
- This slice establishes the neutral definition/identity contract only. It does not claim that add-on, media-server or arbitrary provider adapters are implemented; those remain tied to their dedicated integration phases.

### Slice 6 automated acceptance

Passed in [SENZO Desktop CI run #523](https://github.com/Sensei002/senzo/actions/runs/44259302648) on November 18, 2026:

- Windows and Linux formatting, Clippy and all workspace tests.
- Native Windows x64 and Linux release builds.
- GPUI native window startup/render/shutdown smoke tests on both platforms.
- Repository automation validation, Windows/Linux artifact upload and Cargo.lock housekeeping.

## Slice 7 — Route-aware Home scroll and focus preservation

- Bound the main scrollable viewport to a persistent GPUI `ScrollHandle` and store scroll offsets independently for each primary `AppPage`.
- Save the outgoing page offset before navigation and restore the target page offset on the next frame after Back, Forward or sidebar navigation.
- Added stable focus identities for Home cards using the existing stable section/media-type/media-ID identity; appended pages do not rename previously focused cards.
- Retained GPUI focus handles for Home catalogue cards and the featured More Info action.
- Save the focused Home element when navigating away, then restore it after returning to Home. Opening a title/person from a Home card records the originating card and restores both the Home offset and focus when its details page closes.
- Search/person details entry also snapshots the current page offset so returning does not lose its prior viewport position.
- Added a unit test for stable Home card focus identity across shelf reordering and pagination.

### Slice 7 automated acceptance

Passed in [SENZO Desktop CI run #527](https://github.com/Sensei002/senzo/actions/runs/44261770211) on November 18, 2026:

- Windows and Linux formatting, Clippy and all workspace tests.
- Native Windows x64 and Linux release builds.
- GPUI native window startup/render/shutdown smoke tests on both platforms.
- Windows and Linux binary artifact upload.
- Repository automation validation and Cargo.lock housekeeping.

**Interactive acceptance boundary:** automated navigation, pointer and geometry assertions cover the Phase 4 interaction contract. Final human visual review and physical-device mouse acceptance are release-only checks assigned to Phase 18.

## Slice 8 — Provider-definition registry and persisted Home collection rows

- Added `HomeCatalogProviderAdapter` as the provider-definition boundary and `HomeCatalogProviderRegistry` for validated registration, lookup, deterministic enumeration and removal.
- The registry rejects invalid provider IDs, provider spoofing, malformed catalogue descriptors, duplicate keys within a provider and collisions with already registered catalogue keys. Registration is validated before any definitions are published.
- Registered the built-in TMDB Home catalogues through `BuiltInTmdbHomeCatalogProvider`; the Home loader now obtains its definitions through the same registry rather than bypassing the provider boundary.
- This registry handles catalogue declarations and stable descriptors only. It does not pretend to execute remote provider protocols: Stremio manifests, resource capability matching, catalogue requests and add-on lifecycle remain owned by Phase 6.
- Added versioned `HomeCollectionsSettings` persistence using the existing `SettingsRepository` key/value store (`home.collections`), with normalization and hard limits of 64 visible collections and 100 folders per collection.
- Connected the saved collection projection to Home using the existing reusable native `home_collection_row` and folder-card components. Home renders only locally persisted collection rows; no sample or fabricated collections are inserted.
- Added tests for persistence/normalization, stable provider registry behavior, spoofed providers and duplicate catalogue keys.
- Collection authoring, folder CRUD and Library ownership remain in Phase 12; Phase 4 intentionally consumes and renders a validated local projection rather than duplicating the Library domain.

### Slice 8 automated acceptance

Passed the substantive Windows and Linux gates in [SENZO Desktop CI #535](https://github.com/Sensei002/senzo/actions/runs/44271228972) on November 18, 2026:

- Formatting and Clippy on Windows and Linux.
- All workspace tests on Windows and Linux, including the new registry, collection-persistence and mouse-navigation interaction tests.
- Native Windows x64 and Linux release builds.
- GPUI startup/render/shutdown smoke tests on Windows and Linux.
- Windows and Linux development binaries uploaded; repository automation validation passed.

The workflow's remaining post-job cache cleanup is housekeeping and does not change the result of these completed build/test/smoke-test steps.

## Phase 4 completion gate

**Passed for Phase 4's own scope.** Home is populated by live built-in TMDB metadata; the featured carousel and built-in shelves use real, validated catalogue/artwork services; refresh and pagination are bounded and duplicate-safe; errors are isolated per shelf; local layout/title preferences and collection projections are versioned and normalized; catalogue identities prevent stale responses; and Home scroll/focus is restored across supported navigation flows.

The Windows/Linux automated acceptance for the individual slices is recorded above, including [CI #535](https://github.com/Sensei002/senzo/actions/runs/44271228972) for the provider registry, persisted collection projection and native interaction gates. Responsive Home geometry and window-mode acceptance are included in the Phase 2/4 close-out validation.

## Explicit later-phase hand-offs (not Phase 4 blockers)

- **Phase 6 — Stremio add-on ecosystem:** implement manifest/resource/capability matching, user-authorized add-on lifecycle and actual add-on catalogue requests. Phase 4's provider registry is a validated declaration/identity boundary only. No add-on is bundled, silently installed or enabled by default; Torrentio remains user-installed only.
- **Phase 11 — TV progress and continuity:** implement watched state, resume-position persistence and the progress model; then project real Continue Watching and release-aware shelves into Home. SENZO does not fabricate playback progress before playback exists.
- **Phase 12 — Library and collections:** implement collection/folder creation, editing, deletion, ordering and Library ownership; then update the bounded Home projection from that source of truth.
- **Phase 18 — final release acceptance:** verify physical mouse side-button delivery and conduct human visual/interactive review on actual Windows and Linux desktops. Automated test geometry and Xvfb smoke rendering do not claim to replace these release checks.

These are documented future deliverables with explicit owners. No Phase 4 implementation checkbox or built-in Home acceptance item remains open.

The pinned Nuvio Desktop source baseline and Home architecture findings are documented in `docs/NUVIO_SOURCE_AUDIT.md` and `docs/NUVIO_TO_SENZO_MAPPING.md`. In particular, Nuvio's Home feature uses stable catalogue definitions, descriptor signatures for cache identity, independent request publication, and a separate collection/progress boundary; SENZO is following those product boundaries with native Rust/GPUI equivalents rather than copying Kotlin/Compose code.
