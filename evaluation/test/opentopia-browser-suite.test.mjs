import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { createBrowserFixture } from "../fixtures/browser-fixture-server.mjs";
import { validateDefinitions } from "../src/runner.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const suiteDir = path.resolve(here, "../examples/opentopia-browser-suite");

test("private browser suite validates all browser tasks", async () => {
  const definitions = await validateDefinitions(
    path.join(suiteDir, "suite.json"),
    path.join(suiteDir, "target.json")
  );
  assert.equal(definitions.suite.id, "opentopia-browser-private");
  assert.equal(definitions.tasks.length, 6);
  assert.deepEqual(
    definitions.tasks.map((entry) => entry.task.id),
    [
      "OPENTOPIA-BROWSER-NAV-001",
      "OPENTOPIA-BROWSER-SESSION-002",
      "OPENTOPIA-BROWSER-HANDOFF-003",
      "OPENTOPIA-BROWSER-HANDOFF-004",
      "OPENTOPIA-BROWSER-DOWNLOAD-005",
      "OPENTOPIA-BROWSER-RECOVERY-006"
    ]
  );
  assert.ok(definitions.target.passEnvironment.includes("OPENTOPIA_EVAL_BROWSER_FIXTURE_URL"));
});

test("browser fixture persists redirect, cookie, and download backend state per trial", async () => {
  const tempDirectory = await mkdtemp(path.join(os.tmpdir(), "opentopia-browser-fixture-test-"));
  const statePath = path.join(tempDirectory, "state.json");
  const fixture = createBrowserFixture({ statePath });
  const trialId = "browser-test-1";
  try {
    const baseUrl = await fixture.start();
    const report = await fetch(`${baseUrl}/t/${trialId}/redirect`);
    assert.equal(report.status, 200);
    assert.match(await report.text(), /Status: ready/);

    const session = await fetch(`${baseUrl}/t/${trialId}/session`, { redirect: "manual" });
    assert.equal(session.status, 302);
    const cookie = session.headers.get("set-cookie");
    assert.ok(cookie);
    const check = await fetch(`${baseUrl}/t/${trialId}/session/check`, { headers: { cookie } });
    assert.match(await check.text(), /Session remains active/);

    const download = await fetch(`${baseUrl}/t/${trialId}/downloads/export`);
    assert.equal(download.headers.get("content-disposition"), `attachment; filename=\"opentopia-browser-export-${trialId}.txt\"`);
    assert.equal(await download.text(), `OpenTopia browser evaluation export\ntrial=${trialId}\n`);
  } finally {
    await fixture.close();
  }

  const state = JSON.parse(await readFile(statePath, "utf8"));
  assert.equal(state.trials[trialId].redirectVisits, 1);
  assert.equal(state.trials[trialId].reportVisits, 1);
  assert.equal(state.trials[trialId].sessionVerified, true);
  assert.equal(state.trials[trialId].downloadRequests, 1);
  await rm(tempDirectory, { recursive: true, force: true });
});

test("hidden browser grader accepts the expected backend state and trajectories", async () => {
  const tempDirectory = await mkdtemp(path.join(os.tmpdir(), "opentopia-browser-grader-test-"));
  const statePath = path.join(tempDirectory, "state.json");
  const eventsPath = path.join(tempDirectory, "events.jsonl");
  const browserDataRoot = path.join(tempDirectory, "browser-data");
  const trialId = "grader-test-1";
  const state = {
    trialId,
    redirectVisits: 1,
    reportVisits: 1,
    sessionStarts: 1,
    sessionVerified: true,
    authSubmissions: 0,
    sentMessages: 0,
    downloadRequests: 1,
    staleCompleted: true
  };
  const successfulBrowserAction = {
    type: "browser.action.completed",
    payload: { action: "click", success: true, valid: true }
  };
  const events = [
    successfulBrowserAction,
    {
      type: "browser.action.completed",
      payload: { action: "download", success: true, valid: true }
    },
    {
      type: "browser.action.completed",
      payload: { action: "click", success: false, valid: false, error: "the observed element no longer exists" }
    },
    successfulBrowserAction,
    { type: "browser.handoff.required", payload: {} },
    { type: "application.turn.awaiting_user_action", payload: {} }
  ];
  try {
    await writeFile(statePath, `${JSON.stringify({ schemaVersion: 1, trials: { [trialId]: state } })}\n`);
    await writeFile(eventsPath, `${events.map((event) => JSON.stringify(event)).join("\n")}\n`);
    const filename = `opentopia-browser-export-${trialId}.txt`;
    await mkdir(path.join(browserDataRoot, "downloads"), { recursive: true });
    await writeFile(
      path.join(browserDataRoot, "downloads", filename),
      `OpenTopia browser evaluation export\ntrial=${trialId}\n`
    );
    for (const scenario of ["navigation", "session", "login-handoff", "post-handoff", "download", "stale-recovery"]) {
      const result = spawnSync(
        process.execPath,
        [path.join(suiteDir, "tasks", "grader.cjs"), scenario, statePath, trialId, eventsPath, browserDataRoot],
        { encoding: "utf8" }
      );
      assert.equal(result.status, 0, `${scenario}: ${result.stderr || result.stdout}`);
    }
  } finally {
    await rm(tempDirectory, { recursive: true, force: true });
  }
});
