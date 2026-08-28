import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const appSource = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");

test("initializes a requested URL in the native session before adding its tab", () => {
  const launchStart = appSource.indexOf("function openNewBrowserTab");
  const launchSource = appSource.slice(
    launchStart,
    appSource.indexOf("browserNewTabRequestHandlerRef.current", launchStart),
  );

  const initializeIndex = launchSource.indexOf(
    "initializeBrowserTabSession(browserHost, sessionId, initialUrl)",
  );
  const addTabIndex = launchSource.indexOf("setToolTabs");

  assert.ok(initializeIndex >= 0);
  assert.ok(addTabIndex > initializeIndex);
  assert.doesNotMatch(launchSource, /const browserNavigation = initialUrl/);
});
