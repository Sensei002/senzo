# Phase 9 — Native MPV playback proof and engine

**Status:** Engine and playback acceptance harness implemented; GPUI render-context feasibility prototype documented; physical Windows/Linux audiovisual acceptance pending  
**Branch:** development  
**Engine commit:** 5f73723765a7592c58862f19377dd58422fcc5ae  
**Native acceptance harness commit:** 539db0a103564df56198c31f3e8538f5c06b0594  
**Off-thread GPUI integration commit:** 5f73723765a7592c58862f19377dd58422fcc5ae

## Architecture

- Added the Rust `senzo-player` crate and a narrow typed command/snapshot boundary around libmpv.
- libmpv is loaded dynamically at runtime. The default loader checks platform library names; `SENZO_LIBMPV_PATH` overrides discovery with an explicit library path.
- Exactly one `senzo-mpv-owner` thread owns each libmpv context. Commands are serialized there; the UI never calls the libmpv C API.
- The client handle is cloneable while ownership remains singular. The final client drop signals shutdown and delegates thread joining to a reaper rather than blocking the GPUI thread.
- MPV uses its native GPU video output and a native MPV-owned output window (`force-window`, `vo=gpu`). SENZO does not embed WebView controls.
- `MpvConfig.window_id` preserves an optional native-window embedding seam, but the current GPUI shell deliberately uses the standalone native output window.

## Implemented engine contract

- Direct HTTP and HTTPS source loading with bounded, validated request headers; embedded URL credentials and unsupported schemes are rejected.
- Play, pause, toggle, absolute/relative seek, volume, mute, speed, audio-track selection and subtitle-track selection.
- Playback snapshots expose state, title, position/duration, volume/mute/speed, audio/video codecs and dimensions, and audio/subtitle track metadata.
- Event subscribers receive snapshot changes and bounded diagnostics. MPV logs are not copied verbatim into application logs because they can contain sensitive source details.
- Stop requests can be queued without blocking. The synchronous stop method waits for MPV's end-file/idle acknowledgement and is used only from a blocking worker in the acceptance harness.
- Source replacement uses MPV's `loadfile ... replace` command. Application navigation requests stop, cancels the refresh task and invalidates any in-flight source startup.
- MPV startup, source loading and control commands invoked from GPUI are dispatched through the Tokio blocking pool. Snapshot reads are lock-protected in-memory copies.

## Native GPUI integration

- Direct HTTP source cards and resolved debrid sources now expose Play in MPV actions.
- The native source panel shows playback status, current position/duration, volume and speed, and provides pause/resume, ±10-second seek, volume changes, mute, speed toggle, audio/subtitle track selection and stop.
- A 250 ms state refresh republishes native snapshot updates into GPUI; it is cancelled on stop and route transitions.
- Source initialization failures are presented in the native source panel, including the missing-libmpv runtime case.

## GPU/render-context decision

The Phase 9 feasibility prototype is recorded in PHASE_9_RENDER_CONTEXT_SPIKE.md. The pinned GPUI CE 0.2.2 renderer is WGPU-based and does not expose a supported external native-texture import surface. libmpv's render API uses its own supported graphics contexts (D3D11 or OpenGL) and imposes context/thread ownership requirements. Passing a D3D11 or OpenGL context directly into GPUI's WGPU renderer would therefore be an unsupported cross-backend assumption, not a working integration.

**Decision:** Keep the libmpv-owned native output window as the Phase 9 playback surface. The optional MpvConfig.window_id remains an embedding seam for a future native-window integration, not proof of inline GPUI rendering. A real WGPU/shared-texture bridge is explicitly deferred until GPUI exposes a supported import path and a platform-specific interop prototype is validated.

## Local native acceptance command

The application supports the command:

    senzo --mpv-smoke-test <direct-http-or-https-media-url>

Use a seekable video URL with at least one audio track and at least ten seconds of content remaining after the player starts. Install libmpv in the normal platform loader path or set SENZO_LIBMPV_PATH to the exact DLL/SO path. Run this command in an interactive terminal: it pauses for the operator to type YES separately after confirming moving video is visible and audio is audible. A missing or negative confirmation fails the command. The command opens the native MPV video output and exercises:

- Video stream start with nonzero dimensions and a Playing state.
- Audio-track metadata, explicit audio-track selection and subtitle disable.
- Pause state, seek, resume and a verified increase in the playback position.
- Volume, mute, speed and speed restoration.
- Source replacement while the owner thread stays alive.
- Stop acknowledgement and the published Stopped state.
- A second playback session and final worker shutdown.

The command requires two interactive operator confirmations while the first playback session is actively running: visible moving video and audible audio. This records an explicit human acceptance in the command output rather than merely printing a reminder. The harness also waits for the seeked position to update while paused before measuring post-resume progress. It cannot independently sense the display or audio device; the confirmation is an operator attestation, not machine verification.

## Automated regression coverage

The `senzo-player` unit tests cover direct HTTP(S) URLs, unsafe scheme/credentials/fragment rejection, header validation/bounds, control value limits, header escaping and native output options. Full workspace CI also builds and tests the application on Windows and Linux.

## Acceptance still required

Phase 9 is not closed until the following are verified on real desktops:

- Visible video and audible audio on a real Windows desktop with the packaged/runtime libmpv build.
- Visible video and audible audio on a real Linux desktop and the actual display server.
- Human observation that pause/seek behaves correctly against live media and that the native output window tears down cleanly on each platform.

The render-context feasibility prototype is complete and the separate native output window is the selected Phase 9 architecture. It is not an inline GPUI video surface. To close the remaining platform gate, run the native acceptance command on both OSes, observe actual picture and sound, and verify navigation away and application exit leave no MPV window or worker behind. The roadmap intentionally leaves the physical platform checks unchecked until those observations exist.

## libmpv API compatibility correction

The player now uses libmpv's documented `MPV_EVENT_IDLE = 11` value (rather than `MPV_EVENT_HOOK = 25`) and writes HTTP headers through the documented `options/http-header-fields` runtime option property. Stop completion leaves the snapshot in `Stopped`, including when MPV subsequently emits its idle notification. Regression tests lock the event ID and option-property name.