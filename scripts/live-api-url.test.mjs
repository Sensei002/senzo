import assert from "node:assert/strict";
import test from "node:test";

import { createApiUrlBuilder } from "./live-api-url.mjs";

test("joins a root API path to a host without creating a host-relative URL", () => {
  const build = createApiUrlBuilder("https://senzo-api.example");
  assert.equal(
    build("/api/health").toString(),
    "https://senzo-api.example/api/health",
  );
});

test("preserves a configured API base path", () => {
  const build = createApiUrlBuilder("https://example.test/senzo/api/");
  assert.equal(
    build("/tmdb/search?query=Inception").toString(),
    "https://example.test/senzo/api/tmdb/search?query=Inception",
  );
});

test("supports loopback HTTP for local acceptance tests", () => {
  const build = createApiUrlBuilder("http://127.0.0.1:3000");
  assert.equal(build("/api/health").origin, "http://127.0.0.1:3000");
});

test("rejects remote cleartext HTTP", () => {
  assert.throws(
    () => createApiUrlBuilder("http://example.test"),
    /must use HTTPS/,
  );
});

test("rejects URL credentials, query strings and fragments", () => {
  for (const value of [
    "https://user:secret@example.test",
    "https://example.test?token=secret",
    "https://example.test#fragment",
  ]) {
    assert.throws(
      () => createApiUrlBuilder(value),
      /cannot contain credentials/,
    );
  }
});

test("rejects host-relative and non-absolute API paths", () => {
  const build = createApiUrlBuilder("https://example.test");
  for (const path of ["api/health", "//attacker.test/api", "https://attacker.test/api"]) {
    assert.throws(() => build(path), /origin-relative absolute paths/);
  }
});

test("keeps unusual slash and backslash paths on the configured origin", () => {
  const build = createApiUrlBuilder("https://example.test/base");
  assert.equal(build("/\\\\attacker.test/api").origin, "https://example.test");
});
