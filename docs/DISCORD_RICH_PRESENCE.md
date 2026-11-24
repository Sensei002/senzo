# Discord Rich Presence

SENZO has one Rich Presence implementation, in `app/src/discord.rs`. The player
feeds it from `app/src/mpv_integration.rs`; no second RPC client or duplicate
activity publisher exists.

## Application identity

Rich Presence uses a Discord application ID. The ID previously in the source
(`1452620752263319665`) belongs to **Stremio's official Rich Presence
application**, not SENZO. It was removed. Using another application's ID also
makes Discord display that application's name, which is why the header read
"Watching Stremio".

A SENZO-owned application must be created in the
[Discord Developer Portal](https://discord.com/developers/applications), then
configured at build time:

| Variable | Purpose |
| --- | --- |
| `SENZO_DISCORD_APP_ID` | The SENZO-owned Discord application ID. |
| `SENZO_DISCORD_FALLBACK_IMAGE` | Optional Rich Presence asset key or external image URL. Defaults to the SENZO logo URL in this repository. |

By default the fallback is the SENZO logo already in this repository, served
over HTTPS from the `development` branch. You can instead upload the logo as a
Rich Presence asset named `senzo_logo` and set
`SENZO_DISCORD_FALLBACK_IMAGE=senzo_logo`. The presence always uses
`large_text = "SENZO"`; the former `stremio_logo` fallback is gone.

If `SENZO_DISCORD_APP_ID` is empty or unset, the worker stays disconnected, logs
one warning, and never attempts an IPC connection. SENZO continues normally.

For GitHub Actions, define the repository variable
`SENZO_DISCORD_APP_ID` (and optionally `SENZO_DISCORD_FALLBACK_IMAGE`).
`release.yml` passes both to the build. A local build can set the same
environment variables before `cargo build`.

## What is shared

- Movie title, or series name.
- For episodes, `S01E02` plus the episode title when metadata provides it.
- `Watching` or `Paused`.
- For active playback, Discord start/end timestamps produce elapsed/remaining
  time. Paused presence shows a static `current / duration` value so it does not
  keep counting.
- Poster/background artwork when the metadata provides it.
- The SENZO logo as the fallback image.

SENZO never sends stream URLs, magnets, debrid tokens, cookies, provider
credentials, account identifiers, local paths, or query strings to Discord.

## Settings

The existing toggle in Settings → Interface and the onboarding dialog writes
`discord_rpc_enabled` through the profile settings action. It is persisted per
profile by the existing `update_profile_settings` path. SENZO does not enable
presence on its own; an unset or false existing preference is respected.
Disabling the toggle disconnects the worker, clears the activity, and leaves the
player and catalog code untouched.

## Lifecycle and failure behavior

- A dedicated worker thread owns the IPC client; the UI thread never blocks on
  Discord.
- The worker retries with exponential backoff from 2s to 30s. Repeated failures
  are logged once at warning level, then at debug level.
- Missing Discord, a closed client, a rejected pipe, or a failed `SET_ACTIVITY`
  marks the connection lost and schedules a retry. Playback continues.
- Clearing happens on player stop/unload, on disabling the setting, and on
  client shutdown. Disconnect attempts `clear_activity()` before closing IPC.
- The `DiscordRpc` sender has a `Drop` implementation that requests disconnect.

## Verification status

Source-level verification covers the payload format, title/episode projection,
timestamps, fallback branding, settings persistence path, and lifecycle code.
**Live Rich Presence has not been verified against a running Discord client for
this change.** The previous runtime test used the Stremio application ID, which
is not an acceptable SENZO identity. A live test requires a SENZO-owned
`SENZO_DISCORD_APP_ID` and (optionally) the uploaded `senzo_logo` asset.

Test checklist once those exist:

1. Start Discord desktop on Windows and Linux.
2. Build with `SENZO_DISCORD_APP_ID` set and start SENZO.
3. Enable the Settings toggle; confirm the activity appears under the SENZO
   application name, not Stremio.
4. Play a movie; confirm title, Watching, and elapsed/remaining timestamps.
5. Play a series episode; confirm series name, episode code/title, and status.
6. Pause/resume; confirm Paused shows a static time and Watching resumes.
7. Stop playback; confirm the activity disappears.
8. Disable the toggle; confirm the activity disappears and re-enabling restores
   it.
9. Quit Discord while SENZO is running; confirm SENZO does not freeze and
   reconnects after Discord returns.
10. Quit SENZO; confirm the activity disappears.
