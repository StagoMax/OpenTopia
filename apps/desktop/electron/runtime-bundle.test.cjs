const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const { loadRuntimeBundle, validateProtocol } = require("./runtime-bundle.cjs");

const current = {
  schema: "ai.opentopia.sandbox.protocol",
  protocolVersion: 2,
  helperVersion: "0.1.0",
  features: [
    "run.backend",
    "run.runtime_roots",
    "run.filesystem_capabilities.v1",
  ],
};

test("accepts a helper matching the runtime bundle protocol", () => {
  assert.equal(validateProtocol(current, { ...current }), current);
});

test("rejects a stale helper protocol before backend startup", () => {
  assert.throws(
    () =>
      validateProtocol(
        { ...current, protocolVersion: current.protocolVersion - 1 },
        current,
      ),
    /does not match runtime bundle protocol/,
  );
});

test("rejects a helper missing a required bundle feature", () => {
  assert.throws(
    () => validateProtocol({ ...current, features: ["run.backend"] }, current),
    /missing runtime bundle features: run.runtime_roots/,
  );
});

test("binds the verified Office runtime directory for the backend", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "opentopia-runtime-bundle-"));
  try {
    const server = path.join(root, "opentopia-server");
    const officeDirectory = path.join(root, "office-runtime");
    const officeManifest = path.join(officeDirectory, "office-runtime.json");
    fs.mkdirSync(officeDirectory);
    fs.writeFileSync(server, "server");
    fs.writeFileSync(officeManifest, "office runtime");
    const sha256 = (file) =>
      crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
    fs.writeFileSync(
      path.join(root, "opentopia-runtime-manifest.json"),
      JSON.stringify({
        schemaVersion: 1,
        artifacts: {
          server: { path: "opentopia-server", sha256: sha256(server) },
          officeRuntime: {
            path: "office-runtime/office-runtime.json",
            sha256: sha256(officeManifest),
          },
        },
      }),
    );

    const bundle = loadRuntimeBundle(root, "linux");
    assert.equal(bundle.officeRuntimeRoot, officeDirectory);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});
