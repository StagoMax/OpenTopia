import type {
  GitBranchInfo,
  GitStatusSummary,
  GitWorkflowResponse,
  PreviewDescriptor,
  PreviewTarget,
} from "../../types";
import { isSpreadsheetFileExtension } from "../../spreadsheetFormats.ts";

export type PreviewDescriptorResponse = {
  id: string;
  source: "workspace" | "local" | "artifact" | "attachment";
  path?: string | null;
  name: string;
  kind: "text" | "image" | "pdf" | "document" | "spreadsheet" | "unsupported";
  contentType: string;
  bytes: number;
  readonly: boolean;
  capabilities?: PreviewDescriptor["capabilities"];
  revision: string;
  handlerId?: string | null;
};

export type SpreadsheetWorkbookResponse = {
  previewId: string;
  sheets: Array<{
    name: string;
    kind: string;
    visibility: "visible" | "hidden" | "very_hidden";
    rowCount: number;
    columnCount: number;
  }>;
};

export type SpreadsheetRangeResponse = {
  previewId: string;
  sheet: string;
  range: {
    start: { row: number; column: number };
    end: { row: number; column: number };
  };
  rows: Array<
    Array<{
      value: { type: string; value?: unknown };
      formula?: string | null;
    }>
  >;
};

export function mapPreviewDescriptor(
  response: PreviewDescriptorResponse,
  threadId: string,
  target: PreviewTarget,
): PreviewDescriptor {
  const capabilities = response.capabilities ?? {
    read: true,
    write: !response.readonly,
    watch: false,
    rangeRead: response.kind === "spreadsheet",
    openExternal: Boolean(response.path),
  };
  return {
    id: response.id,
    threadId,
    target,
    renderer:
      response.kind === "text"
        ? previewRenderer(response.name, response.contentType)
        : response.kind,
    title: response.name,
    contentType: response.contentType,
    bytes: response.bytes,
    revision: response.revision,
    readonly: response.readonly,
    capabilities,
    handlerId: response.handlerId,
    externalPath:
      response.source === "local" ||
      response.source === "artifact" ||
      response.source === "attachment"
        ? response.path
        : undefined,
  };
}

function previewRenderer(
  path: string,
  contentType: string,
): PreviewDescriptor["renderer"] {
  const extension = path.split(".").at(-1)?.toLocaleLowerCase() ?? "";
  const mediaType = contentType.split(";", 1)[0]?.trim().toLocaleLowerCase();
  if (mediaType.startsWith("image/")) return "image";
  if (mediaType === "application/pdf" || extension === "pdf") return "pdf";
  if (
    mediaType ===
      "application/vnd.openxmlformats-officedocument.wordprocessingml.document" ||
    extension === "docx"
  )
    return "document";
  if (
    isSpreadsheetFileExtension(extension) ||
    ["text/csv", "text/tab-separated-values"].includes(mediaType)
  )
    return "spreadsheet";
  if (
    [
      "c",
      "cc",
      "cpp",
      "css",
      "go",
      "h",
      "html",
      "java",
      "js",
      "jsx",
      "json",
      "md",
      "py",
      "rs",
      "sh",
      "toml",
      "ts",
      "tsx",
      "xml",
      "yaml",
      "yml",
    ].includes(extension)
  ) {
    return "code";
  }
  if (mediaType.startsWith("text/")) return "text";
  return "unsupported";
}

export function spreadsheetCellValue(value: {
  type: string;
  value?: unknown;
}): string | number | boolean | null {
  if (value.type === "empty") return null;
  if (
    typeof value.value === "string" ||
    typeof value.value === "number" ||
    typeof value.value === "boolean"
  ) {
    return value.value;
  }
  if (value.value && typeof value.value === "object") {
    const serial = (value.value as { serial?: unknown }).serial;
    if (typeof serial === "number") return serial;
  }
  return value.value == null ? null : String(value.value);
}

export function parseGitStatus(output: string): GitStatusSummary {
  let branch: string | null = null;
  let upstream: string | null = null;
  let detached = false;
  let ahead = 0;
  let behind = 0;
  let changed = 0;
  let staged = 0;
  let unstaged = 0;
  let untracked = 0;

  for (const line of output.split(/\r?\n/)) {
    if (line.startsWith("# branch.head ")) {
      const value = line.slice("# branch.head ".length).trim();
      detached = value === "(detached)" || value === "(unknown)";
      branch = detached || !value ? null : value;
      continue;
    }
    if (line.startsWith("# branch.upstream ")) {
      upstream = line.slice("# branch.upstream ".length).trim() || null;
      continue;
    }
    if (line.startsWith("# branch.ab ")) {
      const match = line.match(/^# branch\.ab \+(\d+) -(\d+)$/);
      if (match) {
        ahead = Number(match[1]);
        behind = Number(match[2]);
      }
      continue;
    }
    if (line.startsWith("? ")) {
      changed += 1;
      untracked += 1;
      continue;
    }
    if (!/^[12u] /.test(line)) continue;
    const xy = line.slice(2, 4);
    if (xy.length !== 2) continue;
    changed += 1;
    if (xy[0] !== ".") staged += 1;
    if (xy[1] !== ".") unstaged += 1;
  }

  return {
    branch,
    upstream,
    detached,
    ahead,
    behind,
    changed,
    staged,
    unstaged,
    untracked,
    raw: output,
  };
}

export function parseGitBranches(output: string): GitBranchInfo[] {
  const branches: GitBranchInfo[] = [];
  for (const [index, rawLine] of output.split(/\r?\n/).entries()) {
    if (!rawLine) continue;
    const fields = rawLine.split("\0");
    if (fields.length !== 5 || !fields[0] || !fields[1]) {
      throw new Error(`无法解析第 ${index + 1} 条 Git 分支记录`);
    }
    branches.push({
      fullRef: fields[0],
      name: fields[1],
      current: fields[2] === "*",
      remote: fields[0].startsWith("refs/remotes/"),
      upstream: fields[3] || null,
      symbolicTarget: fields[4] || null,
    });
  }
  return branches.sort((left, right) => {
    if (left.current !== right.current) return left.current ? -1 : 1;
    if (left.remote !== right.remote) return left.remote ? 1 : -1;
    return left.name.localeCompare(right.name);
  });
}

export function gitFailureMessage(result: GitWorkflowResponse): string {
  const detail = result.stderr.trim() || result.stdout.trim();
  return (
    detail || `Git ${result.action} 执行${result.success ? "成功" : "失败"}`
  );
}
