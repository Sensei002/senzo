# SENZO

**A native desktop media center built with Rust and Slint.**

SENZO is a rebranded desktop application based on the open-source [Stremio Native](https://github.com/stremio-native/stremio-native) project. It retains the upstream Rust application and media-engine foundation while developing a distinct SENZO identity and product experience.

> SENZO is an independent community project. It is not affiliated with, endorsed by, or maintained by the official Stremio project.

## Account access: optional

SENZO uses the existing Stremio account system when a user chooses to sign in. **Signing in is optional.**

- **Sign in:** use an existing Stremio account for account-backed features and synchronization provided by Stremio.
- **Continue without signing in:** enter the application as a guest without creating or logging into an account.
- **Guest data:** application settings, local profiles, library state, caches and other supported data are stored on the device.

SENZO does not operate its own hosted authentication service, user accounts, cloud database or application API. The application does keep a **local on-device database** because guest mode and local application features need persistent storage. This is not a SENZO-operated online database.

## Included application foundation

The imported Stremio Native foundation includes the following areas. Individual features are being rebranded and validated as part of the SENZO migration:

- Native Rust desktop application with a Slint UI.
- Stremio Core integration and compatible add-on workflows.
- Embedded streaming server and torrent-streaming support.
- Stream ranking and community add-on integration.
- Native MPV playback, hardware decoding, subtitles and playback controls.
- Debrid provider integrations.
- Local media library scanning and download management.
- Local profiles, PIN protection, themes and configurable hotkeys.
- Native desktop media controls and operating-system credential storage.

Some upstream features may still have limitations; see the upstream changelog and SENZO issue tracker for current implementation status.

## Technology

- **Language:** Rust (2024 edition)
- **Desktop UI:** Slint
- **Media playback:** libmpv
- **Streaming:** embedded Rust stream server / libtorrent
- **Local persistence:** embedded Turso database
- **Account integration:** Stremio account services, optional

## Windows build

SENZO is being developed and validated Windows-first.

### Prerequisites

- Windows 10/11 x64.
- Visual Studio 2022 C++ Build Tools with the MSVC v143 toolset and Windows SDK.
- The pinned Rust toolchain: `nightly-2026-07-31`.
- LLVM/Clang with `libclang.dll`.
- vcpkg at the revision and triplet defined by the repository.

### Build

Install the native manifest dependencies from the repository root:

```powershell
& "$env:VCPKG_ROOT\vcpkg.exe" install `
  --x-manifest-root="$PWD" `
  --x-install-root="$PWD\vcpkg_installed" `
  --triplet=x64-windows-v3-static-md-release `
  --overlay-triplets="$PWD\triplets" `
  --overlay-ports="$PWD\vcpkg-overlays"
```

Then build the desktop application:

```powershell
cargo build --locked --release --package senzo
```

To create the Windows installer and updater archive, install Inno Setup 6 and run:

```cmd
setup\create_setup.cmd
```

The installer script also stages the app-local MSVC runtime and the MPV license notices. Use the repository's pinned vcpkg triplet; the static CRT triplet is not interchangeable with the dynamic-CRT configuration.

Linux packaging is not part of the current Windows-first migration validation and will be addressed separately.

## Upstream and licensing

SENZO's initial source snapshot was imported from the Sensei002 fork of Stremio Native at commit `936ee48771bb3d9c242ebc029abe260dae60c617`. The original upstream project is available at [stremio-native/stremio-native](https://github.com/stremio-native/stremio-native).

SENZO retains the upstream **GPL-3.0-only** license and applicable third-party licenses and notices. See [LICENSE](LICENSE) and [SENZO_UPSTREAM.md](SENZO_UPSTREAM.md).

## Project links

- [SENZO source code](https://github.com/Sensei002/senzo)
- [Report an issue](https://github.com/Sensei002/senzo/issues)
- [Upstream Stremio Native](https://github.com/stremio-native/stremio-native)
