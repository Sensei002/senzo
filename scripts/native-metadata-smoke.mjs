import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { createServer } from "node:http";
import { resolve } from "node:path";
import { Readable } from "node:stream";

import { createApiUrlBuilder } from "./live-api-url.mjs";

const configuredBase = process.env.SENZO_API_BASE_URL ?? "https://senzo-api.vercel.app";
const buildUpstreamUrl = createApiUrlBuilder(configuredBase);
const bypassSecret = process.env.VERCEL_AUTOMATION_BYPASS_SECRET;

const server = createServer(async (request, response) => {
  if (request.method !== "GET" && request.method !== "HEAD") {
    response.writeHead(405, { "content-type": "text/plain; charset=utf-8" });
    response.end("Method not allowed");
    return;
  }

  let target;
  try {
    target = buildUpstreamUrl(request.url ?? "/");
  } catch {
    response.writeHead(400, { "content-type": "text/plain; charset=utf-8" });
    response.end("Invalid metadata API path");
    return;
  }

  if (!target.pathname.includes("/api/")) {
    response.writeHead(404, { "content-type": "text/plain; charset=utf-8" });
    response.end("Only SENZO API routes are proxied");
    return;
  }

  const headers = { Accept: request.headers.accept ?? "application/json" };
  if (bypassSecret) {
    headers["x-vercel-protection-bypass"] = bypassSecret;
  }

  try {
    const upstream = await fetch(target, {
      method: request.method,
      headers,
      redirect: "manual",
      signal: AbortSignal.timeout(30_000),
    });
    const responseHeaders = {};
    for (const name of ["content-type", "cache-control", "retry-after"]) {
      const value = upstream.headers.get(name);
      if (value) responseHeaders[name] = value;
    }
    response.writeHead(upstream.status, responseHeaders);
    if (request.method === "HEAD" || !upstream.body) {
      response.end();
    } else {
      Readable.fromWeb(upstream.body).pipe(response);
    }
  } catch {
    if (!response.headersSent) {
      response.writeHead(502, { "content-type": "text/plain; charset=utf-8" });
      response.end("Hosted SENZO API proxy request failed");
    } else {
      response.destroy();
    }
  }
});

const binary = process.env.SENZO_NATIVE_BINARY
  ? resolve(process.env.SENZO_NATIVE_BINARY)
  : resolve(
      "target",
      "release",
      process.platform === "win32" ? "senzo.exe" : "senzo",
    );

if (!existsSync(binary)) {
  throw new Error("Native SENZO binary was not found at " + binary);
}

await new Promise((resolveListen, rejectListen) => {
  server.once("error", rejectListen);
  server.listen(0, "127.0.0.1", resolveListen);
});

const address = server.address();
if (!address || typeof address === "string") {
  throw new Error("Could not determine the loopback proxy port.");
}
const localBase = "http://127.0.0.1:" + address.port + "/";

const childEnvironment = { ...process.env, SENZO_API_BASE_URL: localBase };
for (const name of Object.keys(childEnvironment)) {
  if (name.toUpperCase() === "VERCEL_AUTOMATION_BYPASS_SECRET") {
    delete childEnvironment[name];
  }
}

console.log("Running the native Rust metadata and artwork smoke test on " + process.platform + ".");

try {
  const exitCode = await new Promise((resolveExit, rejectExit) => {
    const child = spawn(binary, ["--metadata-smoke-test"], {
      cwd: process.cwd(),
      env: childEnvironment,
      windowsHide: true,
      stdio: ["ignore", "pipe", "pipe"],
    });

    let stdout = "";
    let stderr = "";
    let timedOut = false;
    const timer = setTimeout(() => {
      timedOut = true;
      child.kill();
    }, 240_000);

    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    child.stdout.on("data", (chunk) => {
      stdout = (stdout + chunk).slice(-100_000);
    });
    child.stderr.on("data", (chunk) => {
      stderr = (stderr + chunk).slice(-100_000);
    });
    child.once("error", (error) => {
      clearTimeout(timer);
      rejectExit(error);
    });
    child.once("close", (code, signal) => {
      clearTimeout(timer);
      if (stdout) process.stdout.write(stdout);
      if (stderr) process.stderr.write(stderr);
      if (timedOut) {
        rejectExit(new Error("Native metadata smoke test exceeded four minutes."));
      } else if (code === 0) {
        resolveExit(code);
      } else {
        rejectExit(
          new Error(
            "Native SENZO metadata smoke test exited with " +
              (code ?? "no exit code") +
              (signal ? " (signal " + signal + ")" : "") +
              ".",
          ),
        );
      }
    });
  });

  if (exitCode !== 0) {
    throw new Error("Native metadata smoke test failed.");
  }
  console.log("PASS  native Rust metadata and artwork integration");
} finally {
  server.closeAllConnections?.();
  await new Promise((resolveClose) => server.close(resolveClose));
}
