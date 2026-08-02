import assert from "node:assert/strict";
import test from "node:test";

import type * as PluginControlModule from "./pluginControl";

const {
  activationForScope,
  parseJsonObject,
  permissionGrantForScope,
  pluginSettingFields,
  scopeMatches,
} = (await import("./pluginControl" + ".ts")) as typeof PluginControlModule;

test("maps common JSON schema properties to native setting controls", () => {
  const fields = pluginSettingFields(
    {
      type: "object",
      required: ["enabled", "token"],
      properties: {
        enabled: { type: "boolean", title: "Enabled" },
        mode: { type: "string", enum: ["fast", "careful"] },
        retries: { type: "integer", minimum: 0, maximum: 5 },
        endpoint: { type: "string" },
        token: { type: "string" },
        matcher: { type: "object" },
      },
    },
    ["token"],
  );

  assert.deepEqual(
    fields.map(({ key, kind, required }) => ({ key, kind, required })),
    [
      { key: "enabled", kind: "boolean", required: true },
      { key: "mode", kind: "enum", required: false },
      { key: "retries", kind: "integer", required: false },
      { key: "endpoint", kind: "string", required: false },
      { key: "token", kind: "secret", required: true },
      { key: "matcher", kind: "json", required: false },
    ],
  );
});

test("matches normalized workspace scopes and selects scoped records", () => {
  const requested = {
    scopeType: "workspace" as const,
    scopeId: "J:\\Work\\Demo",
  };
  const stored = { scopeType: "workspace" as const, scopeId: "j:/work/demo/" };
  assert.equal(scopeMatches(requested, stored), true);

  const activation = activationForScope(
    [{ pluginId: "plugin", scope: stored, enabled: false, updatedAt: "now" }],
    requested,
  );
  assert.equal(activation?.enabled, false);

  const grant = permissionGrantForScope(
    [
      {
        pluginId: "plugin",
        scope: stored,
        permission: "filesystem:workspace:read",
        constraint: {},
        status: "granted",
        updatedAt: "now",
      },
    ],
    requested,
    "filesystem:workspace:read",
  );
  assert.equal(grant?.status, "granted");
});

test("accepts only JSON objects for permission constraints", () => {
  assert.deepEqual(parseJsonObject('{"roots":["workspace"]}'), {
    roots: ["workspace"],
  });
  assert.throws(() => parseJsonObject("[]"), /JSON object/);
});
