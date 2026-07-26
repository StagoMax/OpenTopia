/**
 * Diff review model.
 *
 * Pure, DOM-free helpers behind the review panel: parsing a unified diff into
 * files and hunks, folding a file into renderable rows for the split and the
 * unified view, collapsing untouched regions, intra-line word diffs, and a
 * line-local syntax tokenizer. Keeping this out of the component lets the
 * tricky parts (hunk arithmetic, pairing, gap expansion) be unit tested.
 */

export type DiffFileStatus = "added" | "deleted" | "modified" | "renamed";

export type DiffLineKind = "context" | "added" | "removed";

export type ParsedDiffLine = {
  kind: DiffLineKind;
  /** 1-based line number on the pre-image side, null for added lines. */
  oldLine: number | null;
  /** 1-based line number on the post-image side, null for removed lines. */
  newLine: number | null;
  text: string;
};

export type ParsedDiffHunk = {
  header: string;
  oldStart: number;
  oldLines: number;
  newStart: number;
  newLines: number;
  lines: ParsedDiffLine[];
  /** Header plus raw body, i.e. a patch body `git apply` accepts. */
  patch: string;
};

export type ParsedDiffFile = {
  /** Display path: the post-image path when the file still exists. */
  path: string;
  oldPath: string | null;
  newPath: string | null;
  status: DiffFileStatus;
  binary: boolean;
  additions: number;
  deletions: number;
  hunks: ParsedDiffHunk[];
  /** The complete `diff --git` section, suitable for `git apply`. */
  patch: string;
};

type MutableFile = ParsedDiffFile & { patchLines: string[] };

type MutableHunk = ParsedDiffHunk & {
  patchLines: string[];
  remainingOld: number;
  remainingNew: number;
};

const hunkHeaderPattern = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/;

/**
 * Parses `git diff` output. Sections without a `diff --git` header (the
 * per-file previews the turn API returns) are accepted too, in which case
 * `fallbackPath` names the file.
 */
export function parseUnifiedDiff(
  text: string,
  fallbackPath?: string | null,
): ParsedDiffFile[] {
  const source = text.replace(/\r\n/g, "\n");
  if (!source.trim()) return [];

  const files: ParsedDiffFile[] = [];
  let file: MutableFile | null = null;
  let hunk: MutableHunk | null = null;

  const closeHunk = () => {
    if (file && hunk) {
      hunk.patch = hunk.patchLines.join("\n");
      file.hunks.push(stripHunkScratch(hunk));
    }
    hunk = null;
  };

  const closeFile = () => {
    closeHunk();
    if (file) {
      file.patch = file.patchLines.join("\n");
      files.push(stripFileScratch(file));
    }
    file = null;
  };

  for (const raw of source.split("\n")) {
    if (raw.startsWith("diff --git ")) {
      closeFile();
      file = createFile(parseDiffGitHeader(raw));
      file.patchLines.push(raw);
      continue;
    }

    if (!file) {
      // A bare per-file patch: synthesize the section the header would have
      // produced so the rest of the loop has somewhere to write.
      if (
        raw.startsWith("--- ") ||
        raw.startsWith("+++ ") ||
        hunkHeaderPattern.test(raw)
      ) {
        file = createFile({
          oldPath: fallbackPath ?? null,
          newPath: fallbackPath ?? null,
        });
      } else {
        continue;
      }
    }

    file.patchLines.push(raw);

    if (hunk) {
      const consumed = consumeHunkLine(hunk, raw);
      if (consumed) continue;
      closeHunk();
    }

    const hunkHeader = hunkHeaderPattern.exec(raw);
    if (hunkHeader) {
      hunk = createHunk(raw, hunkHeader);
      continue;
    }

    applyFileHeaderLine(file, raw);
  }

  closeFile();
  return files;
}

function createFile(paths: {
  oldPath: string | null;
  newPath: string | null;
}): MutableFile {
  return {
    path: paths.newPath ?? paths.oldPath ?? "",
    oldPath: paths.oldPath,
    newPath: paths.newPath,
    status: "modified",
    binary: false,
    additions: 0,
    deletions: 0,
    hunks: [],
    patch: "",
    patchLines: [],
  };
}

function stripFileScratch(file: MutableFile): ParsedDiffFile {
  const { patchLines: _patchLines, ...rest } = file;
  return { ...rest, path: rest.newPath ?? rest.oldPath ?? rest.path };
}

function stripHunkScratch(hunk: MutableHunk): ParsedDiffHunk {
  const {
    patchLines: _patchLines,
    remainingOld: _remainingOld,
    remainingNew: _remainingNew,
    ...rest
  } = hunk;
  return rest;
}

function createHunk(header: string, match: RegExpExecArray): MutableHunk {
  const oldStart = Number(match[1]);
  const oldLines = match[2] === undefined ? 1 : Number(match[2]);
  const newStart = Number(match[3]);
  const newLines = match[4] === undefined ? 1 : Number(match[4]);
  return {
    header,
    oldStart,
    oldLines,
    newStart,
    newLines,
    lines: [],
    patch: "",
    patchLines: [header],
    remainingOld: oldLines,
    remainingNew: newLines,
  };
}

/**
 * Appends one body line to the hunk. Returns false once the hunk has consumed
 * the line counts its header promised, which is what ends a hunk — a blank
 * line inside a hunk is a legitimate empty context line in patches whose
 * trailing space was stripped.
 */
function consumeHunkLine(hunk: MutableHunk, raw: string): boolean {
  if (raw.startsWith("\\")) {
    // "\ No newline at end of file" belongs to the patch but is not a line.
    hunk.patchLines.push(raw);
    return true;
  }
  if (hunk.remainingOld <= 0 && hunk.remainingNew <= 0) return false;

  const marker = raw.slice(0, 1);
  const text = raw.slice(1);
  if (marker === "+") {
    if (hunk.remainingNew <= 0) return false;
    hunk.remainingNew -= 1;
    hunk.lines.push({
      kind: "added",
      oldLine: null,
      newLine: hunk.newStart + (hunk.newLines - hunk.remainingNew) - 1,
      text,
    });
  } else if (marker === "-") {
    if (hunk.remainingOld <= 0) return false;
    hunk.remainingOld -= 1;
    hunk.lines.push({
      kind: "removed",
      oldLine: hunk.oldStart + (hunk.oldLines - hunk.remainingOld) - 1,
      newLine: null,
      text,
    });
  } else if (marker === " " || raw === "") {
    if (hunk.remainingOld <= 0 || hunk.remainingNew <= 0) return false;
    hunk.remainingOld -= 1;
    hunk.remainingNew -= 1;
    hunk.lines.push({
      kind: "context",
      oldLine: hunk.oldStart + (hunk.oldLines - hunk.remainingOld) - 1,
      newLine: hunk.newStart + (hunk.newLines - hunk.remainingNew) - 1,
      text: raw === "" ? "" : text,
    });
  } else {
    return false;
  }

  hunk.patchLines.push(raw);
  return true;
}

function applyFileHeaderLine(file: MutableFile, raw: string): void {
  if (raw.startsWith("new file mode")) {
    file.status = "added";
    return;
  }
  if (raw.startsWith("deleted file mode")) {
    file.status = "deleted";
    return;
  }
  if (raw.startsWith("rename from ")) {
    file.status = "renamed";
    file.oldPath = raw.slice("rename from ".length).trim();
    return;
  }
  if (raw.startsWith("rename to ")) {
    file.status = "renamed";
    file.newPath = raw.slice("rename to ".length).trim();
    file.path = file.newPath;
    return;
  }
  if (raw.startsWith("Binary files") || raw.startsWith("GIT binary patch")) {
    file.binary = true;
    return;
  }
  if (raw.startsWith("--- ")) {
    const path = stripDiffPathPrefix(raw.slice(4));
    file.oldPath = path;
    if (!path) file.status = "added";
    return;
  }
  if (raw.startsWith("+++ ")) {
    const path = stripDiffPathPrefix(raw.slice(4));
    file.newPath = path;
    if (path) file.path = path;
    else file.status = "deleted";
  }
}

function parseDiffGitHeader(raw: string): {
  oldPath: string | null;
  newPath: string | null;
} {
  const rest = raw.slice("diff --git ".length).trim();
  const quoted = /^"(.+)" "(.+)"$/.exec(rest);
  if (quoted) {
    return {
      oldPath: stripDiffPathPrefix(quoted[1]),
      newPath: stripDiffPathPrefix(quoted[2]),
    };
  }
  // Unquoted paths may contain spaces, so split on the " b/" that git puts
  // between the two halves rather than on the first space.
  const separator = rest.indexOf(" b/");
  if (separator > 0) {
    return {
      oldPath: stripDiffPathPrefix(rest.slice(0, separator)),
      newPath: stripDiffPathPrefix(rest.slice(separator + 1)),
    };
  }
  const parts = rest.split(" ");
  return {
    oldPath: stripDiffPathPrefix(parts[0] ?? ""),
    newPath: stripDiffPathPrefix(parts[1] ?? ""),
  };
}

function stripDiffPathPrefix(value: string): string | null {
  const trimmed = value.trim().replace(/^"|"$/g, "");
  if (!trimmed || trimmed === "/dev/null") return null;
  const withoutTimestamp = trimmed.replace(/\t.*$/, "");
  return withoutTimestamp.replace(/^[ab]\//, "");
}

export function fileAdditions(file: ParsedDiffFile): number {
  return file.hunks.reduce(
    (total, hunk) =>
      total + hunk.lines.filter((line) => line.kind === "added").length,
    0,
  );
}

export function fileDeletions(file: ParsedDiffFile): number {
  return file.hunks.reduce(
    (total, hunk) =>
      total + hunk.lines.filter((line) => line.kind === "removed").length,
    0,
  );
}

export function summarizeDiffStats(files: ParsedDiffFile[]): {
  additions: number;
  deletions: number;
} {
  return files.reduce(
    (total, file) => ({
      additions: total.additions + fileAdditions(file),
      deletions: total.deletions + fileDeletions(file),
    }),
    { additions: 0, deletions: 0 },
  );
}

/* ------------------------------------------------------------------ blocks */

export type DiffGap = {
  type: "gap";
  id: string;
  /** Number of untouched lines hidden by this gap. */
  count: number;
  oldStart: number;
  newStart: number;
};

export type DiffBlock =
  | DiffGap
  | { type: "context"; lines: ParsedDiffLine[] }
  | { type: "change"; removed: ParsedDiffLine[]; added: ParsedDiffLine[] };

export type DiffBuildOptions = {
  /** Treat lines that differ only in whitespace as untouched. */
  ignoreWhitespace?: boolean;
  /** Gap ids the reader expanded, or "all" for the full-file toggle. */
  expandedGaps?: ReadonlySet<string> | "all";
  /** Post-image file content, required to render an expanded gap. */
  newFileLines?: readonly string[] | null;
  /** Highlight the words that changed inside a replaced line pair. */
  wordDiff?: boolean;
  /** Language id for the syntax tokenizer, or null to skip highlighting. */
  language?: string | null;
};

/**
 * Folds a parsed file into blocks: hunk bodies split into context runs and
 * change runs, with the untouched space between hunks represented as gaps.
 * A gap that the reader expanded becomes context, provided the file content
 * was loaded — the diff alone does not contain those lines.
 */
export function buildDiffBlocks(
  file: ParsedDiffFile,
  options: DiffBuildOptions = {},
): DiffBlock[] {
  const blocks: DiffBlock[] = [];
  let previousOldEnd = 0;
  let previousNewEnd = 0;

  for (const hunk of file.hunks) {
    const gapCount = hunk.oldStart - 1 - previousOldEnd;
    if (gapCount > 0) {
      pushGap(
        blocks,
        {
          type: "gap",
          id: `gap-${previousOldEnd + 1}-${previousNewEnd + 1}`,
          count: gapCount,
          oldStart: previousOldEnd + 1,
          newStart: previousNewEnd + 1,
        },
        options,
      );
    }

    for (const block of splitHunkLines(hunk.lines, options)) blocks.push(block);
    previousOldEnd = hunk.oldStart + hunk.oldLines - 1;
    previousNewEnd = hunk.newStart + hunk.newLines - 1;
  }

  // The tail is only knowable once the working-tree file has been loaded.
  const totalNewLines = options.newFileLines?.length ?? 0;
  if (totalNewLines > previousNewEnd) {
    pushGap(
      blocks,
      {
        type: "gap",
        id: `gap-${previousOldEnd + 1}-${previousNewEnd + 1}`,
        count: totalNewLines - previousNewEnd,
        oldStart: previousOldEnd + 1,
        newStart: previousNewEnd + 1,
      },
      options,
    );
  }

  return blocks;
}

function pushGap(
  blocks: DiffBlock[],
  gap: DiffGap,
  options: DiffBuildOptions,
): void {
  const expanded =
    options.expandedGaps === "all" || options.expandedGaps?.has(gap.id);
  const content = options.newFileLines;
  if (!expanded || !content) {
    blocks.push(gap);
    return;
  }

  const lines: ParsedDiffLine[] = [];
  for (let offset = 0; offset < gap.count; offset += 1) {
    const newLine = gap.newStart + offset;
    const text = content[newLine - 1];
    if (text === undefined) break;
    lines.push({
      kind: "context",
      oldLine: gap.oldStart + offset,
      newLine,
      text,
    });
  }
  if (lines.length === gap.count) blocks.push({ type: "context", lines });
  // A short read means the loaded content no longer matches the diff; keeping
  // the gap collapsed is better than rendering lines from a stale file.
  else blocks.push(gap);
}

function splitHunkLines(
  lines: ParsedDiffLine[],
  options: DiffBuildOptions,
): DiffBlock[] {
  const blocks: DiffBlock[] = [];
  let context: ParsedDiffLine[] = [];
  let removed: ParsedDiffLine[] = [];
  let added: ParsedDiffLine[] = [];

  const flushContext = () => {
    if (context.length) blocks.push({ type: "context", lines: context });
    context = [];
  };
  const flushChange = () => {
    if (!removed.length && !added.length) return;
    for (const block of settleChange(removed, added, options)) {
      blocks.push(block);
    }
    removed = [];
    added = [];
  };

  for (const line of lines) {
    if (line.kind === "context") {
      flushChange();
      context.push(line);
      continue;
    }
    flushContext();
    if (line.kind === "removed") {
      // An added line followed by a removed line starts a new pairing run.
      if (added.length) flushChange();
      removed.push(line);
    } else {
      added.push(line);
    }
  }
  flushContext();
  flushChange();
  return blocks;
}

/**
 * Demotes whitespace-only edits to context when the reader asked to hide
 * whitespace. Only the paired prefix can be demoted; unpaired lines are real
 * insertions or deletions.
 */
function settleChange(
  removed: ParsedDiffLine[],
  added: ParsedDiffLine[],
  options: DiffBuildOptions,
): DiffBlock[] {
  if (!options.ignoreWhitespace) return [{ type: "change", removed, added }];

  const blocks: DiffBlock[] = [];
  let index = 0;
  let context: ParsedDiffLine[] = [];
  const flushContext = () => {
    if (context.length) blocks.push({ type: "context", lines: context });
    context = [];
  };

  const paired = Math.min(removed.length, added.length);
  while (index < paired) {
    const left = removed[index];
    const right = added[index];
    if (collapseWhitespace(left.text) !== collapseWhitespace(right.text)) break;
    context.push({
      kind: "context",
      oldLine: left.oldLine,
      newLine: right.newLine,
      text: right.text,
    });
    index += 1;
  }
  flushContext();

  const restRemoved = removed.slice(index);
  const restAdded = added.slice(index);
  if (restRemoved.length || restAdded.length) {
    blocks.push({ type: "change", removed: restRemoved, added: restAdded });
  }
  return blocks;
}

function collapseWhitespace(value: string): string {
  return value.replace(/\s+/g, "");
}

/* -------------------------------------------------------------------- rows */

export type SyntaxKind =
  "keyword" | "string" | "number" | "comment" | "type" | "punct";

export type DiffSpan = {
  text: string;
  syntax: SyntaxKind | null;
  /** True when this span is part of what changed inside the line. */
  changed: boolean;
};

export type DiffRowSide = {
  kind: DiffLineKind;
  number: number | null;
  text: string;
  spans: DiffSpan[];
};

export type DiffSplitRow =
  | DiffGap
  | {
      type: "pair";
      id: string;
      /** Null renders as the striped filler that keeps the sides aligned. */
      left: DiffRowSide | null;
      right: DiffRowSide | null;
    };

export type DiffUnifiedRow =
  | DiffGap
  | {
      type: "line";
      id: string;
      oldLine: number | null;
      newLine: number | null;
      side: DiffRowSide;
    };

export function buildSplitRows(
  blocks: DiffBlock[],
  options: DiffBuildOptions = {},
): DiffSplitRow[] {
  const rows: DiffSplitRow[] = [];
  blocks.forEach((block, blockIndex) => {
    if (block.type === "gap") {
      rows.push(block);
      return;
    }
    if (block.type === "context") {
      block.lines.forEach((line, index) => {
        // The same text sits at different line numbers on the two sides once
        // an earlier hunk has shifted the file, so each side is numbered from
        // its own image rather than copied across.
        rows.push({
          type: "pair",
          id: `c${blockIndex}-${index}`,
          left: toSide(line, null, options, "old"),
          right: toSide(line, null, options, "new"),
        });
      });
      return;
    }
    const height = Math.max(block.removed.length, block.added.length);
    for (let index = 0; index < height; index += 1) {
      const left = block.removed[index] ?? null;
      const right = block.added[index] ?? null;
      rows.push({
        type: "pair",
        id: `d${blockIndex}-${index}`,
        left: left ? toSide(left, right, options) : null,
        right: right ? toSide(right, left, options) : null,
      });
    }
  });
  return rows;
}

export function buildUnifiedRows(
  blocks: DiffBlock[],
  options: DiffBuildOptions = {},
): DiffUnifiedRow[] {
  const rows: DiffUnifiedRow[] = [];
  blocks.forEach((block, blockIndex) => {
    if (block.type === "gap") {
      rows.push(block);
      return;
    }
    const push = (
      line: ParsedDiffLine,
      counterpart: ParsedDiffLine | null,
      key: string,
    ) => {
      rows.push({
        type: "line",
        id: key,
        oldLine: line.oldLine,
        newLine: line.newLine,
        side: toSide(line, counterpart, options),
      });
    };
    if (block.type === "context") {
      block.lines.forEach((line, index) =>
        push(line, null, `c${blockIndex}-${index}`),
      );
      return;
    }
    block.removed.forEach((line, index) =>
      push(line, block.added[index] ?? null, `r${blockIndex}-${index}`),
    );
    block.added.forEach((line, index) =>
      push(line, block.removed[index] ?? null, `a${blockIndex}-${index}`),
    );
  });
  return rows;
}

function toSide(
  line: ParsedDiffLine,
  counterpart: ParsedDiffLine | null,
  options: DiffBuildOptions,
  /** Which image numbers this side; defaults to the one the line belongs to. */
  numberFrom?: "old" | "new",
): DiffRowSide {
  const changed =
    options.wordDiff && counterpart && line.kind !== "context"
      ? changedRanges(line.text, counterpart.text)
      : [];
  const image = numberFrom ?? (line.kind === "removed" ? "old" : "new");
  return {
    kind: line.kind,
    number: image === "old" ? line.oldLine : line.newLine,
    text: line.text,
    spans: buildSpans(line.text, options.language ?? null, changed),
  };
}

/* --------------------------------------------------------------- word diff */

export type CharRange = [start: number, end: number];

const wordPattern = /\s+|[A-Za-z0-9_$]+|[^\sA-Za-z0-9_$]/g;

function tokenizeWords(text: string): Array<{ text: string; start: number }> {
  const tokens: Array<{ text: string; start: number }> = [];
  for (const match of text.matchAll(wordPattern)) {
    tokens.push({ text: match[0], start: match.index ?? 0 });
  }
  return tokens;
}

/**
 * Character ranges of `text` that are absent from `other`, computed from a
 * word-level longest common subsequence.
 */
export function changedRanges(text: string, other: string): CharRange[] {
  const mine = tokenizeWords(text);
  const theirs = tokenizeWords(other);
  const table = lcsTable(
    mine.map((token) => token.text),
    theirs.map((token) => token.text),
  );

  const matched = new Set<number>();
  let i = 0;
  let j = 0;
  while (i < mine.length && j < theirs.length) {
    if (mine[i].text === theirs[j].text) {
      matched.add(i);
      i += 1;
      j += 1;
    } else if (table[i + 1][j] >= table[i][j + 1]) {
      i += 1;
    } else {
      j += 1;
    }
  }

  const ranges: CharRange[] = [];
  mine.forEach((token, index) => {
    if (matched.has(index)) return;
    // Whitespace-only shifts are noise unless the whole line is whitespace.
    if (!token.text.trim() && mine.length > 1) return;
    const start = token.start;
    const end = start + token.text.length;
    const last = ranges.at(-1);
    if (last && last[1] === start) last[1] = end;
    else ranges.push([start, end]);
  });
  return ranges;
}

function lcsTable(left: string[], right: string[]): number[][] {
  const table: number[][] = Array.from({ length: left.length + 1 }, () =>
    new Array<number>(right.length + 1).fill(0),
  );
  for (let i = left.length - 1; i >= 0; i -= 1) {
    for (let j = right.length - 1; j >= 0; j -= 1) {
      table[i][j] =
        left[i] === right[j]
          ? table[i + 1][j + 1] + 1
          : Math.max(table[i + 1][j], table[i][j + 1]);
    }
  }
  return table;
}

/* -------------------------------------------------------------- highlights */

export type SyntaxToken = { start: number; end: number; kind: SyntaxKind };

const sharedKeywords = [
  "as",
  "async",
  "await",
  "break",
  "case",
  "catch",
  "class",
  "const",
  "continue",
  "def",
  "default",
  "delete",
  "do",
  "elif",
  "else",
  "enum",
  "export",
  "extends",
  "false",
  "finally",
  "fn",
  "for",
  "from",
  "func",
  "function",
  "if",
  "impl",
  "import",
  "in",
  "instanceof",
  "interface",
  "let",
  "match",
  "mod",
  "mut",
  "new",
  "nil",
  "none",
  "null",
  "package",
  "pub",
  "return",
  "self",
  "static",
  "struct",
  "super",
  "switch",
  "this",
  "throw",
  "trait",
  "true",
  "try",
  "type",
  "typeof",
  "use",
  "var",
  "void",
  "where",
  "while",
  "with",
  "yield",
];

const keywordSet = new Set(sharedKeywords);

const typePattern = /^[A-Z][A-Za-z0-9_]*$/;

const identifierPattern = /[A-Za-z_$][A-Za-z0-9_$]*/y;
const numberPattern =
  /0[xXbBoO][0-9a-fA-F_]+|\d[\d_]*(?:\.\d[\d_]*)?(?:[eE][+-]?\d+)?/y;

type LanguageProfile = {
  lineComments: string[];
  blockComment: [string, string] | null;
  quotes: string[];
};

const defaultProfile: LanguageProfile = {
  lineComments: ["//"],
  blockComment: ["/*", "*/"],
  quotes: ['"', "'", "`"],
};

const hashProfile: LanguageProfile = {
  lineComments: ["#"],
  blockComment: null,
  quotes: ['"', "'"],
};

const languageProfiles: Record<string, LanguageProfile> = {
  bash: hashProfile,
  css: { lineComments: [], blockComment: ["/*", "*/"], quotes: ['"', "'"] },
  json: { lineComments: [], blockComment: null, quotes: ['"'] },
  markdown: { lineComments: [], blockComment: null, quotes: ["`"] },
  powershell: hashProfile,
  python: hashProfile,
  ruby: hashProfile,
  sql: { lineComments: ["--"], blockComment: ["/*", "*/"], quotes: ["'", '"'] },
  toml: hashProfile,
  yaml: hashProfile,
};

const extensionLanguages: Record<string, string> = {
  bash: "bash",
  c: "c",
  cc: "cpp",
  cjs: "javascript",
  cpp: "cpp",
  cs: "csharp",
  css: "css",
  go: "go",
  h: "c",
  hpp: "cpp",
  htm: "html",
  html: "html",
  java: "java",
  js: "javascript",
  json: "json",
  jsonc: "json",
  jsx: "javascript",
  kt: "kotlin",
  less: "css",
  lua: "lua",
  md: "markdown",
  mdx: "markdown",
  mjs: "javascript",
  php: "php",
  ps1: "powershell",
  py: "python",
  rb: "ruby",
  rs: "rust",
  scss: "css",
  sh: "bash",
  sql: "sql",
  swift: "swift",
  toml: "toml",
  ts: "typescript",
  tsx: "typescript",
  yaml: "yaml",
  yml: "yaml",
  zsh: "bash",
};

export function diffLanguageFromPath(path: string): string | null {
  const name = path.replace(/\\/g, "/").split("/").at(-1) ?? "";
  const extension = name.includes(".")
    ? (name.split(".").at(-1) ?? "").toLocaleLowerCase()
    : "";
  return extensionLanguages[extension] ?? null;
}

/**
 * Line-local tokenizer. It never carries state between lines, so a string or
 * block comment that spans lines is highlighted only on its first line — an
 * acceptable trade for a diff, where lines are rendered independently.
 */
export function tokenizeLine(
  text: string,
  language: string | null,
): SyntaxToken[] {
  if (!language || !text) return [];
  const profile = languageProfiles[language] ?? defaultProfile;
  const tokens: SyntaxToken[] = [];
  let index = 0;

  while (index < text.length) {
    const character = text[index];

    if (/\s/.test(character)) {
      index += 1;
      continue;
    }

    const lineComment = profile.lineComments.find((marker) =>
      text.startsWith(marker, index),
    );
    if (lineComment) {
      tokens.push({ start: index, end: text.length, kind: "comment" });
      break;
    }

    if (
      profile.blockComment &&
      text.startsWith(profile.blockComment[0], index)
    ) {
      const close = text.indexOf(profile.blockComment[1], index + 2);
      const end = close === -1 ? text.length : close + 2;
      tokens.push({ start: index, end, kind: "comment" });
      index = end;
      continue;
    }

    if (profile.quotes.includes(character)) {
      const end = findStringEnd(text, index, character);
      tokens.push({ start: index, end, kind: "string" });
      index = end;
      continue;
    }

    numberPattern.lastIndex = index;
    const number = numberPattern.exec(text);
    if (number && number.index === index) {
      tokens.push({
        start: index,
        end: index + number[0].length,
        kind: "number",
      });
      index += number[0].length;
      continue;
    }

    identifierPattern.lastIndex = index;
    const identifier = identifierPattern.exec(text);
    if (identifier && identifier.index === index) {
      const word = identifier[0];
      const end = index + word.length;
      if (keywordSet.has(word))
        tokens.push({ start: index, end, kind: "keyword" });
      else if (typePattern.test(word))
        tokens.push({ start: index, end, kind: "type" });
      index = end;
      continue;
    }

    tokens.push({ start: index, end: index + 1, kind: "punct" });
    index += 1;
  }

  return tokens;
}

function findStringEnd(text: string, start: number, quote: string): number {
  let index = start + 1;
  while (index < text.length) {
    if (text[index] === "\\") {
      index += 2;
      continue;
    }
    if (text[index] === quote) return index + 1;
    index += 1;
  }
  return text.length;
}

/**
 * Cuts a line into spans carrying both the syntax class and the word-diff
 * flag, splitting wherever either boundary falls.
 */
export function buildSpans(
  text: string,
  language: string | null,
  changed: readonly CharRange[] = [],
): DiffSpan[] {
  if (!text) return [];
  const tokens = tokenizeLine(text, language);
  const boundaries = new Set<number>([0, text.length]);
  for (const token of tokens) {
    boundaries.add(token.start);
    boundaries.add(token.end);
  }
  for (const [start, end] of changed) {
    boundaries.add(Math.max(0, start));
    boundaries.add(Math.min(text.length, end));
  }

  const points = [...boundaries].sort((left, right) => left - right);
  const spans: DiffSpan[] = [];
  for (let index = 0; index < points.length - 1; index += 1) {
    const start = points[index];
    const end = points[index + 1];
    if (end <= start) continue;
    const syntax =
      tokens.find((token) => token.start <= start && token.end >= end)?.kind ??
      null;
    const isChanged = changed.some(([from, to]) => from <= start && to >= end);
    const last = spans.at(-1);
    if (last && last.syntax === syntax && last.changed === isChanged) {
      last.text += text.slice(start, end);
      continue;
    }
    spans.push({ text: text.slice(start, end), syntax, changed: isChanged });
  }
  return spans;
}

/* --------------------------------------------------------------- file tree */

export type DiffTreeNode =
  | { type: "directory"; id: string; name: string; children: DiffTreeNode[] }
  | {
      type: "file";
      id: string;
      name: string;
      path: string;
      status: DiffFileStatus;
      additions: number;
      deletions: number;
    };

type TreeBuilder = {
  directories: Map<string, TreeBuilder>;
  files: Array<{ name: string; file: ParsedDiffFile }>;
};

/**
 * Groups changed files into a directory tree, collapsing chains of
 * single-child directories into one row ("apps/desktop/electron").
 */
export function buildDiffFileTree(files: ParsedDiffFile[]): DiffTreeNode[] {
  const root: TreeBuilder = { directories: new Map(), files: [] };
  for (const file of files) {
    const parts = file.path.replace(/\\/g, "/").split("/").filter(Boolean);
    const name = parts.pop() ?? file.path;
    let node = root;
    for (const part of parts) {
      const next = node.directories.get(part) ?? {
        directories: new Map(),
        files: [],
      };
      node.directories.set(part, next);
      node = next;
    }
    node.files.push({ name, file });
  }
  return toTreeNodes(root, "");
}

function toTreeNodes(node: TreeBuilder, prefix: string): DiffTreeNode[] {
  const nodes: DiffTreeNode[] = [];
  for (const [name, child] of node.directories) {
    let label = name;
    let current = child;
    let id = prefix ? `${prefix}/${name}` : name;
    while (current.files.length === 0 && current.directories.size === 1) {
      const [childName, grandChild] = [...current.directories][0];
      label = `${label}/${childName}`;
      id = `${id}/${childName}`;
      current = grandChild;
    }
    nodes.push({
      type: "directory",
      id,
      name: label,
      children: toTreeNodes(current, id),
    });
  }
  for (const entry of node.files) {
    nodes.push({
      type: "file",
      id: entry.file.path,
      name: entry.name,
      path: entry.file.path,
      status: entry.file.status,
      additions: fileAdditions(entry.file),
      deletions: fileDeletions(entry.file),
    });
  }
  return nodes;
}

/** Case-insensitive subsequence match, so "adm" finds "apps/desktop/main". */
export function matchesPathQuery(path: string, query: string): boolean {
  const needle = query.trim().toLocaleLowerCase();
  if (!needle) return true;
  const haystack = path.toLocaleLowerCase();
  if (haystack.includes(needle)) return true;
  let index = 0;
  for (const character of needle) {
    index = haystack.indexOf(character, index);
    if (index === -1) return false;
    index += 1;
  }
  return true;
}

/**
 * Wraps a patch in a here-document so it can be pasted into a shell as-is.
 * Returns null when there is nothing to apply, which is what disables the
 * menu entry.
 */
export function buildGitApplyCommand(patch: string): string | null {
  const body = patch.replace(/\r\n/g, "\n").replace(/\n+$/, "");
  if (!body.trim()) return null;
  return `git apply <<'OPENTOPIA_PATCH'\n${body}\nOPENTOPIA_PATCH`;
}

export function splitFileContent(content: string): string[] {
  const normalized = content.replace(/\r\n/g, "\n");
  const lines = normalized.split("\n");
  if (lines.at(-1) === "") lines.pop();
  return lines;
}

export function diffFileDirectory(path: string): string {
  const normalized = path.replace(/\\/g, "/");
  const index = normalized.lastIndexOf("/");
  return index === -1 ? "" : normalized.slice(0, index);
}

export function diffFileName(path: string): string {
  const normalized = path.replace(/\\/g, "/");
  return normalized.split("/").at(-1) ?? path;
}
