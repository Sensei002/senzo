# SENZO development workflow

## Branches

- main: stable releases only.
- development: active Rust rewrite and integration work.

Do not push rewrite work directly to main.

## Toolchain

- Rust 1.95.0, pinned in rust-toolchain.toml.
- Rust edition 2024.
- GPUI CE gpui-ce 0.2.2.
- Native platform selector gpui_ce_platform 0.1.0.

GPUI CE 0.2.2 requires Rust 1.95 or newer. Keep the GPUI and platform crate versions pinned and update them together only after Windows and Linux CI validation.

## Workspace commands

    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace --all-targets
    cargo build --release -p senzo-app
    cargo run -p senzo-app

The CI workflow resolves and commits Cargo.lock after both platform jobs pass. Do not hand-edit the lockfile.

## Linux build dependencies

On Ubuntu/Debian, install the native dependencies used by CI:

    sudo apt-get update
    sudo apt-get install -y build-essential pkg-config libdbus-1-dev
    sudo apt-get install -y libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libx11-dev
    sudo apt-get install -y libxcb-shape0-dev libxcb-xfixes0-dev libxcb-randr0-dev libxcb-xinput-dev
    sudo apt-get install -y libegl1-mesa-dev libgles2-mesa-dev libglib2.0-dev libfontconfig-dev

A graphical session is required to launch the native GPUI shell. CI uses Xvfb for the Linux startup/shutdown smoke test.

## Architecture and reference policy

Before implementing a Nuvio feature, consult:
- Pinned Nuvio source audit (NUVIO_SOURCE_AUDIT.md)
- Feature mapping (NUVIO_TO_SENZO_MAPPING.md)
- Rewrite architecture (ARCHITECTURE.md)
- Product specification (PRODUCT_SPEC.md)
- Phase implementation record and roadmap

Port desktop behavior and contracts, not Kotlin/Compose code or Nuvio branding. Keep Stremio add-ons, plugin scrapers, debrid providers, media servers, torrent transport and MPV playback behind independent service boundaries.

## Implementation rules

- Keep GPUI event/render work non-blocking.
- The application crate owns one Tokio runtime for the process lifetime; UI code receives a handle and updates GPUI entities only through the GPUI async entity context.
- Keep MPV calls off the UI thread and give the runtime one lifecycle owner.
- Use bounded channels and explicit cancellation for long-running services.
- Use typed models and provider-neutral interfaces.
- Validate external manifests, plugin repositories, URLs and responses.
- Never log credentials, authorization headers or private source URLs.
- Store secrets in OS credential facilities; do not write them to SQLite or logs.
- Preserve user data through ordered, transactional migrations.
- Keep all player controls native GPUI; never add a browser UI.
- Keep Torrentio optional and user-installed.
- Use the hosted SENZO metadata API for TMDB access; never place a TMDB credential in the desktop process.
- Add tests for route contracts, malformed JSON, caching, cancellation, retry and storage migrations.

## Phase acceptance

A phase is complete only when its deliverables, automated checks and relevant runtime acceptance criteria pass. A successful compile is not proof that a native window opened, rendered, navigated and shut down correctly.
