# Phase 11 — TV playback, progress and episode continuity

**Status:** Development implementation complete; Windows automated acceptance gate enabled  
**Branch:** `development`  
**UI:** Rust + GPUI CE 0.2.2  
**Playback:** Native MPV/libmpv from Phase 9, controlled by Phase 10

## Delivered

- Episode-specific stream discovery uses the parent show ID with explicit season/episode coordinates in the provider-facing identity.
- Next/previous episode navigation works within a season and across adjacent seasons. Future release dates are checked before starting a not-yet-released episode.
- Autoplay-next is locally configurable and persisted. It waits for both the requested episode metadata and the next episode's source result, and verifies the returned season/episode identity before switching playback.
- Next-episode source selection prefers the previous source's Stremio `behaviorHints.bingeGroup`, then the previous provider, then stable discovery order. The original binge-group value is preserved through source normalization and the player source list.
- Per-movie and per-episode playback positions, artwork, completion and update timestamps are saved in SQLite. Checkpoints are flushed before player stop or episode navigation; writes are committed before the Home shelf refresh begins.
- Resume prompts offer Resume or Start Over. Resume seeks after MPV reports a usable duration and active playback state.
- Watched markers and playback progress are transactionally reconciled. Completion is recorded at 90%; short placeholder/error clips are excluded. Replaying a manually watched item clears its watched marker when a newer incomplete progress write is accepted.
- Home Continue Watching uses the latest incomplete episode per series and omits content at the completion threshold. Home Up Next combines explicit watched markers with completed progress, including a manually watched episode that had no playback checkpoint, and enriches its title/artwork from TV metadata.
- Intro, recap and credits skip targets are exposed when MPV chapter metadata contains recognizable chapter names.

## Windows regression and acceptance coverage

The Windows workflow runs the complete workspace test suite and an explicit Phase 11 gate. Targeted tests cover:

- Stremio binge-group parsing, source preference and provider/discovery-order fallbacks.
- Autoplay being blocked until both source and the exact requested episode metadata are ready.
- Watched/completed threshold behavior and filtering from Continue Watching.
- Manually watched episodes without progress, Up Next suppression when newer progress exists, and replay clearing a watched marker.
- Progress and watched-state persistence after closing and reopening the SQLite database.
- Named intro/recap/credits chapter skip targets.
- Existing next/previous, source switching, episode identity, resume and progress repository behavior.

The Windows CI gate also performs formatting, Clippy, all workspace tests, a release build, native application startup/shutdown smoke test and Windows artifact upload.

## Explicit boundaries

- Chapter skipping only works when the selected source provides usable MPV chapter titles and times. SENZO does not query a remote IntroDB/SkipIntro service from the desktop player in this phase.
- Binge-group continuity automatically selects matching direct HTTP sources. Torrent/P2P transport remains Phase 13; source types that cannot be passed directly to MPV continue through their provider-specific/manual flow.
- Playback progress and watched state are local and default-profile scoped. Multi-profile and cloud sync belong to Phase 12.
- CI builds and smoke-tests the Windows native application; physical monitor/speaker playback and final human visual review remain Phase 18 release acceptance. A green CI run is not presented as proof of physical-device output.

## Gate

Phase 11 is complete for the Windows development implementation when the targeted regression tests, full Windows workspace checks, native release build and startup/shutdown smoke test all pass.