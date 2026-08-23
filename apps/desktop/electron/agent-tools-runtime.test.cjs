const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const {
  applyAgentToolsEnvironment,
  loadAgentToolsRuntime,
  resolveDevelopmentAgentToolsRuntime,
} = require("./agent-tools-runtime.cjs");

function sha256(contents) {
  return crypto.createHash("sha256").update(contents).digest("hex");
}

function writeRuntime(root) {
  const rgContents = "ripgrep";
  const gitContents = "git";
  fs.mkdirSync(path.join(root, "bin"), { recursive: true });
  fs.mkdirSync(path.join(root, "git", "cmd"), { recursive: true });
  fs.writeFileSync(path.join(root, "bin", "rg.exe"), rgContents);
  fs.writeFileSync(path.join(root, "git", "cmd", "git.exe"), gitContents);
  fs.writeFileSync(
    path.join(root, "agent-tools-runtime.json"),
    JSON.stringify({
      schemaVersion: 1,
      id: "ai.opentopia.agent-tools-runtime",
      version: "test-runtime",
      target: "windows-x86_64",
      pathEntries: ["bin", "git/cmd"],
      tools: {
        rg: {
          version: "15.2.0",
          executable: "bin/rg.exe",
          sha256: sha256(rgContents),
        },
        git: {
          version: "2.55.0.windows.3",
          executable: "git/cmd/git.exe",
          sha256: sha256(gitContents),
        },
      },
    }),
  );
}

test("validates the managed tools and prepends their PATH entries", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "opentopia-agent-tools-"));
  try {
    writeRuntime(root);
    const runtime = loadAgentToolsRuntime(root);
    const env = {
      PATH: ["host-bin", runtime.pathEntries[0]].join(path.delimiter),
    };
    applyAgentToolsEnvironment(env, runtime, "win32");

    assert.equal(env.OPENTOPIA_AGENT_TOOLS_ROOT, root);
    assert.deepEqual(env.PATH.split(path.delimiter).slice(0, 3), [
      path.join(root, "bin"),
      path.join(root, "git", "cmd"),
      "host-bin",
    ]);
    assert.equal(runtime.tools.rg.version, "15.2.0");
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("rejects a tool whose hash no longer matches the manifest", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "opentopia-agent-tools-"));
  try {
    writeRuntime(root);
    fs.writeFileSync(path.join(root, "bin", "rg.exe"), "tampered");
    assert.throws(() => loadAgentToolsRuntime(root), /rg hash does not match/);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("rejects PATH entries that escape the managed runtime", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "opentopia-agent-tools-"));
  try {
    writeRuntime(root);
    const manifestPath = path.join(root, "agent-tools-runtime.json");
    const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
    manifest.pathEntries = ["../outside"];
    fs.writeFileSync(manifestPath, JSON.stringify(manifest));
    assert.throws(() => loadAgentToolsRuntime(root), /escapes its root/);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("honors an explicit development runtime root", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "opentopia-agent-tools-"));
  try {
    writeRuntime(root);
    const runtime = resolveDevelopmentAgentToolsRuntime(
      "unused-repo",
      { OPENTOPIA_AGENT_TOOLS_ROOT: root },
      "win32",
      "x64",
    );
    assert.equal(runtime.root, root);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});
