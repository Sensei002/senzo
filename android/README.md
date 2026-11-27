# SENZO for Android

The Android client of the same Stremio account the desktop application uses.
It is a native Kotlin and Jetpack Compose application: the login, the addon
collection and the addon protocol are implemented directly, so every addon
installed on the account is available on the phone as well.

## What it does

- Signs in with the Stremio account (the password goes to `api.strem.io` only).
- Reads the addon collection of that account and caches it for the next start.
- Shows the board as one row per addon catalogue, like the desktop Board tab.
- Searches every addon that advertises the `search` extra.
- Opens details, seasons and episodes from the addon metadata.
- Plays the HTTP streams the addons return, with the request headers an addon
  asks for in `behaviorHints.proxyHeaders`.

Torrent infohashes are not resolved on the device: a stream that only carries
`infoHash` is skipped, because playing it needs a torrent engine. Streams that
provide an HTTP URL play directly.

## Building

The build needs JDK 17 or newer and the Android SDK with platform 35.

```bash
cd android
gradle :app:assembleRelease
```

The version name is read from `app/Cargo.toml`, so the APK and the desktop
release always carry the same version. CI overrides it with
`-PsenzoVersionName=<tag>` and `-PsenzoVersionCode=<build number>`.

## Signing

A release build is signed when a keystore is available. Either export

| Variable | Meaning |
| --- | --- |
| `SENZO_KEYSTORE` | path to the keystore |
| `SENZO_KEYSTORE_PASSWORD` | store password |
| `SENZO_KEY_ALIAS` | key alias |
| `SENZO_KEY_PASSWORD` | key password |

or write the same values into `android/app/keystore.properties`. That file and
every `*.jks` are ignored by git.

In CI the keystore comes from the `ANDROID_KEYSTORE_BASE64` secret, base64 of
the `.jks` file, with `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS` and
`ANDROID_KEY_PASSWORD` alongside it. Without those secrets the workflow signs
the APK with a throwaway key, which installs but cannot upgrade an earlier
install: set the secrets before the first tagged release.
