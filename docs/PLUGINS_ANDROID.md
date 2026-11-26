# Plugins on Android

SENZO's **Plugins** tab installs Nuvio-compatible provider repositories. This
note records what that costs on Android, which engine to run the providers in,
and how the port is cut into phases. It is a design note, not a promise that
every desktop provider will run: providers remain third-party JavaScript with
network access.

## What a plugin is

A plugin is not an addon. An addon is an HTTP service reached through a
manifest URL; a plugin is a JavaScript scraper that runs **on the device** and
returns stream links for one title.

```
repository manifest  ->  scrapers[]  ->  getStreams(tmdbId, mediaType, season, episode)
```

Repository manifest (the default is the All-in-One collection):

```json
{
  "name": "All-in-One-Nuvio",
  "version": "1.0.0",
  "scrapers": [
    {
      "id": "example",
      "name": "Example",
      "version": "1.0.0",
      "filename": "providers/example.js",
      "supportedTypes": ["movie", "tv"],
      "enabled": true,
      "hasSettings": false
    }
  ]
}
```

The default repository is
`https://raw.githubusercontent.com/D3adlyRocket/All-in-One-Nuvio/refs/heads/main/manifest.json`,
which is what a preinstalled plugin means: the entry ships in the app, the
repository is fetched once, and each provider stays a switch the viewer owns.
Nothing downloads until a title is opened with at least one provider enabled.

## What the desktop runtime provides

`app/src/plugin_runtime/` is a QuickJS host: one runtime per provider query on
a worker thread, 128 MB memory cap, 2 MB stack, 60 s deadline, eight providers
at a time. Provider scripts are wrapped in a CommonJS prelude, then

```js
var streams = await getStreams(tmdbId, mediaType, season, episode);
```

The scripts expect a browser-ish surface that QuickJS does not have. SENZO
supplies it in three plain-JavaScript files, roughly 1,250 lines in total:

| File | Supplies |
| --- | --- |
| `polyfills.js` | `fetch`, `AbortController`, timers, `TextEncoder`/`Decoder`, base64, `Buffer`, `URL`, `URLSearchParams`, `SCRAPER_ID`, `SCRAPER_SETTINGS`, `console` |
| `polyfills_dom.js` | Cheerio-compatible `cheerio.load()` selector facade, Node module aliases |
| `polyfills_crypto.js` | `crypto-js`, a WebCrypto subset (`digest`, `hmac`, `PBKDF2`, AES-CBC/GCM/ECB) |

Every network call, timer, hash and DOM operation goes through a narrow native
surface, so the scripts themselves never touch the filesystem:

```
__senzo_scraper_id()      __senzo_settings_json()   __senzo_call_args()
__senzo_capture_result()  __senzo_capture_error()   __senzo_log()
__senzo_fetch(json)       __senzo_sleep(ms)         __senzo_parse_url()
__senzo_text_encode()     __senzo_text_decode()     __cheerio_* (DOM)
```

`__senzo_fetch` speaks JSON in and JSON out (`url`, `method`, `headers`,
`bodyKind`, `body`, `followRedirects` → `ok`, `status`, `url`, `redirected`,
`headers`, `body`, `bodyBase64`), rejects private hosts before every request
and redirect, and rewrites `api.themoviedb.org` to the SENZO metadata gateway
so no TMDB key ships in the app.

## Engine choice for Android

| Option | Verdict |
| --- | --- |
| **QuickJS via JNI** (`app.cash.quickjs`) | Chosen. Same engine family as the desktop host, so the provider scripts and all three polyfill files carry over unchanged. Prebuilt `.so` per ABI, roughly 1.5 MB uncompressed / 600 KB in the APK. Blocking, so a provider query is one off-main-thread task. |
| WebView sandbox | Full modern JavaScript, but it needs a UI thread and a view, cannot honour a 60 s deadline, and each provider would cost tens of MB. Wrong shape for eight parallel scrapers. |
| Rhino | Pure Java and tiny, but no `async`/`await`; the providers are written in modern JavaScript. |
| GraalJS | Correct semantics, 20 MB+ and AOT configuration; far too heavy for a 2.7 MB app. |
| LuaJ | Runs Lua, not the JavaScript the Nuvio ecosystem publishes. |

## Port shape

The JavaScript shims are engine-agnostic: they only call the `__senzo_*` and
`__cheerio_*` names above. So the port reimplements the **native** side in
Kotlin and ships the shims as assets, unchanged:

```
assets/plugins/polyfills.js        assets/plugins/polyfills_dom.js
assets/plugins/polyfills_crypto.js assets/plugins/bootstrap.js

Kotlin:
  PluginEngine      QuickJs.create(), load shims, eval script, await result slot
  PluginFetch       OkHttp, host allow-list, per-request timeout, 12 MB cap
  PluginDom         jsoup: load / select / find / text / html / attr / children /
                    parent / next / prev / is / closest
  PluginCrypto      javax.crypto + java.security.MessageDigest
  PluginRepository  manifest fetch, validation, enabled-provider storage
```

jsoup is the one new dependency the DOM facade needs; it is a few hundred
kilobytes and is also the parser the facades on other platforms use.

Title resolution needs a TMDB id, because that is the argument providers take.
Android already talks to the same gateway the desktop runtime falls back to:

```
GET https://senzo-api.vercel.app/api/tmdb/search?query=<title>&type=movie|tv
```

which returns results carrying `id`, and the release date used to pick the
right one. The resolved id is cached per title, as on the desktop.

## Phases

1. **Repository install (no execution).** Plugins tab lists repositories and
   their providers with switches, settings JSON per provider, refresh that
   keeps the viewer's choices, and removal. Ships the preinstalled All-in-One
   entry. No JavaScript runs yet.
2. **Execution.** QuickJS engine, shims as assets, the eleven `__senzo_*`
   bridges, fetch with the public-host check, provider test button.
3. **Streams.** Provider results merge into the details stream list under a
   `PLUGIN` badge, next to addon streams; `infoHash` results use the existing
   torrent path, direct URLs the existing player path.
4. **DOM and crypto.** jsoup facade, then `crypto-js`/WebCrypto, which is what
   unlocks the majority of the All-in-One providers.

Phases 1 and 2 are worth doing before 3: a provider list with switches is
useful, and a provider that cannot be tested is a support burden.

## Limits

- The same sandbox limits as the desktop host: memory cap, deadline, public
  hosts only, sanitised display fields, HTTP(S) or torrent hashes only.
- WebAssembly-dependent providers stay unsupported; WebCrypto RSA/ECDSA stays
  unavailable.
- Providers require network access while they run. Only repositories the
  viewer trusts belong in a build, and the preinstalled entry is a default,
  not an endorsement.
## What shipped

The four phases landed together, in `android/app/src/main/kotlin/com/senzo/app/plugins/`:

- `PluginRuntime` owns the config (repositories, providers, per-provider settings
  JSON) in `SharedPreferences` and runs enabled providers four at a time.
- `PluginRepositoryClient` loads manifests and scripts over HTTPS only, with the
  same 4 MB caps and the same three manifest shapes as the desktop loader.
- `PluginEngine` hosts a `QuickJSContext` per provider run: memory 128 MB, stack
  2 MB, 60 s deadline enforced inside fetch/sleep, 48 streams, 80 log lines.
  The nine `__senzo_*`/eleven `__crypto_*`/twelve `__cheerio_*` bridges are
  registered before the prelude, the three shims, the provider script, then the
  async `CALL_CODE` that awaits `getStreams` and captures JSON.
- `PluginFetch` ports the desktop request policy (manual redirects, cookie jar,
  12 MB cap, TMDB gateway rewrite, private-host blocking).
- `PluginDom` maps Cheerio element ids onto jsoup elements; `PluginCrypto` uses
  `MessageDigest`/`Mac`/`Cipher` with the desktop hex wire format.

The Plugins section sits beside the addon directory. Repositories are refreshed
on first start, providers start disabled, and stream results merge with addon
streams: `infoHash`/magnet entries go through `TorrentStreamer`, direct URLs
through the player.

Dependencies: `wang.harlon.quickjs:wrapper-android:3.2.3` (AAR plus the
`wrapper-java` companion it pulls in) and `org.jsoup:jsoup:1.18.3`. R8 keeps
`com.whl.quickjs.**` and all native method names, because the wrapper crosses
into JNI by name.

## APK size

`libtorrent4j.so` is ~15 MB per architecture and QuickJS adds ~1.3 MB, so the
universal APK had grown to 52 MB. The release now builds one APK per ABI:
arm64-v8a ~19.4 MB (the phone download) and armeabi-v7a ~16.9 MB. x86_64 is
available from the dependencies for emulator builds but is not packaged.

## Still unverified on device

- Whether `QuickJSContext.evaluate` drains the promise job queue after the
  async `CALL_CODE` returns, which the timer shim depends on.
- Torrent streaming to Media3 through `http://127.0.0.1:<port>/stream` with
  Range requests on a real swarm.
