# Phase 2 — Native GPUI application shell and design system

**Status:** Implementation started on \`development\`. Acceptance remains pending until Windows and Linux CI pass and the interactive shell is exercised.

## Design reference

The implementation adapts Nuvio Desktop's desktop navigation and application-shell philosophy from the pinned source snapshot documented in \`NUVIO_SOURCE_AUDIT.md\`. SENZO uses its own Rust + GPUI CE components and visual tokens. No Kotlin/Compose code, Nuvio branding or third-party artwork is copied.

## Scope

- Native resizable window with a minimum usable size, native OS titlebar and close controls.
- Native fullscreen toggle through F11 and the application View menu.
- Persistent seven-destination navigation rail: Home, Movies, TV Shows, Search, Library, Add-ons and Settings.
- Typed route history with Back, Forward, Escape, Alt+Left/Right and mouse navigation buttons.
- Per-destination focus handles and focus restoration after route navigation.
- Tab/Shift+Tab focus traversal and directional sidebar navigation.
- SENZO black/cyan design tokens, spacing, typography and reusable status/feature components.
- Native application menus, in-app overflow menu, About and keyboard-shortcut dialogs, and dismissible notifications.
- Loading, empty, informational and error status primitives.
- Canonical SENZO logo embedded from the existing repository asset without changing its artwork.
- Accessibility labels, selected/expanded state, keyboard shortcut metadata and visible keyboard-focus styling.
- Deterministic route-history and design-token unit tests.

## Explicit boundaries

This phase creates the reusable shell and interaction system. It does not implement real TMDB discovery, artwork/catalogue loading, Stremio add-ons, media playback, or final production Home content. Those belong to their own roadmap phases. Placeholder routes must not invent titles or pretend that unimplemented services are connected.

## Acceptance checklist

- [ ] Windows and Linux formatting, Clippy and workspace tests pass.
- [ ] Native Windows and Linux release builds pass.
- [ ] Native window opens at a usable size with titlebar/close controls.
- [ ] Fullscreen toggles and returns to windowed mode.
- [ ] All seven destinations are reachable by mouse and keyboard.
- [ ] Back/Forward, Escape, Alt+Left/Right and mouse side buttons work.
- [ ] Focus is restored to the destination navigation item after route changes.
- [ ] Menus, About/shortcuts dialogs and notification dismissal work.
- [ ] Loading, empty, info and error primitives are available for feature routes.
- [ ] Canonical logo renders from the embedded PNG asset.
- [ ] Human visual review is completed on Windows and Linux at common desktop resolutions.

Automated smoke tests verify startup, render and shutdown. They do not replace the interactive navigation, accessibility or human visual acceptance checks above.
