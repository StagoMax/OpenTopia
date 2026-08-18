export function workspaceName(workspaceRoot: string): string {
  const trimmed = workspaceRoot.replace(/[\\/]+$/, "");
  const parts = trimmed.split(/[\\/]/).filter(Boolean);
  return parts.at(-1) || workspaceRoot;
}
