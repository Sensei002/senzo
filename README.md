# SENZO

<p align="center">
  <img src="assets/branding/senzo-logo.png" width="420" alt="Official SENZO logo" />
</p>

<p align="center">A native desktop media center for Windows.</p>

SENZO is being built with **Rust + GPUI** around a cinematic, library-first media experience. It is a native desktop application, not a web wrapper.

> **Status:** Early development. The current development phase targets Windows only. Linux support is planned for a later phase.

## Product principles

- **Accountless:** no SENZO sign-up, login, or cloud account.
- **Local-first:** watch history, playback resume positions, settings, and installed add-ons are stored in the user's local SENZO app-data directory.
- **TMDB metadata:** movie and TV search, details, cast, artwork, trailers, seasons, and episodes are powered by TMDB.
- **Manual add-ons:** no streaming add-on is bundled or preinstalled. Users explicitly paste a Stremio-compatible manifest URL to add a source.
- **Torrentio is optional:** Torrentio is not built into SENZO's default source list. Users who choose to use it can manually add its configured manifest URL.
- **Extensible sources:** additional providers can be added later behind the same source interface.
- **Native playback:** MPV is the planned playback engine.

## Official brand asset

The image above is the original logo artwork supplied and approved by the project owner. It must be used as-is. Do not redraw, reinterpret, simplify, recolor, or substitute it. The transparent artwork from [Issue #2](https://github.com/Sensei002/senzo/issues/2) is the primary logo; the black-background artwork from [Issue #1](https://github.com/Sensei002/senzo/issues/1) is retained for contexts where that version is appropriate.

## Development

Active development takes place on the development branch. The main branch is reserved for stable releases. Windows builds and tests run through GitHub Actions; Linux CI is intentionally deferred.

See [Architecture](docs/ARCHITECTURE.md) and [Roadmap](docs/ROADMAP.md).

## TMDB attribution

This product uses the TMDB API but is not endorsed or certified by TMDB. TMDB data and artwork are subject to TMDB's terms and attribution requirements.

## License

A license has not yet been selected. Until a license is added, all rights are reserved by the repository owner.
