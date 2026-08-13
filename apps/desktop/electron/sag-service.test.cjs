const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const { EventEmitter } = require("node:events");
const {
  createSagServiceManager,
  endpointInfo,
  resolveSagLaunch,
  sagChildEnv,
} = require("./sag-service.cjs");

function createSagProject(parent, name = "memory-engine") {
  const projectRoot = path.join(parent, name);
  fs.mkdirSync(projectRoot, { recursive: true });
  fs.writeFileSync(
    path.join(projectRoot, "pyproject.toml"),
    '[project.scripts]\nenterprise-sag-panel = "enterprise_sag.panel:main"\n',
  );
  const python =
    process.platform === "win32"
      ? path.join(projectRoot, ".venv", "Scripts", "python.exe")
      : path.join(projectRoot, ".venv", "bin", "python3");
  fs.mkdirSync(path.dirname(python), { recursive: true });
  fs.writeFileSync(python, "test runtime");
  return { projectRoot, python };
}

test("classifies only loopback HTTP endpoints as locally managed", () => {
  assert.equal(endpointInfo("http://127.0.0.1:8765").local, true);
  assert.equal(endpointInfo("https://sag.example.test").local, false);
  assert.match(endpointInfo("not a URL").error, /有效地址/);
});

test("passes only runtime-scoped variables to the SAG child process", () => {
  const selected = sagChildEnv({
    PATH: "runtime-path",
    DEEPSEEK_KEY: "sag-secret",
    SAG_LLM_MODEL: "deepseek",
    OPENTOPIA_API_TOKEN: "must-not-leak",
    OPENTOPIA_CHROME_BRIDGE_TOKEN: "must-not-leak",
  });
  assert.equal(selected.PATH, "runtime-path");
  assert.equal(selected.DEEPSEEK_KEY, "sag-secret");
  assert.equal(selected.SAG_LLM_MODEL, "deepseek");
  assert.equal(selected.OPENTOPIA_API_TOKEN, undefined);
  assert.equal(selected.OPENTOPIA_CHROME_BRIDGE_TOKEN, undefined);
});

test("resolves an explicitly configured SAG project without a machine-specific path", () => {
  const temporaryRoot = fs.mkdtempSync(
    path.join(os.tmpdir(), "opentopia-sag-"),
  );
  try {
    const project = createSagProject(temporaryRoot);
    const launch = resolveSagLaunch({
      endpoint: "http://127.0.0.1:8765",
      env: { OPENTOPIA_SAG_PROJECT_ROOT: project.projectRoot },
    });
    assert.equal(launch.command, project.python);
    assert.equal(launch.cwd, project.projectRoot);
    assert.equal(launch.source, "configured-project");
    assert.deepEqual(launch.args.slice(-4), [
      "--host",
      "127.0.0.1",
      "--port",
      "8765",
    ]);
  } finally {
    fs.rmSync(temporaryRoot, { recursive: true, force: true });
  }
});

test("development discovery uses the SAG project contract instead of a fixed folder name", () => {
  const temporaryRoot = fs.mkdtempSync(
    path.join(os.tmpdir(), "opentopia-sag-"),
  );
  try {
    const repoRoot = path.join(temporaryRoot, "OpenTopia");
    fs.mkdirSync(repoRoot);
    const project = createSagProject(temporaryRoot, "renamed-memory-service");
    const launch = resolveSagLaunch({
      endpoint: "http://127.0.0.1:8765",
      env: {},
      isPackaged: false,
      repoRoot,
    });
    assert.equal(launch.cwd, project.projectRoot);
    assert.equal(launch.source, "development-project");
  } finally {
    fs.rmSync(temporaryRoot, { recursive: true, force: true });
  }
});

test("deduplicates concurrent starts and reports a managed ready service", async () => {
  let fetchCount = 0;
  let spawnCount = 0;
  const manager = createSagServiceManager({
    endpoint: "http://127.0.0.1:8765",
    env: { OPENTOPIA_SAG_EXECUTABLE: process.execPath },
    fetchImpl: async () => ({
      ok: ++fetchCount > 1,
      json: async () => ({
        status: "ready",
        prompt_injection: false,
        agent_loop_integration: false,
      }),
    }),
    healthAttempts: 2,
    healthIntervalMs: 1,
    spawn: () => {
      spawnCount += 1;
      const child = new EventEmitter();
      child.pid = 42;
      child.stdout = new EventEmitter();
      child.stderr = new EventEmitter();
      child.kill = () => true;
      return child;
    },
  });

  const [first, second] = await Promise.all([
    manager.ensureReady(),
    manager.ensureReady(),
  ]);
  assert.equal(spawnCount, 1);
  assert.deepEqual(first, second);
  assert.equal(first.state, "ready");
  assert.equal(first.managed, true);
});

test("returns an actionable unavailable state when no trusted runtime exists", async () => {
  const manager = createSagServiceManager({
    endpoint: "http://127.0.0.1:8765",
    env: {},
    isPackaged: true,
    resourcesPath: path.join(os.tmpdir(), "missing-opentopia-resources"),
    fetchImpl: async () => {
      throw new Error("offline");
    },
  });
  const status = await manager.ensureReady();
  assert.equal(status.state, "unavailable");
  assert.equal(status.canStart, false);
  assert.match(status.message, /OPENTOPIA_SAG_PROJECT_ROOT/);
});
