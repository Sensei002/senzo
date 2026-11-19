# Phase 9 — Native MPV playback proof and engine

**Status:** Complete for development implementation and automated audiovisual acceptance. Physical Windows/Linux monitor-and-speaker validation is retained as a Phase 18 release gate.  
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

## Automated headless audiovisual acceptance

Linux CI has a separate runtime acceptance path using the same `MpvPlayer` owner thread and command/snapshot API as the desktop application:

- It generates a local H.264/AAC test clip with a moving test pattern and a continuous sine-wave audio track; no external media URL or third-party service is involved.
- A small local HTTP server supports byte-range requests, so the test exercises the same validated HTTP(S) source-loading path and seek behavior as real streams.
- libmpv uses its GPU video output against an Xvfb display and PulseAudio against a virtual null sink. This keeps the test independent of physical monitors and speakers.
- CI captures the X11 display while playback is running, samples frames at 1 fps, and checks that multiple frames contain visible video and that successive visible frames differ (motion threshold). It separately records the virtual sink monitor and checks that decoded PCM contains non-silent samples.
- The application acceptance mode exercises track selection, pause, seek, resume/progress, volume, mute, speed, source replacement, stop acknowledgement, repeated playback and owner-thread shutdown. It does not auto-answer the physical operator prompts.

The generated clips, captured frames, PCM recording and MPV logs are uploaded as the `senzo-mpv-headless-diagnostics` workflow artifact. This is real libmpv decode/render/audio-output-path testing in virtual devices; it does **not** establish that a physical Windows or Linux monitor/speaker works. The original interactive `--mpv-smoke-test` remains available for physical-device acceptance.

## Phase 9 completion record

**Development gate passed:** November 19, 2026 — [SENZO Desktop CI run #683](https://github.com/Sensei002/senzo/actions/runs/44554085871).

The run passed Windows and Linux formatting, Clippy, workspace tests and native release builds. Windows also passed the native application startup/shutdown smoke test. Linux passed the actual libmpv headless audiovisual integration using generated H.264/AAC HTTP fixtures. The player decoded a 640×360 video stream and AAC audio, advanced playback, applied volume/mute/speed, paused, sought and resumed, replaced the source, stopped, started a second session and shut down its owner thread cleanly.

The virtual display capture contained a visible moving test pattern. The virtual audio monitor recorded 5.45 seconds of non-silent stereo PCM (44.1 kHz; peak 5,007; RMS 1,453.9). The CI artifact was independently inspected frame-by-frame: 5 of 12 sampled frames contained visible video and the maximum frame-to-frame difference was 3.07. Follow-up CI now enforces this frame-motion check automatically.

**Boundary:** This closes Phase 9's implementation and automated development gate. It does not claim that physical Windows or Linux monitors and speakers have been tested.

## Deferred physical-device validation — Phase 18 release gate

The following checks are intentionally deferred until packaging and release acceptance. They do not block Phase 10:

- Confirm visible moving video and audible audio on a real Windows desktop using the packaged/runtime libmpv build.
- Confirm visible moving video and audible audio on a real Linux desktop and its actual display server.
- On both platforms, manually verify pause/seek against live media and that navigation away, stop and application exit leave no MPV window or worker behind.

The render-context feasibility prototype is complete and the separate native output window remains the selected Phase 9 architecture. It is not an inline GPUI video surface. The interactive `--mpv-smoke-test` command remains the manual tool for these Phase 18 checks.
## libmpv API compatibility correction

The player now uses libmpv's documented `MPV_EVENT_IDLE = 11` value (rather than `MPV_EVENT_HOOK = 25`) and writes HTTP headers through the documented `options/http-header-fields` runtime option property. Stop completion leaves the snapshot in `Stopped`, including when MPV subsequently emits its idle notification. Regression tests lock the event ID and option-property name.