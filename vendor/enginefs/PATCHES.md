# Vendored `enginefs`

Copied verbatim from `stremio-native/stream-server` revision
`f585ab6eda9b1411034548c131bb0dc30c6f5f9e`, then patched for SENZO.

## Why this lives here

`DISK_READINESS_BUFFER_BYTES` and `DISK_READINESS_MAX_PIECES` gate the first
HTTP byte of a torrent stream. Upstream waits for an 8 MiB window capped at
four pieces; with 16 MiB pieces that is 64 MiB of the selected file before
playback starts. On a 1080p episode small enough to fit in 192 MiB this shows
up as a third of the bar filled before the first frame.

The vendored copy targets one verified piece. That preserves the guarantee
that bytes are only served after libtorrent verifies them, while letting the
player's own read-ahead fill the rest of the window concurrently. The server
still prioritizes the initial three-piece cluster, so the next pieces are
already in flight when mpv asks for them.

## Re-vendoring a new stream-server release

1. Copy `enginefs/` from the new checkout over this directory.
2. Replace the `libtorrent-sys` path dependency with the same git revision
   used by the root `Cargo.toml`.
3. Reapply the one-piece readiness constants in
   `src/backend/libtorrent/handle.rs` (search for `SENZO:`).
4. Run `cargo metadata` and a release build; the patch must resolve
   `enginefs 0.1.8` from `vendor/enginefs`, not the git checkout.
