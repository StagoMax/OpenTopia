export function splitWorkspacePath(path: string): string[] {
  if (!path || path === ".") return [];
  return path.split(/[\\/]/).filter(Boolean);
}

export function toWorkspaceAbsolutePath(
  workspaceRoot: string,
  targetPath: string,
): string {
  if (!targetPath) return workspaceRoot;
  if (/^[a-zA-Z]:[\\/]/.test(targetPath) || targetPath.startsWith("\\\\")) {
    return targetPath;
  }
  const separator = workspaceRoot.includes("\\") ? "\\" : "/";
  const root = workspaceRoot.replace(/[\\/]+$/, "");
  const child = targetPath.replace(/^[\\/]+/, "").replace(/[\\/]+/g, separator);
  return child ? `${root}${separator}${child}` : root;
}

export function formatBytes(value?: number | null): string {
  if (value === undefined || value === null) return "";
  if (value < 1024) return `${value} B`;
  const units = ["KB", "MB", "GB"];
  let amount = value / 1024;
  let unitIndex = 0;
  while (amount >= 1024 && unitIndex < units.length - 1) {
    amount /= 1024;
    unitIndex += 1;
  }
  return `${amount.toFixed(amount >= 10 ? 0 : 1)} ${units[unitIndex]}`;
}

export function formatNumber(value: number): string {
  return new Intl.NumberFormat().format(value);
}

export function formatTime(value: string): string {
  return new Date(value).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}
