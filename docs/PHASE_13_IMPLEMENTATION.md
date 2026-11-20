# Phase 13 — Downloads and P2P transport

**Implementation target:** Windows-first native desktop build on `development`. Linux CI remains deferred as requested.

## Delivered

### Local HTTP downloads

- Added the `senzo-downloads` Rust crate, separate from source discovery and MPV playback.
- Added a profile-scoped, durable queue with queued/downloading/paused/completed/failed states, progress, retry with bounded backoff, pause, resume, cancel and completed-file removal.
- HTTP transfers write to a `.part` file, request `Range: bytes=<offset>-` when resuming, validate the server's `Content-Range`, restart safely if a server ignores or invalidates the range, and atomically finalize a completed file.
- A restart converts in-flight records to Paused while retaining partial bytes for a later resume. Missing completed files are surfaced as Failed rather than silently playable.
- Source URLs and request headers are stored in the operating-system credential vault. The queue JSON contains profile/media metadata and file paths, not provider URLs or authorization headers.
- The new Downloads screen supports pause/resume/cancel/remove and Play Offline. MPV accepts a local `file:` URL only when it resolves to an existing local file; remote file hosts are rejected.

### P2P/torrent playback

- Added a transport boundary around `librqbit 9.1.0`, reusing the prior SENZO prototype's Rust implementation after comparing its selected-file stream and byte-range semantics.
- Torrent actions are shown separately from direct HTTP Play and Debrid resolve actions. P2P is disabled by default and must be explicitly enabled in the current profile under Settings.
- The consent notice explains that the public IP address is visible to peers and asks users to access only media they have rights to use.
- Uploading is disabled in this build. The engine is configured without peer upload, UPnP, local service discovery or an externally-bound stream listener, and peer concurrency is bounded.
- The engine enumerates supported video files, honours a provider-supplied file index when available, and otherwise selects the first supported video file. The current UI does not yet provide a separate multi-file picker.
- Selected-file streams are served from a random loopback-only HTTP endpoint. Byte-range requests support seeking. Engine progress reports downloaded/total/uploaded byte counts and state in the player controls.
- Torrent temporary directories are isolated per session and protected by cross-process file locks. Normal playback close stops MPV first and then shuts down the torrent server/engine. Startup recovery removes unlocked abandoned session directories while preserving active sessions owned by another SENZO instance.

## Automated regression coverage

- A local Axum HTTP fixture seeds an interrupted partial download, returns a real HTTP 206 response for the Range request, and verifies the completed file's bytes.
- The same test verifies startup recovery to Paused, profile isolation, local-file playback URL creation, and that the queue file does not contain source URLs or authorization headers.
- Unit tests cover safe file naming, header validation, Content-Range parsing, torrent info-hash/range validation and MPV local-file URL validation.
- Windows CI runs the Phase 13 crate acceptance tests, full workspace tests, Clippy, formatting, native Windows release build and application startup/shutdown smoke test.

## Acceptance boundary

The automated HTTP download path is exercised against a local server. The torrent transport's metadata, file-selection, loopback server, byte-range, statistics and cleanup paths have unit-level coverage, but a live public torrent swarm was not exercised by the hosted GitHub runner. Final real-source validation still needs a user-selected, legally accessible torrent source to confirm real-peer availability, sustained buffering, seek behavior and shutdown on a Windows desktop. Linux validation remains deferred. The implementation must not be described as having passed live torrent playback until that check is run.

## Security and lifecycle notes

- Torrent peer networking is an explicit per-profile opt-in.
- P2P upload remains disabled; the app does not seed files for other peers.
- HTTP source authorization data stays in the OS credential vault, not queue files or logs.
- The temporary P2P HTTP server binds to loopback only and uses a per-session token.
- The session folder is retained for startup cleanup if orderly shutdown cannot finish; the app does not recursively delete arbitrary paths.
