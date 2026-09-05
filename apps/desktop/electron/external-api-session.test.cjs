const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const {
  SESSION_FILE_NAME,
  normalizeLoopbackBackendUrl,
  publishExternalApiSession,
} = require("./external-api-session.cjs");

test("publishes and removes the current desktop API session", () => {
  const directory = fs.mkdtempSync(
    path.join(os.tmpdir(), "opentopia-session-"),
  );
  try {
    const session = publishExternalApiSession({
      apiToken: "a".repeat(32),
      backendUrl: "http://127.0.0.1:8787/",
      directory,
      now: () => new Date("2026-09-04T00:00:00.000Z"),
      pid: 1234,
      sessionId: "session-one",
    });
    const document = JSON.parse(fs.readFileSync(session.filePath, "utf8"));

    assert.equal(session.filePath, path.join(directory, SESSION_FILE_NAME));
    assert.deepEqual(document, {
      schemaVersion: 1,
      sessionId: "session-one",
      pid: 1234,
      backendUrl: "http://127.0.0.1:8787",
      apiToken: "a".repeat(32),
      createdAt: "2026-09-04T00:00:00.000Z",
    });

    session.dispose();
    assert.equal(fs.existsSync(session.filePath), false);
  } finally {
    fs.rmSync(directory, { force: true, recursive: true });
  }
});

test("an old process cannot delete a replacement session", () => {
  const directory = fs.mkdtempSync(
    path.join(os.tmpdir(), "opentopia-session-"),
  );
  try {
    const first = publishExternalApiSession({
      apiToken: "a".repeat(32),
      backendUrl: "http://localhost:8787",
      directory,
      sessionId: "session-one",
    });
    const second = publishExternalApiSession({
      apiToken: "b".repeat(32),
      backendUrl: "http://localhost:8787",
      directory,
      sessionId: "session-two",
    });

    first.dispose();
    assert.equal(
      JSON.parse(fs.readFileSync(second.filePath, "utf8")).sessionId,
      "session-two",
    );

    second.dispose();
    assert.equal(fs.existsSync(second.filePath), false);
  } finally {
    fs.rmSync(directory, { force: true, recursive: true });
  }
});

test("only loopback HTTP backend URLs can receive the desktop token", () => {
  assert.equal(
    normalizeLoopbackBackendUrl("http://[::1]:8787/"),
    "http://[::1]:8787",
  );
  assert.throws(
    () => normalizeLoopbackBackendUrl("https://example.com"),
    /loopback URL/,
  );
  assert.throws(
    () => normalizeLoopbackBackendUrl("http://user:pass@127.0.0.1:8787"),
    /loopback URL/,
  );
});
