# SENZO UI design specification

## Goal

SENZO should feel like a dedicated media console when opened: an inviting, artwork-led home screen where users browse movie and television recommendations by category, move naturally between rows, open a title, choose a source, and watch. The current engineering-oriented interface is not the intended final product.

The primary visual and interaction reference is the Home experience in the official [Nuvio Desktop repository](https://github.com/NuvioMedia/NuvioDesktop/tree/Dev). Match its overall homepage structure and browsing feel closely, while keeping SENZO's own branding, logo, visual assets, and native Rust + GPUI implementation.

## Home screen composition

1. **Application navigation**
   - Clear Home, Movies, TV Shows, Search, Library, Add-ons, and Settings destinations.
   - Navigation should be compact and should not consume the visual focus of the Home screen.
   - Home is the default landing destination.

2. **Featured hero**
   - Large cinematic backdrop with sensible image fallback.
   - Featured title, year, media type, genres, rating, and concise overview when metadata exists.
   - Prominent Play and Details actions.
   - Hero content can be selected from available discovery catalogs; no fake titles or fabricated recommendations.

3. **Continue Watching**
   - Appears only when the local history contains in-progress items.
   - Shows meaningful progress and episode context for TV.
   - Activating an item resumes through the existing playback flow.

4. **Horizontal category shelves**
   - Each shelf has a category heading, a horizontal poster rail, and View All when a full catalog route exists.
   - Initial metadata shelves: Trending, Popular Movies, Popular TV.
   - Add-on shelves are generated dynamically from catalogs declared by enabled, user-installed Stremio add-ons.
   - Optional shelves may include genres and user collections when their data sources are available.
   - Users can hide and reorder shelves; empty shelves are omitted or show a deliberate, non-intrusive empty state.

5. **Poster cards**
   - Artwork-first cards with correct aspect ratios and graceful fallback images.
   - Title and year are readable without opening the detail page.
   - Watched/progress markers are subtle but visible.
   - Pointer hover may reveal a preview, but directional focus is the primary interaction model.

## Console-like interaction

- Full-window, vertically scrolling Home page containing horizontally scrolling shelves.
- Consistent left/right movement within a shelf and up/down movement between shelves.
- Visible focus treatment and sensible focus restoration after opening/closing a detail view.
- Keyboard operation for all primary actions, including directional keys, Enter/Space, and Back/Escape.
- Game-controller support should be added where the native GPUI/input APIs permit it without compromising keyboard accessibility.
- Smooth but restrained motion; no excessive animations or background polling.
- Large, legible controls and artwork at common desktop resolutions and Windows display scales.
- Mouse remains supported but the interface must not require mouse hover to discover or activate content.

## Data and behavior

- Home shelves are data-driven, not hard-coded placeholder cards.
- TMDB is the metadata source for built-in discovery shelves.
- Add-on shelves are derived only from enabled, explicitly user-installed providers. No add-ons are bundled or enabled by default.
- Loading, partial results, network errors, missing artwork, and no-installed-add-on states must all be handled.
- Lazy-load poster/backdrop images and catalog pages; use bounded caches.
- Preserve shelf ordering, scroll positions, and focus where practical when navigating.
- Hero, poster, Details, View All, and Play actions must connect to actual metadata, episode, source-selection, and MPV flows.
- Keep technical settings, diagnostics, and provider management accessible but separate from the main browsing surface.

## Implementation reference and adaptation rules

Study these official Nuvio Desktop source files before implementing the corresponding SENZO feature:

- `composeApp/src/commonMain/kotlin/com/nuvio/app/features/home/HomeScreen.kt`
- `composeApp/src/commonMain/kotlin/com/nuvio/app/features/home/HomeRepository.kt`
- `composeApp/src/commonMain/kotlin/com/nuvio/app/features/home/HomeCatalogDefinitions.kt`
- `composeApp/src/commonMain/kotlin/com/nuvio/app/features/home/components/HomeHeroSection.kt`
- `composeApp/src/commonMain/kotlin/com/nuvio/app/features/home/components/HomeCatalogSection.kt`
- `composeApp/src/commonMain/kotlin/com/nuvio/app/features/home/components/HomePosterCard.kt`
- `composeApp/src/commonMain/kotlin/com/nuvio/app/features/home/components/HomeSectionLayout.kt`

Use these as architecture and UX references for dynamic catalog shelves, hero composition, poster rails, layout sizing, and Home data orchestration. SENZO must implement equivalent native components and state management in Rust + GPUI. Do not copy Kotlin/Compose implementation code, logos, artwork, or other assets. Preserve SENZO's independent identity.

## Acceptance criteria

- Opening SENZO presents a finished-looking Home screen rather than a developer/settings panel.
- The Home screen shows a real featured item and multiple populated category shelves when metadata is available.
- Continue Watching appears only when applicable.
- Enabled add-on catalogs can contribute shelves without any provider being preinstalled.
- Users can navigate the entire Home experience with keyboard; controller navigation is included where supported.
- Poster selection opens the correct title details; Play/source selection reaches the existing playback pipeline.
- Focus and scroll position behave predictably across Home/details/back navigation.
- Loading, error, empty, and missing-artwork states are polished.
- Windows CI passes, and the resulting packaged application is manually reviewed at common desktop resolutions before this batch is marked complete.

## Settings experience

Settings must be redesigned as a simple, approachable consumer-facing screen, using the official Nuvio Desktop Settings experience as the primary UX reference. The overall SENZO interface should follow Nuvio's user-facing layout and interaction philosophy closely (approximately 90% similarity in structure, clarity, and behavior), while retaining SENZO's own identity, logo, visual language, and original implementation.

- Present a small set of clearly named, plain-language categories and everyday controls first (for example: Appearance, Playback, Subtitles, Language, and About).
- Use short descriptions and sensible defaults. Avoid exposing raw backend terminology, credential fields, renderer internals, decoder flags, cache limits, or diagnostics in the normal settings flow.
- Use progressive disclosure: place infrequently changed or technical options under clearly labelled Advanced sections, with safe defaults and brief explanations.
- Keep diagnostics and support actions in a separate, low-prominence area.
- Settings must be keyboard accessible, focusable, readable, and consistent with the console-style navigation model.
- Persist user choices locally and make reset/default behavior clear.

## Built-in provider integrations and credentials

TMDB metadata/discovery and OpenSubtitles subtitle search must be enabled by default in official SENZO builds. An ordinary user must not be asked to obtain, paste, or manage API credentials to use these integrations.

- Use application-managed integration credentials/configuration supplied through the official release/build process, never committed to source control or printed in logs.
- Do not expose API-key entry as a required first-run or ordinary Settings task. Keep optional developer/self-build override configuration in Advanced settings or documented environment/build configuration only where necessary.
- If an official integration credential is unavailable or a provider is temporarily unreachable, show a clear, friendly status and preserve the rest of the app; never present a raw credential error as the normal user experience.
- Explain provider attribution and independent service availability in About/Credits.
- This requirement does not authorize a SENZO account, mandatory cloud profile, or remote watch-history service.

## Settings acceptance criteria

- A first-time user can understand and use the main Settings screen without knowing what TMDB, API keys, decoders, renderers, or cache internals are.
- TMDB discovery and OpenSubtitles search work in an official build without user-entered credentials.
- Advanced configuration remains available without dominating the normal Settings experience.
- No API secret is committed to the repository, embedded in public source, or written to logs. Release credential handling must be reviewed for provider terms and practical client-side exposure before public distribution.
