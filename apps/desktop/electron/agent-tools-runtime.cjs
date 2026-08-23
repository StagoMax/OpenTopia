const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");

const agentToolsManifestName = "agent-tools-runtime.json";
const agentToolsRuntimeId = "ai.opentopia.agent-tools-runtime";
const agentToolsLockId = "ai.opentopia.agent-tools-runtime.lock";

function sha256File(filePath) {
  return crypto
    .createHash("sha256")
    .update(fs.readFileSync(filePath))
    .digest("hex");
}

function resolveContainedPath(root, relativePath, label, expectedType) {
  if (typeof relativePath !== "string" || relativePath.trim() === "") {
    throw new Error(`Agent tools runtime has no valid ${label} path.`);
  }
  const resolvedRoot = path.resolve(root);
  const resolved = path.resolve(resolvedRoot, relativePath);
  const relative = path.relative(resolvedRoot, resolved);
  if (relative.startsWith("..") || path.isAbsolute(relative)) {
    throw new Error(`Agent tools runtime ${label} path escapes its root.`);
  }
  if (!fs.existsSync(resolved)) {
    throw new Error(`Agent tools runtime ${label} is missing at ${resolved}.`);
  }
  const stat = fs.statSync(resolved);
  if (expectedType === "file" && !stat.isFile()) {
    throw new Error(`Agent tools runtime ${label} is not a file.`);
  }
  if (expectedType === "directory" && !stat.isDirectory()) {
    throw new Error(`Agent tools runtime ${label} is not a directory.`);
  }
  return resolved;
}

function loadAgentToolsRuntime(root, requiredTools = ["rg", "git"]) {
  const resolvedRoot = path.resolve(root);
  const manifestPath = path.join(resolvedRoot, agentToolsManifestName);
  const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
  if (manifest.schemaVersion !== 1 || manifest.id !== agentToolsRuntimeId) {
    throw new Error(
      `Unsupported OpenTopia agent tools manifest at ${manifestPath}.`,
    );
  }
  if (
    !Array.isArray(manifest.pathEntries) ||
    manifest.pathEntries.length === 0
  ) {
    throw new Error("Agent tools runtime manifest has no PATH entries.");
  }

  const pathEntries = manifest.pathEntries.map((entry, index) =>
    resolveContainedPath(
      resolvedRoot,
      entry,
      `PATH entry ${index + 1}`,
      "directory",
    ),
  );
  const tools = {};
  for (const name of requiredTools) {
    const descriptor = manifest.tools?.[name];
    if (
      !descriptor ||
      typeof descriptor.version !== "string" ||
      typeof descriptor.sha256 !== "string"
    ) {
      throw new Error(
        `Agent tools runtime manifest has no valid ${name} tool.`,
      );
    }
    const executable = resolveContainedPath(
      resolvedRoot,
      descriptor.executable,
      `${name} executable`,
      "file",
    );
    if (
      sha256File(executable).toLowerCase() !== descriptor.sha256.toLowerCase()
    ) {
      throw new Error(
        `Agent tools runtime ${name} hash does not match its manifest.`,
      );
    }
    tools[name] = { ...descriptor, executable };
  }

  return {
    root: resolvedRoot,
    manifestPath,
    manifest,
    pathEntries,
    tools,
  };
}

function currentAgentToolsTarget(
  platform = process.platform,
  arch = process.arch,
) {
  if (platform !== "win32") return null;
  if (arch === "x64") return "windows-x86_64";
  if (arch === "arm64") return "windows-aarch64";
  return null;
}

function resolveDevelopmentAgentToolsRuntime(
  repoRoot,
  env = process.env,
  platform = process.platform,
  arch = process.arch,
) {
  if (env.OPENTOPIA_AGENT_TOOLS_ROOT) {
    return loadAgentToolsRuntime(env.OPENTOPIA_AGENT_TOOLS_ROOT);
  }

  const target = currentAgentToolsTarget(platform, arch);
  if (!target) return null;
  const runtimeDirectory = path.join(repoRoot, "runtime", "agent-tools");
  const lockPath = path.join(runtimeDirectory, "runtime-lock.json");
  const lock = JSON.parse(fs.readFileSync(lockPath, "utf8"));
  if (lock.schemaVersion !== 1 || lock.id !== agentToolsLockId) {
    throw new Error(`Unsupported OpenTopia agent tools lock at ${lockPath}.`);
  }

  const candidates = [
    path.join(runtimeDirectory, "dist"),
    path.join(runtimeDirectory, "cache", lock.runtimeVersion, target),
  ];
  const preparedRoot = candidates.find((candidate) => fs.existsSync(candidate));
  return preparedRoot ? loadAgentToolsRuntime(preparedRoot) : null;
}

function applyAgentToolsEnvironment(env, runtime, platform = process.platform) {
  if (!runtime) return env;
  const existingPath = env.PATH || env.Path || "";
  const normalize = (entry) =>
    platform === "win32" ? entry.toLowerCase() : entry;
  const seen = new Set();
  const entries = [
    ...runtime.pathEntries,
    ...existingPath.split(path.delimiter),
  ]
    .filter(Boolean)
    .filter((entry) => {
      const key = normalize(entry);
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });
  env.PATH = entries.join(path.delimiter);
  env.OPENTOPIA_AGENT_TOOLS_ROOT = runtime.root;
  return env;
}

module.exports = {
  agentToolsManifestName,
  applyAgentToolsEnvironment,
  currentAgentToolsTarget,
  loadAgentToolsRuntime,
  resolveDevelopmentAgentToolsRuntime,
};
