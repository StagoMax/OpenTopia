const fs = require("node:fs");
const path = require("node:path");
const { spawn: defaultSpawn, spawnSync } = require("node:child_process");

const DEFAULT_SAG_URL = "http://127.0.0.1:8765";
const SAG_ENTRYPOINT_PATTERN =
  /enterprise-sag-panel\s*=\s*["']enterprise_sag\.panel:main["']/;
const LOOPBACK_HOSTS = new Set(["127.0.0.1", "::1", "localhost"]);
const SAG_CHILD_ENV_NAMES = new Set([
  "APPDATA",
  "COMSPEC",
  "CUDA_VISIBLE_DEVICES",
  "HF_HOME",
  "HOME",
  "HTTPS_PROXY",
  "HTTP_PROXY",
  "LOCALAPPDATA",
  "NO_PROXY",
  "PATH",
  "PATHEXT",
  "PROGRAMDATA",
  "SYSTEMDRIVE",
  "SYSTEMROOT",
  "TEMP",
  "TMP",
  "TRANSFORMERS_CACHE",
  "USERPROFILE",
  "WINDIR",
]);
const SAG_CHILD_ENV_PREFIXES = [
  "CUDA_",
  "DEEPSEEK_",
  "NVIDIA_",
  "OPENTOPIA_SAG_",
  "PYTHON",
  "SAG_",
];

function endpointInfo(rawEndpoint = DEFAULT_SAG_URL) {
  let endpoint;
  try {
    endpoint = new URL(String(rawEndpoint || DEFAULT_SAG_URL).trim());
  } catch {
    return {
      error: "OPENTOPIA_SAG_URL 不是有效地址。",
      endpoint: String(rawEndpoint || DEFAULT_SAG_URL),
    };
  }

  const host = endpoint.hostname.replace(/^\[|\]$/g, "");
  const port = Number(
    endpoint.port || (endpoint.protocol === "https:" ? "443" : "80"),
  );
  if (
    !matchesSupportedProtocol(endpoint) ||
    !Number.isInteger(port) ||
    port < 1 ||
    port > 65535
  ) {
    return {
      error: "OPENTOPIA_SAG_URL 必须是有效的 HTTP 或 HTTPS 地址。",
      endpoint: endpoint.toString().replace(/\/$/, ""),
    };
  }

  endpoint.pathname = endpoint.pathname.replace(/\/$/, "");
  endpoint.search = "";
  endpoint.hash = "";
  return {
    endpoint: endpoint.toString().replace(/\/$/, ""),
    host,
    port,
    local: endpoint.protocol === "http:" && LOOPBACK_HOSTS.has(host),
  };
}

function matchesSupportedProtocol(endpoint) {
  return endpoint.protocol === "http:" || endpoint.protocol === "https:";
}

function sagChildEnv(env) {
  const selected = {};
  for (const [name, value] of Object.entries(env || {})) {
    const normalized = name.toUpperCase();
    if (
      SAG_CHILD_ENV_NAMES.has(normalized) ||
      SAG_CHILD_ENV_PREFIXES.some((prefix) => normalized.startsWith(prefix))
    ) {
      selected[name] = value;
    }
  }
  selected.PYTHONUTF8 ||= "1";
  return selected;
}

function isSagProject(projectRoot) {
  if (!projectRoot) return false;
  try {
    const pyproject = fs.readFileSync(
      path.join(projectRoot, "pyproject.toml"),
      "utf8",
    );
    return SAG_ENTRYPOINT_PATTERN.test(pyproject);
  } catch {
    return false;
  }
}

function projectPython(projectRoot) {
  const candidates =
    process.platform === "win32"
      ? [
          path.join(projectRoot, ".venv", "Scripts", "python.exe"),
          path.join(projectRoot, ".venv", "Scripts", "python3.exe"),
        ]
      : [
          path.join(projectRoot, ".venv", "bin", "python3"),
          path.join(projectRoot, ".venv", "bin", "python"),
        ];
  return candidates.find((candidate) => fs.existsSync(candidate)) || null;
}

function discoverSagProject(searchRoot) {
  if (!searchRoot || !fs.existsSync(searchRoot)) return null;
  if (isSagProject(searchRoot) && projectPython(searchRoot)) return searchRoot;

  let entries;
  try {
    entries = fs.readdirSync(searchRoot, { withFileTypes: true });
  } catch {
    return null;
  }
  for (const entry of entries) {
    if (!entry.isDirectory() || entry.name.startsWith(".")) continue;
    const candidate = path.join(searchRoot, entry.name);
    if (isSagProject(candidate) && projectPython(candidate)) return candidate;
  }
  return null;
}

function projectLaunch(projectRoot, source, endpoint) {
  const python = projectPython(projectRoot);
  if (!python) {
    return {
      error: `SAG 项目缺少可用的 .venv Python：${projectRoot}`,
      source,
    };
  }
  return {
    command: python,
    args: [
      "-m",
      "enterprise_sag.panel",
      "--host",
      endpoint.host === "localhost" ? "127.0.0.1" : endpoint.host,
      "--port",
      String(endpoint.port),
    ],
    cwd: projectRoot,
    source,
  };
}

function resolveSagLaunch({
  endpoint,
  env = process.env,
  isPackaged = false,
  repoRoot,
  resourcesPath,
} = {}) {
  const info = endpointInfo(
    endpoint || env.OPENTOPIA_SAG_URL || DEFAULT_SAG_URL,
  );
  if (info.error) return info;
  if (!info.local) {
    return {
      ...info,
      external: true,
      error: "远程 SAG 地址由外部环境管理，OpenTopia 不会在本机启动它。",
    };
  }

  const configuredExecutable = String(
    env.OPENTOPIA_SAG_EXECUTABLE || "",
  ).trim();
  if (configuredExecutable) {
    const command = path.resolve(configuredExecutable);
    if (!fs.existsSync(command)) {
      return {
        ...info,
        error: `OPENTOPIA_SAG_EXECUTABLE 指向的文件不存在：${command}`,
      };
    }
    return {
      ...info,
      command,
      args: [
        "--host",
        info.host === "localhost" ? "127.0.0.1" : info.host,
        "--port",
        String(info.port),
      ],
      cwd: path.dirname(command),
      source: "configured-executable",
    };
  }

  const configuredRoot = String(env.OPENTOPIA_SAG_PROJECT_ROOT || "").trim();
  if (configuredRoot) {
    const projectRoot = path.resolve(configuredRoot);
    if (!isSagProject(projectRoot)) {
      return {
        ...info,
        error: `OPENTOPIA_SAG_PROJECT_ROOT 不是有效的 SAG 项目：${projectRoot}`,
      };
    }
    return {
      ...info,
      ...projectLaunch(projectRoot, "configured-project", info),
    };
  }

  const packagedExecutable = path.join(
    resourcesPath || "",
    "sag",
    process.platform === "win32"
      ? "enterprise-sag-panel.exe"
      : "enterprise-sag-panel",
  );
  if (isPackaged && fs.existsSync(packagedExecutable)) {
    return {
      ...info,
      command: packagedExecutable,
      args: [
        "--host",
        info.host === "localhost" ? "127.0.0.1" : info.host,
        "--port",
        String(info.port),
      ],
      cwd: path.dirname(packagedExecutable),
      source: "packaged-sidecar",
    };
  }

  if (!isPackaged && repoRoot) {
    const projectRoot = discoverSagProject(
      path.dirname(path.resolve(repoRoot)),
    );
    if (projectRoot) {
      return {
        ...info,
        ...projectLaunch(projectRoot, "development-project", info),
      };
    }
  }

  return {
    ...info,
    error:
      "未找到可启动的 SAG 运行时。请配置 OPENTOPIA_SAG_PROJECT_ROOT 或 OPENTOPIA_SAG_EXECUTABLE。",
  };
}

function delay(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function createSagServiceManager({
  endpoint = DEFAULT_SAG_URL,
  env = process.env,
  isPackaged = false,
  repoRoot,
  resourcesPath,
  logger = () => {},
  spawn = defaultSpawn,
  healthAttempts = 90,
  healthIntervalMs = 500,
  fetchImpl = globalThis.fetch,
} = {}) {
  const launch = resolveSagLaunch({
    endpoint,
    env,
    isPackaged,
    repoRoot,
    resourcesPath,
  });
  let child = null;
  let startup = null;
  let lastExit = null;

  function publicStatus(state, options = {}) {
    return {
      state,
      endpoint: launch.endpoint || String(endpoint),
      managed: Boolean(child),
      canStart: Boolean(launch.command),
      source: options.source || launch.source || null,
      message: options.message,
    };
  }

  async function isHealthy() {
    if (launch.error && !launch.endpoint) return false;
    try {
      const response = await fetchImpl(`${launch.endpoint}/api/status`, {
        signal: AbortSignal.timeout(1200),
      });
      if (!response.ok) return false;
      const payload = await response.json();
      return (
        (payload?.status === "ready" || payload?.status === "ok") &&
        payload?.prompt_injection === false &&
        payload?.agent_loop_integration === false
      );
    } catch {
      return false;
    }
  }

  async function waitUntilHealthy() {
    for (let attempt = 0; attempt < healthAttempts; attempt += 1) {
      if (await isHealthy()) return true;
      if (!child) return false;
      await delay(healthIntervalMs);
    }
    return false;
  }

  async function start() {
    if (await isHealthy()) {
      return publicStatus("ready", {
        source: child ? launch.source : "existing-service",
        message: child
          ? "SAG 本地服务已就绪。"
          : "已连接到正在运行的 SAG 服务。",
      });
    }
    if (!launch.command) {
      return publicStatus("unavailable", { message: launch.error });
    }
    if (child) {
      const ready = await waitUntilHealthy();
      return publicStatus(ready ? "ready" : "unavailable", {
        message: ready
          ? "SAG 本地服务已就绪。"
          : lastExit || "SAG 服务尚未就绪，请重试。",
      });
    }

    logger("info", "sag.spawn.starting", {
      endpoint: launch.endpoint,
      source: launch.source,
      cwd: launch.cwd,
    });
    try {
      const spawned = spawn(launch.command, launch.args, {
        cwd: launch.cwd,
        env: sagChildEnv(env),
        stdio: ["ignore", "pipe", "pipe"],
        windowsHide: true,
      });
      child = spawned;
      lastExit = null;
      spawned.stdout?.on("data", (chunk) =>
        logger("info", "sag.stdout", { chunk: chunk.toString() }),
      );
      spawned.stderr?.on("data", (chunk) =>
        logger("info", "sag.stderr", { chunk: chunk.toString() }),
      );
      spawned.once("error", (error) => {
        lastExit = `SAG 进程启动失败：${error.message}`;
        if (child === spawned) child = null;
        logger("error", "sag.spawn.failed", { error });
      });
      spawned.once("exit", (code) => {
        lastExit = `SAG 进程已退出（代码 ${code ?? "未知"}）。`;
        if (child === spawned) child = null;
        logger(code === 0 ? "info" : "error", "sag.spawn.exited", { code });
      });
    } catch (error) {
      child = null;
      return publicStatus("unavailable", {
        message: `SAG 进程启动失败：${error.message}`,
      });
    }

    const ready = await waitUntilHealthy();
    if (ready) {
      logger("info", "sag.spawn.ready", {
        endpoint: launch.endpoint,
        source: launch.source,
      });
      return publicStatus("ready", { message: "SAG 本地服务已自动启动。" });
    }
    return publicStatus("unavailable", {
      message: lastExit || "SAG 服务启动超时，请查看 OpenTopia 日志。",
    });
  }

  function ensureReady() {
    if (startup) return startup;
    startup = start().finally(() => {
      startup = null;
    });
    return startup;
  }

  function stopSync() {
    const running = child;
    child = null;
    if (!running?.pid) return;
    if (process.platform === "win32") {
      spawnSync("taskkill", ["/pid", String(running.pid), "/t", "/f"], {
        windowsHide: true,
        stdio: "ignore",
      });
    } else {
      running.kill("SIGTERM");
    }
  }

  return { ensureReady, isHealthy, stopSync };
}

module.exports = {
  DEFAULT_SAG_URL,
  createSagServiceManager,
  discoverSagProject,
  endpointInfo,
  resolveSagLaunch,
  sagChildEnv,
};
