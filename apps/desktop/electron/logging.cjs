const fs = require("node:fs");
const path = require("node:path");
const { URL } = require("node:url");

function createAppLogger({ app, apiToken, getBackendUrl, isDev }) {
  let initialized = false;
  let logFilePath = null;
  let crashLogFilePath = null;
  let logsDirPath = null;
  let crashLogsDirPath = null;
  let logFlushHandle = null;
  let logWriteInFlight = false;
  const pendingLogLines = [];

  function isSecretName(name) {
    return /api[_-]?key|token|secret|password|authorization|credential/i.test(
      String(name || ""),
    );
  }

  function redactSecrets(value) {
    let output = String(value).split(apiToken).join("[redacted:api-token]");
    for (const [key, secretValue] of Object.entries(process.env)) {
      if (!isSecretName(key) || !secretValue || secretValue.length < 4)
        continue;
      output = output.split(secretValue).join(`[redacted:${key}]`);
    }

    return output
      .replace(/(Bearer\s+)[^\s"'`]+/gi, "$1[redacted]")
      .replace(
        /([?&][^=&\s]*(?:api[_-]?key|token|secret|password)[^=&\s]*=)[^&\s]+/gi,
        "$1[redacted]",
      )
      .replace(
        /((?:api[_-]?key|token|secret|password|authorization)\s*[:=]\s*)("[^"]*"|'[^']*'|[^\s,;]+)/gi,
        "$1[redacted]",
      )
      .replace(/\bsk-[A-Za-z0-9_-]{8,}\b/g, "[redacted-api-key]");
  }

  function serializeError(error) {
    if (!error) return null;
    return {
      name: error.name || "Error",
      message: redactSecrets(error.message || String(error)),
      stack: error.stack ? redactSecrets(error.stack) : undefined,
      code: error.code,
    };
  }

  function sanitizeForLog(value, key = "", depth = 0) {
    if (isSecretName(key)) return "[redacted]";
    if (value instanceof Error) return serializeError(value);
    if (typeof value === "string") return redactSecrets(value);
    if (
      value === null ||
      value === undefined ||
      typeof value === "number" ||
      typeof value === "boolean"
    ) {
      return value;
    }
    if (depth > 6) return "[max-depth]";
    if (Array.isArray(value)) {
      return value.map((entry) => sanitizeForLog(entry, key, depth + 1));
    }
    if (typeof value === "object") {
      const sanitized = {};
      for (const [entryKey, entryValue] of Object.entries(value)) {
        sanitized[entryKey] = sanitizeForLog(entryValue, entryKey, depth + 1);
      }
      return sanitized;
    }
    return redactSecrets(String(value));
  }

  function backendEndpointInfo() {
    const backendUrl = getBackendUrl();
    try {
      const parsed = new URL(backendUrl);
      return {
        url: parsed.toString(),
        protocol: parsed.protocol,
        host: parsed.hostname,
        port:
          parsed.port ||
          (parsed.protocol === "https:"
            ? "443"
            : parsed.protocol === "http:"
              ? "80"
              : ""),
      };
    } catch {
      return { url: redactSecrets(backendUrl) };
    }
  }

  function formatLogLine(level, event, metadata) {
    const record = {
      ts: new Date().toISOString(),
      level,
      event,
      metadata: sanitizeForLog(metadata || {}),
    };
    return `${JSON.stringify(record)}\n`;
  }

  function appendLogLineSync(targetPath, level, event, metadata) {
    if (!targetPath) return;
    fs.appendFileSync(
      targetPath,
      formatLogLine(level, event, metadata),
      "utf8",
    );
  }

  function scheduleLogFlush() {
    if (
      logFlushHandle !== null ||
      logWriteInFlight ||
      pendingLogLines.length === 0
    ) {
      return;
    }
    logFlushHandle = setImmediate(() => {
      logFlushHandle = null;
      flushQueuedLogLines();
    });
  }

  function flushQueuedLogLines() {
    if (logWriteInFlight || pendingLogLines.length === 0 || !logFilePath) {
      return;
    }
    const batch = pendingLogLines.splice(0).join("");
    logWriteInFlight = true;
    fs.appendFile(logFilePath, batch, "utf8", (error) => {
      logWriteInFlight = false;
      if (error) {
        console.error(
          "[opentopia] failed to write buffered log",
          serializeError(error),
        );
      }
      scheduleLogFlush();
    });
  }

  function flushLogsSync() {
    if (logFlushHandle !== null) {
      clearImmediate(logFlushHandle);
      logFlushHandle = null;
    }
    if (!logFilePath || pendingLogLines.length === 0) return;
    const batch = pendingLogLines.splice(0).join("");
    fs.appendFileSync(logFilePath, batch, "utf8");
  }

  function writeLog(level, event, metadata = {}) {
    try {
      if (!logFilePath) return;
      pendingLogLines.push(formatLogLine(level, event, metadata));
      scheduleLogFlush();
    } catch (error) {
      console.error("[opentopia] failed to write log", serializeError(error));
    }
  }

  function writeCrashLog(level, event, metadata = {}) {
    writeLog(level, event, metadata);
    try {
      appendLogLineSync(crashLogFilePath, level, event, metadata);
    } catch (error) {
      console.error(
        "[opentopia] failed to write crash log",
        serializeError(error),
      );
    }
  }

  function logConsole(level, message, metadata = {}) {
    writeLog(level, message, metadata);
    const line = `[opentopia] ${message}`;
    const sanitized = sanitizeForLog(metadata);
    if (level === "error") {
      console.error(line, sanitized);
    } else if (level === "warn") {
      console.warn(line, sanitized);
    } else {
      console.log(line, sanitized);
    }
  }

  function ensureLoggingInitialized() {
    if (initialized) return;
    initialized = true;

    logsDirPath = path.join(app.getPath("userData"), "logs");
    crashLogsDirPath = path.join(logsDirPath, "crashes");
    fs.mkdirSync(crashLogsDirPath, { recursive: true });

    const timestamp = new Date().toISOString().replace(/[:.]/g, "-");
    logFilePath = path.join(
      logsDirPath,
      `startup-${timestamp}-${process.pid}.jsonl`,
    );
    crashLogFilePath = path.join(
      crashLogsDirPath,
      `crash-${timestamp}-${process.pid}.jsonl`,
    );

    writeLog("info", "app.logging.ready", {
      pid: process.pid,
      isDev,
      userData: app.getPath("userData"),
      logsDir: logsDirPath,
      crashLogsDir: crashLogsDirPath,
      backend: backendEndpointInfo(),
    });

    process.on("uncaughtExceptionMonitor", (error) => {
      writeCrashLog("error", "process.uncaughtException", { error });
    });
    process.on("unhandledRejection", (reason) => {
      writeCrashLog("error", "process.unhandledRejection", {
        reason: reason instanceof Error ? serializeError(reason) : reason,
      });
    });
    app.on("render-process-gone", (_event, webContents, details) => {
      writeCrashLog("error", "crash.render-process-gone", {
        url: webContents?.getURL?.(),
        details,
      });
    });
    app.on("child-process-gone", (_event, details) => {
      writeCrashLog("error", "crash.child-process-gone", { details });
    });
  }

  function getLogPaths() {
    return { logsDirPath, crashLogsDirPath };
  }

  return {
    backendEndpointInfo,
    ensureLoggingInitialized,
    flushLogsSync,
    getLogPaths,
    isSecretName,
    logConsole,
    redactSecrets,
    sanitizeForLog,
    serializeError,
    writeCrashLog,
    writeLog,
  };
}

module.exports = { createAppLogger };
