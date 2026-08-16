const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const {
  createGraphRagServiceManager,
  resolveGraphRagLaunch,
} = require("./graph-rag-service.cjs");

test("discovers the Graph RAG project through its entrypoint contract", () => {
  const temporaryRoot = fs.mkdtempSync(
    path.join(os.tmpdir(), "opentopia-graph-rag-"),
  );
  try {
    const repoRoot = path.join(temporaryRoot, "OpenTopia");
    const projectRoot = path.join(temporaryRoot, "knowledge-engine");
    fs.mkdirSync(repoRoot);
    fs.mkdirSync(projectRoot);
    fs.writeFileSync(
      path.join(projectRoot, "pyproject.toml"),
      '[project.scripts]\nenterprise-graph-rag-panel = "enterprise_rag.main:main"\n',
    );
    const python =
      process.platform === "win32"
        ? path.join(projectRoot, ".venv", "Scripts", "python.exe")
        : path.join(projectRoot, ".venv", "bin", "python3");
    fs.mkdirSync(path.dirname(python), { recursive: true });
    fs.writeFileSync(python, "test runtime");

    const launch = resolveGraphRagLaunch({
      endpoint: "http://127.0.0.1:8000",
      env: {},
      isPackaged: false,
      repoRoot,
    });
    assert.equal(launch.cwd, projectRoot);
    assert.deepEqual(launch.args.slice(0, 2), ["-m", "enterprise_rag.main"]);
  } finally {
    fs.rmSync(temporaryRoot, { recursive: true, force: true });
  }
});

test("requires review-only health flags before Graph RAG is ready", async () => {
  const manager = createGraphRagServiceManager({
    endpoint: "http://127.0.0.1:8000",
    env: {},
    isPackaged: true,
    resourcesPath: path.join(os.tmpdir(), "missing-opentopia-resources"),
    fetchImpl: async () => ({
      ok: true,
      json: async () => ({
        status: "ok",
        prompt_injection: true,
        agent_loop_integration: false,
      }),
    }),
  });
  assert.equal(await manager.isHealthy(), false);
});
