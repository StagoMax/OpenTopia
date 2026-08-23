const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const { createAppLogger } = require("./logging.cjs");

function createLogger() {
  return createAppLogger({
    app: { getPath: () => "unused" },
    apiToken: "desktop-api-token-for-test",
    getBackendUrl: () => "http://127.0.0.1:8787",
    isDev: true,
  });
}

test("redacts the desktop API token and authorization values", () => {
  const { redactSecrets } = createLogger();
  assert.equal(
    redactSecrets(
      "token=desktop-api-token-for-test Authorization: Bearer visible-token",
    ),
    "token=[redacted] Authorization: [redacted] [redacted]",
  );
});

test("sanitizes nested secret fields without mutating safe metadata", () => {
  const { sanitizeForLog } = createLogger();
  assert.deepEqual(
    sanitizeForLog({
      request: { apiKey: "hidden", model: "gpt-test" },
      count: 3,
    }),
    {
      request: { apiKey: "[redacted]", model: "gpt-test" },
      count: 3,
    },
  );
});

test("serializes errors through the same redaction boundary", () => {
  const { serializeError } = createLogger();
  const error = new Error("desktop-api-token-for-test failed");
  error.code = "E_TEST";
  const serialized = serializeError(error);
  assert.equal(serialized.name, "Error");
  assert.equal(serialized.message, "[redacted:api-token] failed");
  assert.equal(serialized.code, "E_TEST");
  assert.doesNotMatch(serialized.stack, /desktop-api-token-for-test/);
});

test("buffers regular log writes and flushes them as one batch", () => {
  const userData = fs.mkdtempSync(path.join(os.tmpdir(), "opentopia-logs-"));
  try {
    const logger = createAppLogger({
      app: { getPath: () => userData, on: () => {} },
      apiToken: "desktop-api-token-for-test",
      getBackendUrl: () => "http://127.0.0.1:8787",
      isDev: true,
    });
    logger.ensureLoggingInitialized();
    logger.writeLog("info", "test.first", { value: 1 });
    logger.writeLog("info", "test.second", { value: 2 });
    logger.flushLogsSync();

    const logName = fs
      .readdirSync(path.join(userData, "logs"))
      .find((name) => name.startsWith("startup-"));
    assert.ok(logName);
    const records = fs
      .readFileSync(path.join(userData, "logs", logName), "utf8")
      .trim()
      .split("\n")
      .map((line) => JSON.parse(line));
    assert.deepEqual(
      records.slice(-2).map((record) => record.event),
      ["test.first", "test.second"],
    );
  } finally {
    fs.rmSync(userData, { recursive: true, force: true });
  }
});
