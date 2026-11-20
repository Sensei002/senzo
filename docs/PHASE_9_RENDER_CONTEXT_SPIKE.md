# Phase 9 — GPUI/libmpv render-context feasibility prototype

Status: Evaluated; a direct shared-GPU bridge is not supported by the pinned stack
Pinned GPUI: gpui-ce 0.2.2
Selected Phase 9 output: libmpv-owned native GPU window

## Question

Can libmpv render each decoded frame directly into an inline GPUI rectangle without CPU readback or relying on unsupported private GPUI renderer internals?

## Findings

1. SENZO pins GPUI CE 0.2.2 and uses GPUI's WGPU renderer. The published 0.2.2 feature/API surface exposes platform and testing features but no supported API for importing an arbitrary external D3D11/OpenGL texture into the GPUI compositor.
2. libmpv's render API requires a renderer context created before playback. Its documented hardware render backends are Direct3D 11 and OpenGL; its API also imposes render-thread and context-current requirements. In particular, an OpenGL render context must remain current on the thread that calls the render API.
3. A GPUI WGPU device is not interchangeable with a separately-owned D3D11 or OpenGL context. Correct zero-copy integration requires native shared-resource export/import and explicit synchronization for each operating system and graphics backend. The pinned GPUI version does not provide the public texture-import boundary needed to implement that safely.
4. Using libmpv's software render backend would avoid sharing a GPU context but would require copying full video frames into CPU memory and uploading them again for GPUI. That is not an acceptable default playback path for a media-center application and is not a zero-copy render-context integration.

## Prototype result and decision

The feasibility prototype is complete: a supported inline WGPU/libmpv bridge cannot be implemented against the pinned GPUI CE 0.2.2 public API without introducing a private GPUI fork or an unvalidated platform-specific interop layer.

Phase 9 therefore keeps the existing libmpv-owned native output window (vo=gpu) as the supported playback surface. This is a deliberate native architecture decision, not an assertion that inline video rendering works. MpvConfig.window_id remains an optional future native-window embedding seam.

A future inline player must first establish a supported GPUI external-texture/surface import contract, then prove D3D11-to-WGPU interoperability on Windows and OpenGL-to-WGPU interoperability on Linux with explicit GPU synchronization and device-loss handling. It must not be implemented by passing a graphics-context pointer across APIs.

## References

- libmpv render API: https://github.com/mpv-player/mpv/blob/master/include/mpv/render.h
- GPUI CE 0.2.2 published feature/API surface: https://docs.rs/crate/gpui-ce/0.2.2/features
- SENZO's pinned GPUI dependency: Cargo.toml

## Acceptance boundary

This prototype closes the feasibility investigation item in Phase 9. It does not close real-desktop playback acceptance. Windows and Linux still require a human to confirm visible video, audible sound, pause/seek behavior and clean output-window teardown on physical desktops.
