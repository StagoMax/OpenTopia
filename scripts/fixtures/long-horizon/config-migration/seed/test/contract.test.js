import assert from "node:assert/strict";
import test from "node:test";
import {
  applyOverrides,
  parseLegacyConfig,
  serializeConfig,
} from "../src/config.js";

const legacy = JSON.stringify({
  version: 1,
  endpoint: "https://example.test/v1/",
  model: "model-a",
  timeout_seconds: 30,
  features: ["stream"],
});

test("parses and normalizes a legacy config", () => {
  assert.deepEqual(parseLegacyConfig(legacy), {
    schemaVersion: 2,
    provider: { baseUrl: "https://example.test/v1", model: "model-a" },
    timeoutMs: 30000,
    capabilities: { stream: true, tools: false },
  });
});

test("applies recognized environment overrides without mutation", () => {
  const config = parseLegacyConfig(legacy);
  const overridden = applyOverrides(config, {
    APP_MODEL: "model-b",
    APP_TIMEOUT_MS: "45000",
    IGNORED: "value",
  });
  assert.equal(config.provider.model, "model-a");
  assert.equal(overridden.provider.model, "model-b");
  assert.equal(overridden.timeoutMs, 45000);
});

test("rejects invalid legacy input", () => {
  assert.throws(
    () =>
      parseLegacyConfig(
        JSON.stringify({
          version: 1,
          endpoint: "file:///tmp/config",
          model: "x",
          timeout_seconds: 10,
          features: [],
        }),
      ),
    /url|scheme|http/i,
  );
});

test("serializes deterministic JSON with one trailing newline", () => {
  const output = serializeConfig(parseLegacyConfig(legacy));
  assert.ok(output.endsWith("\n"));
  assert.equal(output.endsWith("\n\n"), false);
  assert.deepEqual(JSON.parse(output), parseLegacyConfig(legacy));
});
