const path = require("node:path");
const { spawnSync } = require("node:child_process");
const {
  resolveDevelopmentAgentToolsRuntime,
} = require("../apps/desktop/electron/agent-tools-runtime.cjs");

if (process.platform !== "win32") {
  process.exit(0);
}

const repoRoot = path.resolve(__dirname, "..");
const prepared = resolveDevelopmentAgentToolsRuntime(repoRoot);
if (prepared) {
  console.log(`Reusing OpenTopia agent tools runtime: ${prepared.root}`);
  process.exit(0);
}

const preparer = path.join(
  repoRoot,
  "scripts",
  "prepare-agent-tools-runtime.ps1",
);
const result = spawnSync(
  "powershell.exe",
  ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", preparer],
  {
    cwd: repoRoot,
    env: process.env,
    stdio: "inherit",
    windowsHide: true,
  },
);
if (result.error) {
  throw result.error;
}
if (result.status !== 0) {
  process.exit(result.status ?? 1);
}
