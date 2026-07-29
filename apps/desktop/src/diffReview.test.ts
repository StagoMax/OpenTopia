import assert from "node:assert/strict";
import test from "node:test";

import type * as DiffReviewModule from "./diffReview";

const diffReview: typeof DiffReviewModule = await import(
  "./diffReview" + ".ts"
);

const {
  buildDiffBlocks,
  buildDiffFileTree,
  buildGitApplyCommand,
  buildSpans,
  buildSplitRows,
  buildUnifiedRows,
  changedRanges,
  countDiffRows,
  diffLanguageFromPath,
  matchesPathQuery,
  parseUnifiedDiff,
  splitFileContent,
  summarizeDiffStats,
  tokenizeLine,
} = diffReview;

const twoFileDiff = [
  "diff --git a/apps/desktop/electron/main.cjs b/apps/desktop/electron/main.cjs",
  "index 1111111..2222222 100644",
  "--- a/apps/desktop/electron/main.cjs",
  "+++ b/apps/desktop/electron/main.cjs",
  "@@ -20,3 +20,4 @@",
  " const isDev = !app.isPackaged;",
  "-const title = 'OpenTopia';",
  "+const title = isDev ? 'OpenTopia Dev' : 'OpenTopia';",
  "+app.setName(title);",
  " module.exports = { title };",
  "@@ -60,2 +61,2 @@",
  "-await start();",
  "+await start({ dev: isDev });",
  " process.exit(0);",
  "diff --git a/README.md b/README.md",
  "--- a/README.md",
  "+++ b/README.md",
  "@@ -1,2 +1,2 @@",
  "-# OpenTopia",
  "+# OpenTopia Desktop",
  " Local first agent.",
].join("\n");

test("parses every file section of a multi-file diff", () => {
  const files = parseUnifiedDiff(twoFileDiff);
  assert.equal(files.length, 2);
  assert.equal(files[0].path, "apps/desktop/electron/main.cjs");
  assert.equal(files[0].status, "modified");
  assert.equal(files[0].hunks.length, 2);
  assert.equal(files[1].path, "README.md");
  assert.deepEqual(summarizeDiffStats(files), { additions: 4, deletions: 3 });
});

test("numbers lines from the hunk header on both sides", () => {
  const [file] = parseUnifiedDiff(twoFileDiff);
  const [hunk] = file.hunks;
  assert.deepEqual(
    hunk.lines.map((line) => [line.kind, line.oldLine, line.newLine]),
    [
      ["context", 20, 20],
      ["removed", 21, null],
      ["added", null, 21],
      ["added", null, 22],
      ["context", 22, 23],
    ],
  );
});

test("keeps a hunk patch that git apply would accept", () => {
  const [file] = parseUnifiedDiff(twoFileDiff);
  assert.equal(file.hunks[0].patch.split("\n")[0], "@@ -20,3 +20,4 @@");
  assert.ok(file.patch.startsWith("diff --git a/apps/desktop"));
  assert.ok(file.patch.includes("+app.setName(title);"));
  assert.ok(!file.patch.includes("README.md"));
});

test("reads added, deleted, renamed and binary sections", () => {
  const files = parseUnifiedDiff(
    [
      "diff --git a/new.ts b/new.ts",
      "new file mode 100644",
      "--- /dev/null",
      "+++ b/new.ts",
      "@@ -0,0 +1,1 @@",
      "+export const value = 1;",
      "diff --git a/gone.ts b/gone.ts",
      "deleted file mode 100644",
      "--- a/gone.ts",
      "+++ /dev/null",
      "@@ -1,1 +0,0 @@",
      "-export const gone = true;",
      "diff --git a/old/name.ts b/new/name.ts",
      "similarity index 92%",
      "rename from old/name.ts",
      "rename to new/name.ts",
      "diff --git a/logo.png b/logo.png",
      "Binary files a/logo.png and b/logo.png differ",
    ].join("\n"),
  );

  assert.deepEqual(
    files.map((file) => [file.path, file.status, file.binary]),
    [
      ["new.ts", "added", false],
      ["gone.ts", "deleted", false],
      ["new/name.ts", "renamed", false],
      ["logo.png", "modified", true],
    ],
  );
});

test("accepts a bare per-file patch with no diff --git header", () => {
  const files = parseUnifiedDiff(
    ["@@ -1,1 +1,1 @@", "-a", "+b"].join("\n"),
    "docs/plan.md",
  );
  assert.equal(files.length, 1);
  assert.equal(files[0].path, "docs/plan.md");
});

test("treats a blank body line as an empty context line", () => {
  const [file] = parseUnifiedDiff(
    ["@@ -1,3 +1,3 @@", " first", "", "-third", "+3rd"].join("\n"),
    "a.txt",
  );
  assert.deepEqual(
    file.hunks[0].lines.map((line) => [line.kind, line.text]),
    [
      ["context", "first"],
      ["context", ""],
      ["removed", "third"],
      ["added", "3rd"],
    ],
  );
});

test("collapses untouched regions into gaps between hunks", () => {
  const [file] = parseUnifiedDiff(twoFileDiff);
  const blocks = buildDiffBlocks(file);
  const gaps = blocks.filter((block) => block.type === "gap");
  assert.deepEqual(
    gaps.map((gap) => [gap.count, gap.oldStart, gap.newStart]),
    [
      [19, 1, 1],
      [37, 23, 24],
    ],
  );
});

test("expands a gap only when the file content is loaded", () => {
  const [file] = parseUnifiedDiff(
    ["@@ -3,1 +3,1 @@", "-old", "+new"].join("\n"),
    "a.txt",
  );
  const content = ["one", "two", "old"];

  const collapsed = buildDiffBlocks(file, { expandedGaps: "all" });
  assert.equal(collapsed[0].type, "gap");

  const expanded = buildDiffBlocks(file, {
    expandedGaps: "all",
    newFileLines: content,
  });
  assert.equal(expanded[0].type, "context");
  assert.deepEqual(
    expanded[0].type === "context"
      ? expanded[0].lines.map((line) => [line.newLine, line.text])
      : [],
    [
      [1, "one"],
      [2, "two"],
    ],
  );
});

test("adds a trailing gap once the file length is known", () => {
  const [file] = parseUnifiedDiff(
    ["@@ -1,1 +1,1 @@", "-old", "+new"].join("\n"),
    "a.txt",
  );
  const blocks = buildDiffBlocks(file, {
    newFileLines: ["new", "two", "three"],
  });
  const last = blocks.at(-1);
  assert.equal(last?.type, "gap");
  assert.equal(last?.type === "gap" ? last.count : 0, 2);
});

test("pads the shorter side of a change so split rows stay aligned", () => {
  const [file] = parseUnifiedDiff(
    ["@@ -1,1 +1,3 @@", "-one", "+one", "+two", "+three"].join("\n"),
    "a.txt",
  );
  const rows = buildSplitRows(buildDiffBlocks(file));
  const pairs = rows.filter((row) => row.type === "pair");
  assert.deepEqual(
    pairs.map((row) => [row.left?.text ?? null, row.right?.text ?? null]),
    [
      ["one", "one"],
      [null, "two"],
      [null, "three"],
    ],
  );
});

test("numbers each split side from its own image", () => {
  // The second hunk starts at old line 60 but new line 61, so a context line
  // must not show the same number in both gutters.
  const [file] = parseUnifiedDiff(
    ["@@ -60,2 +61,2 @@", " context", "-old", "+new"].join("\n"),
    "a.txt",
  );
  const rows = buildSplitRows(buildDiffBlocks(file));
  const pairs = rows.filter((row) => row.type === "pair");
  assert.deepEqual(
    pairs.map((row) => [row.left?.number ?? null, row.right?.number ?? null]),
    [
      [60, 61],
      [61, 62],
    ],
  );
});

test("orders unified rows as removals then additions", () => {
  const [file] = parseUnifiedDiff(twoFileDiff);
  const rows = buildUnifiedRows(buildDiffBlocks(file));
  const kinds = rows
    .filter((row) => row.type === "line")
    .map((row) => (row.type === "line" ? row.side.kind : ""));
  assert.deepEqual(kinds.slice(0, 5), [
    "context",
    "removed",
    "added",
    "added",
    "context",
  ]);
});

test("counts full rows while only building the visible row budget", () => {
  const [file] = parseUnifiedDiff(
    [
      "@@ -1,3 +1,3 @@",
      "-one",
      "-two",
      "-three",
      "+ONE",
      "+TWO",
      "+THREE",
    ].join("\n"),
    "sample.ts",
  );
  const blocks = buildDiffBlocks(file);

  assert.equal(countDiffRows(blocks, "split"), 3);
  assert.equal(countDiffRows(blocks, "unified"), 6);
  assert.equal(buildSplitRows(blocks, {}, 2).length, 2);
  assert.equal(buildUnifiedRows(blocks, {}, 2).length, 2);
});

test("hides whitespace-only edits when asked", () => {
  const [file] = parseUnifiedDiff(
    [
      "@@ -1,2 +1,2 @@",
      "-  const a = 1;",
      "-const b = 2;",
      "+const a = 1;",
      "+const b = 3;",
    ].join("\n"),
    "a.ts",
  );

  const plain = buildDiffBlocks(file);
  assert.equal(plain.filter((block) => block.type === "change").length, 1);

  const hidden = buildDiffBlocks(file, { ignoreWhitespace: true });
  assert.deepEqual(
    hidden.map((block) => block.type),
    ["context", "change"],
  );
  const change = hidden.find((block) => block.type === "change");
  assert.deepEqual(
    change?.type === "change" ? change.removed.map((line) => line.text) : [],
    ["const b = 2;"],
  );
});

test("marks only the words that changed inside a line", () => {
  const ranges = changedRanges(
    "const title = 'OpenTopia Dev';",
    "const title = 'OpenTopia';",
  );
  const marked = ranges.map(([start, end]) =>
    "const title = 'OpenTopia Dev';".slice(start, end),
  );
  assert.deepEqual(marked, ["Dev"]);
});

test("splits a line into spans that carry syntax and word-diff state", () => {
  const spans = buildSpans("const a = 1;", "typescript", [[6, 7]]);
  assert.deepEqual(
    spans.map((span) => [span.text, span.syntax, span.changed]),
    [
      ["const", "keyword", false],
      [" ", null, false],
      ["a", null, true],
      [" ", null, false],
      ["=", "punct", false],
      [" ", null, false],
      ["1", "number", false],
      [";", "punct", false],
    ],
  );
});

test("tokenizes comments and strings per language", () => {
  assert.deepEqual(
    tokenizeLine("# note", "python").map((token) => token.kind),
    ["comment"],
  );
  // "#" is not a comment in TypeScript, and a plain identifier stays unstyled.
  assert.deepEqual(
    tokenizeLine("# Note", "typescript").map((token) => token.kind),
    ["punct", "type"],
  );
  assert.deepEqual(
    tokenizeLine("# note", "typescript").map((token) => token.kind),
    ["punct"],
  );
  const jsTokens = tokenizeLine('const s = "a // b"; // tail', "typescript");
  assert.deepEqual(
    jsTokens.map((token) => token.kind),
    ["keyword", "punct", "string", "punct", "comment"],
  );
  assert.equal(diffLanguageFromPath("apps/desktop/src/App.tsx"), "typescript");
  assert.equal(diffLanguageFromPath("Cargo.toml"), "toml");
  assert.equal(diffLanguageFromPath("LICENSE"), null);
});

test("collapses single-child directory chains in the file tree", () => {
  const files = parseUnifiedDiff(twoFileDiff);
  const tree = buildDiffFileTree(files);
  assert.deepEqual(
    tree.map((node) => [node.type, node.name]),
    [
      ["directory", "apps/desktop/electron"],
      ["file", "README.md"],
    ],
  );
  const directory = tree[0];
  assert.deepEqual(
    directory.type === "directory"
      ? directory.children.map((child) => child.name)
      : [],
    ["main.cjs"],
  );
});

test("matches file queries by substring and by subsequence", () => {
  assert.ok(matchesPathQuery("apps/desktop/electron/main.cjs", "main"));
  assert.ok(matchesPathQuery("apps/desktop/electron/main.cjs", "adem"));
  assert.ok(!matchesPathQuery("apps/desktop/electron/main.cjs", "zz"));
  assert.ok(matchesPathQuery("README.md", "  "));
});

test("builds a copyable git apply command, or nothing to copy", () => {
  const [file] = parseUnifiedDiff(twoFileDiff);
  const command = buildGitApplyCommand(file.patch);
  assert.ok(command?.startsWith("git apply <<'OPENTOPIA_PATCH'\n"));
  assert.ok(command?.endsWith("\nOPENTOPIA_PATCH"));
  assert.equal(buildGitApplyCommand("   \n"), null);
});

test("builds a PowerShell git apply command without embedding patch text", () => {
  const patch = "diff --git a/你好.txt b/你好.txt\n+新内容\n";
  const command = buildGitApplyCommand(patch, "powershell");
  assert.ok(command?.includes("[Convert]::FromBase64String"));
  assert.ok(command?.includes("git apply -- $opentopiaPatch"));
  assert.ok(command?.includes("Remove-Item -LiteralPath $opentopiaPatch"));
  assert.ok(!command?.includes("新内容"));

  const payload = command?.match(/FromBase64String\('([^']+)'\)/)?.[1];
  assert.ok(payload);
  assert.equal(Buffer.from(payload, "base64").toString("utf8"), patch);
});

test("splits file content without inventing a trailing line", () => {
  assert.deepEqual(splitFileContent("a\r\nb\r\n"), ["a", "b"]);
  assert.deepEqual(splitFileContent("a\nb"), ["a", "b"]);
});
