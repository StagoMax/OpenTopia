import assert from "node:assert/strict";
import test from "node:test";

import type * as ToolActivityModule from "./toolActivity";

const toolActivity: typeof ToolActivityModule = await import(
  "./toolActivity" + ".ts"
);

const {
  buildToolActivity,
  displayPath,
  groupSearchHits,
  parsePatchLines,
  parseSearchHits,
  parseShellCommand,
  parseShellEnvelope,
  parseShellStreams,
  patchTargets,
} = toolActivity;

const call = (name: string, input: unknown) => ({ id: "call-1", name, input });
const result = (output: string, metadata: unknown) => ({
  callId: "call-1",
  output,
  metadata,
});

test("classifies read, list and search shell commands", () => {
  assert.equal(parseShellCommand("cat design/prompt_v3.txt").kind, "read");
  assert.equal(
    parseShellCommand("cat design/prompt_v3.txt").target,
    "design/prompt_v3.txt",
  );
  assert.equal(parseShellCommand("ls -la src").kind, "list");
  assert.equal(parseShellCommand("ls -la src").target, "src");

  const search = parseShellCommand('rg -n "tool_call" apps/desktop');
  assert.equal(search.kind, "search");
  assert.equal(search.query, "tool_call");
  assert.equal(search.target, "apps/desktop");
});

test("classifies build and quality commands by program", () => {
  assert.equal(parseShellCommand("cargo test --workspace").kind, "test");
  assert.equal(parseShellCommand("cargo clippy").kind, "lint");
  assert.equal(parseShellCommand("cargo fmt -- --check").kind, "format");
  assert.equal(parseShellCommand("cargo check --workspace").kind, "build");
  assert.equal(parseShellCommand("git status --short").kind, "git");
  assert.equal(parseShellCommand("node scripts/run.mjs").kind, "other");
});

test("skips directory prefixes and pipelines when classifying", () => {
  const parsed = parseShellCommand("cd apps/desktop && head -n 40 src/App.tsx");
  assert.equal(parsed.kind, "read");
  assert.equal(parsed.target, "src/App.tsx");

  // A pipeline does something the head alone does not, so it keeps its command
  // line instead of being labelled a plain read.
  assert.equal(parseShellCommand("cat notes.md | wc -l").kind, "other");
});

test("parses the shell output envelope into streams", () => {
  const envelope = parseShellEnvelope(
    "$ cargo check\n\n[stdout]\nok\n\n[stderr]\nwarning: unused\n",
  );
  assert.equal(envelope.command, "cargo check");
  assert.equal(envelope.stdout, "ok");
  assert.equal(envelope.stderr, "warning: unused\n");
});

test("prefers structured stream metadata over the envelope", () => {
  const streams = parseShellStreams(
    "$ cargo check\n\n[stdout]\nstale\n\n[stderr]\n",
    { command: "cargo check", stdout: "fresh", stderr: "", exitCode: 0 },
  );
  assert.equal(streams.stdout, "fresh");
  assert.equal(streams.exitCode, 0);
});

test("falls back to the envelope for events without stream metadata", () => {
  const streams = parseShellStreams(
    "$ wc -l\n\n[stdout]\n\n\n[stderr]\nwc : command not found\n",
    { exitCode: 1, success: false },
  );
  assert.equal(streams.command, "wc -l");
  assert.equal(streams.stderr, "wc : command not found\n");
  assert.equal(streams.exitCode, 1);
});

test("parses ripgrep hits including windows drive paths", () => {
  const hits = parseSearchHits(
    [
      "src/app.ts:12:5:const value = 1;",
      "src/app.ts:40:1:export default app;",
      "J:/Project/OpenTopia/src/main.rs:7:3:fn main() {}",
    ].join("\n"),
  );
  assert.equal(hits.length, 3);
  assert.deepEqual(hits[0], {
    path: "src/app.ts",
    line: 12,
    text: "const value = 1;",
  });
  assert.equal(hits[2].path, "J:/Project/OpenTopia/src/main.rs");

  const groups = groupSearchHits(hits);
  assert.equal(groups.length, 2);
  assert.equal(groups[0].hits.length, 2);
});

test("hides Windows extended-length prefixes in displayed paths", () => {
  assert.equal(
    displayPath(String.raw`\\?\J:\Project\OpenTopia\inspect.mjs`),
    "J:/Project/OpenTopia/inspect.mjs",
  );
  assert.equal(
    displayPath("//?/J:/Project/OpenTopia/inspect.mjs"),
    "J:/Project/OpenTopia/inspect.mjs",
  );
  assert.equal(
    displayPath(String.raw`\\?\UNC\server\share\file.txt`),
    "//server/share/file.txt",
  );
});

test("parses git and apply_patch diffs into numbered lines", () => {
  const gitDiff = [
    "diff --git a/src/app.ts b/src/app.ts",
    "index 1111111..2222222 100644",
    "--- a/src/app.ts",
    "+++ b/src/app.ts",
    "@@ -3,3 +3,4 @@",
    " const a = 1;",
    "-const b = 2;",
    "+const b = 3;",
    "+const c = 4;",
  ].join("\n");
  const lines = parsePatchLines(gitDiff);
  assert.deepEqual(
    lines.map((line) => line.kind),
    ["file", "hunk", "context", "deleted", "added", "added"],
  );
  assert.equal(lines[0].text, "src/app.ts");
  assert.equal(lines[2].oldLine, 3);
  assert.equal(lines[3].oldLine, 4);
  assert.equal(lines[4].newLine, 4);

  const custom = parsePatchLines(
    [
      "*** Begin Patch",
      "*** Update File: src/app.ts",
      "+added",
      "*** End Patch",
    ].join("\n"),
  );
  assert.equal(custom[0].text, "修改 src/app.ts");
  assert.equal(custom[1].kind, "added");
  assert.deepEqual(patchTargets(gitDiff), ["src/app.ts"]);
});

test("builds a terminal body for shell calls", () => {
  const view = buildToolActivity(
    call("shell", { command: "cat design/prompt_v3.txt" }),
    result("$ cat design/prompt_v3.txt\n\n[stdout]\nhello\n\n[stderr]\n", {
      exitCode: 0,
      success: true,
    }),
  );
  assert.equal(view.kind, "read");
  assert.equal(view.title, "读取 design/prompt_v3.txt");
  assert.equal(view.body.type, "terminal");
  assert.equal(view.failed, false);
  if (view.body.type === "terminal") {
    assert.equal(view.body.streams.stdout, "hello");
  }
});

test("marks a non-zero exit as failed and labels it as failed", () => {
  const view = buildToolActivity(
    call("shell", { command: "wc -l" }),
    result("$ wc -l\n\n[stdout]\n\n\n[stderr]\nnot found\n", {
      exitCode: 1,
      success: false,
    }),
  );
  assert.equal(view.failed, true);
  assert.deepEqual(
    view.chips.map((chip) => chip.label),
    ["失败"],
  );
});

test("labels a successful shell call as successful", () => {
  const view = buildToolActivity(
    call("shell", { command: "echo ok" }),
    result("$ echo ok\n\n[stdout]\nok\n", {
      exitCode: 0,
      success: true,
    }),
  );
  assert.deepEqual(
    view.chips.map((chip) => chip.label),
    ["成功"],
  );
});

test("renders a pending body while a tool is still running", () => {
  const view = buildToolActivity(call("shell", { command: "cargo test" }));
  assert.equal(view.body.type, "pending");
  assert.equal(view.title, "cargo test");
});

test("uses per-tool bodies for reads, listings and searches", () => {
  const read = buildToolActivity(
    call("read_file", { path: "src/app.ts" }),
    result("const a = 1;", { path: "src/app.ts", bytes: 12 }),
  );
  assert.equal(read.body.type, "file");
  assert.deepEqual(
    read.chips.map((chip) => chip.label),
    ["12 B"],
  );

  const list = buildToolActivity(
    call("list_files", { path: "src" }),
    result("app.ts\nmain.ts", { count: 2 }),
  );
  assert.equal(list.body.type, "entries");
  assert.equal(list.title, "列出 src");

  const search = buildToolActivity(
    call("search", { query: "tool_call", path: "src" }),
    result("src/app.ts:3:1:tool_call", {
      query: "tool_call",
      returnedMatches: 1,
    }),
  );
  assert.equal(search.body.type, "matches");
  assert.equal(search.title, "搜索 tool_call");
});

test("renders read paths without the Windows device prefix", () => {
  const rawPath = String.raw`\\?\J:\Project\OpenTopia\inspect.mjs`;
  const read = buildToolActivity(
    call("read_file", { path: rawPath }),
    result("export {};", { path: rawPath, bytes: 10 }),
  );

  assert.equal(read.title, "读取 J:/Project/OpenTopia/inspect.mjs");
  assert.equal(read.body.type, "file");
  if (read.body.type === "file") {
    assert.equal(read.body.path, "J:/Project/OpenTopia/inspect.mjs");
  }
});

test("labels MCP tools by server and tool name", () => {
  const view = buildToolActivity(
    call("github__list_issues", { repo: "acme/app" }),
    result("[]", { success: true }),
  );
  assert.equal(view.kind, "mcp");
  assert.equal(view.title, "github · list_issues");
  assert.equal(view.body.type, "fields");
});

test("redacts secrets in rendered bodies", () => {
  const view = buildToolActivity(
    call("shell", { command: "echo hi" }),
    result("$ echo hi\n\n[stdout]\ntoken=abc123\n\n[stderr]\n", {
      exitCode: 0,
    }),
  );
  if (view.body.type === "terminal") {
    assert.equal(view.body.streams.stdout, "token=[已隐藏]");
  } else {
    assert.fail("expected a terminal body");
  }
});
