# Phase 2 — Native GPUI application shell and design system

**Status:** Complete on `development` for the native shell, interaction model, responsive layout fixtures and automated Windows/Linux acceptance. Physical HID delivery and final human visual review are explicitly release-only checks in Phase 18; they are not unimplemented Phase 2 code.

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
- The same interaction test performs a pointer click on the rendered Movies navigation item and dispatches GPUI mouse Navigate Back/Forward events, validating the route-history behavior used by physical side buttons.
- The rendered shell is checked at 1024×680 (minimum), 1280×720 (standard) and 1920×1080 (Full HD): the fixed sidebar, page viewport, featured hero and Home refresh control must remain within the available window geometry.
- F11 and Alt+Enter are exercised as state transitions: each enters/exits fullscreen and restores the original window mode.

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
- [x] Windows and Linux pointer-click plus mouse Navigate Back/Forward interaction assertions in [SENZO Desktop CI #535](https://github.com/Sensei002/senzo/actions/runs/44271228972).
- [x] Windows and Linux release builds, GPUI startup/render/shutdown smoke tests and development binary uploads in [SENZO Desktop CI #535](https://github.com/Sensei002/senzo/actions/runs/44271228972).
- [x] Responsive geometry assertions at 1024×680, 1280×720 and 1920×1080, plus F11/Alt+Enter state-transition tests, in the Phase 2/4 close-out validation.

**Artifacts:**
- `senzo-windows-x64-foundation-29397f507b2f89692f657a1266ac6eb9e3e240a8`
- `senzo-linux-x64-foundation-29397f507b2f89692f657a1266ac6eb9e3e240a8`

Download both from the [Actions run](https://github.com/Sensei002/senzo/actions/runs/44088401293). These are development binaries, not production installers or stable releases.

## Completion and release-only validation

Phase 2's implementation and automated acceptance criteria are complete. Windows and Linux CI validate compilation, Clippy, workspace tests, native release builds, startup/render/shutdown, pointer interaction, synthesized mouse Navigate events, responsive geometry and fullscreen keyboard actions.

- [x] Automated GPUI event routing for Back/Forward via mouse Navigate events on Windows and Linux.
- [x] Deterministic structural geometry fixtures at the minimum, standard and Full HD window sizes.
- [x] F11 and Alt+Enter fullscreen transitions and restoration of the previous window mode.
- [x] Keyboard route traversal, directional wraparound, dialog dismissal and focus restoration.

The pinned GPUI CE 0.2.2 test-support API exposes rendered bounds and input simulation, but not pixel-frame readback. For that reason, deterministic geometry/input regression fixtures are used instead of pretending to have pixel screenshot baselines.

The following checks remain explicitly assigned to the final Windows/Linux release acceptance in [Phase 18](ROADMAP.md#phase-18--cross-platform-packaging-and-nuvio-parity-release), because they require real physical devices or human visual judgement. They do not block Phase 5:
- [ ] Verify physical mouse side-button Back/Forward delivery with actual hardware on Windows and Linux.
- [ ] Review windowed/fullscreen and complete visual layout on real Windows and Linux desktops, including common resolutions and display scaling.

The CI smoke tests prove native startup, rendering and shutdown. Automated geometry tests check layout containment, not artistic/pixel-perfect visual quality. Real hardware and human review remain mandatory before a stable release.

## Explicit boundaries

This phase creates the reusable shell and interaction system. It does not implement real TMDB discovery, artwork/catalogue loading, Stremio add-ons, media playback, or final production Home content. Those belong to their own roadmap phases. Placeholder routes must not invent titles or pretend that unimplemented services are connected.
