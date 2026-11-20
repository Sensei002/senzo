# SENZO

**A native desktop media center built on the Stremio Native Rust application.**

SENZO adopts the Stremio Native application foundation and develops its own product identity, interface, and release experience.

## Account access

- Sign in with an existing Stremio account when account-backed features are wanted.
- Choose **Guest login** to use the application without creating or signing into a Stremio account.
- SENZO does not operate its own hosted authentication service, user database, or application backend.
- The desktop application still uses local on-device storage for settings, profiles, library data, and other application state.

## Technology

- Rust workspace
- Slint native desktop UI
- Stremio Core integration
- Embedded streaming server
- libmpv playback
- Windows-first development and packaging

## Build from source

See the upstream build prerequisites and project scripts in this repository. The Windows native dependencies and vcpkg triplet are part of the build configuration; use the documented setup rather than substituting a generic vcpkg triplet.

## Upstream and license

SENZO is based on [Stremio Native](https://github.com/stremio-native/stremio-native), imported initially from the Sensei002 fork at commit `936ee48771bb3d9c242ebc029abe260dae60c617`.

This project retains the upstream **GPL-3.0-only** licensing and required notices. See [LICENSE](LICENSE) and [SENZO_UPSTREAM.md](SENZO_UPSTREAM.md).
