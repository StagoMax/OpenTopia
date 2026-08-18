import type { TurnFileChange, TurnFileDiffPreview } from "../../types";

export const defaultVisibleDiffLines = 48;

export type TurnFilePreviewState = {
  key: string;
  loading: boolean;
  loadingMore: boolean;
  error: string | null;
  preview: TurnFileDiffPreview | null;
  visibleLines: number;
};

export type TurnDiffLine = {
  kind: "context" | "added" | "deleted" | "hunk" | "meta";
  text: string;
  oldLine: number | null;
  newLine: number | null;
};

export function parseTurnDiffLines(diff: string): TurnDiffLine[] {
  const sourceLines = diff.replace(/\r\n/g, "\n").split("\n");
  if (sourceLines.at(-1) === "") sourceLines.pop();
  let oldLine = 0;
  let newLine = 0;
  // Everything before the first hunk header is git plumbing ("diff --git",
  // "index ...", "--- /dev/null", "+++ b/..."). It is noise for the reader, so
  // it is dropped instead of rendered. Tracking the first hunk also keeps
  // content lines that happen to start with "---"/"+++" classified correctly.
  let inHunk = false;
  const lines: TurnDiffLine[] = [];

  for (const text of sourceLines) {
    // Re-arm the header skip at each file boundary so a multi-file diff does
    // not leak the second file's plumbing back into the output.
    if (text.startsWith("diff --git ")) {
      inHunk = false;
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
      const line = newLine;
      newLine += 1;
      lines.push({ kind: "added", text, oldLine: null, newLine: line });
      continue;
    }
    if (text.startsWith("-")) {
      const line = oldLine;
      oldLine += 1;
      lines.push({ kind: "deleted", text, oldLine: line, newLine: null });
      continue;
    }
    if (text.startsWith(" ")) {
      lines.push({ kind: "context", text, oldLine, newLine });
      oldLine += 1;
      newLine += 1;
      continue;
    }
    // "\ No newline at end of file" and friends.
    lines.push({ kind: "meta", text, oldLine: null, newLine: null });
  }

  return lines;
}

export function turnChangeFileKey(file: TurnFileChange) {
  return `${file.kind}:${file.oldPath ?? ""}:${file.newPath ?? ""}`;
}

export function turnChangeFileRequestPath(file: TurnFileChange) {
  return file.newPath ?? file.oldPath ?? null;
}

export function turnFilePreviewError(error: unknown) {
  const message = error instanceof Error ? error.message : String(error);
  try {
    const parsed = JSON.parse(message) as { error?: string };
    return parsed.error || message;
  } catch {
    return message || "代码差异加载失败，请重试。";
  }
}

export function utf8ByteLength(value: string) {
  return new TextEncoder().encode(value).length;
}

export function formatPreviewBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.ceil(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function turnChangeFilePath(file: TurnFileChange) {
  if (file.kind === "renamed" && file.oldPath && file.newPath) {
    return `${file.oldPath} → ${file.newPath}`;
  }
  return file.newPath ?? file.oldPath ?? "未知文件";
}

export function turnChangeKind(kind: TurnFileChange["kind"]) {
  if (kind === "added") return { code: "A", label: "新增" };
  if (kind === "deleted") return { code: "D", label: "删除" };
  if (kind === "renamed") return { code: "R", label: "重命名" };
  return { code: "M", label: "修改" };
}
