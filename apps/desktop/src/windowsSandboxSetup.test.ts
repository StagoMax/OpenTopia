import assert from "node:assert/strict";
import test from "node:test";

import { shouldPromptForWindowsSandboxSetup } from "./windowsSandboxSetup.ts";
import type { WindowsSandboxSetupStatus } from "./types.ts";

function status(
  state: WindowsSandboxSetupStatus["state"],
): WindowsSandboxSetupStatus {
  return {
    supported: state !== "unavailable",
    helperAvailable: state !== "unavailable",
    state,
    backend: "dedicated_user",
    components: {
      credentials: state === "ready",
      offlineIdentity: state === "ready",
      onlineIdentity: state === "ready",
      offlineNetworkPolicy: state === "ready",
    },
    issues: [],
  };
}

test("prompts on every Windows launch when the sandbox is not configured", () => {
  assert.equal(
    shouldPromptForWindowsSandboxSetup({
      isWindows: true,
      status: status("not_configured"),
      dismissedForLaunch: false,
    }),
    true,
  );
});

test("prompts for degraded and unavailable installations", () => {
  for (const state of ["degraded", "unavailable"] as const) {
    assert.equal(
      shouldPromptForWindowsSandboxSetup({
        isWindows: true,
        status: status(state),
        dismissedForLaunch: false,
      }),
      true,
    );
  }
});

test("does not prompt when ready, on another platform, or after later is chosen", () => {
  assert.equal(
    shouldPromptForWindowsSandboxSetup({
      isWindows: true,
      status: status("ready"),
      dismissedForLaunch: false,
    }),
    false,
  );
  assert.equal(
    shouldPromptForWindowsSandboxSetup({
      isWindows: false,
      status: status("not_configured"),
      dismissedForLaunch: false,
    }),
    false,
  );
  assert.equal(
    shouldPromptForWindowsSandboxSetup({
      isWindows: true,
      status: status("not_configured"),
      dismissedForLaunch: true,
    }),
    false,
  );
});
