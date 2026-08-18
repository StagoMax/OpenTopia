import type { WorkbenchTab } from "./components/WorkbenchPanel";
import {
  Activity,
  Box,
  CirclePlus,
  FileCode2,
  FileImage,
  Folder,
  GitBranch,
  GitFork,
  Globe2,
  Monitor,
  Plug,
  TerminalSquare,
} from "lucide-react";
import type { ImagePreviewSource } from "./components/PreviewHost";
import type { BrowserNavigationRequest, PreviewTarget } from "./types";

export type ToolTabKind =
  | WorkbenchTab
  | "flow"
  | "browser"
  | "computer"
  | "image"
  | "preview"
  | "side-task"
  | "usage";

export type ToolTab = {
  id: string;
  kind: ToolTabKind;
  title: string;
  imagePreview?: ImagePreviewSource;
  sideTaskThreadId?: string;
  previewTarget?: PreviewTarget;
  browserNavigation?: BrowserNavigationRequest;
};

export const toolTabMenuItems: Array<{
  kind: "flow" | "terminal" | "browser" | "computer" | "files";
  shortcut: string | null;
}> = [
  { kind: "flow", shortcut: null },
  { kind: "terminal", shortcut: null },
  { kind: "browser", shortcut: "Ctrl+T" },
  { kind: "computer", shortcut: null },
  { kind: "files", shortcut: "Ctrl+P" },
];

export const toolStageLauncherKinds: Array<{
  kind: Exclude<ToolTabKind, "image" | "preview" | "side-task">;
  label: string;
}> = [
  { kind: "flow", label: "Flow" },
  { kind: "diff", label: "代码审阅" },
  { kind: "terminal", label: "终端" },
  { kind: "browser", label: "浏览器" },
  { kind: "computer", label: "桌面观察" },
  { kind: "files", label: "文件" },
];

export function toolTabTitle(kind: ToolTabKind): string {
  switch (kind) {
    case "flow":
      return "Flow";
    case "files":
      return "文件";
    case "terminal":
      return "终端";
    case "diff":
      return "审查";
    case "extensions":
      return "Plugins";
    case "sandbox":
      return "沙箱";
    case "browser":
      return "浏览器";
    case "computer":
      return "电脑";
    case "usage":
      return "使用日志";
    case "side-task":
      return "侧边任务";
    case "image":
      return "图片";
    case "preview":
      return "预览";
  }
}

export function toolTabIcon(kind: ToolTabKind): typeof Folder {
  switch (kind) {
    case "flow":
      return GitFork;
    case "files":
      return Folder;
    case "terminal":
      return TerminalSquare;
    case "diff":
      return GitBranch;
    case "extensions":
      return Plug;
    case "sandbox":
      return Box;
    case "browser":
      return Globe2;
    case "computer":
      return Monitor;
    case "usage":
      return Activity;
    case "side-task":
      return CirclePlus;
    case "image":
      return FileImage;
    case "preview":
      return FileCode2;
  }
}
