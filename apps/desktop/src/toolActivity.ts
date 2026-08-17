import type { ToolCall, ToolResult } from "./types";

// The activity timeline renders a tool call from the call input and the result
// metadata alone — never from prose the model wrote. Codex does the same: the
// agent emits structured begin/end records and the client decides how a shell
// command, a patch or a search should look. Everything in this module is that
// decision layer, kept out of React so it can be unit tested.

export type ToolActivityKind =
  | "shell"
  | "read"
  | "list"
  | "search"
  | "edit"
  | "diff"
  | "browser"
  | "computer"
  | "spreadsheet"
  | "agent"
  | "plan"
  | "skill"
  | "attachment"
  | "mcp"
  | "tool";

export type ToolActivityIconKind =
  ToolActivityKind | "image" | "document" | "code" | "archive";

export type ShellCommandKind =
  | "read"
  | "list"
  | "search"
  | "git"
  | "test"
  | "lint"
  | "format"
  | "build"
  | "other";

export type ParsedShellCommand = {
  kind: ShellCommandKind;
  /** Head of the classified segment, e.g. "rg" or "cargo test". */
  program: string;
  /** Primary path argument for read/list commands. */
  target?: string;
  /** Search pattern for search commands. */
  query?: string;
};

export type ShellStreams = {
  command: string;
  stdout: string;
  stderr: string;
  exitCode: number | null;
  truncated: boolean;
};

export type SearchHit = {
  path: string;
  line: number | null;
  text: string;
};

export type SearchHitGroup = {
  path: string;
  hits: SearchHit[];
};

export type PatchLine = {
  kind: "file" | "hunk" | "added" | "deleted" | "context" | "meta";
  text: string;
  oldLine: number | null;
  newLine: number | null;
};

export type ToolActivityField = {
  label: string;
  value: string;
  mono?: boolean;
};

export type ToolActivityBody =
  | { type: "pending" }
  | { type: "terminal"; streams: ShellStreams }
  | { type: "patch"; lines: PatchLine[]; additions: number; deletions: number }
  | { type: "file"; path: string; text: string; bytes?: number }
  | { type: "entries"; entries: string[]; total?: number }
  | { type: "matches"; groups: SearchHitGroup[]; total: number }
  | { type: "fields"; fields: ToolActivityField[]; text?: string }
  | { type: "text"; text: string };

export type ToolActivityChip = {
  label: string;
  tone?: "neutral" | "success" | "danger" | "warning";
  title?: string;
};

export type ToolActivityView = {
  kind: ToolActivityKind;
  /** More specific icon when one activity kind can contain several resources. */
  iconKind?: ToolActivityIconKind;
  /** Single-line headline, e.g. "读取 prompt_v3.txt". */
  title: string;
  /** Secondary detail shown after the title, e.g. a search path. */
  detail?: string;
  chips: ToolActivityChip[];
  body: ToolActivityBody;
  failed: boolean;
};

export type ToolActivityGroup =
  | "explore"
  | "shell"
  | "edit"
  | "browser"
  | "computer"
  | "spreadsheet"
  | "agent"
  | "plan"
  | "skill"
  | "attachment"
  | "mcp"
  | "tool";

const maxBodyText = 20_000;
const maxEntries = 400;
const maxHits = 400;
const maxPatchLines = 800;

/**
 * The activity kind of a call, decided without touching the result. Grouping in
 * the timeline needs this before any output exists, so it stays separate from
 * the (heavier) body building below.
 */
export function classifyToolCall(call: ToolCall): ToolActivityKind {
  if (call.name === "shell") {
    const input = asRecord(call.input);
    return shellActivityKind(
      parseShellCommand(stringField(input, "command")).kind,
    );
  }

  if (call.name === "filesystem") {
    const operation = stringField(asRecord(call.input), "operation");
    if (operation === "read" || operation === "stat") return "read";
    if (operation === "list") return "list";
    if (operation === "find") return "search";
    return "edit";
  }

  if (call.name === "git_diff") return "diff";
  if (call.name === "read_file") return "read";
  if (call.name === "list_files") return "list";
  if (call.name === "search" || call.name === "workspace_search") {
    return "search";
  }
  if (call.name === "write_file" || call.name === "apply_patch") return "edit";
  if (call.name === "browser") return "browser";
  if (call.name === "computer") return "computer";
  if (call.name === "document" || call.name === "pdf") return "attachment";
  if (call.name === "spreadsheet") return "spreadsheet";
  if (call.name === "view_attachment" || call.name === "read_attachment") {
    return "attachment";
  }
  if (
    [
      "spawn_agent",
      "send_input",
      "send_message",
      "followup_task",
      "interrupt_agent",
      "cancel_agent",
      "wait_agent",
      "wait_agents",
      "list_agents",
    ].includes(call.name)
  ) {
    return "agent";
  }
  if (["set_plan", "update_plan"].includes(call.name)) {
    return "plan";
  }
  if (["list_skills", "read_skill", "create_skill"].includes(call.name)) {
    return "skill";
  }
  if (mcpToolNameParts(call.name)) return "mcp";
  return "tool";
}

/**
 * Consecutive lookups collapse into one "explored" row the way codex folds
 * reads, listings and searches together instead of printing a row each.
 */
export function toolActivityGroup(kind: ToolActivityKind): ToolActivityGroup {
  if (
    kind === "read" ||
    kind === "list" ||
    kind === "search" ||
    kind === "diff"
  ) {
    return "explore";
  }
  return kind;
}

export function buildToolActivity(
  call: ToolCall,
  result?: ToolResult,
): ToolActivityView {
  const view = buildToolActivityView(call, result);
  // A failure the tool did not explain otherwise still has to read as one.
  if (view.failed && !view.chips.some((chip) => chip.tone === "danger")) {
    return {
      ...view,
      chips: [...view.chips, { label: "失败", tone: "danger" }],
    };
  }
  return view;
}

function buildToolActivityView(
  call: ToolCall,
  result?: ToolResult,
): ToolActivityView {
  const kind = classifyToolCall(call);
  const input = asRecord(call.input) ?? {};
  const metadata = asRecord(result?.metadata) ?? {};
  const failed = toolResultFailed(result);
  const rawOutput = stripArtifactMarker(result?.output ?? "");
  const output =
    call.name === "view_attachment" || call.name === "read_attachment"
      ? stripAttachmentBoundary(rawOutput)
      : rawOutput;

  if (call.name === "shell") {
    const streams = parseShellStreams(
      output,
      metadata,
      stringField(input, "command"),
    );
    const parsed = parseShellCommand(streams.command);
    return {
      kind,
      title: shellCommandTitle(parsed, streams.command),
      chips: shellChips(streams, result),
      body: result ? { type: "terminal", streams } : { type: "pending" },
      failed: failed || (streams.exitCode !== null && streams.exitCode !== 0),
    };
  }

  if (call.name === "git_diff") {
    return {
      kind,
      title: "查看 Git 变更",
      chips: [],
      body: result ? patchBody(output) : { type: "pending" },
      failed,
    };
  }

  if (
    call.name === "document" ||
    call.name === "pdf" ||
    call.name === "view_attachment" ||
    call.name === "read_attachment"
  ) {
    const name = stringField(metadata, "name") || stringField(input, "path");
    const contentType =
      stringField(metadata, "contentType") ||
      (call.name === "pdf"
        ? "application/pdf"
        : call.name === "document"
          ? "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
          : "");
    const action = stringField(input, "action");
    const attachment = attachmentPresentation(
      call.name,
      name,
      contentType,
      Boolean(result),
    );
    const chips: ToolActivityChip[] = [];
    if (attachment.format) {
      chips.push({
        label: attachment.format,
        title: contentType || undefined,
      });
    }
    chips.push(...bytesChip(numberField(metadata, "bytes")));
    return {
      kind,
      iconKind: attachment.iconKind,
      title:
        call.name === "pdf" || call.name === "document"
          ? officeActivityTitle(call.name, action, Boolean(result))
          : attachment.title,
      detail: name || undefined,
      chips,
      body: bodyFromFields(input, output, result, ["attachmentId", "action"]),
      failed,
    };
  }

  if (call.name === "apply_patch") {
    const patch = stringField(input, "patch");
    const body = patchBody(patch);
    const targets = patchTargets(patch);
    return {
      kind,
      title:
        targets.length === 1
          ? `修改 ${targets[0]}`
          : targets.length > 1
            ? `修改 ${targets.length} 个文件`
            : "应用补丁",
      detail: targets.length > 1 ? targets.join("、") : undefined,
      chips: [],
      body,
      failed,
    };
  }

  if (call.name === "filesystem") {
    const operation = stringField(input, "operation") || "operation";
    const source = stringField(input, "source");
    const destination = stringField(input, "destination");
    const path =
      stringField(input, "path") ||
      stringField(metadata, "path") ||
      stringField(metadata, "changedPath") ||
      destination ||
      source;
    const title =
      operation === "read"
        ? `读取 ${displayPath(path) || "文件"}`
        : operation === "list"
          ? `列出 ${displayPath(path) || "."}`
          : operation === "find"
            ? `查找 ${stringField(input, "nameContains") || "文件"}`
            : operation === "stat"
              ? `检查 ${displayPath(path) || "路径"}`
              : operation === "write"
                ? `写入 ${displayPath(path) || "文件"}`
                : operation === "copy"
                  ? `复制到 ${displayPath(destination) || "目标文件"}`
                  : operation === "move"
                    ? `移动到 ${displayPath(destination) || "目标文件"}`
                    : operation === "delete"
                      ? `删除 ${displayPath(path) || "文件"}`
                      : "文件操作";
    const content = stringField(input, "content");
    return {
      kind,
      title,
      detail:
        operation === "find" && path ? displayPath(path) : undefined,
      chips:
        numberField(metadata, "bytes") !== undefined
          ? [{ label: formatBytes(numberField(metadata, "bytes") ?? 0) }]
          : numberField(metadata, "count") !== undefined
            ? [{ label: `${numberField(metadata, "count")} 项` }]
            : [],
      body:
        content && operation === "write"
          ? { type: "file", path: displayPath(path), text: clampText(content) }
          : result
            ? operation === "read"
              ? { type: "file", path: displayPath(path), text: clampText(output) }
              : { type: "text", text: output }
            : { type: "pending" },
      failed,
    };
  }

  if (call.name === "write_file") {
    const path =
      stringField(input, "path") || stringField(metadata, "changedPath");
    const content = stringField(input, "content");
    return {
      kind,
      title: `写入 ${displayPath(path) || "文件"}`,
      chips:
        numberField(metadata, "bytes") !== undefined
          ? [{ label: formatBytes(numberField(metadata, "bytes") ?? 0) }]
          : [],
      body: content
        ? { type: "file", path: displayPath(path), text: clampText(content) }
        : result
          ? { type: "text", text: output }
          : { type: "pending" },
      failed,
    };
  }

  if (call.name === "read_file") {
    const path = stringField(metadata, "path") || stringField(input, "path");
    return {
      kind,
      title: `读取 ${displayPath(path) || "文件"}`,
      chips: bytesChip(numberField(metadata, "bytes")),
      body: result
        ? { type: "file", path: displayPath(path), text: clampText(output) }
        : { type: "pending" },
      failed,
    };
  }

  if (call.name === "list_files") {
    const path = stringField(input, "path") || ".";
    const entries = splitLines(output).slice(0, maxEntries);
    const total = numberField(metadata, "count") ?? entries.length;
    return {
      kind,
      title: `列出 ${displayPath(path)}`,
      chips: result ? [{ label: `${total} 项` }] : [],
      body: result ? { type: "entries", entries, total } : { type: "pending" },
      failed,
    };
  }

  if (call.name === "search" || call.name === "workspace_search") {
    const query =
      stringField(metadata, "query") ||
      stringField(input, "query") ||
      stringField(input, "pattern");
    const path = stringField(input, "path") || stringField(metadata, "path");
    const groups = groupSearchHits(parseSearchHits(output));
    const total =
      numberField(metadata, "returnedMatches") ??
      groups.reduce((sum, group) => sum + group.hits.length, 0);
    const chips: ToolActivityChip[] = result
      ? [{ label: `${total} 处匹配` }]
      : [];
    if (metadata.truncated === true) {
      chips.push({ label: "结果已截断", tone: "warning" });
    }
    return {
      kind,
      title: `搜索 ${query || "内容"}`,
      detail: path ? displayPath(path) : undefined,
      chips,
      body: result
        ? groups.length > 0
          ? { type: "matches", groups, total }
          : { type: "text", text: output || "没有匹配结果。" }
        : { type: "pending" },
      failed,
    };
  }

  if (call.name === "browser" || call.name === "computer") {
    const action = stringField(input, "action") || "操作";
    const target =
      stringField(input, "url") ||
      stringField(input, "selector") ||
      stringField(input, "text");
    return {
      kind,
      title: `${call.name === "browser" ? "浏览器" : "计算机"} · ${action}`,
      detail: target ? truncateLine(target, 90) : undefined,
      chips: [],
      body: bodyFromFields(input, output, result, ["action"]),
      failed,
    };
  }

  if (call.name === "spreadsheet") {
    const action = stringField(input, "action") || "操作";
    return {
      kind,
      title: `表格 · ${action}`,
      detail:
        displayPath(
          stringField(input, "path") || stringField(input, "outputPath"),
        ) || undefined,
      chips: [],
      body: bodyFromFields(input, output, result, ["action"]),
      failed,
    };
  }

  if (
    [
      "spawn_agent",
      "send_input",
      "send_message",
      "followup_task",
      "interrupt_agent",
      "cancel_agent",
      "wait_agent",
      "wait_agents",
      "list_agents",
    ].includes(call.name)
  ) {
    return {
      kind,
      title: subagentTitle(call.name, input),
      chips: [],
      body: bodyFromFields(input, output, result),
      failed,
    };
  }

  if (["set_plan", "update_plan"].includes(call.name)) {
    return {
      kind,
      title: "更新执行计划",
      chips: [],
      body: bodyFromFields(input, output, result),
      failed,
    };
  }

  if (["list_skills", "read_skill", "create_skill"].includes(call.name)) {
    return {
      kind,
      title:
        call.name === "list_skills"
          ? "查看可用 Skill"
          : `${call.name === "read_skill" ? "读取" : "创建"} Skill ${stringField(
              input,
              "name",
            )}`.trim(),
      chips: [],
      body: bodyFromFields(input, output, result),
      failed,
    };
  }

  const mcp = mcpToolNameParts(call.name);
  if (mcp) {
    return {
      kind,
      title: `${mcp.server} · ${mcp.tool}`,
      chips: [{ label: "MCP" }],
      body: bodyFromFields(input, output, result),
      failed,
    };
  }

  return {
    kind,
    title: call.name,
    chips: [],
    body: bodyFromFields(input, output, result),
    failed,
  };
}

function officeActivityTitle(
  toolName: "pdf" | "document",
  action: string,
  complete: boolean,
) {
  const subject = toolName === "pdf" ? "PDF" : "Word 文档";
  const labels: Record<string, [string, string]> = {
    inspect: [`检查 ${subject}`, `检查了 ${subject}`],
    extract: [`提取 ${subject}内容`, `提取了 ${subject}内容`],
    render: [`渲染 ${subject}`, `渲染了 ${subject}`],
    validate: [`验证 ${subject}`, `验证了 ${subject}`],
  };
  const [pending, completed] = labels[action] ?? [
    `读取 ${subject}`,
    `读取了 ${subject}`,
  ];
  return complete ? completed : pending;
}

type AttachmentPresentation = {
  iconKind: ToolActivityIconKind;
  title: string;
  format?: string;
};

function attachmentPresentation(
  toolName: string,
  name: string,
  contentType: string,
  complete: boolean,
): AttachmentPresentation {
  const mime = contentType.toLowerCase().split(";", 1)[0].trim();
  const extension = fileExtension(name);
  const format = attachmentFormat(mime, extension);

  if (
    toolName === "view_attachment" ||
    mime.startsWith("image/") ||
    imageExtensions.has(extension)
  ) {
    return {
      iconKind: "image",
      title: complete ? "查看了一张图片" : "查看图片",
      format,
    };
  }

  if (spreadsheetMimes.has(mime) || spreadsheetExtensions.has(extension)) {
    return {
      iconKind: "spreadsheet",
      title: complete ? "读取了一个表格文件" : "读取表格文件",
      format,
    };
  }

  if (archiveMimes.has(mime) || archiveExtensions.has(extension)) {
    return {
      iconKind: "archive",
      title: complete ? "读取了一个压缩文件" : "读取压缩文件",
      format,
    };
  }

  if (dataMimes.has(mime) || dataExtensions.has(extension)) {
    return {
      iconKind: "code",
      title: complete ? "读取了一个数据文件" : "读取数据文件",
      format,
    };
  }

  if (codeExtensions.has(extension)) {
    return {
      iconKind: "code",
      title: complete ? "读取了一个代码文件" : "读取代码文件",
      format,
    };
  }

  if (documentMimes.has(mime) || documentExtensions.has(extension)) {
    return {
      iconKind: "document",
      title:
        format === "PDF"
          ? complete
            ? "读取了一个 PDF 文档"
            : "读取 PDF 文档"
          : complete
            ? "读取了一个文档"
            : "读取文档",
      format,
    };
  }

  if (mime.startsWith("text/") || textExtensions.has(extension)) {
    return {
      iconKind: "document",
      title: complete ? "读取了一个文本文件" : "读取文本文件",
      format,
    };
  }

  return {
    iconKind: "attachment",
    title: complete ? "读取了一个附件" : "读取附件",
    format,
  };
}

const mimeFormatLabels = new Map<string, string>([
  ["image/jpeg", "JPEG"],
  ["image/png", "PNG"],
  ["image/gif", "GIF"],
  ["image/avif", "AVIF"],
  ["image/webp", "WEBP"],
  ["image/svg+xml", "SVG"],
  ["image/bmp", "BMP"],
  ["image/tiff", "TIFF"],
  ["application/pdf", "PDF"],
  ["application/json", "JSON"],
  ["application/xml", "XML"],
  ["application/yaml", "YAML"],
  ["text/yaml", "YAML"],
  ["text/csv", "CSV"],
  ["text/tab-separated-values", "TSV"],
  ["text/markdown", "MD"],
  ["text/plain", "TXT"],
  ["application/zip", "ZIP"],
  ["application/x-7z-compressed", "7Z"],
  ["application/vnd.rar", "RAR"],
  ["application/x-rar-compressed", "RAR"],
  ["application/msword", "DOC"],
  [
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "DOCX",
  ],
  ["application/vnd.ms-excel", "XLS"],
  ["application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", "XLSX"],
]);

const imageExtensions = new Set([
  "png",
  "jpg",
  "jpeg",
  "gif",
  "webp",
  "avif",
  "svg",
  "bmp",
  "tif",
  "tiff",
]);
const spreadsheetExtensions = new Set(["csv", "tsv", "xls", "xlsx", "ods"]);
const spreadsheetMimes = new Set([
  "text/csv",
  "text/tab-separated-values",
  "application/vnd.ms-excel",
  "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
  "application/vnd.oasis.opendocument.spreadsheet",
]);
const archiveExtensions = new Set([
  "zip",
  "7z",
  "rar",
  "tar",
  "gz",
  "bz2",
  "xz",
]);
const archiveMimes = new Set([
  "application/zip",
  "application/x-7z-compressed",
  "application/vnd.rar",
  "application/x-rar-compressed",
  "application/x-tar",
  "application/gzip",
]);
const dataExtensions = new Set(["json", "jsonl", "yaml", "yml", "xml", "toml"]);
const dataMimes = new Set([
  "application/json",
  "application/x-ndjson",
  "application/xml",
  "application/yaml",
  "text/yaml",
]);
const codeExtensions = new Set([
  "c",
  "cc",
  "cpp",
  "cs",
  "css",
  "go",
  "h",
  "hpp",
  "html",
  "java",
  "js",
  "jsx",
  "kt",
  "mjs",
  "php",
  "py",
  "rb",
  "rs",
  "sh",
  "sql",
  "swift",
  "ts",
  "tsx",
  "vue",
]);
const documentExtensions = new Set(["pdf", "doc", "docx", "odt", "rtf"]);
const documentMimes = new Set([
  "application/pdf",
  "application/msword",
  "application/rtf",
  "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
  "application/vnd.oasis.opendocument.text",
]);
const textExtensions = new Set(["txt", "md", "mdx", "log"]);

function fileExtension(name: string) {
  const basename = name.replace(/\\/g, "/").split("/").pop() ?? "";
  const dot = basename.lastIndexOf(".");
  return dot > 0 && dot < basename.length - 1
    ? basename.slice(dot + 1).toLowerCase()
    : "";
}

const knownAttachmentExtensions = new Set([
  ...imageExtensions,
  ...spreadsheetExtensions,
  ...archiveExtensions,
  ...dataExtensions,
  ...codeExtensions,
  ...documentExtensions,
  ...textExtensions,
]);

function attachmentFormat(mime: string, extension: string) {
  return (
    mimeFormatLabels.get(mime) ??
    (knownAttachmentExtensions.has(extension)
      ? extension.toUpperCase()
      : undefined)
  );
}

/**
 * Split a shell command into the semantic action it performs, the way codex
 * turns a command line into a parsed_cmd before rendering it. A `cat foo.txt`
 * is a file read, not a mystery shell invocation, and the row should say so.
 */
export function parseShellCommand(command: string): ParsedShellCommand {
  const segments = commandSegments(command);
  // `cat notes.md | wc -l` counts lines; calling it "read notes.md" would put a
  // label on the row that the command does not deliver. Only a single command
  // gets a semantic label, and the rest keep their command line verbatim.
  if (segments.length !== 1) {
    return {
      kind: "other",
      program: programName(tokenizeCommand(command)[0] ?? ""),
    };
  }
  const segment = segments[0];
  const tokens = tokenizeCommand(segment);
  if (tokens.length === 0) {
    return { kind: "other", program: "" };
  }

  const head = tokens[0];
  const name = programName(head);
  const rest = tokens.slice(1);
  const subcommand = rest.find((token) => !token.startsWith("-")) ?? "";
  const program = subcommand ? `${name} ${subcommand}` : name;

  if (["cat", "bat", "head", "tail", "more", "type", "nl"].includes(name)) {
    return { kind: "read", program: name, target: lastPathArgument(rest) };
  }
  if (["get-content", "gc"].includes(name)) {
    return { kind: "read", program: name, target: lastPathArgument(rest) };
  }
  if (["ls", "dir", "tree", "find", "get-childitem", "gci"].includes(name)) {
    return { kind: "list", program: name, target: lastPathArgument(rest) };
  }
  if (
    ["rg", "grep", "egrep", "fgrep", "ag", "ack", "findstr"].includes(name) ||
    ["select-string", "sls"].includes(name)
  ) {
    const positional = rest.filter((token) => !token.startsWith("-"));
    return {
      kind: "search",
      program: name,
      query: positional[0],
      target: positional[1],
    };
  }
  if (name === "git") {
    return { kind: "git", program };
  }
  if (isTestCommand(name, subcommand)) return { kind: "test", program };
  if (isLintCommand(name, subcommand)) return { kind: "lint", program };
  if (isFormatCommand(name, subcommand)) return { kind: "format", program };
  if (isBuildCommand(name, subcommand)) return { kind: "build", program };
  return { kind: "other", program: name };
}

/**
 * The shell tool reports structured streams in its result metadata. Older
 * events only carry the model-facing envelope ("$ cmd\n\n[stdout]\n…"), so the
 * envelope stays supported as a fallback for history recorded before that.
 */
export function parseShellStreams(
  output: string,
  metadata: Record<string, unknown>,
  fallbackCommand = "",
): ShellStreams {
  const exitCode = numberField(metadata, "exitCode") ?? null;
  const truncated = metadata.truncated === true;
  const metadataCommand = stringField(metadata, "command");

  if (
    typeof metadata.stdout === "string" ||
    typeof metadata.stderr === "string"
  ) {
    return {
      command: metadataCommand || fallbackCommand,
      stdout: clampText(String(metadata.stdout ?? "")),
      stderr: clampText(String(metadata.stderr ?? "")),
      exitCode,
      truncated,
    };
  }

  const envelope = parseShellEnvelope(output);
  return {
    command: envelope.command || metadataCommand || fallbackCommand,
    stdout: clampText(envelope.stdout),
    stderr: clampText(envelope.stderr),
    exitCode,
    truncated,
  };
}

export function parseShellEnvelope(output: string): {
  command: string;
  stdout: string;
  stderr: string;
} {
  const normalized = output.replace(/\r\n/g, "\n");
  const stdoutMarker = "\n\n[stdout]\n";
  const stderrMarker = "\n\n[stderr]\n";
  const stdoutAt = normalized.indexOf(stdoutMarker);
  if (stdoutAt < 0) {
    return { command: "", stdout: normalized, stderr: "" };
  }
  const header = normalized.slice(0, stdoutAt);
  const command = header.startsWith("$ ") ? header.slice(2) : "";
  const body = normalized.slice(stdoutAt + stdoutMarker.length);
  // The tool always appends the stderr section last, so the final marker is the
  // real separator even when stdout itself contains the literal text.
  const stderrAt = body.lastIndexOf(stderrMarker);
  if (stderrAt < 0) {
    return { command, stdout: body, stderr: "" };
  }
  return {
    command,
    stdout: body.slice(0, stderrAt),
    stderr: body.slice(stderrAt + stderrMarker.length),
  };
}

export function parseSearchHits(output: string): SearchHit[] {
  const hits: SearchHit[] = [];
  for (const line of splitLines(output)) {
    // rg --line-number --column: "path:line:column:text" on Windows paths that
    // start with a drive letter, so the drive colon must not be a separator.
    const match = line.match(/^(.+?):(\d+):(?:(\d+):)?(.*)$/);
    if (!match) {
      if (line.trim()) hits.push({ path: "", line: null, text: line });
      continue;
    }
    hits.push({
      path: match[1],
      line: Number(match[2]),
      text: match[4] ?? "",
    });
    if (hits.length >= maxHits) break;
  }
  return hits;
}

export function groupSearchHits(hits: SearchHit[]): SearchHitGroup[] {
  const groups: SearchHitGroup[] = [];
  for (const hit of hits) {
    if (!hit.path) continue;
    const previous = groups[groups.length - 1];
    if (previous?.path === hit.path) {
      previous.hits.push(hit);
    } else {
      groups.push({ path: hit.path, hits: [hit] });
    }
  }
  return groups;
}

/**
 * Parse both the git unified diff and the "*** Update File:" envelope the
 * apply_patch tool accepts, so an edit renders as a diff either way.
 */
export function parsePatchLines(patch: string): PatchLine[] {
  const lines: PatchLine[] = [];
  let oldLine = 0;
  let newLine = 0;
  let inHunk = false;

  for (const text of splitLines(patch)) {
    const custom = text.match(/^\*\*\* (Add|Update|Delete) File:\s*(.+)$/);
    if (custom) {
      lines.push({
        kind: "file",
        text: `${customPatchLabel(custom[1])} ${custom[2].trim()}`,
        oldLine: null,
        newLine: null,
      });
      inHunk = true;
      oldLine = 0;
      newLine = 0;
      continue;
    }
    if (
      text.startsWith("*** End Patch") ||
      text.startsWith("*** Begin Patch")
    ) {
      continue;
    }
    if (text.startsWith("diff --git ")) {
      const path = text.split(/\s+/).pop() ?? "";
      lines.push({
        kind: "file",
        text: cleanDiffPath(path),
        oldLine: null,
        newLine: null,
      });
      inHunk = false;
      continue;
    }
    if (
      text.startsWith("index ") ||
      text.startsWith("--- ") ||
      text.startsWith("+++ ") ||
      text.startsWith("new file mode") ||
      text.startsWith("deleted file mode") ||
      text.startsWith("similarity index") ||
      text.startsWith("rename ")
    ) {
      continue;
    }
    const hunk = text.match(/^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/);
    if (hunk) {
      oldLine = Number(hunk[1]);
      newLine = Number(hunk[2]);
      inHunk = true;
      lines.push({ kind: "hunk", text, oldLine: null, newLine: null });
      continue;
    }
    if (!inHunk) continue;
    if (text.startsWith("+")) {
      lines.push({
        kind: "added",
        text: text.slice(1),
        oldLine: null,
        newLine: newLine || null,
      });
      if (newLine) newLine += 1;
      continue;
    }
    if (text.startsWith("-")) {
      lines.push({
        kind: "deleted",
        text: text.slice(1),
        oldLine: oldLine || null,
        newLine: null,
      });
      if (oldLine) oldLine += 1;
      continue;
    }
    if (text.startsWith("\\")) {
      lines.push({ kind: "meta", text, oldLine: null, newLine: null });
      continue;
    }
    lines.push({
      kind: "context",
      text: text.startsWith(" ") ? text.slice(1) : text,
      oldLine: oldLine || null,
      newLine: newLine || null,
    });
    if (oldLine) oldLine += 1;
    if (newLine) newLine += 1;
  }

  return lines.slice(0, maxPatchLines);
}

export function patchTargets(patch: string): string[] {
  const targets: string[] = [];
  for (const line of splitLines(patch)) {
    const custom = line.match(/^\*\*\* (?:Add|Update|Delete) File:\s*(.+)$/);
    if (custom) {
      targets.push(displayPath(custom[1].trim()));
      continue;
    }
    const git = line.match(/^diff --git\s+\S+\s+(\S+)$/);
    if (git) targets.push(displayPath(cleanDiffPath(git[1])));
  }
  return [...new Set(targets.filter(Boolean))];
}

export function toolResultFailed(result?: ToolResult) {
  if (!result) return false;
  const metadata = asRecord(result.metadata);
  return metadata?.success === false || metadata?.isError === true;
}

export function mcpToolNameParts(
  name: string,
): { server: string; tool: string } | null {
  const separator = name.indexOf("__");
  if (separator <= 0 || separator >= name.length - 2) return null;
  return { server: name.slice(0, separator), tool: name.slice(separator + 2) };
}

export function redactText(value: string) {
  return value
    .replace(/(Bearer\s+)[^\s"'`]+/gi, "$1[已隐藏]")
    .replace(
      /((?:api[_-]?key|token|secret|password|authorization|credential)\s*[:=]\s*)("[^"]*"|'[^']*'|[^\s,;]+)/gi,
      "$1[已隐藏]",
    )
    .replace(/\bsk-[A-Za-z0-9_-]{8,}\b/g, "[已隐藏]");
}

export function truncateText(value: string, limit: number) {
  return value.length <= limit
    ? value
    : `${value.slice(0, limit)}\n\n… 输出已截断，共 ${value.length} 个字符`;
}

export function truncateLine(value: string, limit: number) {
  const line = value.replace(/\s+/g, " ").trim();
  return line.length <= limit ? line : `${line.slice(0, limit - 1)}…`;
}

export function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.ceil(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function displayPath(value: string) {
  return value
    .trim()
    .replace(/\\/g, "/")
    .replace(/^\/\/\?\/UNC\//i, "//")
    .replace(/^\/\/\?\//, "")
    .replace(/^\.\//, "");
}

export function asRecord(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

export function stringField(
  record: Record<string, unknown> | null,
  key: string,
) {
  const value = record?.[key];
  return typeof value === "string" ? value : "";
}

export function numberField(
  record: Record<string, unknown> | null,
  key: string,
) {
  const value = record?.[key];
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string" && /^-?\d+$/.test(value)) return Number(value);
  return undefined;
}

export function sanitizeValue(value: unknown, key = "", depth = 0): unknown {
  if (/api[_-]?key|token|secret|password|authorization|credential/i.test(key)) {
    return "[已隐藏]";
  }
  if (depth > 8) return "[内容层级过深]";
  if (typeof value === "string") return redactText(value);
  if (Array.isArray(value)) {
    return value
      .slice(0, 100)
      .map((item) => sanitizeValue(item, key, depth + 1));
  }
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>).map(
        ([entryKey, item]) => [
          entryKey,
          sanitizeValue(item, entryKey, depth + 1),
        ],
      ),
    );
  }
  return value;
}

function shellActivityKind(kind: ShellCommandKind): ToolActivityKind {
  if (kind === "read") return "read";
  if (kind === "list") return "list";
  if (kind === "search") return "search";
  if (kind === "git") return "diff";
  return "shell";
}

function shellCommandTitle(parsed: ParsedShellCommand, command: string) {
  if (parsed.kind === "read" && parsed.target) {
    return `读取 ${displayPath(parsed.target)}`;
  }
  if (parsed.kind === "list") {
    return `列出 ${parsed.target ? displayPath(parsed.target) : "当前目录"}`;
  }
  if (parsed.kind === "search" && parsed.query) {
    return `搜索 ${truncateLine(parsed.query, 60)}${
      parsed.target ? ` · ${displayPath(parsed.target)}` : ""
    }`;
  }
  return truncateLine(command || "运行命令", 140);
}

function shellChips(streams: ShellStreams, result?: ToolResult) {
  const chips: ToolActivityChip[] = [];
  if (result) {
    const failed =
      streams.exitCode !== null && streams.exitCode !== 0
        ? true
        : toolResultFailed(result);
    chips.push({
      label: failed ? "失败" : "成功",
      tone: failed ? "danger" : "success",
    });
  }
  if (streams.truncated) {
    chips.push({ label: "输出已截断", tone: "warning" });
  }
  return chips;
}

function patchBody(patch: string): ToolActivityBody {
  const lines = parsePatchLines(patch);
  if (lines.length === 0) {
    return { type: "text", text: clampText(patch) || "没有可显示的差异。" };
  }
  return {
    type: "patch",
    lines,
    additions: lines.filter((line) => line.kind === "added").length,
    deletions: lines.filter((line) => line.kind === "deleted").length,
  };
}

function bodyFromFields(
  input: Record<string, unknown>,
  output: string,
  result?: ToolResult,
  skipKeys: string[] = [],
): ToolActivityBody {
  const fields = inputFields(input, skipKeys);
  if (!result) {
    return fields.length > 0 ? { type: "fields", fields } : { type: "pending" };
  }
  const text = clampText(output);
  if (fields.length === 0) {
    return { type: "text", text: text || "工具执行完成，未返回文本输出。" };
  }
  return { type: "fields", fields, text: text || undefined };
}

function inputFields(
  input: Record<string, unknown>,
  skipKeys: string[],
): ToolActivityField[] {
  return Object.entries(input)
    .filter(([key]) => !skipKeys.includes(key))
    .slice(0, 24)
    .map(([key, value]) => {
      const sanitized = sanitizeValue(value, key);
      if (typeof sanitized === "string") {
        return {
          label: key,
          value: truncateText(sanitized, 4_000),
          mono: sanitized.includes("\n") || /path|file|url|cmd/i.test(key),
        };
      }
      return {
        label: key,
        value: truncateText(JSON.stringify(sanitized, null, 2) ?? "", 4_000),
        mono: true,
      };
    });
}

function subagentTitle(name: string, input: Record<string, unknown>) {
  if (name === "spawn_agent") {
    return `创建子智能体 ${stringField(input, "name")}`.trim();
  }
  if (name === "list_agents") return "查看子智能体";
  if (name === "cancel_agent") return "取消子智能体";
  if (name === "interrupt_agent") return "中断子智能体";
  if (name === "wait_agent" || name === "wait_agents")
    return "等待子智能体完成";
  return "向子智能体发送消息";
}

function bytesChip(bytes?: number): ToolActivityChip[] {
  return bytes === undefined ? [] : [{ label: formatBytes(bytes) }];
}

function commandSegments(command: string) {
  return command
    .replace(/\r?\n/g, " ")
    .split(/\s*(?:\|\||&&|[|;])\s*/)
    .map((segment) => segment.trim())
    .filter(Boolean)
    .filter((segment) => {
      // A leading `cd repo && …` says nothing about what the command does.
      const head = programName(tokenizeCommand(segment)[0] ?? "");
      return !["cd", "pushd", "set", "export", "env"].includes(head);
    });
}

function tokenizeCommand(segment: string) {
  const tokens: string[] = [];
  const pattern = /"([^"]*)"|'([^']*)'|(\S+)/g;
  let match: RegExpExecArray | null;
  while ((match = pattern.exec(segment)) !== null) {
    tokens.push(match[1] ?? match[2] ?? match[3] ?? "");
  }
  return tokens;
}

function programName(token: string) {
  const bare = token.split(/[\\/]/).pop() ?? token;
  return bare.replace(/\.(exe|cmd|bat|ps1)$/i, "").toLowerCase();
}

function lastPathArgument(tokens: string[]) {
  for (let index = tokens.length - 1; index >= 0; index -= 1) {
    const token = tokens[index];
    if (token.startsWith("-")) continue;
    if (/^\d+$/.test(token)) continue;
    return token;
  }
  return undefined;
}

function isTestCommand(name: string, subcommand: string) {
  if (["pytest", "jest", "vitest", "mocha", "phpunit"].includes(name)) {
    return true;
  }
  return (
    ["cargo", "go", "dotnet", "npm", "pnpm", "yarn", "bun"].includes(name) &&
    subcommand === "test"
  );
}

function isLintCommand(name: string, subcommand: string) {
  if (["eslint", "ruff", "flake8", "pylint", "stylelint"].includes(name)) {
    return true;
  }
  return name === "cargo" && subcommand === "clippy";
}

function isFormatCommand(name: string, subcommand: string) {
  if (["prettier", "black", "gofmt", "rustfmt"].includes(name)) return true;
  return name === "cargo" && subcommand === "fmt";
}

function isBuildCommand(name: string, subcommand: string) {
  if (["make", "tsc", "vite", "webpack"].includes(name)) return true;
  return (
    ["cargo", "go", "dotnet"].includes(name) &&
    ["build", "check"].includes(subcommand)
  );
}

function customPatchLabel(operation: string) {
  if (operation === "Add") return "新建";
  if (operation === "Delete") return "删除";
  return "修改";
}

function cleanDiffPath(value: string) {
  return value
    .trim()
    .replace(/^"|"$/g, "")
    .replace(/^(?:a|b)[\\/]/, "")
    .replace(/\\/g, "/");
}

function stripArtifactMarker(output: string) {
  return output.replace(/\n\n\[Artifact: [^\]]+\]\s*$/, "");
}

function stripAttachmentBoundary(output: string) {
  return output.replace(/^Attachment content:\s*/i, "");
}

function splitLines(value: string) {
  const lines = value.replace(/\r\n/g, "\n").split("\n");
  if (lines.at(-1) === "") lines.pop();
  return lines;
}

function clampText(value: string) {
  return truncateText(redactText(value), maxBodyText);
}
