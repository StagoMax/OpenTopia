import type { WindowsSandboxSetupStatus } from "./types";

/**
 * The setup reminder reflects machine readiness, not the task policy selected
 * in Settings. This keeps the app ready for a later switch to enforce mode and
 * makes the check deterministic on every Windows app launch.
 */
export function shouldPromptForWindowsSandboxSetup(input: {
  isWindows: boolean;
  status: WindowsSandboxSetupStatus | null;
  dismissedForLaunch: boolean;
}): boolean {
  return (
    input.isWindows &&
    input.status !== null &&
    input.status.state !== "ready" &&
    !input.dismissedForLaunch
  );
}
