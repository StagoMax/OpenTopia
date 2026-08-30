import assert from "node:assert/strict";
import test from "node:test";

import {
  classifyFailure,
  parseCsv,
  toolResultIsError,
} from "./tool-failure-attribution.mjs";

function failure(overrides = {}) {
  return {
    tool: "shell",
    code: "tool_execution_failed",
    message: "failure",
    input: {},
    ...overrides,
  };
}

test("parses quoted CSV fields without splitting embedded commas", () => {
  assert.deepEqual(parseCsv('snapshot,task,source_path\nafter,"a,b","J:\\x\\r.json"\n'), [
    { snapshot: "after", task: "a,b", source_path: "J:\\x\\r.json" },
  ]);
});

test("uses the desktop error-envelope semantics", () => {
  assert.equal(toolResultIsError({ metadata: { success: false } }), true);
  assert.equal(toolResultIsError({ metadata: { errorRecord: {} } }), true);
  assert.equal(toolResultIsError({ metadata: { success: true } }), false);
  assert.equal(toolResultIsError({}), false);
});

test("attributes mutation journal rollback to engineering", () => {
  const result = classifyFailure(
    failure({
      tool: "apply_patch",
      message:
        "file mutation batch was rolled back: failed to persist the turn file-mutation journal: turn file mutation arrived after capture finalized",
    }),
  );
  assert.deepEqual([result.owner, result.cause], [
    "engineering",
    "file_mutation_journal",
  ]);
});

test("separates semantically valid schema supersets from missing required fields", () => {
  const superset = classifyFailure(
    failure({
      tool: "background_output",
      code: "invalid_tool_arguments",
      input: {
        action: "read",
        jobId: "7f9d8a90-c15e-4496-a337-94bc68863322",
        timeoutMs: 0,
        appendNewline: false,
        data: null,
      },
    }),
  );
  const incomplete = classifyFailure(
    failure({
      tool: "background_output",
      code: "invalid_tool_arguments",
      input: { action: "write", data: "hello" },
    }),
  );
  assert.deepEqual([superset.owner, superset.cause], [
    "engineering",
    "tool_schema_compatibility",
  ]);
  assert.deepEqual([incomplete.owner, incomplete.cause], [
    "agent",
    "invalid_tool_arguments",
  ]);
});

test("counts a missing runtime for the built-in search tool as engineering only", () => {
  const search = classifyFailure(
    failure({
      tool: "search",
      code: "execution_runtime_unavailable",
      message:
        "failed to run rg search: execution failed during ResolveRuntime: executable was not found on PATH: rg",
    }),
  );
  const shell = classifyFailure(
    failure({
      tool: "shell",
      code: "command_exit_nonzero",
      message: "sh: rg: command not found",
    }),
  );
  assert.deepEqual([search.owner, search.cause], [
    "engineering",
    "bundled_tool_runtime_resolution",
  ]);
  assert.deepEqual([shell.owner, shell.cause], [
    "task_environment",
    "task_dependency_unavailable",
  ]);
});

test("counts a missing runtime for another built-in tool as engineering", () => {
  const result = classifyFailure(
    failure({
      tool: "git_diff",
      code: "tool_execution_failed",
      message:
        "git diff execution failed: execution failed during ResolveRuntime: executable was not found on PATH: git",
    }),
  );
  assert.deepEqual([result.owner, result.cause], [
    "engineering",
    "bundled_tool_runtime_resolution",
  ]);
});

test("does not turn an ordinary missing workspace path into a normalization bug", () => {
  const result = classifyFailure(
    failure({
      tool: "filesystem",
      message: "path does not exist: /app/anon.py: No such file or directory",
    }),
  );
  assert.deepEqual([result.owner, result.cause], ["agent", "path_assumption"]);
});

test("recognizes a leaked Windows extended-length path prefix", () => {
  const result = classifyFailure(
    failure({
      tool: "filesystem",
      message: "path does not exist: \\\\?\\C:\\workspace\\src\\main.rs",
    }),
  );
  assert.deepEqual([result.owner, result.cause], [
    "engineering",
    "path_normalization",
  ]);
});

test("attributes partial multi-file reads with a missing path to the agent", () => {
  const result = classifyFailure(
    failure({
      tool: "read_files",
      message:
        '{"files":[{"ok":true,"path":"setup.py"},{"ok":false,"error":"failed to read /testbed/changelog"}]}',
    }),
  );
  assert.deepEqual([result.owner, result.cause], ["agent", "path_assumption"]);
});

test("does not infer an external network failure from a task filename", () => {
  const result = classifyFailure(
    failure({
      tool: "shell",
      message: "grep: ontology_network.owl: No such file or directory",
    }),
  );
  assert.equal(result.owner, "task_environment");
});

test("attributes Windows sandbox setup and ACL ledger errors to engineering", () => {
  for (const message of [
    "execution failed during PrepareSandbox: parse ACL ledger C:\\Users\\x\\acl-ledger.json",
    "sandbox setup failed: CheckTokenMembership returned Windows error 1309",
  ]) {
    const result = classifyFailure(failure({ message }));
    assert.deepEqual([result.owner, result.cause], ["engineering", "sandbox_acl"]);
  }
});
