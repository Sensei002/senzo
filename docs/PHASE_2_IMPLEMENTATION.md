# Phase 2 — Native GPUI application shell and design system

**Status:** Implementation and automated CI acceptance passed. Manual visual acceptance remains pending.

## Design reference

The implementation adapts Nuvio Desktop's desktop navigation and application-shell philosophy from the pinned source snapshot documented in `NUVIO_SOURCE_AUDIT.md`. SENZO uses its own Rust + GPUI CE components and visual tokens. No Kotlin/Compose code, Nuvio branding or third-party artwork is copied.

## Implemented

- Native resizable window with a 1280×720 initial size, 1024×680 minimum size, native OS titlebar and close controls.
- Native fullscreen toggle through F11, Alt+Enter and the application View menu.
- Persistent seven-destination navigation: Home, Movies, TV Shows, Search, Library, Add-ons and Settings.
- Typed route history with Back, Forward, Escape, Alt+Left/Right and native mouse navigation-button handlers.
- Per-destination focus handles and focus restoration after route navigation and dialog dismissal.
- Tab/Shift+Tab focus traversal and wrapping directional sidebar navigation.
- SENZO black/cyan design tokens, spacing, typography and reusable feature/status components.
- Native application menus, in-app overflow menu, About and keyboard-shortcut dialogs, and dismissible notifications.
- Loading, empty, informational and error status primitives.
- Canonical SENZO logo embedded from the existing repository asset without changing its artwork.
- Accessibility labels, selected/expanded state, keyboard shortcut metadata and visible keyboard-focus styling.
- Unit tests for route history and design tokens.
- A GPUI interaction test covering all seven destinations, directional wraparound, Back/Forward, Escape, keyboard-shortcut dialog dismissal and focus restoration.

## CI acceptance

**Successful run:** [SENZO Desktop CI #44088401293](https://github.com/Sensei002/senzo/actions/runs/44088401293)

- [x] Windows formatting, Clippy and workspace tests.
- [x] Linux formatting, Clippy and workspace tests.
- [x] Windows x64 native release build.
- [x] Linux x64 native release build on Ubuntu 24.04.
- [x] Windows GPUI window startup, render and shutdown smoke test.
- [x] Linux GPUI window startup, render and shutdown smoke test under Xvfb.
- [x] GPUI keyboard route/focus interaction test.
- [x] Windows and Linux development binaries uploaded as workflow artifacts.

**Artifacts:**
- `senzo-windows-x64-foundation-29397f507b2f89692f657a1266ac6eb9e3e240a8`
- `senzo-linux-x64-foundation-29397f507b2f89692f657a1266ac6eb9e3e240a8`

Download both from the [Actions run](https://github.com/Sensei002/senzo/actions/runs/44088401293). These are development binaries, not production installers or stable releases.

## Remaining manual acceptance

- [ ] Verify mouse-click navigation and mouse side-button Back/Forward on Windows.
- [ ] Verify mouse-click navigation and mouse side-button Back/Forward on Linux.
- [ ] Review windowed/fullscreen transitions and the complete visual layout at common desktop resolutions on both platforms.

The CI smoke tests prove native startup, rendering and shutdown. The GPUI interaction test proves keyboard navigation and focus behavior. They do not replace human visual review or physical mouse testing.

No pixel screenshot baseline is committed yet. The Linux CI runner uses Xvfb for a native startup smoke test; it is not currently configured as a deterministic cross-platform pixel-regression environment.

## Explicit boundaries

This phase creates the reusable shell and interaction system. It does not implement real TMDB discovery, artwork/catalogue loading, Stremio add-ons, media playback, or final production Home content. Those belong to their own roadmap phases. Placeholder routes must not invent titles or pretend that unimplemented services are connected.
