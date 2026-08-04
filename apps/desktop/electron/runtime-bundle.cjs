const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const protocolSchema = "ai.opentopia.sandbox.protocol";
const runtimeManifestName = "opentopia-runtime-manifest.json";

function sha256File(filePath) {
  const hash = crypto.createHash("sha256");
  hash.update(fs.readFileSync(filePath));
  return hash.digest("hex");
}

function validateProtocol(info, expected) {
  if (!info || info.schema !== protocolSchema) {
    throw new Error(`Sandbox helper returned an unknown protocol schema.`);
  }
  if (!Number.isInteger(info.protocolVersion) || info.protocolVersion < 1) {
    throw new Error(`Sandbox helper returned an invalid protocol version.`);
  }
  if (!Array.isArray(info.features)) {
    throw new Error(`Sandbox helper protocol descriptor has no feature list.`);
  }
  if (expected) {
    if (
      expected.schema !== info.schema ||
      expected.protocolVersion !== info.protocolVersion
    ) {
      throw new Error(
        `Sandbox helper protocol ${info.protocolVersion} does not match runtime bundle protocol ${expected.protocolVersion}.`,
      );
    }
    const missing = (expected.features || []).filter(
      (feature) => !info.features.includes(feature),
    );
    if (missing.length > 0) {
      throw new Error(
        `Sandbox helper is missing runtime bundle features: ${missing.join(", ")}.`,
      );
    }
  }
  return info;
}

function inspectSandboxProtocol(binaryPath, expected) {
  const result = spawnSync(binaryPath, ["protocol", "--json"], {
    encoding: "utf8",
    timeout: 5000,
    windowsHide: true,
  });
  if (result.error) {
    throw new Error(
      `Sandbox protocol handshake failed at ${binaryPath}: ${result.error.message}`,
    );
  }
  if (result.status !== 0) {
    throw new Error(
      `Sandbox helper at ${binaryPath} does not support the required protocol handshake (exit ${result.status}): ${(result.stderr || "no diagnostic").trim()}`,
    );
  }
  let info;
  try {
    info = JSON.parse(result.stdout);
  } catch (error) {
    throw new Error(
      `Sandbox helper at ${binaryPath} returned an invalid protocol descriptor: ${error.message}`,
    );
  }
  return validateProtocol(info, expected);
}

function resolveArtifact(bundleRoot, descriptor, label) {
  if (
    !descriptor ||
    typeof descriptor.path !== "string" ||
    typeof descriptor.sha256 !== "string"
  ) {
    throw new Error(`Runtime manifest has no valid ${label} artifact.`);
  }
  const resolved = path.resolve(bundleRoot, descriptor.path);
  const rootPrefix = `${path.resolve(bundleRoot)}${path.sep}`;
  if (!resolved.startsWith(rootPrefix)) {
    throw new Error(`Runtime manifest ${label} path escapes the bundle root.`);
  }
  if (!fs.existsSync(resolved)) {
    throw new Error(`Runtime bundle ${label} is missing at ${resolved}.`);
  }
  const actualHash = sha256File(resolved);
  if (actualHash.toLowerCase() !== descriptor.sha256.toLowerCase()) {
    throw new Error(`Runtime bundle ${label} hash does not match its manifest.`);
  }
  return resolved;
}

function loadRuntimeBundle(resourcesPath, platform = process.platform) {
  const manifestPath = path.join(resourcesPath, runtimeManifestName);
  const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
  if (manifest.schemaVersion !== 1) {
    throw new Error(
      `Unsupported OpenTopia runtime manifest version ${manifest.schemaVersion}.`,
    );
  }
  const server = resolveArtifact(resourcesPath, manifest.artifacts?.server, "server");
  let sandbox = null;
  let sandboxProtocol = null;
  if (platform === "win32") {
    sandbox = resolveArtifact(
      resourcesPath,
      manifest.artifacts?.sandbox,
      "sandbox helper",
    );
    sandboxProtocol = inspectSandboxProtocol(sandbox, manifest.sandboxProtocol);
  }
  return { manifestPath, server, sandbox, sandboxProtocol, manifest };
}

module.exports = {
  inspectSandboxProtocol,
  loadRuntimeBundle,
  runtimeManifestName,
  validateProtocol,
};

