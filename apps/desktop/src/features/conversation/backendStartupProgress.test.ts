import assert from "node:assert/strict";
import test from "node:test";
import type * as BackendStartupProgressModule from "./backendStartupProgress";

const backendStartupProgress: typeof BackendStartupProgressModule =
  await import("./backendStartupProgress" + ".ts");
const { backendStartupLabel, formatBackendStartupElapsed } =
  backendStartupProgress;

test("shows the current compile unit instead of inventing a percentage", () => {
  assert.equal(
    backendStartupLabel(
      {
        phase: "compiling",
        detail: "opentopia-server",
        startedAt: "2026-08-21T00:00:00.000Z",
        updatedAt: "2026-08-21T00:00:01.000Z",
      },
      true,
    ),
    "正在编译 opentopia-server",
  );
});

test("does not call a failed startup a retry while the backend owns recovery", () => {
  assert.equal(
    backendStartupLabel(
      {
        phase: "failed",
        detail: null,
        startedAt: "2026-08-21T00:00:00.000Z",
        updatedAt: "2026-08-21T00:00:01.000Z",
      },
      false,
    ),
    "本地服务启动失败",
  );
});

test("formats elapsed startup time for short and long waits", () => {
  const startedAt = "2026-08-21T00:00:00.000Z";
  assert.equal(
    formatBackendStartupElapsed(startedAt, Date.parse(startedAt) + 42_000),
    "已等待 42 秒",
  );
  assert.equal(
    formatBackendStartupElapsed(startedAt, Date.parse(startedAt) + 125_000),
    "已等待 2 分 5 秒",
  );
});
