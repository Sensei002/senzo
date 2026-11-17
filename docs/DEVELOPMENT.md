# SENZO development workflow

## Branches

- `main`: stable releases only.
- `development`: active Rust rewrite and integration work.

Do not push rewrite work directly to `main`.

## Reference and architecture

Before implementing a Nuvio feature, consult:
- [Pinned Nuvio source audit](NUVIO_SOURCE_AUDIT.md)
- [Feature mapping](NUVIO_TO_SENZO_MAPPING.md)
- [Rewrite architecture](ARCHITECTURE.md)
- [Product specification](PRODUCT_SPEC.md)
- [Roadmap](ROADMAP.md)

Port the desktop behavior and contracts, not Kotlin/Compose code or Nuvio branding. Keep Stremio add-ons, plugin scrapers, debrid providers, media servers, torrent transport and MPV playback behind independent service boundaries.

## Build and validation policy

GitHub Actions is the authoritative build and test environment for this project. Use CI to run formatting, lint, unit/integration tests and platform builds. Do not report a feature as complete merely because its source was committed or CI compiled it.

Runtime acceptance is separate from CI. Native video must be visibly rendered and audio audible on the target OS. Stop, source replacement, navigation, repeated sessions and shutdown must be tested.

## Implementation rules

- Keep GPUI event/render work non-blocking.
- Keep MPV calls off the UI thread and give the runtime one lifecycle owner.
- Use bounded channels and explicit cancellation/teardown.
- Use typed models and provider-neutral interfaces.
- Validate all external manifests, plugin data, URLs and responses.
- Never log credentials or private source URLs.
- Preserve user data through migrations and updates.
- Keep all player controls native GPUI; never add a browser UI.
- Keep Torrentio optional and user-installed.
- Add tests for protocol parsing, state transitions, storage migrations and lifecycle behavior.
