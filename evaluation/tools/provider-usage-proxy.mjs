#!/usr/bin/env node

import { appendFile, mkdir } from "node:fs/promises";
import http from "node:http";
import { dirname, resolve } from "node:path";
import { Readable } from "node:stream";

const DEFAULT_PORT = 9010;
const DEFAULT_UPSTREAM = "https://api.deepseek.com";

function parseArguments(argv) {
  const options = {
    port: DEFAULT_PORT,
    upstream: DEFAULT_UPSTREAM,
    log: "",
  };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    const value = argv[index + 1];
    if (argument === "--port" && value) {
      options.port = Number(value);
      index += 1;
    } else if (argument === "--upstream" && value) {
      options.upstream = value;
      index += 1;
    } else if (argument === "--log" && value) {
      options.log = value;
      index += 1;
    }
  }
  if (!Number.isInteger(options.port) || options.port < 1 || options.port > 65535) {
    throw new Error("--port must be an integer between 1 and 65535");
  }
  if (!options.log) throw new Error("--log is required");
  return options;
}

function numeric(value) {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function usageFromPayload(payload) {
  const usage = payload?.usage;
  if (!usage || typeof usage !== "object") return null;
  const inputTokens = numeric(usage.prompt_tokens) ?? numeric(usage.input_tokens);
  const outputTokens = numeric(usage.completion_tokens) ?? numeric(usage.output_tokens);
  const totalTokens = numeric(usage.total_tokens);
  const cachedInputTokens =
    numeric(usage.prompt_tokens_details?.cached_tokens) ??
    numeric(usage.input_tokens_details?.cached_tokens) ??
    numeric(usage.prompt_cache_hit_tokens);
  const cacheWriteTokens =
    numeric(usage.prompt_tokens_details?.cache_write_tokens) ??
    numeric(usage.input_tokens_details?.cache_write_tokens) ??
    numeric(usage.cache_write_tokens);
  const reasoningTokens =
    numeric(usage.completion_tokens_details?.reasoning_tokens) ??
    numeric(usage.output_tokens_details?.reasoning_tokens);
  return {
    inputTokens,
    outputTokens,
    totalTokens: totalTokens ?? (inputTokens ?? 0) + (outputTokens ?? 0),
    cachedInputTokens,
    cacheWriteTokens,
    reasoningTokens,
  };
}

function usagesFromResponseText(text, contentType) {
  const payloads = [];
  if (contentType.includes("text/event-stream")) {
    for (const line of text.split(/\r?\n/)) {
      if (!line.startsWith("data:")) continue;
      const data = line.slice(5).trim();
      if (!data || data === "[DONE]") continue;
      try {
        payloads.push(JSON.parse(data));
      } catch {
        // A malformed provider event is surfaced to the caller unchanged; it
        // simply cannot contribute to token telemetry.
      }
    }
  } else {
    try {
      payloads.push(JSON.parse(text));
    } catch {
      // Non-JSON error responses have no usage payload.
    }
  }
  return payloads.map(usageFromPayload).filter(Boolean);
}

function sanitizeResponseHeaders(headers) {
  const result = {};
  for (const [key, value] of headers.entries()) {
    if (["connection", "keep-alive", "transfer-encoding"].includes(key.toLowerCase())) {
      continue;
    }
    result[key] = value;
  }
  return result;
}

async function readRequestBody(request) {
  const chunks = [];
  for await (const chunk of request) chunks.push(chunk);
  return Buffer.concat(chunks);
}

const options = parseArguments(process.argv.slice(2));
const upstreamRoot = new URL(options.upstream.endsWith("/") ? options.upstream : `${options.upstream}/`);
const logPath = resolve(options.log);
await mkdir(dirname(logPath), { recursive: true });

let sequence = 0;
let writeQueue = Promise.resolve();
function appendRecord(record) {
  writeQueue = writeQueue.then(() => appendFile(logPath, `${JSON.stringify(record)}\n`, "utf8"));
  return writeQueue;
}

const server = http.createServer(async (request, response) => {
  const startedAt = Date.now();
  const requestId = ++sequence;
  const requestUrl = new URL(request.url ?? "/", upstreamRoot);
  let requestBody;
  let model = null;
  try {
    requestBody = await readRequestBody(request);
    if (requestBody.length > 0) {
      try {
        model = JSON.parse(requestBody.toString("utf8"))?.model ?? null;
      } catch {
        // The proxy only records metadata and never persists prompt bodies.
      }
    }
    const headers = new Headers();
    for (const [key, value] of Object.entries(request.headers)) {
      if (!value || ["host", "connection", "content-length"].includes(key.toLowerCase())) continue;
      headers.set(key, Array.isArray(value) ? value.join(", ") : value);
    }
    const upstream = await fetch(requestUrl, {
      method: request.method,
      headers,
      body: requestBody.length > 0 ? requestBody : undefined,
    });
    const contentType = upstream.headers.get("content-type") ?? "";
    void upstream
      .clone()
      .text()
      .then((text) => appendRecord({
        schemaVersion: 1,
        requestId,
        startedAt: new Date(startedAt).toISOString(),
        durationMs: Date.now() - startedAt,
        method: request.method,
        path: requestUrl.pathname,
        model,
        upstreamStatus: upstream.status,
        usages: usagesFromResponseText(text, contentType),
      }))
      .catch(() => appendRecord({
        schemaVersion: 1,
        requestId,
        startedAt: new Date(startedAt).toISOString(),
        durationMs: Date.now() - startedAt,
        method: request.method,
        path: requestUrl.pathname,
        model,
        upstreamStatus: upstream.status,
        usages: [],
        telemetryError: "response_body_unavailable",
      }));
    response.writeHead(upstream.status, sanitizeResponseHeaders(upstream.headers));
    if (upstream.body) Readable.fromWeb(upstream.body).pipe(response);
    else response.end();
  } catch (error) {
    await appendRecord({
      schemaVersion: 1,
      requestId,
      startedAt: new Date(startedAt).toISOString(),
      durationMs: Date.now() - startedAt,
      method: request.method,
      path: requestUrl.pathname,
      model,
      upstreamStatus: null,
      usages: [],
      transportError: String(error?.message ?? error),
    });
    if (!response.headersSent) response.writeHead(502, { "content-type": "application/json" });
    response.end(JSON.stringify({ error: "provider_usage_proxy_upstream_error" }));
  }
});

server.listen(options.port, "127.0.0.1", () => {
  process.stdout.write(`provider usage proxy listening on http://127.0.0.1:${options.port}\n`);
});

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => server.close(() => process.exit(0)));
}
