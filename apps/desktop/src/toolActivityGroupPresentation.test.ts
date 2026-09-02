import assert from "node:assert/strict";
import test from "node:test";

import type * as ToolGroupPresentationModule from "./components/turnActivityTimeline/toolGroupPresentation";

const { buildToolGroupPresentation } = (await import(
  "./components/turnActivityTimeline/toolGroupPresentation" + ".ts"
)) as typeof ToolGroupPresentationModule;

const finished = (callId: string) => ({
  callId,
  output: "ok",
  metadata: { success: true },
});

test("shows the latest running tool instead of a premature batch count", () => {
  const presentation = buildToolGroupPresentation("mcp", [
    {
      call: { id: "first", name: "github__list_issues", input: {} },
      result: finished("first"),
    },
    {
      call: { id: "second", name: "github__get_issue", input: {} },
    },
  ]);

  assert.deepEqual(presentation, {
    iconKind: "mcp",
    label: "github · get_issue",
    running: true,
  });
});

test("falls back to the newest unfinished call when parallel tools settle out of order", () => {
  const presentation = buildToolGroupPresentation("shell", [
    {
      call: { id: "first", name: "shell", input: { command: "pnpm test" } },
    },
    {
      call: {
        id: "second",
        name: "shell",
        input: { command: "pnpm typecheck" },
      },
      result: finished("second"),
    },
  ]);

  assert.equal(presentation.label, "pnpm test");
  assert.equal(presentation.running, true);
});

test("shows the batch count only after every tool has completed", () => {
  const presentation = buildToolGroupPresentation("mcp", [
    {
      call: { id: "first", name: "github__list_issues", input: {} },
      result: finished("first"),
    },
    {
      call: { id: "second", name: "github__get_issue", input: {} },
      result: finished("second"),
    },
  ]);

  assert.deepEqual(presentation, {
    iconKind: "mcp",
    label: "调用了 2 个 MCP 工具",
    running: false,
  });
});

test("keeps the last fast tool visible briefly before replacing it with the count", () => {
  const now = Date.parse("2026-08-20T10:00:01.200Z");
  const executions = [
    {
      call: { id: "first", name: "github__list_issues", input: {} },
      result: finished("first"),
      finishedAt: "2026-08-20T10:00:01.000Z",
    },
    {
      call: { id: "second", name: "github__get_issue", input: {} },
      result: finished("second"),
      finishedAt: "2026-08-20T10:00:01.100Z",
    },
  ];

  const settling = buildToolGroupPresentation("mcp", executions, now);
  assert.equal(settling.label, "github · get_issue");
  assert.equal(settling.running, false);
  assert.equal(settling.settleUntil, now + 400);

  const complete = buildToolGroupPresentation(
    "mcp",
    executions,
    settling.settleUntil,
  );
  assert.equal(complete.label, "调用了 2 个 MCP 工具");
  assert.equal(complete.settleUntil, undefined);
});

test("keeps a completed single call named while an active turn may add more tools", () => {
  const presentation = buildToolGroupPresentation("mcp", [
    {
      call: { id: "first", name: "github__list_issues", input: {} },
      result: finished("first"),
    },
  ]);

  assert.equal(presentation.label, "github · list_issues");
  assert.equal(presentation.running, false);
});
