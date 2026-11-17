export function createApiUrlBuilder(configuredBase) {
  let base;
  try {
    base = new URL(configuredBase);
  } catch {
    throw new Error("SENZO_API_BASE_URL must be a valid absolute URL.");
  }

  const loopbackHosts = new Set(["localhost", "127.0.0.1", "[::1]"]);
  if (
    base.protocol !== "https:" &&
    !(base.protocol === "http:" && loopbackHosts.has(base.hostname))
  ) {
    throw new Error("SENZO_API_BASE_URL must use HTTPS (HTTP is allowed only for loopback testing).");
  }
  if (base.username || base.password || base.search || base.hash) {
    throw new Error("SENZO_API_BASE_URL cannot contain credentials, a query string or a fragment.");
  }

  const basePath = base.pathname.replace(/\/+$/, "");

  return function buildApiUrl(path) {
    if (typeof path !== "string" || !path.startsWith("/") || path.startsWith("//")) {
      throw new Error("SENZO API paths must be origin-relative absolute paths.");
    }

    const url = new URL(basePath + path, base.origin);
    if (url.origin !== base.origin) {
      throw new Error("SENZO API path escaped the configured API origin.");
    }
    return url;
  };
}
