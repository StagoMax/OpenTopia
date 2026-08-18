const assert = require("node:assert/strict");
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
