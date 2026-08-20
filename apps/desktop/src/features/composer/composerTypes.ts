export type ExecutionPermissionMode = "auto" | "approve" | "unrestricted";
export type NewTaskLaunchMode = "local" | "new_worktree";
export type ComposerOpenMenu =
  "actions" | "permission" | "model" | "workspace" | "environment" | null;

export function newTaskLaunchModeLabel(mode: NewTaskLaunchMode): string {
  return mode === "new_worktree" ? "新工作树" : "在本地处理";
}
