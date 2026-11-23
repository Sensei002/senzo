# SENZO Direct Sources

Direct Sources is an independent provider subsystem that runs beside the
existing Stremio add-on system. It has its own navigation entry, provider
manager, stream results and filters. It does not install, modify, replace or
proxy Stremio add-ons. Torrentio remains an optional, separately installed
Stremio add-on and is never imported as a Direct Source.

## What ships enabled

New installs start with the subsystem enabled and only two vetted built-ins
enabled:

| Provider | Method | Content | Notes |
| --- | --- | --- | --- |
| Internet Archive | Official `archive.org` search and metadata JSON APIs | Films | Only items with an explicit open license are offered. |
| Wikimedia Commons | Official MediaWiki API | Freely licensed video | Each result links to its Commons file page for attribution. |

Both providers are read-only and use public HTTPS endpoints. SENZO does not run
their web pages, plugin manifests or JavaScript.

Existing profiles migrate to config version 5. The migration adds missing
built-ins but never changes an existing provider's enabled flag, so a provider
you disabled stays disabled. The Direct Sources settings are stored locally in
the existing application configuration; there is no account or cloud service.

## My Sources

A custom source must be an HTTPS URL to a declarative document owned or
authorized by the user. Two connectors are supported:

### M3U / M3U8

```m3u
#EXTM3U
#EXTINF:-1 tvg-id="tt1234567" tvg-name="Example Film" group-title="Movies",Example Film (2008)
#EXTVLCOPT:http-referrer=https://provider.example/
https://cdn.provider.example/film/master.m3u8
```

Parsed attributes: `tvg-id`, `tvg-name`, `group-title`, `tvg-language`,
`tvg-logo`, `tvg-audio`, plus `http-referrer` and `http-user-agent` options.
Only `Referer` and `User-Agent` are accepted from `#EXTVLCOPT`. For series,
include `S01E02` or `1x02` in the title/name so episode identity is preserved.

### JSON feed

```json
{
  "name": "Authorized Example Feed",
  "items": [
    {
      "id": "tt1234567",
      "title": "Example Film",
      "type": "movie",
      "year": 2008,
      "url": "https://cdn.provider.example/film/master.m3u8",
      "quality": "1080p",
      "language": "en",
      "format": "hls",
      "subtitles": ["https://cdn.provider.example/film/en.vtt"]
    },
    {
      "id": "tt7654321:1:2",
      "title": "Example Show",
      "type": "series",
      "season": 1,
      "episode": 2,
      "url": "https://cdn.provider.example/show/s01e02.mp4",
      "format": "mp4"
    }
  ]
}
```

`streams` is accepted as an alias for `items`. Optional fields include
`headers`, `audio`, `expires_at`, and a small `metadata` object. Header names
`Host`, `Content-Length`, `Transfer-Encoding` and `Connection` are rejected.

Custom sources can be imported/exported through the clipboard. Exports contain
source URLs (which may contain access tokens), so treat an export as private.

## Discover

Discover takes one domain at a time. It:

- loads `robots.txt` first and fails closed if it cannot be read;
- follows only same-host links and a small set of conventional catalog paths;
- requests at most a bounded number of documents with a delay;
- recognises M3U playlists, SENZO JSON feeds, RSS/Atom and remote plugin
  manifests;
- reports remote plugin manifests as unsupported and never executes them;
- never adds a result automatically.

Anything that is not a supported M3U or JSON catalog is shown for review but
cannot be added. Unsupported RSS/Atom mapping is intentionally not exposed in
this release.

## Matching and playback

Provider results are matched against the details page's existing TMDB/IMDb
identity. A conflicting IMDb id is authoritative and is never overridden by a
title. Series results also require season and episode equality. Title-only
matching is used only when the provider has no id for that item.

Direct results appear in the normal native stream drawer, tagged `DIRECT`, with
an `Add-ons` / `Direct Sources` group filter. Playback continues through the
existing native MPV pipeline, so resume/progress, audio/subtitle tracks, player
controls and the loading overlay are unchanged. Direct results do not use
torrent ranking and do not create a second player.

## Security model

All provider metadata, URLs and headers are untrusted.

- HTTPS only; public DNS resolution is checked before every request.
- Loopback, private, link-local, documentation, multicast and unique-local
  addresses are blocked, including redirect targets.
- Requests have connect and total timeouts, redirect limits, a 4 MiB response
  cap and a content-type allowlist.
- No downloaded JavaScript, plugin, scraper or executable is run.
- SENZO does not bypass logins, paywalls, DRM or CAPTCHAs and does not add
  authentication credentials of its own.
- Provider URLs are never logged with their query string. Authorized headers
  are passed to the player in memory and are never logged.
- A custom source is not a way to proxy arbitrary local or private network
  requests.

The existing catalog deadlock protections are untouched: cache revalidation is
non-blocking, bodyless/304 responses are retried and reported safely, and
asynchronous paths do not write synchronously to stderr.

## Attribution and licensing

SENZO is licensed under GPL-3.0. The Direct Sources implementation in
`app/src/direct_sources` is original SENZO code. The public
`D3adlyRocket/All-in-One-Nuvio` and `NuvioPlugin/All-in-One-Nuvio` manifests were
reviewed only as a structural reference for provider metadata and discovery
concepts; no code, manifest or provider list was copied, and none of their
JavaScript is executed. Torrentio is not part of Direct Sources and is not
installed or enabled by this feature.

## Known limitations

- Internet Archive and Commons are film-oriented built-ins; series support for
  them is intentionally not guessed.
- RSS/Atom field mapping is reported but not configurable yet.
- Discovery does not crawl across domains or paginate.
- Network provider behavior depends on the provider's public API being
  reachable; a provider failure is reported in the manager and never stops the
  player.
