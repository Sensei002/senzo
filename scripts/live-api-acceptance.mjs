const configuredBase = process.env.SENZO_API_BASE_URL ?? "https://senzo-api.vercel.app";
const base = new URL(configuredBase);

if (base.protocol !== "https:" && !(base.protocol === "http:" && ["localhost", "127.0.0.1", "::1"].includes(base.hostname))) {
  throw new Error("SENZO_API_BASE_URL must use HTTPS (HTTP is allowed only for loopback testing).");
}

base.pathname = base.pathname.replace(/\/$/, "");

async function getJson(path) {
  const url = new URL(`${base.pathname}${path}`, base.origin);
  const response = await fetch(url, {
    headers: { Accept: "application/json" },
    signal: AbortSignal.timeout(20_000),
  });
  if (!response.ok) {
    const body = (await response.text()).slice(0, 300);
    throw new Error(`${path} returned HTTP ${response.status}: ${body}`);
  }
  const contentType = response.headers.get("content-type") ?? "";
  if (!contentType.toLowerCase().includes("application/json")) {
    throw new Error(`${path} did not return JSON (content-type: ${contentType || "missing"}).`);
  }
  return response.json();
}

function requireValue(condition, message) {
  if (!condition) throw new Error(message);
}

function report(name) {
  console.log(`PASS  ${name}`);
}

console.log(`Checking hosted SENZO API: ${base.origin}`);

const health = await getJson("/api/health");
requireValue(health.service === "SENZO API" && health.status === "ok", "API health response is not healthy.");
requireValue(health.tmdbConfigured === true, "The hosted API does not report TMDB as configured.");
report("API health and server-side TMDB configuration");

const search = await getJson("/api/tmdb/search?query=Inception&type=multi&page=1&language=en-US");
requireValue(Array.isArray(search.results) && search.results.some((item) => item.id === 27205), "Search did not return the expected Inception result.");
report("Search");

const trending = await getJson("/api/tmdb/trending/all/week?page=1&language=en-US");
requireValue(Array.isArray(trending.results) && trending.results.length > 0, "Trending returned no results.");
report("Trending catalogue");

const movie = await getJson("/api/tmdb/movie/27205?language=en-US");
requireValue(movie.id === 27205 && typeof movie.title === "string", "Movie details response is incomplete.");
requireValue(typeof movie.poster_path === "string" && movie.poster_path.startsWith("/"), "Movie details did not include a valid poster path.");
requireValue(movie.credits && Array.isArray(movie.credits.cast), "Movie credits are missing.");
report("Movie details and credits");

const tv = await getJson("/api/tmdb/tv/1399?language=en-US");
requireValue(tv.id === 1399 && typeof tv.name === "string", "TV details response is incomplete.");
requireValue(Array.isArray(tv.seasons) && tv.seasons.length > 0, "TV season metadata is missing.");
report("TV details and seasons");

const person = await getJson("/api/tmdb/person/287?language=en-US");
requireValue(person.id === 287 && person.name === "Brad Pitt", "Person details response is incomplete or mismatched.");
requireValue(person.combined_credits && Array.isArray(person.combined_credits.cast) && Array.isArray(person.combined_credits.crew), "Person combined cast/crew credits are missing.");
requireValue(typeof person.profile_path === "string" && person.profile_path.startsWith("/"), "Person profile artwork path is missing.");
report("Person details and combined cast/crew credits");

const artworkUrl = new URL(`https://image.tmdb.org/t/p/w342${movie.poster_path}`);
const artworkResponse = await fetch(artworkUrl, { signal: AbortSignal.timeout(20_000) });
const artworkType = artworkResponse.headers.get("content-type") ?? "";
requireValue(artworkResponse.ok, `TMDB artwork returned HTTP ${artworkResponse.status}.`);
requireValue(/^image\/(jpeg|png|webp)(?:;|$)/i.test(artworkType), `TMDB artwork returned an unsupported content type: ${artworkType || "missing"}.`);
const artworkBytes = new Uint8Array(await artworkResponse.arrayBuffer());
requireValue(artworkBytes.length > 8, "TMDB artwork response is empty.");
report("TMDB artwork CDN");

console.log("Live hosted API acceptance passed.");
