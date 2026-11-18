import { lookup } from "node:dns/promises";

import { createApiUrlBuilder } from "./live-api-url.mjs";

const configuredBase = process.env.SENZO_API_BASE_URL ?? "https://senzo-api.vercel.app";
const base = new URL(configuredBase);
const buildApiUrl = createApiUrlBuilder(configuredBase);

console.log("Network diagnostics: NODE_USE_ENV_PROXY=" + (process.env.NODE_USE_ENV_PROXY ?? "unset"));
for (const name of ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"]) {
  const value = process.env[name];
  if (!value) continue;
  try {
    console.log("Network diagnostics: " + name + " proxy host=" + new URL(value).hostname);
  } catch {
    console.log("Network diagnostics: " + name + " is set but is not a valid URL");
  }
}
try {
  const record = await lookup(base.hostname);
  console.log("Network diagnostics: API hostname " + base.hostname + " resolves (IPv" + record.family + ").");
} catch (error) {
  console.log(
    "Network diagnostics: API hostname " +
      base.hostname +
      " DNS failed (" +
      (error.code ?? "unknown") +
      "; resolver target " +
      (error.hostname ?? "unknown") +
      ").",
  );
}

const requestHeaders = { Accept: "application/json" };
const vercelBypassSecret = process.env.VERCEL_AUTOMATION_BYPASS_SECRET;
if (vercelBypassSecret) {
  requestHeaders["x-vercel-protection-bypass"] = vercelBypassSecret;
}

async function getJson(path) {
  const url = buildApiUrl(path);
  let response;
  try {
    response = await fetch(url, {
      headers: requestHeaders,
      redirect: "manual",
      signal: AbortSignal.timeout(20_000),
    });
  } catch (error) {
    const cause = error?.cause;
    console.error(
      "Fetch transport failure for " +
        url.host +
        " (" +
        (cause?.code ?? error?.code ?? "unknown") +
        "; resolver target " +
        (cause?.hostname ?? "unknown") +
        ").",
    );
    throw error;
  }
  if (response.status >= 300 && response.status < 400) {
    throw new Error(
      path +
        " redirected (HTTP " +
        response.status +
        ") to " +
        (response.headers.get("location") ?? "<missing Location header>") +
        ". The live acceptance runner does not follow cross-origin redirects.",
    );
  }
  if (!response.ok) {
    const body = (await response.text()).slice(0, 300);
    if (response.status === 401 && body.includes('"vercel_auth_enabled":true')) {
      throw new Error(
        "The Vercel Preview is protected by Vercel Authentication. Add the VERCEL_AUTOMATION_BYPASS_SECRET GitHub Actions repository secret to enable protected Preview acceptance.",
      );
    }
    throw new Error(path + " returned HTTP " + response.status + ": " + body);
  }
  const contentType = response.headers.get("content-type") ?? "";
  if (!contentType.toLowerCase().includes("application/json")) {
    throw new Error(path + " did not return JSON (content-type: " + (contentType || "missing") + ").");
  }
  return response.json();
}

async function expectStatus(path, expectedStatus) {
  const response = await fetch(buildApiUrl(path), {
    headers: requestHeaders,
    redirect: "manual",
    signal: AbortSignal.timeout(20_000),
  });
  requireValue(
    response.status === expectedStatus,
    path + " returned HTTP " + response.status + ", expected " + expectedStatus + ".",
  );
}

function requireValue(condition, message) {
  if (!condition) throw new Error(message);
}

function report(name) {
  console.log("PASS  " + name);
}

console.log("Checking hosted SENZO API: " + base.origin);

const health = await getJson("/api/health");
requireValue(health.service === "SENZO API" && health.status === "ok", "API health response is not healthy.");
requireValue(health.tmdbConfigured === true, "The selected Vercel environment is missing its server-side TMDB_READ_ACCESS_TOKEN. Add it to the Vercel senzo-api Preview environment and redeploy the Preview; VERCEL_AUTOMATION_BYPASS_SECRET only bypasses deployment protection.");
report("API health and server-side TMDB configuration");

const search = await getJson("/api/tmdb/search?query=Inception&type=multi&page=1&language=en-US");
requireValue(
  Array.isArray(search.results) && search.results.some((item) => item.id === 27205),
  "Multi-search did not return the expected Inception result.",
);
const movieSearch = await getJson("/api/tmdb/search?query=Inception&type=movie&page=1&language=en-US");
requireValue(
  Array.isArray(movieSearch.results) && movieSearch.results.some((item) => item.id === 27205),
  "Movie-only search did not return the expected Inception result.",
);
const tvSearch = await getJson("/api/tmdb/search?query=Game%20of%20Thrones&type=tv&page=1&language=en-US");
requireValue(
  Array.isArray(tvSearch.results) && tvSearch.results.some((item) => item.id === 1399),
  "TV-only search did not return the expected Game of Thrones result.",
);
report("Multi, movie-only and TV-only Search");

const trending = await getJson("/api/tmdb/trending/all/week?page=1&language=en-US");
requireValue(Array.isArray(trending.results) && trending.results.length > 0, "Trending returned no results.");
const popularMovies = await getJson("/api/tmdb/movie/popular?page=1&language=en-US");
requireValue(Array.isArray(popularMovies.results) && popularMovies.results.length > 0, "Popular movies returned no results.");
const popularTv = await getJson("/api/tmdb/tv/popular?page=1&language=en-US");
requireValue(Array.isArray(popularTv.results) && popularTv.results.length > 0, "Popular TV returned no results.");
report("Home catalogues: trending, popular movies and popular TV");

const movieGenres = await getJson("/api/tmdb/genre/movie/list?language=en-US");
requireValue(
  Array.isArray(movieGenres.genres) && movieGenres.genres.some((genre) => genre.id === 28 && genre.name === "Action"),
  "Movie genre list did not return Action.",
);
const tvGenres = await getJson("/api/tmdb/genre/tv/list?language=en-US");
requireValue(
  Array.isArray(tvGenres.genres) && tvGenres.genres.some((genre) => genre.id === 18 && genre.name === "Drama"),
  "TV genre list did not return Drama.",
);
const discoveredMovies = await getJson("/api/tmdb/discover/movie?with_genres=28&sort_by=popularity.desc&page=1&language=en-US");
requireValue(Array.isArray(discoveredMovies.results) && discoveredMovies.results.length > 0, "Filtered movie discovery returned no titles.");
const discoveredTv = await getJson("/api/tmdb/discover/tv?with_genres=18&sort_by=popularity.desc&page=1&language=en-US");
requireValue(Array.isArray(discoveredTv.results) && discoveredTv.results.length > 0, "Filtered TV discovery returned no titles.");
await expectStatus("/api/tmdb/discover/movie?sort_by=untrusted.field&page=1", 400);
await expectStatus("/api/tmdb/discover/tv?first_air_date_year=2200&page=1", 400);
await expectStatus("/api/tmdb/discover/movie?with_genres=0&page=1", 400);
report("Movie/TV genres, filtered discovery, and invalid-filter rejection");

const movie = await getJson("/api/tmdb/movie/27205?language=en-US");
requireValue(movie.id === 27205 && typeof movie.title === "string", "Movie details response is incomplete.");
requireValue(typeof movie.poster_path === "string" && movie.poster_path.startsWith("/"), "Movie details did not include a valid poster path.");
requireValue(movie.credits && Array.isArray(movie.credits.cast), "Movie credits are missing.");
requireValue(movie.recommendations && Array.isArray(movie.recommendations.results), "Movie recommendations are missing.");
requireValue(movie.similar && Array.isArray(movie.similar.results), "Movie similar titles are missing.");
requireValue(movie.videos && Array.isArray(movie.videos.results), "Movie videos are missing.");
requireValue(
  movie.images && Array.isArray(movie.images.posters) && Array.isArray(movie.images.backdrops),
  "Movie image metadata is missing.",
);
requireValue(movie.external_ids && typeof movie.external_ids.imdb_id === "string", "Movie external IDs are missing.");
const movieExternalIds = await getJson("/api/tmdb/movie/27205/external_ids?language=en-US");
requireValue(movieExternalIds.imdb_id === movie.external_ids.imdb_id, "The dedicated movie external-ID route is inconsistent.");
report("Movie details, credits, videos, images and external IDs");

const tv = await getJson("/api/tmdb/tv/1399?language=en-US");
requireValue(tv.id === 1399 && typeof tv.name === "string", "TV details response is incomplete.");
requireValue(Array.isArray(tv.seasons) && tv.seasons.length > 0, "TV season metadata is missing.");
requireValue(tv.aggregate_credits && Array.isArray(tv.aggregate_credits.cast), "TV aggregate credits are missing.");
requireValue(tv.recommendations && Array.isArray(tv.recommendations.results), "TV recommendations are missing.");
requireValue(tv.similar && Array.isArray(tv.similar.results), "TV similar titles are missing.");
requireValue(tv.videos && Array.isArray(tv.videos.results), "TV videos are missing.");
requireValue(
  tv.images && Array.isArray(tv.images.posters) && Array.isArray(tv.images.backdrops),
  "TV image metadata is missing.",
);
requireValue(tv.external_ids && typeof tv.external_ids.imdb_id === "string", "TV external IDs are missing.");
const tvExternalIds = await getJson("/api/tmdb/tv/1399/external_ids?language=en-US");
requireValue(tvExternalIds.imdb_id === tv.external_ids.imdb_id, "The dedicated TV external-ID route is inconsistent.");
const season = await getJson("/api/tmdb/tv/1399/season/1?language=en-US");
requireValue(season.season_number === 1 && Array.isArray(season.episodes) && season.episodes.length > 0, "TV season details are incomplete.");
const episode = await getJson("/api/tmdb/tv/1399/season/1/episode/1?language=en-US");
requireValue(episode.id > 0 && typeof episode.name === "string", "TV episode details are incomplete.");
report("TV details, credits, videos, images, external IDs, season and episode");

const person = await getJson("/api/tmdb/person/287?language=en-US");
requireValue(person.id === 287 && person.name === "Brad Pitt", "Person details response is incomplete or mismatched.");
requireValue(
  person.combined_credits &&
    Array.isArray(person.combined_credits.cast) &&
    Array.isArray(person.combined_credits.crew),
  "Person combined cast/crew credits are missing.",
);
requireValue(
  typeof person.profile_path === "string" && person.profile_path.startsWith("/"),
  "Person profile artwork path is missing.",
);
report("Person details and combined cast/crew credits");

const artworkUrl = new URL("https://image.tmdb.org/t/p/w342" + movie.poster_path);
const artworkResponse = await fetch(artworkUrl, { signal: AbortSignal.timeout(20_000) });
const artworkType = artworkResponse.headers.get("content-type") ?? "";
requireValue(artworkResponse.ok, "TMDB artwork returned HTTP " + artworkResponse.status + ".");
requireValue(
  /^image\/(jpeg|png|webp)(?:;|$)/i.test(artworkType),
  "TMDB artwork returned an unsupported content type: " + (artworkType || "missing") + ".",
);
const declaredArtworkBytes = Number(artworkResponse.headers.get("content-length") ?? 0);
const maximumArtworkBytes = 8 * 1024 * 1024;
requireValue(
  !declaredArtworkBytes || declaredArtworkBytes <= maximumArtworkBytes,
  "TMDB artwork exceeded the live-acceptance byte limit.",
);
const artworkBytes = new Uint8Array(await artworkResponse.arrayBuffer());
requireValue(
  artworkBytes.length > 8 && artworkBytes.length <= maximumArtworkBytes,
  "TMDB artwork response is empty or exceeded the byte limit.",
);
const isJpeg = artworkBytes[0] === 0xff && artworkBytes[1] === 0xd8 && artworkBytes[2] === 0xff;
const isPng =
  artworkBytes[0] === 0x89 &&
  artworkBytes[1] === 0x50 &&
  artworkBytes[2] === 0x4e &&
  artworkBytes[3] === 0x47;
const isWebp =
  String.fromCharCode(...artworkBytes.slice(0, 4)) === "RIFF" &&
  String.fromCharCode(...artworkBytes.slice(8, 12)) === "WEBP";
requireValue(isJpeg || isPng || isWebp, "TMDB artwork MIME type and file signature do not match.");
report("TMDB artwork CDN, bounded bytes and verified image signature");

console.log("Live hosted API acceptance passed.");
