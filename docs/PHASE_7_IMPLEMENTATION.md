# Phase 7 — Nuvio plugin/scraper runtime

**Status:** Complete  
**Reference implementation:** Nuvio Desktop, pinned source audit commit `65061186dde35fc0a0f90b05d380f2318133f227`  
**SENZO branch:** `development`  
**Acceptance:** [SENZO Desktop CI #642](https://github.com/Sensei002/senzo/actions/runs/44477224036)

## Delivered

- Audited Nuvio's QuickJS plugin/scraper API and used it as the compatibility reference.
- Added the isolated Rust `senzo-plugins` crate using `rquickjs 0.14.0`; the plugin runtime is separate from Stremio add-ons.
- Implemented Nuvio-compatible repository manifest parsing, repository and scraper management, installation/refresh/removal, enabled state, and scraper execution.
- Implemented CommonJS `module.exports.getStreams` and scraper-settings exports, plus supported scraper metadata and desktop platform filtering.
- Added a bounded JavaScript host environment with the web APIs and helper shims used by the supported scraper contract, including URL/URLSearchParams, Headers, fetch, AbortController, timers, console, base64, TextEncoder/TextDecoder, Cheerio, and WebAssembly compatibility behavior.
- Implemented Cheerio-backed DOM querying and the selector behavior covered by the repository compatibility fixtures, including `:contains(...)`.
- Implemented the CryptoJS module and Web Crypto bridge for supported digest, HMAC, PBKDF2, AES, secure-random, and signature operations.
- Implemented UTF-8 and binary fetch request bodies, binary `arrayBuffer()` responses, and byte-preserving base64 conversion.
- Mapped scraper results into SENZO's provider-neutral stream model, retaining stream headers and subtitle metadata.
- Added profile-aware plugin state and per-scraper settings. Scraper settings are stored through the operating-system credential-store interface.
- Added explicit per-scraper network permissions and native Settings controls for repository/scraper management, settings, permission grants, testing, and execution.
- Kept JavaScript execution and network work off the GPUI render path.

## Security and resource limits

Network access is opt-in for each scraper. The runtime validates HTTPS repository URLs, resolves and blocks private/local/reserved network destinations, revalidates redirects, and applies request, response, header, and timeout bounds. SENZO's TMDB credential remains server-side and is not exposed to untrusted scraper code.

| Resource | Enforced bound |
|---|---:|
| Repositories | 32 |
| Scrapers per repository | 128 |
| Manifest size / scraper script size | 512 KiB each |
| QuickJS memory / stack | 64 MiB / 512 KiB |
| JavaScript execution timeout | 60 seconds |
| Network request timeout | 10 seconds |
| Concurrent scraper executions | 10 |
| Redirects | 4 |
| Fetch response / request body | 2 MiB / 1 MiB |
| Fetch headers / individual header value | 64 / 8 KiB |
| Returned streams | 100 |
| Cheerio elements | 20,000 |
| Timer delay | 60 seconds maximum |
| Crypto input / random bytes | 4 MiB / 65,536 bytes |
| PBKDF2 iterations / output | 1,000,000 / 512 bytes |

## Compatibility regression coverage

The `senzo-plugins` crate passed all 26 tests on both Windows and Linux in CI #642. Relevant fixtures verify:

- Nuvio manifest parsing, scraper type normalization, and desktop platform filters.
- CommonJS scraper exports and settings behavior.
- Provider-neutral stream mapping and metadata preservation.
- Explicit network grants, private/local destination blocking, execution interruption, concurrency bounds, and profile-scoped settings.
- Cheerio repository selectors and host-owned element bounds.
- CryptoJS SHA-256, MD5, HMAC-SHA256, PBKDF2 and AES passphrase encryption/decryption.
- Web Crypto digest and AES-GCM round trip, backed by NIST AES-GCM vectors for 128-, 192-, and 256-bit keys and non-96-bit nonce coverage.
- Binary fetch request/response bodies, byte-preserving base64, ArrayBuffer and typed-array handling, and response headers.
- TextDecoder handling of ArrayBuffers and typed-array views.

## Acceptance evidence

**CI #642** — [run details](https://github.com/Sensei002/senzo/actions/runs/44477224036)

- Windows native: formatting, Clippy, workspace tests, native release build, and startup/shutdown smoke test — passed.
- Linux native: formatting, Clippy, workspace tests, native release build, and startup/shutdown smoke test — passed.
- Repository automation validation — passed.
- Lockfile housekeeping — passed.
- Live Stremio add-on and hosted API jobs were skipped by their workflow conditions; neither is a Phase 7 gate. The separate Phase 6 Stremio/Cinemeta, Torrentio, and OpenSubtitles v3 acceptance had already passed in [CI #616](https://github.com/Sensei002/senzo/actions/runs/44401216814).

Phase 7 is closed. Phase 8 owns source aggregation and provider integrations; those features are not added to this phase.
