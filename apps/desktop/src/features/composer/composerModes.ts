import {
  Hand,
  Lightbulb,
  ShieldAlert,
  ShieldCheck,
  Target,
  Zap,
} from "lucide-react";
import type { AppSettings, CollaborationMode } from "../../types";
import type { ExecutionPermissionMode } from "./composerTypes";

export const collaborationModeOptions: Array<{
  value: CollaborationMode;
  label: string;
  detail: string;
  icon: typeof Zap;
}> = [
  {
    value: "plan",
    label: "计划模式",
    detail: "开启计划模式",
    icon: Lightbulb,
  },
  {
    value: "goal",
    label: "目标",
    detail: "设置要持续追求的目标",
    icon: Target,
  },
];

export function collaborationModePlaceholder(mode: CollaborationMode): string {
  if (mode === "plan") return "描述任务；需要选择时会列出方案";
  if (mode === "goal") return "描述要持续推进的目标";
  return "请求后续更改";
}

export const permissionModeOptions: Array<{
  value: ExecutionPermissionMode;
  label: string;
  detail: string;
  icon: typeof Hand;
  appearance: "default" | "auto" | "full-access";
}> = [
  {
    value: "approve",
    label: "请求批准",
    detail: "编辑外部文件和使用互联网时始终询问",
    icon: Hand,
    appearance: "default",
  },
  {
    value: "auto",
    label: "替我审批",
    detail: "仅对检测到的风险操作请求批准",
    icon: ShieldCheck,
    appearance: "auto",
  },
  {
    value: "unrestricted",
    label: "完整系统访问",
    detail: "无系统沙箱，并跳过所有工具审批",
    icon: ShieldAlert,
    appearance: "full-access",
  },
];

export const sandboxModeOptions: Array<{
  value: AppSettings["sandbox"]["sandboxMode"];
  label: string;
  detail: string;
}> = [
  { value: "read-only", label: "只读沙箱", detail: "禁止写入" },
  { value: "workspace-write", label: "工作区写入", detail: "默认" },
  { value: "danger-full-access", label: "完全访问", detail: "无 OS 沙箱" },
];

export function sandboxModeLabel(
  mode: AppSettings["sandbox"]["sandboxMode"],
): string {
  return (
    sandboxModeOptions.find((option) => option.value === mode)?.label ?? mode
  );
}

export function normalizedPermissionMode(
  mode: AppSettings["permissionMode"],
): ExecutionPermissionMode {
  if (mode === "approve") return "approve";
  if (mode === "full_access" || mode === "unrestricted") {
    return "unrestricted";
  }
  return "auto";
}
