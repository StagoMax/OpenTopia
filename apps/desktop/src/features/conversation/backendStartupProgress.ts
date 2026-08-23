import type { BackendStartupStatus } from "../../types";

export function backendStartupLabel(
  status: BackendStartupStatus | null,
  isProbing: boolean,
): string {
  switch (status?.phase) {
    case "compiling":
      return status.detail ? `正在编译 ${status.detail}` : "正在编译本地服务";
    case "starting":
      return "正在启动本地服务";
    case "waiting_for_health":
      return "正在等待本地服务响应";
    case "ready":
      return "本地服务已连接，正在加载工作区";
    case "failed":
      return status.detail
        ? `本地服务启动未完成：${status.detail}`
        : "本地服务启动未完成，正在继续重试";
    case "checking":
      return "正在检查本地服务";
    default:
      return isProbing ? "正在尝试连接本地服务" : "正在等待本地服务响应";
  }
}

export function formatBackendStartupElapsed(
  startedAt: string | null | undefined,
  now = Date.now(),
): string {
  const startedAtMs = startedAt ? Date.parse(startedAt) : Number.NaN;
  const elapsedSeconds = Math.max(
    0,
    Math.floor((now - (Number.isNaN(startedAtMs) ? now : startedAtMs)) / 1000),
  );
  const seconds = elapsedSeconds % 60;
  const minutes = Math.floor(elapsedSeconds / 60) % 60;
  const hours = Math.floor(elapsedSeconds / 3600);

  if (hours > 0) return `已等待 ${hours} 小时 ${minutes} 分`;
  if (minutes > 0) return `已等待 ${minutes} 分 ${seconds} 秒`;
  return `已等待 ${seconds} 秒`;
}
