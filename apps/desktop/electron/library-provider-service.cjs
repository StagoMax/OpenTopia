const fs = require("node:fs");
const path = require("node:path");
const { spawn: defaultSpawn, spawnSync } = require("node:child_process");

const LOOPBACK_HOSTS = new Set(["127.0.0.1", "::1", "localhost"]);

function endpointInfo(rawEndpoint, spec) {
  let endpoint;
  try {
    endpoint = new URL(String(rawEndpoint || spec.defaultUrl).trim());
  } catch {
    return {
      error: `${spec.urlEnv} 不是有效地址。`,
      endpoint: String(rawEndpoint || spec.defaultUrl),
    };
  }

  const host = endpoint.hostname.replace(/^\[|\]$/g, "");
  const port = Number(
    endpoint.port || (endpoint.protocol === "https:" ? "443" : "80"),
  );
  if (
    !["http:", "https:"].includes(endpoint.protocol) ||
    !Number.isInteger(port) ||
    port < 1 ||
    port > 65535
  ) {
    return {
      error: `${spec.urlEnv} 必须是有效的 HTTP 或 HTTPS 地址。`,
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

function providerChildEnv(env, spec) {
  const selected = {};
  const names = new Set([
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
    ...(spec.childEnvNames || []),
  ]);
  const prefixes = [
    "CUDA_",
    "DEEPSEEK_",
    "NVIDIA_",
    "PYTHON",
    ...(spec.childEnvPrefixes || []),
  ];
  for (const [name, value] of Object.entries(env || {})) {
    const normalized = name.toUpperCase();
    if (
      names.has(normalized) ||
      prefixes.some((prefix) => normalized.startsWith(prefix))
    ) {
      selected[name] = value;
    }
  }
  selected.PYTHONUTF8 ||= "1";
  return selected;
}

function isProviderProject(projectRoot, spec) {
  if (!projectRoot) return false;
  try {
    const pyproject = fs.readFileSync(
      path.join(projectRoot, "pyproject.toml"),
      "utf8",
    );
    return spec.entrypointPattern.test(pyproject);
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

function discoverProviderProject(searchRoot, spec) {
  if (!searchRoot || !fs.existsSync(searchRoot)) return null;
  if (isProviderProject(searchRoot, spec) && projectPython(searchRoot)) {
    return searchRoot;
  }
  let entries;
  try {
    entries = fs.readdirSync(searchRoot, { withFileTypes: true });
  } catch {
    return null;
  }
  for (const entry of entries) {
    if (!entry.isDirectory() || entry.name.startsWith(".")) continue;
    const candidate = path.join(searchRoot, entry.name);
    if (isProviderProject(candidate, spec) && projectPython(candidate)) {
      return candidate;
    }
  }
  return null;
}

function projectLaunch(projectRoot, source, endpoint, spec) {
  const python = projectPython(projectRoot);
  if (!python) {
    return {
      error: `${spec.label} 项目缺少可用的 .venv Python：${projectRoot}`,
      source,
    };
  }
  return {
    command: python,
    args: [
      "-m",
      spec.module,
      "--host",
      endpoint.host === "localhost" ? "127.0.0.1" : endpoint.host,
      "--port",
      String(endpoint.port),
    ],
    cwd: projectRoot,
    source,
  };
}

function resolveProviderLaunch({
  spec,
  endpoint,
  env = process.env,
  isPackaged = false,
  repoRoot,
  resourcesPath,
} = {}) {
  const info = endpointInfo(
    endpoint || env[spec.urlEnv] || spec.defaultUrl,
    spec,
  );
  if (info.error) return info;
  if (!info.local) {
    return {
      ...info,
      external: true,
      error: `远程 ${spec.label} 地址由外部环境管理，OpenTopia 不会在本机启动它。`,
    };
  }

  const configuredExecutable = String(env[spec.executableEnv] || "").trim();
  if (configuredExecutable) {
    const command = path.resolve(configuredExecutable);
    if (!fs.existsSync(command)) {
      return {
        ...info,
        error: `${spec.executableEnv} 指向的文件不存在：${command}`,
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

  const configuredRoot = String(env[spec.projectRootEnv] || "").trim();
  if (configuredRoot) {
    const projectRoot = path.resolve(configuredRoot);
    if (!isProviderProject(projectRoot, spec)) {
      return {
        ...info,
        error: `${spec.projectRootEnv} 不是有效的 ${spec.label} 项目：${projectRoot}`,
      };
    }
    return {
      ...info,
      ...projectLaunch(projectRoot, "configured-project", info, spec),
    };
  }

  const packagedExecutable = path.join(
    resourcesPath || "",
    spec.packagedDirectory,
    process.platform === "win32"
      ? `${spec.packagedExecutable}.exe`
      : spec.packagedExecutable,
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
    const projectRoot = discoverProviderProject(
      path.dirname(path.resolve(repoRoot)),
      spec,
    );
    if (projectRoot) {
      return {
        ...info,
        ...projectLaunch(projectRoot, "development-project", info, spec),
      };
    }
  }

  return {
    ...info,
    error: `未找到可启动的 ${spec.label} 运行时。请配置 ${spec.projectRootEnv} 或 ${spec.executableEnv}。`,
  };
}

function delay(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function createLibraryProviderServiceManager({
  spec,
  endpoint = spec.defaultUrl,
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
  const launch = resolveProviderLaunch({
    spec,
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
      provider: spec.id,
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
      const response = await fetchImpl(
        `${launch.endpoint}/${spec.healthPath.replace(/^\//, "")}`,
        { signal: AbortSignal.timeout(1200) },
      );
      if (!response.ok) return false;
      return spec.validateHealth(await response.json());
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
          ? `${spec.label} 本地服务已就绪。`
          : `已连接到正在运行的 ${spec.label} 服务。`,
      });
    }
    if (!launch.command) {
      return publicStatus("unavailable", { message: launch.error });
    }
    if (child) {
      const ready = await waitUntilHealthy();
      return publicStatus(ready ? "ready" : "unavailable", {
        message: ready
          ? `${spec.label} 本地服务已就绪。`
          : lastExit || `${spec.label} 服务尚未就绪，请重试。`,
      });
    }

    logger("info", `library.${spec.id}.spawn.starting`, {
      endpoint: launch.endpoint,
      source: launch.source,
      cwd: launch.cwd,
    });
    try {
      const spawned = spawn(launch.command, launch.args, {
        cwd: launch.cwd,
        env: providerChildEnv(env, spec),
        stdio: ["ignore", "pipe", "pipe"],
        windowsHide: true,
      });
      child = spawned;
      lastExit = null;
      spawned.stdout?.on("data", (chunk) =>
        logger("info", `library.${spec.id}.stdout`, {
          chunk: chunk.toString(),
        }),
      );
      spawned.stderr?.on("data", (chunk) =>
        logger("info", `library.${spec.id}.stderr`, {
          chunk: chunk.toString(),
        }),
      );
      spawned.once("error", (error) => {
        lastExit = `${spec.label} 进程启动失败：${error.message}`;
        if (child === spawned) child = null;
        logger("error", `library.${spec.id}.spawn.failed`, { error });
      });
      spawned.once("exit", (code) => {
        lastExit = `${spec.label} 进程已退出（代码 ${code ?? "未知"}）。`;
        if (child === spawned) child = null;
        logger(
          code === 0 ? "info" : "error",
          `library.${spec.id}.spawn.exited`,
          {
            code,
          },
        );
      });
    } catch (error) {
      child = null;
      return publicStatus("unavailable", {
        message: `${spec.label} 进程启动失败：${error.message}`,
      });
    }

    const ready = await waitUntilHealthy();
    if (ready) {
      logger("info", `library.${spec.id}.spawn.ready`, {
        endpoint: launch.endpoint,
        source: launch.source,
      });
      return publicStatus("ready", {
        message: `${spec.label} 本地服务已自动启动。`,
      });
    }
    return publicStatus("unavailable", {
      message:
        lastExit || `${spec.label} 服务启动超时，请查看 OpenTopia 日志。`,
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
  createLibraryProviderServiceManager,
  discoverProviderProject,
  endpointInfo,
  isProviderProject,
  providerChildEnv,
  resolveProviderLaunch,
};
