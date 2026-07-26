import { spawn } from "node:child_process";
import process from "node:process";

const SAFE_ENVIRONMENT_KEYS = [
  "PATH",
  "Path",
  "PATHEXT",
  "SystemRoot",
  "WINDIR",
  "ComSpec",
  "TEMP",
  "TMP",
  "TMPDIR",
  "LANG",
  "LC_ALL"
];

export function minimalEnvironment() {
  const environment = {};
  for (const key of SAFE_ENVIRONMENT_KEYS) {
    if (process.env[key] !== undefined) environment[key] = process.env[key];
  }
  return environment;
}

async function terminateProcessTree(child) {
  if (!child || child.exitCode !== null) return;
  if (process.platform === "win32") {
    await new Promise((resolve) => {
      const killer = spawn("taskkill.exe", ["/PID", String(child.pid), "/T", "/F"], {
        stdio: "ignore",
        windowsHide: true
      });
      killer.once("close", resolve);
      killer.once("error", resolve);
    });
    return;
  }
  try {
    process.kill(-child.pid, "SIGKILL");
  } catch {
    child.kill("SIGKILL");
  }
}

export async function runCommand({
  command,
  args = [],
  cwd,
  env = {},
  inheritEnvironment = false,
  stdin,
  timeoutMs = 300000,
  maximumOutputBytes = 2 * 1024 * 1024
}) {
  const startedAt = Date.now();
  const childEnvironment = {
    ...(inheritEnvironment ? process.env : minimalEnvironment()),
    ...env
  };
  const child = spawn(command, args, {
    cwd,
    env: childEnvironment,
    detached: process.platform !== "win32",
    windowsHide: true,
    shell: false,
    stdio: ["pipe", "pipe", "pipe"]
  });

  let stdout = "";
  let stderr = "";
  let stdoutTruncated = false;
  let stderrTruncated = false;
  const append = (current, chunk, markTruncated) => {
    if (Buffer.byteLength(current) >= maximumOutputBytes) {
      markTruncated();
      return current;
    }
    const remaining = maximumOutputBytes - Buffer.byteLength(current);
    if (chunk.length > remaining) markTruncated();
    return current + chunk.subarray(0, remaining).toString("utf8");
  };
  child.stdout.on("data", (chunk) => {
    stdout = append(stdout, chunk, () => { stdoutTruncated = true; });
  });
  child.stderr.on("data", (chunk) => {
    stderr = append(stderr, chunk, () => { stderrTruncated = true; });
  });

  let spawnError = null;
  child.once("error", (error) => { spawnError = error; });

  if (stdin !== undefined && stdin !== null) child.stdin.end(stdin);
  else child.stdin.end();

  let timedOut = false;
  const timer = setTimeout(async () => {
    timedOut = true;
    await terminateProcessTree(child);
  }, timeoutMs);
  timer.unref();

  const completion = await new Promise((resolve) => {
    child.once("close", (exitCode, signal) => resolve({ exitCode, signal }));
    child.once("error", () => resolve({ exitCode: null, signal: null }));
  });
  clearTimeout(timer);

  if (timedOut && child.exitCode === null) await terminateProcessTree(child);

  return {
    command,
    args,
    cwd,
    exitCode: completion.exitCode,
    signal: completion.signal,
    timedOut,
    spawnError: spawnError?.message ?? null,
    stdout,
    stderr,
    stdoutTruncated,
    stderrTruncated,
    elapsedMs: Date.now() - startedAt
  };
}
