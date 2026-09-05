const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");

const SESSION_FILE_NAME = "external-api-session.json";
const LOOPBACK_HOSTS = new Set(["127.0.0.1", "::1", "[::1]", "localhost"]);

function publishExternalApiSession({
  apiToken,
  backendUrl,
  directory,
  now = () => new Date(),
  pid = process.pid,
  sessionId = crypto.randomUUID(),
}) {
  const normalizedDirectory = path.resolve(String(directory || ""));
  const normalizedBackendUrl = normalizeLoopbackBackendUrl(backendUrl);
  const normalizedToken = String(apiToken || "").trim();
  if (normalizedToken.length < 32) {
    throw new Error("OpenTopia external API session token is too short");
  }
  if (!Number.isInteger(pid) || pid <= 0) {
    throw new Error("OpenTopia external API session pid is invalid");
  }

  fs.mkdirSync(normalizedDirectory, { recursive: true });
  const filePath = path.join(normalizedDirectory, SESSION_FILE_NAME);
  const temporaryPath = `${filePath}.${pid}.${sessionId}.tmp`;
  const document = {
    schemaVersion: 1,
    sessionId,
    pid,
    backendUrl: normalizedBackendUrl,
    apiToken: normalizedToken,
    createdAt: now().toISOString(),
  };

  try {
    fs.writeFileSync(temporaryPath, JSON.stringify(document), {
      encoding: "utf8",
      flag: "wx",
      mode: 0o600,
    });
    fs.chmodSync(temporaryPath, 0o600);
    fs.rmSync(filePath, { force: true });
    fs.renameSync(temporaryPath, filePath);
  } catch (error) {
    fs.rmSync(temporaryPath, { force: true });
    throw error;
  }

  let disposed = false;
  return {
    filePath,
    sessionId,
    dispose() {
      if (disposed) return;
      disposed = true;
      removeOwnedSessionFile(filePath, sessionId);
    },
  };
}

function normalizeLoopbackBackendUrl(value) {
  const url = new URL(String(value || ""));
  if (
    url.protocol !== "http:" ||
    !LOOPBACK_HOSTS.has(url.hostname) ||
    url.username ||
    url.password
  ) {
    throw new Error(
      "OpenTopia external API session requires an unauthenticated HTTP loopback URL",
    );
  }
  return url.toString().replace(/\/$/, "");
}

function removeOwnedSessionFile(filePath, sessionId) {
  let current;
  try {
    current = JSON.parse(fs.readFileSync(filePath, "utf8"));
  } catch (error) {
    if (error?.code !== "ENOENT") return;
    return;
  }
  if (current?.sessionId === sessionId) {
    fs.rmSync(filePath, { force: true });
  }
}

module.exports = {
  SESSION_FILE_NAME,
  normalizeLoopbackBackendUrl,
  publishExternalApiSession,
};
