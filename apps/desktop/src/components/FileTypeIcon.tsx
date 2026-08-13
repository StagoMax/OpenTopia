import {
  File,
  FileArchive,
  FileCode2,
  FileImage,
  FileJson,
  FileMusic,
  FileSpreadsheet,
  FileText,
  FileType,
  FileVideoCamera,
  NotebookText,
  Presentation,
  type LucideIcon,
} from "lucide-react";
import type { CSSProperties } from "react";
import { fileVisualKind, type FileVisualKind } from "../fileVisualKind";
import "./FileTypeIcon.css";

const iconsByKind: Record<FileVisualKind, LucideIcon> = {
  pdf: FileText,
  word: FileType,
  spreadsheet: FileSpreadsheet,
  presentation: Presentation,
  image: FileImage,
  audio: FileMusic,
  video: FileVideoCamera,
  archive: FileArchive,
  data: FileJson,
  code: FileCode2,
  text: NotebookText,
  generic: File,
};

const labelsByKind: Record<FileVisualKind, string> = {
  pdf: "PDF",
  word: "DOC",
  spreadsheet: "XLS",
  presentation: "PPT",
  image: "IMG",
  audio: "AUD",
  video: "VID",
  archive: "ZIP",
  data: "DAT",
  code: "DEV",
  text: "TXT",
  generic: "FIL",
};

export function fileTypeIcon(
  nameOrExtension: string,
  contentType = "",
): LucideIcon {
  return iconsByKind[fileVisualKind(nameOrExtension, contentType)];
}

export function FileTypeIcon({
  name,
  contentType = "",
  size = 14,
  className = "",
}: {
  name: string;
  contentType?: string;
  size?: number;
  className?: string;
}) {
  const kind = fileVisualKind(name, contentType);
  return (
    <span
      className={`file-type-icon ${className}`.trim()}
      data-file-kind={kind}
      style={{ "--file-type-icon-size": `${size}px` } as CSSProperties}
      aria-hidden="true"
    >
      <File className="file-type-icon__outline" />
      <span className="file-type-icon__mark">{labelsByKind[kind]}</span>
    </span>
  );
}
