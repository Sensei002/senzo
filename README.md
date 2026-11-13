# SENZO

<p align="center">
  <img src="assets/branding/senzo-logo.png" width="420" alt="Official SENZO logo" />
</p>

<p align="center">A native desktop media center for Windows.</p>

SENZO is being built with **Rust + GPUI** around a cinematic, library-first media experience. It is a native desktop application, not a web wrapper.

> **Status:** Early development. The current development phase targets Windows only. Linux support is planned for a later phase.

## Official brand asset

The image above is the original logo artwork supplied and approved by the project owner. It must be used as-is. Do not redraw, reinterpret, simplify, recolor, or substitute it. The transparent artwork from [Issue #2](https://github.com/Sensei002/senzo/issues/2) is the primary logo; the black-background artwork from [Issue #1](https://github.com/Sensei002/senzo/issues/1) is retained for contexts where that version is appropriate.

## Project principles

- Native Windows desktop UI built with Rust + GPUI.
- Linux support will be addressed in a later development phase; it is not part of current CI.
- MPV is the planned playback engine.
- SQLite is the planned local data store.
- Provider and add-on integrations are isolated behind internal interfaces.
- Windows builds and tests run through GitHub Actions.

## Development

Active development takes place on the `development` branch. The `main` branch is reserved for stable releases.

See [Architecture](docs/ARCHITECTURE.md) and [Roadmap](docs/ROADMAP.md).

## License

A license has not yet been selected. Until a license is added, all rights are reserved by the repository owner.
