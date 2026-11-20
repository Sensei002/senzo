# Phase 10 — Native player experience

**Status:** Development implementation complete on the native MPV-window architecture  
**Branch:** development  
**UI:** Rust + GPUI CE 0.2.2  
**Playback:** Existing single-owner libmpv worker from Phase 9

## Delivered

- Full-screen GPUI player control plane that overlays the SENZO shell and returns to the active Details route on close.
- Pointer-driven control reveal with a five-second idle auto-hide. Escape closes the player, and player-scoped keyboard controls cover playback, seek, volume, mute, fullscreen, resize, audio, subtitles, chapters, stream information, sources and PiP.
- Timeline slider with click/drag seeking, elapsed and remaining time, current playback status and libmpv cache-buffering percentage.
- Native audio and subtitle track menus, including explicit subtitle disable.
- External subtitle loading from validated direct HTTP(S) URLs. Subtitle search results already returned by the Stremio add-on workflow can also be selected in the player.
- Subtitle visibility, font-size and delay controls.
- Fit, Fill, Stretch and Zoom resize modes; fullscreen toggle; native-window mode restoration during player teardown.
- Chapter list and chapter seeking, plus ±10-second skip controls.
- Stream information panel showing source, media metadata, video/audio codecs, resolution and track counts.
- Source panel with current source, source selection, next-source fallback, retry and surfaced playback errors. A synchronous MPV loadfile rejection automatically advances to the next available direct candidate; manual retry/next-source remain available.
- Best-effort PiP-style native output window using MPV always-on-top, borderless and reduced window-scale hints.

## Keyboard controls

| Key | Action |
|---|---|
| Space | Play / pause |
| Left / Right | Seek −10s / +10s |
| Up / Down | Volume +5 / −5 |
| M | Mute / unmute |
| F | Fullscreen / restore |
| R | Cycle Fit / Fill / Stretch / Zoom |
| A | Audio tracks |
| S | Subtitles |
| C | Chapters |
| I | Stream information |
| O | Sources / retry |
| P | PiP-style native window |
| Esc | Close player and return to Details |

## Important rendering boundary

The GPUI layer is the full-screen control plane, not the video renderer. Video remains in libmpv's separate native output window. Phase 9's render-context investigation established that the pinned GPUI CE 0.2.2 WGPU renderer has no supported MPV shared-texture import path. This implementation does not fake an embedded video rectangle or use WebView/WebView2. The player UI explicitly labels the native output arrangement.

Fullscreen and restoration target the MPV video window. The PiP action is a best-effort native window hint (always-on-top, borderless, reduced scale) rather than a compositor/window-manager-managed system PiP interface; behavior depends on platform and window manager.

## Explicit limitations

- The current metadata model does not include parental content-age classifications. The information panel shows an unavailable state instead of treating the ordinary metadata score as an age rating.
- Subtitle appearance currently covers visibility, font size and delay. Font family, colour, outline, background, margin and external font installation are not yet included.
- Add-on subtitle entries are displayed when an add-on subtitle search has already populated results. The player does not automatically issue an add-on subtitle query for each title at playback start.
- Source recovery automatically advances only when MPV rejects a loadfile command synchronously. Runtime stream failures still use surfaced error, retry and next-source actions; the UI does not silently infer all network failures from MPV warning logs.
- The player remains a native output window with GPUI controls. A true shared-surface embedded renderer is still deferred until the pinned GPUI renderer offers a supported interop path.

## Validation

The Phase 10 source changes are included in the Windows and Linux CI workflow. Formatting, Clippy, workspace tests, release builds, the Linux Xvfb/PulseAudio libmpv audiovisual acceptance and native shell lifecycle smoke tests remain mandatory. A green CI result validates those jobs, not physical display/audio behavior or every platform window manager's PiP hints.

Physical Windows/Linux video and audio review remains a Phase 18 release gate alongside clean installs, packaging and full cross-platform end-to-end review.
