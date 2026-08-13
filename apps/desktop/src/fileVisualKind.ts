export type FileVisualKind =
  | "pdf"
  | "word"
  | "spreadsheet"
  | "presentation"
  | "image"
  | "audio"
  | "video"
  | "archive"
  | "data"
  | "code"
  | "text"
  | "generic";

const extensionsByKind: ReadonlyArray<
  readonly [Exclude<FileVisualKind, "generic">, ReadonlySet<string>]
> = [
  ["pdf", new Set(["pdf"])],
  ["word", new Set(["doc", "docx", "odt", "rtf"])],
  [
    "spreadsheet",
    new Set(["csv", "tsv", "xls", "xlsx", "xlsm", "xlsb", "ods"]),
  ],
  [
    "presentation",
    new Set(["ppt", "pptx", "pps", "ppsx", "pot", "potx", "odp"]),
  ],
  [
    "image",
    new Set([
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
      "heic",
    ]),
  ],
  ["audio", new Set(["mp3", "wav", "ogg", "m4a", "aac", "flac", "opus"])],
  ["video", new Set(["mp4", "webm", "mov", "avi", "mkv", "m4v", "mpeg"])],
  ["archive", new Set(["zip", "7z", "rar", "tar", "gz", "bz2", "xz", "zst"])],
  ["data", new Set(["json", "jsonc", "jsonl", "xml", "yaml", "yml"])],
  [
    "code",
    new Set([
      "rs",
      "ts",
      "tsx",
      "js",
      "jsx",
      "mjs",
      "cjs",
      "c",
      "h",
      "cc",
      "cpp",
      "hpp",
      "cs",
      "py",
      "go",
      "java",
      "kt",
      "swift",
      "rb",
      "php",
      "sh",
      "ps1",
      "bat",
      "cmd",
      "sql",
      "graphql",
      "gql",
      "proto",
      "diff",
      "patch",
      "html",
      "htm",
      "css",
      "scss",
      "less",
      "toml",
      "vue",
      "svelte",
    ]),
  ],
  [
    "text",
    new Set(["md", "mdx", "txt", "log", "ini", "conf", "config", "properties"]),
  ],
];

const mimeKinds = new Map<string, FileVisualKind>([
  ["application/pdf", "pdf"],
  ["application/msword", "word"],
  [
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "word",
  ],
  ["application/vnd.oasis.opendocument.text", "word"],
  ["application/rtf", "word"],
  ["text/rtf", "word"],
  ["text/csv", "spreadsheet"],
  ["text/tab-separated-values", "spreadsheet"],
  ["application/vnd.ms-excel", "spreadsheet"],
  [
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    "spreadsheet",
  ],
  ["application/vnd.oasis.opendocument.spreadsheet", "spreadsheet"],
  ["application/vnd.ms-powerpoint", "presentation"],
  [
    "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    "presentation",
  ],
  ["application/vnd.oasis.opendocument.presentation", "presentation"],
  ["application/zip", "archive"],
  ["application/x-7z-compressed", "archive"],
  ["application/vnd.rar", "archive"],
  ["application/x-rar-compressed", "archive"],
  ["application/x-tar", "archive"],
  ["application/gzip", "archive"],
  ["application/x-bzip2", "archive"],
  ["application/x-xz", "archive"],
  ["application/zstd", "archive"],
  ["application/json", "data"],
  ["application/x-ndjson", "data"],
  ["application/xml", "data"],
  ["application/yaml", "data"],
  ["text/yaml", "data"],
  ["application/javascript", "code"],
  ["application/typescript", "code"],
  ["application/wasm", "code"],
]);

/**
 * Resolve a stable visual category from a filename/extension and optional MIME
 * type. A known extension wins because desktop attachment providers often use
 * the generic `application/octet-stream` MIME type.
 */
export function fileVisualKind(
  nameOrExtension: string,
  contentType = "",
): FileVisualKind {
  const extension = normalizedExtension(nameOrExtension);
  for (const [kind, extensions] of extensionsByKind) {
    if (extensions.has(extension)) return kind;
  }

  const mime = contentType.toLocaleLowerCase().split(";", 1)[0]?.trim() ?? "";
  const exactKind = mimeKinds.get(mime);
  if (exactKind) return exactKind;
  if (mime.startsWith("image/")) return "image";
  if (mime.startsWith("audio/")) return "audio";
  if (mime.startsWith("video/")) return "video";
  if (mime.startsWith("text/")) return "text";
  return "generic";
}

function normalizedExtension(nameOrExtension: string): string {
  const withoutSuffix = nameOrExtension.trim().split(/[?#]/, 1)[0] ?? "";
  const basename = withoutSuffix.replace(/\\/g, "/").split("/").pop() ?? "";
  const normalized = basename.toLocaleLowerCase();
  const dot = normalized.lastIndexOf(".");
  if (dot >= 0 && dot < normalized.length - 1) return normalized.slice(dot + 1);
  return /^[a-z0-9]{1,12}$/.test(normalized) ? normalized : "";
}
